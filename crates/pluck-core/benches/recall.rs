//! Labeled recall@K bench for natural-language hybrid search.
//!
//! Gated by `PLUCK_RUN_RECALL_BENCH=1` because it loads the embedding
//! model and, when present, indexes the real tokio checkout.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pluck_core::chunker::{chunk_file, chunk_source, Language};
use pluck_core::index::{PluckIndex, SearchHit};
use pluck_core::semantic::{selected_model_id, StaticEncoder};

#[derive(Clone, Copy)]
struct Label {
    path: &'static str,
    symbol: &'static str,
}

struct QueryCase {
    query: &'static str,
    labels: &'static [Label],
}

struct Metrics {
    recall5: usize,
    recall10: usize,
    reciprocal_rank_sum: f32,
}

const SYNTHETIC_CASES: &[QueryCase] = &[
    QueryCase {
        query: "validate user credential token",
        labels: &[Label {
            path: "auth.rs",
            symbol: "validate_bearer",
        }],
    },
    QueryCase {
        query: "persist customer invoice payment",
        labels: &[Label {
            path: "billing.rs",
            symbol: "charge_primary_card",
        }],
    },
    QueryCase {
        query: "limit repeated client requests",
        labels: &[Label {
            path: "rate.rs",
            symbol: "too_many_requests",
        }],
    },
    QueryCase {
        query: "refresh stale cached value",
        labels: &[Label {
            path: "cache.rs",
            symbol: "refresh_entry",
        }],
    },
    QueryCase {
        query: "retry network call after transient error",
        labels: &[Label {
            path: "network.rs",
            symbol: "retry_request",
        }],
    },
    QueryCase {
        query: "remove expired sessions",
        labels: &[Label {
            path: "session.rs",
            symbol: "prune_expired_sessions",
        }],
    },
    QueryCase {
        query: "parse incoming webhook payload",
        labels: &[Label {
            path: "webhook.rs",
            symbol: "decode_webhook",
        }],
    },
    QueryCase {
        query: "schedule background cleanup job",
        labels: &[Label {
            path: "jobs.rs",
            symbol: "enqueue_cleanup",
        }],
    },
    QueryCase {
        query: "check whether feature flag is active",
        labels: &[Label {
            path: "flags.rs",
            symbol: "flag_enabled",
        }],
    },
    QueryCase {
        query: "redact private fields before logging",
        labels: &[Label {
            path: "audit.rs",
            symbol: "redact_sensitive",
        }],
    },
    QueryCase {
        query: "normalize email address casing",
        labels: &[Label {
            path: "identity.rs",
            symbol: "canonical_email",
        }],
    },
    QueryCase {
        query: "serialize response as json",
        labels: &[Label {
            path: "http.rs",
            symbol: "render_json",
        }],
    },
];

const TOKIO_CASES: &[QueryCase] = &[
    QueryCase {
        query: "spawn a task on the runtime",
        labels: &[
            Label {
                path: "tokio/src/runtime/runtime.rs",
                symbol: "spawn",
            },
            Label {
                path: "tokio/src/runtime/handle.rs",
                symbol: "spawn",
            },
            Label {
                path: "tokio/src/task/spawn.rs",
                symbol: "spawn",
            },
        ],
    },
    QueryCase {
        query: "receive value from channel asynchronously",
        labels: &[
            Label {
                path: "tokio/src/sync/mpsc/bounded.rs",
                symbol: "recv",
            },
            Label {
                path: "tokio/src/sync/mpsc/unbounded.rs",
                symbol: "recv",
            },
            Label {
                path: "tokio/src/sync/broadcast.rs",
                symbol: "recv",
            },
        ],
    },
    QueryCase {
        query: "pause execution for a duration",
        labels: &[Label {
            path: "tokio/src/time/sleep.rs",
            symbol: "sleep",
        }],
    },
    QueryCase {
        query: "read bytes into a buffer",
        labels: &[
            Label {
                path: "tokio/src/io/util/async_read_ext.rs",
                symbol: "read_buf",
            },
            Label {
                path: "tokio/src/io/util/read_buf.rs",
                symbol: "read_buf",
            },
        ],
    },
    QueryCase {
        query: "mutually exclusive access shared state",
        labels: &[Label {
            path: "tokio/src/sync/mutex.rs",
            symbol: "lock",
        }],
    },
    QueryCase {
        query: "exclusive writer access to shared state",
        labels: &[Label {
            path: "tokio/src/sync/rwlock.rs",
            symbol: "write",
        }],
    },
    QueryCase {
        query: "run blocking work on a dedicated thread pool",
        labels: &[
            Label {
                path: "tokio/src/runtime/runtime.rs",
                symbol: "spawn_blocking",
            },
            Label {
                path: "tokio/src/task/blocking.rs",
                symbol: "spawn_blocking",
            },
        ],
    },
];

fn synthetic_repo() -> Vec<(&'static str, &'static str)> {
    vec![
        ("auth.rs", "/// Validate a bearer credential for a user session.\npub fn validate_bearer(secret: &str) -> bool {\n    secret.starts_with(\"tk_\")\n}\n"),
        ("billing.rs", "/// Charge the primary customer card and persist the invoice.\npub fn charge_primary_card(user_id: &str, cents: u64) -> bool {\n    !user_id.is_empty() && cents > 0\n}\n"),
        ("rate.rs", "/// Decide whether a client exceeded its request budget.\npub fn too_many_requests(client: &str, seen: u32) -> bool {\n    !client.is_empty() && seen > 100\n}\n"),
        ("cache.rs", "/// Refresh a stale cached value from the origin store.\npub fn refresh_entry(key: &str) -> bool {\n    !key.is_empty()\n}\n"),
        ("network.rs", "/// Retry a remote request after a transient network failure.\npub fn retry_request(attempt: u8) -> bool {\n    attempt < 3\n}\n"),
        ("session.rs", "/// Remove expired sessions from the active session table.\npub fn prune_expired_sessions(now: u64) -> usize {\n    now as usize\n}\n"),
        ("webhook.rs", "/// Parse an incoming webhook payload into an event.\npub fn decode_webhook(body: &[u8]) -> usize {\n    body.len()\n}\n"),
        ("jobs.rs", "/// Schedule a background cleanup job for later execution.\npub fn enqueue_cleanup(queue: &str) -> bool {\n    !queue.is_empty()\n}\n"),
        ("flags.rs", "/// Check whether a feature flag is active for this account.\npub fn flag_enabled(name: &str) -> bool {\n    name == \"on\"\n}\n"),
        ("audit.rs", "/// Redact private fields before writing audit log lines.\npub fn redact_sensitive(line: &str) -> String {\n    line.replace(\"secret\", \"[redacted]\")\n}\n"),
        ("identity.rs", "/// Normalize an email address by trimming and lowercasing it.\npub fn canonical_email(email: &str) -> String {\n    email.trim().to_ascii_lowercase()\n}\n"),
        ("http.rs", "/// Serialize a response body as JSON text.\npub fn render_json(body: &str) -> String {\n    format!(\"{{\\\"body\\\":\\\"{}\\\"}}\", body)\n}\n"),
    ]
}

fn add_synthetic(idx: &PluckIndex) -> Result<()> {
    let mut writer = idx.writer()?;
    for (path, src) in synthetic_repo() {
        for chunk in chunk_source(src, Language::Rust)? {
            writer.add_chunk(path, &chunk)?;
        }
    }
    writer.commit()
}

fn add_tokio(idx: &PluckIndex, root: &Path) -> Result<bool> {
    if !root.exists() {
        return Ok(false);
    }

    let mut files = Vec::new();
    collect_rs_files(root, &mut files)?;
    let mut writer = idx.writer()?;
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for chunk in chunk_file(&path).with_context(|| format!("chunk {}", path.display()))? {
            writer.add_chunk(&rel, &chunk)?;
        }
    }
    writer.commit()?;
    Ok(true)
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn rank_of(hits: &[SearchHit], labels: &[Label]) -> Option<usize> {
    hits.iter().position(|hit| {
        labels
            .iter()
            .any(|label| hit.path == label.path && hit.symbol == label.symbol)
    })
}

fn fmt_rank(rank: Option<usize>) -> String {
    rank.map(|idx| format!("#{}", idx + 1))
        .unwrap_or_else(|| "miss".to_string())
}

fn measure(idx: &PluckIndex, cases: &[QueryCase], alpha: Option<f32>) -> Metrics {
    let mut metrics = Metrics {
        recall5: 0,
        recall10: 0,
        reciprocal_rank_sum: 0.0,
    };

    for case in cases {
        let hits = idx
            .search_hybrid(case.query, 10, 0.0, alpha)
            .unwrap_or_else(|e| panic!("search {:?}: {e}", case.query));
        if let Some(rank) = rank_of(&hits, case.labels) {
            if rank < 5 {
                metrics.recall5 += 1;
            }
            if rank < 10 {
                metrics.recall10 += 1;
            }
            metrics.reciprocal_rank_sum += 1.0 / (rank as f32 + 1.0);
        }
    }

    metrics
}

fn print_summary_row(label: &str, cases: &[QueryCase], idx: &PluckIndex, alpha: Option<f32>) {
    let metrics = measure(idx, cases, alpha);
    let total = cases.len() as f32;
    println!(
        "| {label} | {} | {:.3} | {:.3} | {:.3} |",
        cases.len(),
        metrics.recall5 as f32 / total,
        metrics.recall10 as f32 / total,
        metrics.reciprocal_rank_sum / total
    );
}

fn print_details(name: &str, cases: &[QueryCase], idx: &PluckIndex, alpha: Option<f32>) {
    println!();
    println!("### {name}");
    println!("| Query | Rank | Top hit |");
    println!("|-------|-----:|---------|");
    for case in cases {
        let hits = idx.search_hybrid(case.query, 10, 0.0, alpha).unwrap();
        let rank = rank_of(&hits, case.labels);
        let top = hits
            .first()
            .map(|hit| format!("{}::{}", hit.path, hit.symbol))
            .unwrap_or_else(|| "(none)".to_string());
        println!("| `{}` | {} | `{}` |", case.query, fmt_rank(rank), top);
    }
}

fn parse_alpha_sweep() -> Option<Vec<Option<f32>>> {
    let raw = std::env::var("PLUCK_RECALL_ALPHA_SWEEP").ok()?;
    let mut alphas: Vec<Option<f32>> = Vec::new();
    for tok in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if tok.eq_ignore_ascii_case("inferred") || tok.eq_ignore_ascii_case("none") {
            alphas.push(None);
        } else {
            match tok.parse::<f32>() {
                Ok(v) if (0.0..=1.0).contains(&v) => alphas.push(Some(v)),
                _ => {
                    eprintln!("ignoring invalid alpha token: {tok:?}");
                }
            }
        }
    }
    if alphas.is_empty() {
        None
    } else {
        Some(alphas)
    }
}

fn label_for(alpha: Option<f32>) -> String {
    match alpha {
        None => "inferred".to_string(),
        Some(v) => format!("α={v:.2}"),
    }
}

fn main() -> Result<()> {
    if std::env::var("PLUCK_RUN_RECALL_BENCH").is_err() {
        eprintln!("Skipped — set PLUCK_RUN_RECALL_BENCH=1 to run the labeled recall bench.");
        return Ok(());
    }

    let model_id = selected_model_id();
    let encoder = Arc::new(StaticEncoder::load_or_fetch(&model_id)?);

    let synthetic_idx = PluckIndex::in_ram()?.with_encoder(Arc::clone(&encoder));
    add_synthetic(&synthetic_idx)?;

    let tokio_root = std::env::var("PLUCK_RECALL_TOKIO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/tokio"));
    let tokio_idx = PluckIndex::in_ram()?.with_encoder(encoder);
    let has_tokio = add_tokio(&tokio_idx, &tokio_root)?;

    println!();
    println!("model: `{model_id}`");

    let alphas = parse_alpha_sweep().unwrap_or_else(|| vec![None]);

    for alpha in &alphas {
        println!();
        println!("alpha: `{}`", label_for(*alpha));
        println!();
        println!("| Dataset | Queries | Recall@5 | Recall@10 | MRR |");
        println!("|---------|--------:|---------:|----------:|----:|");
        print_summary_row("synthetic", SYNTHETIC_CASES, &synthetic_idx, *alpha);
        if has_tokio {
            print_summary_row("tokio", TOKIO_CASES, &tokio_idx, *alpha);
        } else {
            eprintln!(
                "tokio dataset skipped — {} does not exist",
                tokio_root.display()
            );
        }
    }

    // Per-query detail only for the first alpha; otherwise the output
    // gets unwieldy.
    let detail_alpha = alphas[0];
    print_details("synthetic", SYNTHETIC_CASES, &synthetic_idx, detail_alpha);
    if has_tokio {
        print_details("tokio", TOKIO_CASES, &tokio_idx, detail_alpha);
    }
    println!();

    Ok(())
}
