//! Session-dedup token-savings bench.
//!
//! Models a realistic agent session where the same chunks get touched by
//! multiple related queries (the typical exploration pattern). Compares:
//!
//!   no-dedup  — every call returns full bodies, no session state
//!   pluck     — second hit of the same chunk gets a one-line placeholder
//!
//! Both workflows are byte-for-byte identical on the metadata they expose
//! (chunk_id, path, line range, symbol, score). The only difference is
//! that the placeholder rendering elides the body bytes the agent already
//! has in its context window. Lossless savings — the agent never loses
//! information, just stops paying for it twice.

use std::path::PathBuf;

use pluck_mcp::server::{PluckServer, SearchParams};
use rmcp::handler::server::wrapper::Parameters;
use tiktoken_rs::cl100k_base;

#[tokio::main]
async fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let bpe = cl100k_base().expect("cl100k_base");
    let tok = |s: &str| bpe.encode_with_special_tokens(s).len();

    // A natural exploration session: each subsequent query overlaps with
    // the previous ones in the chunks it surfaces. This is the pattern
    // dedup is built for.
    let queries = [
        "chunk source",
        "tree sitter query",
        "search index chunk",
        "chunk source tree sitter",
        "BM25 search chunk",
    ];

    // Run A: no dedup (fresh server per call so the session never sees
    // a repeat).
    let mut a_total = 0usize;
    let mut a_per_call = Vec::new();
    for q in &queries {
        let server = PluckServer::new(repo.clone()).expect("server");
        let out = server
            .search(Parameters(SearchParams {
                query: (*q).into(),
                top_k: 8,
                compact: false,
            }))
            .await
            .expect("search");
        let t = tok(&out);
        a_total += t;
        a_per_call.push(t);
    }

    // Run B: with dedup (single persistent server, full session history).
    let server = PluckServer::new(repo.clone()).expect("server");
    let mut b_total = 0usize;
    let mut b_per_call = Vec::new();
    for q in &queries {
        let out = server
            .search(Parameters(SearchParams {
                query: (*q).into(),
                top_k: 8,
                compact: false,
            }))
            .await
            .expect("search");
        let t = tok(&out);
        b_total += t;
        b_per_call.push(t);
    }

    println!();
    println!(
        "Repo: pluck itself. {} queries, top_k=8 each, full-body rendering.",
        queries.len()
    );
    println!();
    println!("| # | Query | No-dedup tokens | With-dedup tokens | Savings |");
    println!("|--:|-------|----------------:|------------------:|--------:|");
    for (i, q) in queries.iter().enumerate() {
        let a = a_per_call[i];
        let b = b_per_call[i];
        let pct = if a > 0 {
            (100.0 * (a as f64 - b as f64) / a as f64).round() as i64
        } else {
            0
        };
        println!("| {} | `{q}` | {a} | {b} | {pct}% |", i + 1);
    }
    let total_pct = if a_total > 0 {
        (100.0 * (a_total as f64 - b_total as f64) / a_total as f64).round() as i64
    } else {
        0
    };
    println!("| Σ | total | **{a_total}** | **{b_total}** | **{total_pct}%** |");
    println!();
    println!("Session dedup elides body bytes the agent already received in earlier calls,");
    println!("replacing them with a one-line `[already-shown: …]` reference. Same chunks,");
    println!("same metadata, zero information loss.");
    println!();
}
