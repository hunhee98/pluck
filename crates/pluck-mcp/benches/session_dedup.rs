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

use std::{fs, path::PathBuf, process};

use pluck_mcp::server::{PluckServer, SearchParams};
use rmcp::handler::server::wrapper::Parameters;
use tiktoken_rs::cl100k_base;

#[tokio::main]
async fn main() {
    let repo = write_fixture_repo();

    let bpe = cl100k_base().expect("cl100k_base");
    let tok = |s: &str| bpe.encode_with_special_tokens(s).len();

    // A natural exploration session: each subsequent query overlaps with
    // the previous ones in the chunks it surfaces. This is the pattern
    // dedup is built for. The fixture keeps this functional metric stable
    // as the real pluck repo grows and search rankings shift.
    let queries = [
        "auth token refresh",
        "refresh token store",
        "token expiry validation",
        "auth session cache",
        "refresh token validation store",
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
        "Repo: deterministic auth-token fixture. {} queries, top_k=8 each, full-body rendering.",
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

fn write_fixture_repo() -> PathBuf {
    let repo = std::env::temp_dir().join(format!("pluck-session-dedup-fixture-{}", process::id()));
    if repo.exists() {
        fs::remove_dir_all(&repo).expect("clear old session_dedup fixture");
    }
    fs::create_dir_all(repo.join("src/auth")).expect("create session_dedup fixture");
    fs::write(
        repo.join("src/auth/token_flow.rs"),
        r#"
pub fn refresh_access_token_flow(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let retry = "refresh token store keeps auth session cache coherent";
    let audit = "token expiry validation records refresh attempts";
    if input.contains("expired") && input.contains("auth") {
        "auth token refresh accepted after expiry validation"
    } else if input.contains("store") {
        retry
    } else {
        note
    }
}

pub fn validate_refresh_token_claims(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let claims = "validation checks token expiry before refresh";
    let audit = "auth session cache stores refresh token validation state";
    if input.contains("validation") && input.contains("expiry") {
        claims
    } else if input.contains("cache") {
        audit
    } else {
        note
    }
}

pub fn persist_refresh_token_store(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let write_path = "token store persists auth refresh session cache";
    let audit = "expiry validation snapshot protects refresh token store";
    if input.contains("store") && input.contains("refresh") {
        write_path
    } else if input.contains("expiry") {
        audit
    } else {
        note
    }
}

pub fn warm_auth_session_cache(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let cache = "auth session cache warms token refresh validation results";
    let audit = "refresh token expiry validation keeps cache current";
    if input.contains("cache") && input.contains("auth") {
        cache
    } else if input.contains("token") {
        audit
    } else {
        note
    }
}

pub fn revoke_expired_refresh_tokens(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let revoke = "expired token refresh path revokes auth session store entries";
    let audit = "expiry validation removes stale refresh cache records";
    if input.contains("expired") || input.contains("expiry") {
        revoke
    } else if input.contains("validation") {
        audit
    } else {
        note
    }
}

pub fn load_auth_token_snapshot(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let snapshot = "token store snapshot feeds auth refresh validation";
    let audit = "session cache compares token expiry before refresh";
    if input.contains("snapshot") || input.contains("store") {
        snapshot
    } else if input.contains("session") {
        audit
    } else {
        note
    }
}

pub fn audit_refresh_validation_event(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let event = "auth refresh validation event captures token store state";
    let audit = "session cache records expiry validation for refresh token";
    if input.contains("audit") || input.contains("validation") {
        event
    } else if input.contains("cache") {
        audit
    } else {
        note
    }
}

pub fn reconcile_token_store_cache(input: &str) -> &'static str {
    let note = "auth token refresh expiry validation store session cache";
    let reconcile = "refresh token store reconciles auth session cache";
    let audit = "expiry validation confirms cache and store consistency";
    if input.contains("reconcile") || input.contains("store") {
        reconcile
    } else if input.contains("expiry") {
        audit
    } else {
        note
    }
}
"#,
    )
    .expect("write session_dedup fixture source");
    repo
}
