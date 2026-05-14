//! Repo indexer: walks a tree, chunks every supported file, and pushes
//! the chunks into a `PluckIndex`. Respects `.gitignore` via the `ignore`
//! crate; skips files we know we can't process (too large, non-UTF8, no
//! known extension).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;

use crate::chunker::{chunk_source, Language};
use crate::index::PluckIndex;

/// Files larger than this are skipped — they're usually generated assets,
/// minified bundles, or vendored data, not authored source.
pub const MAX_FILE_BYTES: u64 = 1_000_000;

#[derive(Debug, Default, Clone)]
pub struct IndexStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_skipped_size: usize,
    pub files_skipped_lang: usize,
    pub files_skipped_read: usize,
    pub chunks_indexed: usize,
}

/// Walk `repo_root`, chunk every supported file, and push each chunk into
/// `index`. `index.commit()` is *not* called here — caller decides when to
/// flush.
pub fn index_repo(index: &PluckIndex, repo_root: &Path) -> Result<IndexStats> {
    let mut writer = index.writer().context("open writer")?;
    let stats = index_repo_into(&mut writer, repo_root)?;
    writer.commit().context("commit writer")?;
    Ok(stats)
}

/// Same as `index_repo` but lets the caller batch multiple roots or sources
/// into a single writer/commit.
pub fn index_repo_into(
    writer: &mut crate::index::IndexBatch,
    repo_root: &Path,
) -> Result<IndexStats> {
    let mut stats = IndexStats::default();

    let walker = WalkBuilder::new(repo_root)
        .standard_filters(true) // .gitignore (in git repos), .ignore, hidden
        // Honor .gitignore even when the tree isn't a git repo (e.g. a
        // tarball checkout). Without this, the `ignore` crate's
        // `git_ignore` filter is skipped outside of a git working tree.
        .require_git(false)
        .build();

    for entry in walker {
        let Ok(entry) = entry else { continue };
        let Some(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        stats.files_seen += 1;

        let path = entry.path();

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some(lang) = Language::from_extension(ext) else {
            stats.files_skipped_lang += 1;
            continue;
        };

        let md = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                stats.files_skipped_read += 1;
                continue;
            }
        };
        if md.len() > MAX_FILE_BYTES {
            stats.files_skipped_size += 1;
            continue;
        }

        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                stats.files_skipped_read += 1;
                continue;
            }
        };

        let chunks = match chunk_source(&src, lang) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("chunk failed for {}: {e}", path.display());
                stats.files_skipped_read += 1;
                continue;
            }
        };

        let rel = path
            .strip_prefix(repo_root)
            .unwrap_or(path)
            .to_string_lossy();

        for c in &chunks {
            writer
                .add_chunk(rel.as_ref(), c)
                .with_context(|| format!("add_chunk failed for {}", path.display()))?;
            stats.chunks_indexed += 1;
        }
        stats.files_indexed += 1;
    }

    Ok(stats)
}

/// Convenience: index a synthetic in-memory list of files (used by benches
/// and tests where we don't want to touch disk).
pub fn index_files_in_memory(
    index: &PluckIndex,
    files: &[(impl AsRef<str>, impl AsRef<str>)],
) -> Result<IndexStats> {
    let mut writer = index.writer()?;
    let mut stats = IndexStats::default();
    for (path, src) in files {
        stats.files_seen += 1;
        let path_str = path.as_ref();
        let ext = std::path::Path::new(path_str)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let Some(lang) = Language::from_extension(ext) else {
            stats.files_skipped_lang += 1;
            continue;
        };
        let chunks = chunk_source(src.as_ref(), lang)?;
        for c in &chunks {
            writer.add_chunk(path_str, c)?;
            stats.chunks_indexed += 1;
        }
        stats.files_indexed += 1;
    }
    writer.commit()?;
    Ok(stats)
}

/// Incremental update: delete every chunk for each path in `paths`, then
/// re-add chunks for files that still exist on disk. Used by the file
/// watcher to keep the index in lock-step with the source tree.
///
/// Paths can be absolute or relative to `repo_root`. The index is keyed
/// on paths relative to `repo_root`, so the function normalizes before
/// hitting tantivy.
pub fn reindex_paths(
    index: &PluckIndex,
    repo_root: &Path,
    paths: &[PathBuf],
) -> Result<IndexStats> {
    let mut writer = index.writer().context("open writer for reindex")?;
    let mut stats = IndexStats::default();

    for path in paths {
        let abs = if path.is_absolute() {
            path.clone()
        } else {
            repo_root.join(path)
        };
        let rel = abs
            .strip_prefix(repo_root)
            .unwrap_or(&abs)
            .to_string_lossy()
            .into_owned();

        // Always delete first — covers deletions and the "old version
        // of a modified file" case.
        writer.delete_path(&rel);
        stats.files_seen += 1;

        // If the file is gone (delete event) we're done with it.
        if !abs.is_file() {
            continue;
        }

        let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some(lang) = Language::from_extension(ext) else {
            stats.files_skipped_lang += 1;
            continue;
        };

        let md = match std::fs::metadata(&abs) {
            Ok(m) => m,
            Err(_) => {
                stats.files_skipped_read += 1;
                continue;
            }
        };
        if md.len() > MAX_FILE_BYTES {
            stats.files_skipped_size += 1;
            continue;
        }

        let src = match std::fs::read_to_string(&abs) {
            Ok(s) => s,
            Err(_) => {
                stats.files_skipped_read += 1;
                continue;
            }
        };

        let chunks = match chunk_source(&src, lang) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("reindex chunk failed for {}: {e}", abs.display());
                stats.files_skipped_read += 1;
                continue;
            }
        };

        for c in &chunks {
            writer
                .add_chunk(&rel, c)
                .with_context(|| format!("reindex add_chunk for {}", abs.display()))?;
            stats.chunks_indexed += 1;
        }
        stats.files_indexed += 1;
    }

    writer.commit().context("commit reindex")?;
    Ok(stats)
}

/// Surface the index path for a repo, creating parent dirs as needed.
pub fn resolved_index_dir(repo_root: &Path) -> Result<PathBuf> {
    let dir = crate::store::tantivy_dir(repo_root)?;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_files_in_memory_counts_chunks() {
        let idx = PluckIndex::in_ram().unwrap();
        let files = vec![
            (
                "auth.ts".to_string(),
                "function login(x: string) { return x; }\n".to_string(),
            ),
            (
                "user.ts".to_string(),
                "class User { greet() {} logout() {} }\n".to_string(),
            ),
            (
                "skip.unknown".to_string(),
                "ignored".to_string(),
            ),
        ];
        let stats = index_files_in_memory(&idx, &files).unwrap();
        assert_eq!(stats.files_seen, 3);
        assert_eq!(stats.files_indexed, 2);
        assert_eq!(stats.files_skipped_lang, 1);
        assert!(stats.chunks_indexed >= 4); // login, User, greet, logout
    }

    #[test]
    fn index_repo_on_tempdir_walks_files() {
        let tmp = tempdir().expect("temp dir");
        std::fs::write(
            tmp.path().join("a.ts"),
            "function alpha() { return 1; }\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
        std::fs::write(
            tmp.path().join("sub/b.rs"),
            "fn beta() -> i32 { 2 }\n",
        )
        .unwrap();
        // `.ignore` is respected without a git working tree — `.gitignore`
        // is only honored inside a real git repo (or with require_git(false)
        // on some versions of the ignore crate). `.ignore` covers the
        // non-git tarball-checkout case the test exercises.
        std::fs::write(tmp.path().join(".ignore"), "skipped.ts\n").unwrap();
        std::fs::write(
            tmp.path().join("skipped.ts"),
            "function skipped() {}\n",
        )
        .unwrap();

        let idx = PluckIndex::in_ram().unwrap();
        let stats = index_repo(&idx, tmp.path()).unwrap();
        assert_eq!(
            stats.files_indexed, 2,
            ".ignore should have hidden skipped.ts; got stats: {stats:?}"
        );
        let hits = idx.search("alpha", 5).unwrap();
        assert!(hits.iter().any(|h| h.symbol == "alpha"));
        let hits = idx.search("beta", 5).unwrap();
        assert!(hits.iter().any(|h| h.symbol == "beta"));
        let hits = idx.search("skipped", 5).unwrap();
        assert!(
            hits.iter().all(|h| h.symbol != "skipped"),
            "skipped.ts must not be indexed"
        );
    }

    // Tiny inline temp-dir helper so we don't add a new dev-dep for one test.
    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    fn tempdir() -> Result<TempDir> {
        let base = std::env::temp_dir();
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = base.join(format!("pluck-test-{nano}"));
        std::fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }
}
