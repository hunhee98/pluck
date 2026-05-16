//! Labeled recall@K / NDCG@10 bench for natural-language hybrid search.
//!
//! Gated by `PLUCK_RUN_RECALL_BENCH=1` because it loads the embedding
//! model and, when present, indexes the real tokio checkout.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pluck_core::chunker::{chunk_file, chunk_source, Language};
use pluck_core::index::{PluckIndex, SearchHit};
use pluck_core::semantic::{selected_model_id, StaticEncoder};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
struct Label {
    path: String,
    symbol: String,
    relevance: u8,
}

#[derive(Clone, Deserialize)]
struct QueryCase {
    query: String,
    labels: Vec<Label>,
}

#[derive(Clone, Deserialize)]
struct Dataset {
    name: String,
    kind: DatasetKind,
    cases: Vec<QueryCase>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DatasetKind {
    SyntheticRust,
    TokioRust,
}

#[derive(Deserialize)]
struct QualitySuite {
    datasets: Vec<Dataset>,
}

#[derive(Clone, Copy, Default)]
struct Metrics {
    recall5: usize,
    recall10: usize,
    reciprocal_rank_sum: f32,
    ndcg10_sum: f32,
}

#[derive(Serialize)]
struct SuiteReport {
    model: String,
    alpha: String,
    datasets: Vec<DatasetReport>,
}

#[derive(Serialize)]
struct DatasetReport {
    name: String,
    queries: usize,
    recall5: f32,
    recall10: f32,
    mrr: f32,
    ndcg10: f32,
    cases: Vec<CaseReport>,
}

#[derive(Serialize)]
struct CaseReport {
    query: String,
    rank: Option<usize>,
    reciprocal_rank: f32,
    ndcg10: f32,
    top_hit: Option<String>,
}

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
        labels.iter().any(|label| {
            hit.path == label.path.as_str() && hit.symbol == label.symbol.as_str()
        })
    })
}

fn fmt_rank(rank: Option<usize>) -> String {
    rank.map(|idx| format!("#{}", idx + 1))
        .unwrap_or_else(|| "miss".to_string())
}

fn relevance_for(hit: &SearchHit, labels: &[Label]) -> u8 {
    labels
        .iter()
        .filter(|label| {
            hit.path == label.path.as_str() && hit.symbol == label.symbol.as_str()
        })
        .map(|label| label.relevance)
        .max()
        .unwrap_or(0)
}

fn dcg(relevances: impl IntoIterator<Item = u8>) -> f32 {
    relevances
        .into_iter()
        .enumerate()
        .map(|(idx, rel)| {
            let gain = 2_f32.powi(rel as i32) - 1.0;
            gain / ((idx as f32 + 2.0).log2())
        })
        .sum()
}

fn ndcg_at_10(hits: &[SearchHit], labels: &[Label]) -> f32 {
    let actual = dcg(
        hits.iter()
            .take(10)
            .map(|hit| relevance_for(hit, labels)),
    );
    let mut ideal: Vec<u8> = labels.iter().map(|label| label.relevance).collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let ideal = dcg(ideal.into_iter().take(10));
    if ideal == 0.0 {
        0.0
    } else {
        actual / ideal
    }
}

fn measure(idx: &PluckIndex, cases: &[QueryCase], alpha: Option<f32>) -> Metrics {
    let mut metrics = Metrics::default();

    for case in cases {
        let hits = idx
            .search_hybrid(&case.query, 10, 0.0, alpha)
            .unwrap_or_else(|e| panic!("search {:?}: {e}", case.query));
        metrics.ndcg10_sum += ndcg_at_10(&hits, &case.labels);
        if let Some(rank) = rank_of(&hits, &case.labels) {
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

fn dataset_report(
    name: &str,
    cases: &[QueryCase],
    idx: &PluckIndex,
    alpha: Option<f32>,
) -> DatasetReport {
    let metrics = measure(idx, cases, alpha);
    let total = cases.len() as f32;
    let mut case_reports = Vec::with_capacity(cases.len());
    for case in cases {
        let hits = idx.search_hybrid(&case.query, 10, 0.0, alpha).unwrap();
        let rank = rank_of(&hits, &case.labels);
        let reciprocal_rank = rank.map(|r| 1.0 / (r as f32 + 1.0)).unwrap_or(0.0);
        let ndcg10 = ndcg_at_10(&hits, &case.labels);
        let top_hit = hits
            .first()
            .map(|hit| format!("{}::{}", hit.path, hit.symbol));
        case_reports.push(CaseReport {
            query: case.query.clone(),
            rank: rank.map(|idx| idx + 1),
            reciprocal_rank,
            ndcg10,
            top_hit,
        });
    }

    DatasetReport {
        name: name.to_string(),
        queries: cases.len(),
        recall5: metrics.recall5 as f32 / total,
        recall10: metrics.recall10 as f32 / total,
        mrr: metrics.reciprocal_rank_sum / total,
        ndcg10: metrics.ndcg10_sum / total,
        cases: case_reports,
    }
}

fn print_summary_row(report: &DatasetReport) {
    println!(
        "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
        report.name, report.queries, report.recall5, report.recall10, report.mrr,
        report.ndcg10
    );
}

fn print_details(name: &str, cases: &[QueryCase], idx: &PluckIndex, alpha: Option<f32>) {
    println!();
    println!("### {name}");
    println!("| Query | Rank | Top hit |");
    println!("|-------|-----:|---------|");
    for case in cases {
        let hits = idx.search_hybrid(&case.query, 10, 0.0, alpha).unwrap();
        let rank = rank_of(&hits, &case.labels);
        let top = hits
            .first()
            .map(|hit| format!("{}::{}", hit.path, hit.symbol))
            .unwrap_or_else(|| "(none)".to_string());
        println!("| `{}` | {} | `{}` |", case.query, fmt_rank(rank), top);
    }
}

fn suite_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/quality/recall.json")
}

fn load_suite() -> Result<QualitySuite> {
    let path = suite_path();
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read quality suite {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse quality suite {}", path.display()))
}

fn default_report_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/results/recall-quality.json")
}

fn write_report(report: &SuiteReport) -> Result<()> {
    let path = std::env::var("PLUCK_RECALL_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_report_path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create report dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&path, format!("{json}\n"))
        .with_context(|| format!("write recall report {}", path.display()))?;
    eprintln!("recall report saved -> {}", path.display());
    Ok(())
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
    let suite = load_suite()?;

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
    let mut first_report: Option<SuiteReport> = None;

    for alpha in &alphas {
        println!();
        println!("alpha: `{}`", label_for(*alpha));
        println!();
        println!("| Dataset | Queries | Recall@5 | Recall@10 | MRR | NDCG@10 |");
        println!("|---------|--------:|---------:|----------:|----:|--------:|");

        let mut reports = Vec::new();
        for dataset in &suite.datasets {
            match dataset.kind {
                DatasetKind::SyntheticRust => {
                    let report =
                        dataset_report(&dataset.name, &dataset.cases, &synthetic_idx, *alpha);
                    print_summary_row(&report);
                    reports.push(report);
                }
                DatasetKind::TokioRust if has_tokio => {
                    let report = dataset_report(&dataset.name, &dataset.cases, &tokio_idx, *alpha);
                    print_summary_row(&report);
                    reports.push(report);
                }
                DatasetKind::TokioRust => {
                    eprintln!(
                        "{} dataset skipped — {} does not exist",
                        dataset.name,
                        tokio_root.display()
                    );
                }
            }
        }

        if first_report.is_none() {
            first_report = Some(SuiteReport {
                model: model_id.clone(),
                alpha: label_for(*alpha),
                datasets: reports,
            });
        }
    }

    // Per-query detail only for the first alpha; otherwise the output
    // gets unwieldy.
    let detail_alpha = alphas[0];
    for dataset in &suite.datasets {
        match dataset.kind {
            DatasetKind::SyntheticRust => {
                print_details(&dataset.name, &dataset.cases, &synthetic_idx, detail_alpha);
            }
            DatasetKind::TokioRust if has_tokio => {
                print_details(&dataset.name, &dataset.cases, &tokio_idx, detail_alpha);
            }
            DatasetKind::TokioRust => {}
        }
    }
    println!();

    if let Some(report) = &first_report {
        write_report(report)?;
    }

    Ok(())
}
