//! Hybrid search quality bench — BM25-only vs BM25 + semantic RRF on a
//! synthetic repo with deliberately oblique natural-language queries.
//!
//! For each query we know which chunk *should* be returned. We measure:
//!   1. Does BM25 alone surface the target inside top-K?
//!   2. Does hybrid surface it?
//!   3. At what rank, for each strategy?
//!
//! Gated by `PLUCK_RUN_MODEL_TESTS=1`. The encoder download is ~60 MB on
//! first run and cached under `~/.pluck/models/`.
//!
//! Phase 2 acceptance: hybrid recall on natural-language queries must
//! beat BM25-only on this fixture without regressing the keyword-match
//! cases. Numbers go into README's Performance section once a Phase 2
//! release ships.

use std::sync::Arc;

use pluck_core::chunker::{chunk_source, Language};
use pluck_core::index::PluckIndex;
use pluck_core::semantic::{StaticEncoder, DEFAULT_MODEL_ID};

/// (label, source_lines, target_symbol) — the target is the chunk we
/// expect the search to surface. Each source intentionally omits the
/// query's literal keywords so BM25 is forced to rank it low.
fn fixtures() -> Vec<(&'static str, String, &'static str)> {
    vec![
        (
            "session.ts",
            String::from(
                r#"// Validate a bearer credential against the active store.
function validateBearer(secret: string): boolean {
  if (!secret) return false;
  return secret.length === 36 && secret.startsWith("tk_");
}
"#,
            ),
            "validateBearer",
        ),
        (
            "billing.ts",
            String::from(
                r#"// Charge the customer's primary card for the given cents.
function chargePrimaryCard(userId: string, cents: number): Promise<boolean> {
  return processTransaction(userId, cents);
}
function processTransaction(u: string, c: number): Promise<boolean> {
  return Promise.resolve(true);
}
"#,
            ),
            "chargePrimaryCard",
        ),
        (
            "rate.ts",
            String::from(
                r#"// Decide if this client has exceeded the budget for the window.
function tooManyRequests(client: string, window: number): boolean {
  const seen = lookupCounter(client, window);
  return seen > 100;
}
function lookupCounter(c: string, w: number): number { return 0; }
"#,
            ),
            "tooManyRequests",
        ),
    ]
}

/// Queries are written the way an agent would phrase them — natural,
/// semantic, no literal keyword overlap with the target chunk.
fn queries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("auth token expiry", "validateBearer"),
        ("payment processing for user", "chargePrimaryCard"),
        ("rate limit enforcement", "tooManyRequests"),
    ]
}

fn rank_of(hits: &[pluck_core::index::SearchHit], symbol: &str) -> Option<usize> {
    hits.iter().position(|h| h.symbol == symbol)
}

fn fmt_rank(r: Option<usize>) -> String {
    match r {
        Some(i) => format!("#{}", i + 1),
        None => "miss".to_string(),
    }
}

fn main() {
    if std::env::var("PLUCK_RUN_MODEL_TESTS").is_err() {
        eprintln!(
            "Skipped — set PLUCK_RUN_MODEL_TESTS=1 to download the embedding model and run."
        );
        return;
    }

    let enc = Arc::new(
        StaticEncoder::load_or_fetch(DEFAULT_MODEL_ID)
            .expect("load embedding model"),
    );

    let idx = PluckIndex::in_ram()
        .expect("in_ram")
        .with_encoder(Arc::clone(&enc));

    let fx = fixtures();
    {
        let mut w = idx.writer().expect("writer");
        for (path, src, _target) in &fx {
            for c in chunk_source(src, Language::TypeScript).unwrap() {
                w.add_chunk(path, &c).expect("add_chunk");
            }
        }
        w.commit().expect("commit");
    }

    println!();
    println!("| Query | Target | BM25 rank | Hybrid rank |");
    println!("|-------|--------|----------:|------------:|");

    let mut bm25_hits = 0;
    let mut hybrid_hits = 0;
    let queries = queries();
    let total = queries.len();

    for (q, target) in &queries {
        let bm25 = idx.search_with_cutoff(q, 10, 0.0).unwrap_or_default();
        let hybrid = idx.search_hybrid(q, 10, 0.0).unwrap_or_default();
        let br = rank_of(&bm25, target);
        let hr = rank_of(&hybrid, target);
        if br.is_some() {
            bm25_hits += 1;
        }
        if hr.is_some() {
            hybrid_hits += 1;
        }
        println!(
            "| `{q}` | `{target}` | {} | {} |",
            fmt_rank(br),
            fmt_rank(hr)
        );
    }

    println!();
    println!(
        "Recall@10 — BM25-only: {}/{}, hybrid: {}/{}",
        bm25_hits, total, hybrid_hits, total
    );
    println!();
}
