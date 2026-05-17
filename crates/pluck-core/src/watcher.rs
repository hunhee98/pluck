//! File-watcher that keeps the index in lock-step with the source tree.
//!
//! Wraps the `notify` crate. Coalesces rapid events (editor save bursts
//! emit several modify events per file) inside a small debounce window,
//! then fires one `reindex_paths` call for the deduplicated batch. Drops
//! events for non-source files via extension match (delegated to
//! `Language::from_extension`).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{recommended_watcher, EventKind, RecursiveMode, Watcher as NotifyWatcher};

use crate::chunker::Language;
use crate::index::PluckIndex;
use crate::indexer::reindex_paths;

/// Default debounce — long enough that a typical editor save (which fires
/// 1–3 events in rapid succession) coalesces into one reindex, short
/// enough that the agent sees fresh content within a turn.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(150);

/// Handle to a running watcher. Drop to stop watching.
pub struct WatcherHandle {
    _watcher: notify::RecommendedWatcher,
    _task: tokio::task::JoinHandle<()>,
}

/// Spawn a watcher rooted at `repo_root`. Every debounced batch of file
/// changes is reindexed against `index`. Returns a handle whose `Drop`
/// stops the underlying notify watcher and the tokio task.
pub fn spawn_watcher(
    repo_root: PathBuf,
    index: Arc<PluckIndex>,
    debounce: Duration,
) -> Result<WatcherHandle> {
    // notify reports events with canonicalized paths (on macOS, `/var/...`
    // becomes `/private/var/...`). Canonicalize the root so our later
    // `strip_prefix` produces the same rel-path that the indexer used.
    let repo_root = std::fs::canonicalize(&repo_root)
        .with_context(|| format!("canonicalize watcher root {repo_root:?}"))?;

    // Bridge notify's sync callback into a tokio mpsc channel so the
    // reindex work can live in async land.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Event>();
    let mut watcher = recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })
    .context("create notify watcher")?;
    watcher
        .watch(&repo_root, RecursiveMode::Recursive)
        .context("notify watch")?;

    let task = tokio::spawn(async move {
        let mut pending: HashSet<PathBuf> = HashSet::new();
        let mut deadline: Option<Instant> = None;

        loop {
            let sleep_until = deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(60 * 60)); // ~idle wait

            tokio::select! {
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    if !is_source_event(&event) {
                        continue;
                    }
                    for path in keep_source_paths(event.paths) {
                        pending.insert(path);
                    }
                    deadline = Some(Instant::now() + debounce);
                }
                _ = tokio::time::sleep(sleep_until) => {
                    if pending.is_empty() {
                        deadline = None;
                        continue;
                    }
                    let batch: Vec<PathBuf> = pending.drain().collect();
                    deadline = None;
                    let idx = Arc::clone(&index);
                    let repo = repo_root.clone();
                    // Reindex on a blocking pool — tantivy commits do disk I/O.
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Err(e) = reindex_paths(&idx, &repo, &batch) {
                            tracing::warn!("reindex batch failed: {e}");
                        } else {
                            tracing::info!(count = batch.len(), "reindexed batch");
                        }
                    }).await;
                }
            }
        }
    });

    Ok(WatcherHandle {
        _watcher: watcher,
        _task: task,
    })
}

fn is_source_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn keep_source_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|p| Language::from_path(p).is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::PluckIndex;

    fn temp_repo() -> PathBuf {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("pluck-watcher-{nano}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(p: &std::path::Path, s: &str) {
        std::fs::write(p, s).unwrap();
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    async fn wait_until<F: FnMut() -> bool>(mut cond: F, deadline_ms: u64) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(deadline_ms) {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        cond()
    }

    #[tokio::test]
    async fn watcher_reindexes_on_modify() {
        let repo = temp_repo();
        let file = repo.join("a.ts");
        write(&file, "function alpha() { return 1; }\n");

        let idx = Arc::new(PluckIndex::in_ram().unwrap());
        // Initial seed so the watcher only has to handle the change.
        let mut w = idx.writer().unwrap();
        let chunks = crate::chunker::chunk_source(
            &std::fs::read_to_string(&file).unwrap(),
            Language::TypeScript,
        )
        .unwrap();
        for c in &chunks {
            w.add_chunk("a.ts", c).unwrap();
        }
        w.commit().unwrap();

        let _watcher =
            spawn_watcher(repo.clone(), Arc::clone(&idx), DEFAULT_DEBOUNCE).expect("spawn watcher");

        // Brief grace so notify has registered the watch before we
        // generate the change event.
        tokio::time::sleep(Duration::from_millis(100)).await;

        write(
            &file,
            "function alpha() { return 2; }\nfunction beta() {}\n",
        );

        let saw_beta = wait_until(
            || {
                idx.search("beta", 5)
                    .map(|h| h.iter().any(|x| x.symbol == "beta"))
                    .unwrap_or(false)
            },
            8000,
        )
        .await;
        assert!(saw_beta, "watcher should have surfaced `beta` after modify");

        cleanup(&repo);
    }

    #[tokio::test]
    async fn watcher_drops_chunks_on_delete() {
        let repo = temp_repo();
        let file = repo.join("doomed.ts");
        write(&file, "function doomed() { return 1; }\n");

        let idx = Arc::new(PluckIndex::in_ram().unwrap());
        let mut w = idx.writer().unwrap();
        let chunks =
            crate::chunker::chunk_source("function doomed() { return 1; }\n", Language::TypeScript)
                .unwrap();
        for c in &chunks {
            w.add_chunk("doomed.ts", c).unwrap();
        }
        w.commit().unwrap();
        assert!(idx
            .search("doomed", 5)
            .unwrap()
            .iter()
            .any(|h| h.symbol == "doomed"));

        let _watcher =
            spawn_watcher(repo.clone(), Arc::clone(&idx), DEFAULT_DEBOUNCE).expect("spawn watcher");
        tokio::time::sleep(Duration::from_millis(100)).await;

        std::fs::remove_file(&file).unwrap();

        let gone = wait_until(
            || {
                idx.search("doomed", 5)
                    .map(|h| h.iter().all(|x| x.symbol != "doomed"))
                    .unwrap_or(false)
            },
            8000,
        )
        .await;
        assert!(
            gone,
            "watcher should have dropped chunks for the deleted file"
        );

        cleanup(&repo);
    }
}
