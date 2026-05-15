//! Post-fusion ranking pipeline.
//!
//! The BM25 + RRF + cutoff stages produce a candidate list ordered by
//! pure IR signal. This module layers structured-doc-aware boosts on
//! top — signals the IR pipeline can't see because they live in the
//! chunk metadata (path, symbol, kind) rather than the indexed text.
//!
//! Three boosts apply in order, then we re-sort by the adjusted score:
//!
//!   1. Symbol-match boost — chunks whose `symbol` field exactly equals
//!      a query token climb 50 %. BM25F already weights the `symbol`
//!      field; this layers an extra "this *is* the thing they asked for"
//!      bonus on top.
//!   2. Sibling-chunk boost — files that contributed more than one
//!      chunk to the candidate pool get every contributing chunk lifted
//!      by ~5 % per additional sibling, capped at +25 %. Models the
//!      "if half the file is relevant, the relevant chunks deserve a
//!      lift, not just the strongest one" intuition. The cap is the
//!      cap so a huge file can't run away with the
//!      ranking.
//!   3. Test-file penalty — chunks under `test/`, `tests/`, `__tests__/`,
//!      `.test.`, `.spec.`, `_test.`, `_spec.` lose 50 % of their score
//!      unless the query itself mentions "test" or "spec". Test files
//!      are usually noise when the agent is looking for production code.
//!
//! All knobs live as constants below so a future per-query tuner can
//! override them or a benchmark can A/B-test them.

use std::collections::HashMap;

use crate::index::SearchHit;

const SYMBOL_MATCH_BOOST: f32 = 1.5;
const SIBLING_BOOST_PER_EXTRA: f32 = 0.05;
const SIBLING_BOOST_CAP: f32 = 0.25; // +25 % maximum
const TEST_FILE_PENALTY: f32 = 0.5; // ×0.5

/// Apply every post-fusion boost in `hits` in place, then re-sort by
/// the adjusted score (descending). `query` is the original user query
/// string — already lowercased internally for case-insensitive token
/// matching.
pub fn apply_boosts(hits: &mut [SearchHit], query: &str) {
    if hits.is_empty() {
        return;
    }
    let q_lower = query.to_lowercase();
    let q_tokens: Vec<&str> = q_lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    let query_mentions_tests = q_tokens
        .iter()
        .any(|t| *t == "test" || *t == "tests" || *t == "spec" || *t == "specs");

    // Count candidate chunks per file for the sibling-chunk boost.
    let mut chunks_per_file: HashMap<String, usize> = HashMap::new();
    for h in hits.iter() {
        *chunks_per_file.entry(h.path.clone()).or_insert(0) += 1;
    }

    for h in hits.iter_mut() {
        // 1. Symbol-match boost.
        let sym_lower = h.symbol.to_lowercase();
        if q_tokens.iter().any(|t| *t == sym_lower) {
            h.score *= SYMBOL_MATCH_BOOST;
        }

        // 2. Sibling-chunk boost.
        let n = chunks_per_file.get(&h.path).copied().unwrap_or(1);
        if n > 1 {
            let raw = (n - 1) as f32 * SIBLING_BOOST_PER_EXTRA;
            h.score *= 1.0 + raw.min(SIBLING_BOOST_CAP);
        }

        // 3. Test-file penalty.
        if !query_mentions_tests && is_test_path(&h.path) {
            h.score *= TEST_FILE_PENALTY;
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

pub fn is_test_path(path: &str) -> bool {
    let p = path.to_lowercase();
    if p.contains("/test/")
        || p.contains("/tests/")
        || p.contains("/__tests__/")
        || p.starts_with("test/")
        || p.starts_with("tests/")
        || p.contains(".test.")
        || p.contains(".spec.")
        || p.contains("_test.")
        || p.contains("_spec.")
        || p.ends_with("_test.rs")
        || p.ends_with("_test.go")
    {
        return true;
    }
    // Workspace test-utility crates: a leading path segment that ends
    // in `-test` or `-tests` (e.g. `tokio-test/src/task.rs`). These
    // crates ship test scaffolding, not real APIs, so they should be
    // demoted on non-test queries the same way `tests/` directories are.
    if let Some(first) = p.split('/').next() {
        if first.ends_with("-test") || first.ends_with("-tests") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::ChunkKind;

    fn hit(path: &str, symbol: &str, score: f32) -> SearchHit {
        SearchHit {
            score,
            chunk_id: 0,
            path: path.into(),
            symbol: symbol.into(),
            kind: ChunkKind::Function,
            start_line: 1,
            end_line: 10,
            signature: format!("fn {symbol}()"),
            content: format!("fn {symbol}() {{}}"),
        }
    }

    #[test]
    fn symbol_match_boost_promotes_exact_name() {
        let mut hits = vec![
            hit("src/a.rs", "render", 1.0),
            hit("src/b.rs", "renderTree", 1.2), // higher raw, but not the exact name
        ];
        apply_boosts(&mut hits, "render");
        assert_eq!(hits[0].symbol, "render"); // 1.0 × 1.5 = 1.5 wins
    }

    #[test]
    fn sibling_chunk_boost_lifts_multi_chunk_file() {
        // 6 chunks in src/a.rs vs 1 in src/b.rs. The src/b.rs chunk has
        // a slightly higher raw score, but the sibling boost (5 % per
        // extra sibling, capped at +25 %) is large enough at 5
        // additional siblings (+25 %) to flip the order for the top
        // chunk of src/a.rs.
        let mut hits = vec![
            hit("src/a.rs", "fn1", 1.0),
            hit("src/a.rs", "fn2", 0.4),
            hit("src/a.rs", "fn3", 0.4),
            hit("src/a.rs", "fn4", 0.4),
            hit("src/a.rs", "fn5", 0.4),
            hit("src/a.rs", "fn6", 0.4),
            hit("src/b.rs", "baz", 1.15),
        ];
        apply_boosts(&mut hits, "irrelevant");
        // src/a.rs top chunk: 1.0 × 1.25 = 1.25  (+25 % at 6 chunks)
        // src/b.rs lone:     1.15 × 1.0  = 1.15
        // fn1 should win.
        assert_eq!(hits[0].path, "src/a.rs");
        assert_eq!(hits[0].symbol, "fn1");
    }

    #[test]
    fn test_path_penalty_kicks_in_for_tests_dir() {
        let mut hits = vec![
            hit("src/auth/login.rs", "login", 1.0),
            hit("tests/auth_login_test.rs", "login", 1.5),
        ];
        apply_boosts(&mut hits, "login flow");
        // Test file at 1.5 × 0.5 = 0.75. Real impl at 1.0 × 1.5 = 1.5
        // (symbol-match). Real should win.
        assert_eq!(hits[0].path, "src/auth/login.rs");
    }

    #[test]
    fn test_query_disables_test_penalty() {
        let mut hits = vec![
            hit("src/auth/login.rs", "login", 1.0),
            hit("tests/auth_login_test.rs", "login", 1.2),
        ];
        // Query explicitly asks about tests — the penalty should NOT
        // apply, and the test file's raw advantage should carry through.
        apply_boosts(&mut hits, "login test");
        // Both get symbol boost (1.5×), only the second avoids penalty.
        // 1.0×1.5 = 1.5  vs  1.2×1.5 = 1.8 → test file wins.
        assert_eq!(hits[0].path, "tests/auth_login_test.rs");
    }

    #[test]
    fn is_test_path_covers_common_conventions() {
        assert!(is_test_path("tests/auth.rs"));
        assert!(is_test_path("src/__tests__/auth.ts"));
        assert!(is_test_path("src/auth.test.ts"));
        assert!(is_test_path("src/auth.spec.js"));
        assert!(is_test_path("src/auth_test.go"));
        assert!(is_test_path("src/auth_spec.rb"));
        assert!(!is_test_path("src/auth/login.rs"));
        assert!(!is_test_path("src/test_helpers/mod.rs")); // utility, not test
    }

    #[test]
    fn is_test_path_covers_workspace_test_crates() {
        // Workspace pattern: tokio's `tokio-test` crate ships test
        // scaffolding (Mock, task::spawn for tests, etc.) and should
        // not outrank the real `tokio/src/...` APIs on prose queries.
        assert!(is_test_path("tokio-test/src/task.rs"));
        assert!(is_test_path("tokio-test/src/io.rs"));
        assert!(is_test_path("foo-tests/src/lib.rs"));
        // Adjacent but legitimate utility crates must not be demoted.
        assert!(!is_test_path("tokio-util/src/codec.rs"));
        assert!(!is_test_path("tokio-stream/src/lib.rs"));
    }

    #[test]
    fn empty_hits_no_op() {
        let mut hits: Vec<SearchHit> = Vec::new();
        apply_boosts(&mut hits, "anything");
        assert!(hits.is_empty());
    }
}
