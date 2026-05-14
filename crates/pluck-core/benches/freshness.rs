//! Watcher freshness bench.
//!
//! Models the agent's freshness invariant: after the user saves a file,
//! how long until `pluck.search` reflects the change?
//!
//! For each scenario, the bench:
//!   1. Seeds an N-file repo
//:   2. Indexes it once
//:   3. Starts the watcher
//:   4. Modifies one file with a unique marker
//:   5. Polls `idx.search(<marker>)` until the marker surfaces
//:   6. Records the wall-clock time from write to first surface
//!
//! Goal: stay under 1s end-to-end (debounce 150ms + reindex + tantivy
//! commit). The number gets posted in README under the speed-invariant
//! section.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pluck_core::index::PluckIndex;
use pluck_core::indexer::index_repo;
use pluck_core::watcher::{spawn_watcher, DEFAULT_DEBOUNCE};

fn temp_dir(label: &str) -> PathBuf {
    let nano = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("pluck-freshness-{label}-{nano}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write_module(path: &std::path::Path, i: usize) {
    let body = format!(
        "import {{ Logger }} from \"./logger\";\n\nexport function fn_{i}(x: number): number {{\n  return x * {i};\n}}\n"
    );
    std::fs::write(path, body).unwrap();
}

fn seed_repo(dir: &std::path::Path, n: usize) {
    for i in 0..n {
        write_module(&dir.join(format!("mod_{i}.ts")), i);
    }
}

async fn measure_freshness(n_files: usize, n_trials: usize) -> Vec<Duration> {
    let repo = temp_dir(&format!("{n_files}"));
    seed_repo(&repo, n_files);

    let idx = Arc::new(PluckIndex::in_ram().unwrap());
    index_repo(&idx, &repo).unwrap();

    let _watcher = spawn_watcher(repo.clone(), Arc::clone(&idx), DEFAULT_DEBOUNCE)
        .expect("spawn watcher");

    // Brief grace for notify to register before the first measurement.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut samples = Vec::with_capacity(n_trials);

    for trial in 0..n_trials {
        let marker = format!("FreshnessProbe_{n_files}_{trial}");
        let target = repo.join(format!("mod_{}.ts", trial % n_files));
        let body = format!(
            "import {{ Logger }} from \"./logger\";\n\nexport function {marker}(x: number): number {{\n  return x;\n}}\n"
        );

        let t_write = Instant::now();
        std::fs::write(&target, body).unwrap();

        // Poll until the marker surfaces.
        let deadline = t_write + Duration::from_secs(5);
        loop {
            if let Ok(hits) = idx.search(&marker, 3) {
                if hits.iter().any(|h| h.symbol == marker) {
                    samples.push(t_write.elapsed());
                    break;
                }
            }
            if Instant::now() > deadline {
                samples.push(t_write.elapsed());
                eprintln!("WARN: marker {marker} did not surface within 5s");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Give the watcher a moment so back-to-back trials don't pile up.
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    let _ = std::fs::remove_dir_all(&repo);
    samples
}

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort();
    xs[xs.len() / 2]
}

fn p95(mut xs: Vec<Duration>) -> Duration {
    xs.sort();
    let i = (xs.len() as f64 * 0.95).floor() as usize;
    xs[i.min(xs.len() - 1)]
}

#[tokio::main]
async fn main() {
    let scenarios = [
        ("small (50 files)", 50usize, 10usize),
        ("medium (500 files)", 500, 10),
        ("large (2000 files)", 2000, 5),
    ];

    println!();
    println!("| Repo | Trials | Save→search-visible (p50) | p95 |");
    println!("|------|------:|--------------------------:|----:|");
    for (label, files, trials) in scenarios {
        let samples = measure_freshness(files, trials).await;
        let med = median(samples.clone());
        let p = p95(samples);
        println!(
            "| {label} | {trials} | **{:.0} ms** | {:.0} ms |",
            med.as_secs_f64() * 1000.0,
            p.as_secs_f64() * 1000.0,
        );
    }
    println!();
    println!(
        "Debounce: {} ms. Watcher coalesces editor save bursts inside that window before reindexing.",
        DEFAULT_DEBOUNCE.as_millis()
    );
    println!();
}
