//! End-to-end indexing benchmark.
//!
//! Builds a synthetic repo on disk, then measures:
//!   - bulk in-memory indexing throughput (files / sec, chunks / sec)
//!   - cold-start search latency (open mmap, run query, return hits)
//!   - warm search latency (same reader reused)
//!
//! Why both: cold-start is the worst case the agent ever pays — first call
//! after `pluck` binary launches or after a long idle. Warm is the common
//! case. Together they bound the latency story end-to-end.
//!
//! Output is a markdown table for the README; not a Criterion harness.

use std::time::Instant;

use pluck_core::index::PluckIndex;
use pluck_core::indexer::index_repo;

fn ts_module(i: usize) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "import {{ Logger }} from \"../utils/logger\";\n\nexport interface Cfg_{i} {{\n  name: string;\n  flags: string[];\n}}\n\n"
    ));
    for j in 0..6 {
        s.push_str(&format!(
            "export async function handle_{i}_{j}(req: Request, cfg: Cfg_{i}): Promise<Response> {{\n  const logger = new Logger(\"handle_{i}_{j}\");\n  logger.debug(`processing ${{req.url}}`);\n  if (!cfg.flags.includes(\"enabled\")) {{\n    return new Response(\"disabled\", {{ status: 403 }});\n  }}\n  const start = Date.now();\n  const body = await req.text();\n  logger.info(`handled in ${{Date.now() - start}}ms`);\n  return new Response(body, {{ status: 200 }});\n}}\n\n"
        ));
    }
    s
}

fn build_synthetic_repo(dir: &std::path::Path, n_files: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for i in 0..n_files {
        let p = dir.join(format!("mod_{i}.ts"));
        std::fs::write(p, ts_module(i))?;
    }
    Ok(())
}

fn measure_index(dir: &std::path::Path) -> (std::time::Duration, usize, usize) {
    let idx = PluckIndex::in_ram().expect("in_ram");
    let t0 = Instant::now();
    let stats = index_repo(&idx, dir).expect("index_repo");
    let elapsed = t0.elapsed();
    (elapsed, stats.files_indexed, stats.chunks_indexed)
}

fn measure_search_warm(idx: &PluckIndex, queries: &[&str], iters: usize) -> std::time::Duration {
    let t0 = Instant::now();
    for _ in 0..iters {
        for q in queries {
            let _ = idx.search_with_cutoff(q, 10, 0.12).expect("search");
        }
    }
    t0.elapsed() / (iters as u32 * queries.len() as u32)
}

fn measure_search_cold(dir: &std::path::Path, queries: &[&str]) -> std::time::Duration {
    // Build once on disk, then time fresh `open_or_create` + single search.
    let idx_dir = dir.join("_pluck_idx");
    let _ = std::fs::remove_dir_all(&idx_dir);
    std::fs::create_dir_all(&idx_dir).unwrap();
    let idx = PluckIndex::open_or_create(&idx_dir).expect("open");
    let _ = index_repo(&idx, dir).expect("index");

    let mut total = std::time::Duration::ZERO;
    for q in queries {
        // Re-open the index to truly model "cold" — the daemon would only
        // pay this on first request after start.
        let fresh = PluckIndex::open_or_create(&idx_dir).expect("reopen");
        let t0 = Instant::now();
        let _ = fresh.search_with_cutoff(q, 10, 0.12).expect("search");
        total += t0.elapsed();
    }
    total / queries.len() as u32
}

fn main() {
    let tmp = std::env::temp_dir().join(format!(
        "pluck-bench-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    let scenarios: &[(&str, usize)] = &[
        ("small (50 files)", 50),
        ("medium (500 files)", 500),
        ("large (2000 files)", 2000),
    ];

    let queries = &[
        "handle request",
        "logger debug",
        "Response status",
        "config flags enabled",
    ];

    println!();
    println!("| Repo | Files indexed | Chunks | Index time | Files/s | Chunks/s | Search warm (p50) | Search cold (p50) |");
    println!("|------|-------------:|------:|----------:|--------:|---------:|------------------:|------------------:|");

    for (label, n) in scenarios {
        let dir = tmp.join(format!("scenario_{n}"));
        build_synthetic_repo(&dir, *n).expect("build repo");

        // Throughput (in-memory).
        let (elapsed, files, chunks) = measure_index(&dir);
        let files_per_sec = files as f64 / elapsed.as_secs_f64();
        let chunks_per_sec = chunks as f64 / elapsed.as_secs_f64();

        // Warm search: reuse one in-RAM index.
        let warm_idx = PluckIndex::in_ram().expect("warm");
        let _ = index_repo(&warm_idx, &dir).expect("warm idx");
        let warm = measure_search_warm(&warm_idx, queries, 10);

        // Cold search: open mmap tantivy dir per query.
        let cold = measure_search_cold(&dir, queries);

        println!(
            "| {label} | {files} | {chunks} | {:.0} ms | {files_per_sec:.0} | {chunks_per_sec:.0} | {:.2} ms | {:.2} ms |",
            elapsed.as_secs_f64() * 1000.0,
            warm.as_secs_f64() * 1000.0,
            cold.as_secs_f64() * 1000.0,
        );
    }

    println!();
    std::fs::remove_dir_all(&tmp).ok();
}
