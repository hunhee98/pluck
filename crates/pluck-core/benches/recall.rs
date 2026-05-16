//! Labeled recall@K / NDCG@10 bench for natural-language hybrid search.
//!
//! Gated by `PLUCK_RUN_RECALL_BENCH=1` because it loads the embedding
//! model and, when present, indexes real tokio, django, and next.js
//! checkouts.

use std::collections::BTreeMap;
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
    root_env: Option<String>,
    root_default: Option<String>,
    cases: Vec<QueryCase>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DatasetKind {
    SyntheticRust,
    SyntheticMultilingual,
    RepoBacked,
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
    by_language: Vec<LanguageReport>,
    cases: Vec<CaseReport>,
}

#[derive(Serialize)]
struct LanguageReport {
    language: String,
    queries: usize,
    recall5: f32,
    recall10: f32,
    mrr: f32,
    ndcg10: f32,
}

#[derive(Serialize)]
struct CaseReport {
    query: String,
    language: String,
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

fn synthetic_multilingual_repo() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "i18n_auth.rs",
            "/// 사용자토큰검증 진입점.\npub fn korean_token_check(token: &str) -> bool {\n    !token.is_empty()\n}\n",
        ),
        (
            "i18n_user.rs",
            "/// 用户认证入口。\npub fn chinese_user_auth(user: &str) -> bool {\n    !user.is_empty()\n}\n",
        ),
        (
            "i18n_cache.rs",
            "/// キャッシュ更新処理。\npub fn japanese_cache_refresh(key: &str) -> bool {\n    !key.is_empty()\n}\n",
        ),
    ]
}

fn add_synthetic_multilingual(idx: &PluckIndex) -> Result<()> {
    let mut writer = idx.writer()?;
    for (path, src) in synthetic_multilingual_repo() {
        for chunk in chunk_source(src, Language::Rust)? {
            writer.add_chunk(path, &chunk)?;
        }
    }
    writer.commit()
}

fn add_repo(idx: &PluckIndex, root: &Path) -> Result<bool> {
    if !root.exists() {
        return Ok(false);
    }

    let mut files = Vec::new();
    collect_supported_files(root, &mut files)?;
    let mut writer = idx.writer()?;
    let mut skipped = 0usize;
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        match chunk_file(&path) {
            Ok(chunks) => {
                for chunk in chunks {
                    writer.add_chunk(&rel, &chunk)?;
                }
            }
            Err(err) => {
                skipped += 1;
                eprintln!("skipping unchunkable file {}: {err:#}", path.display());
            }
        }
    }
    if skipped > 0 {
        eprintln!(
            "skipped {skipped} unchunkable files under {}",
            root.display()
        );
    }
    writer.commit()?;
    Ok(true)
}

fn collect_supported_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(
            name.as_ref(),
            "target"
                | ".git"
                | "node_modules"
                | ".next"
                | "__pycache__"
                | ".venv"
                | "dist"
                | "compiled"
                | "fixtures"
                | "__fixtures__"
                | "__testfixtures__"
                | "__tests__"
        ) {
            continue;
        }
        if path.is_dir() {
            collect_supported_files(&path, out)?;
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(Language::from_extension)
            .is_some()
        {
            out.push(path);
        }
    }
    Ok(())
}

fn rank_of(hits: &[SearchHit], labels: &[Label]) -> Option<usize> {
    hits.iter().position(|hit| {
        labels
            .iter()
            .any(|label| hit.path == label.path.as_str() && hit.symbol == label.symbol.as_str())
    })
}

fn fmt_rank(rank: Option<usize>) -> String {
    rank.map(|idx| format!("#{}", idx + 1))
        .unwrap_or_else(|| "miss".to_string())
}

fn relevance_for(hit: &SearchHit, labels: &[Label]) -> u8 {
    labels
        .iter()
        .filter(|label| hit.path == label.path.as_str() && hit.symbol == label.symbol.as_str())
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
    let actual = dcg(hits.iter().take(10).map(|hit| relevance_for(hit, labels)));
    let mut ideal: Vec<u8> = labels.iter().map(|label| label.relevance).collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let ideal = dcg(ideal.into_iter().take(10));
    if ideal == 0.0 {
        0.0
    } else {
        actual / ideal
    }
}

fn language_for_case(case: &QueryCase) -> String {
    case.labels
        .first()
        .map(|label| language_for_path(&label.path))
        .unwrap_or_else(|| "unknown".to_string())
}

fn language_for_path(path: &str) -> String {
    match Path::new(path).extension().and_then(|ext| ext.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        Some("go") => "go",
        _ => "unknown",
    }
    .to_string()
}

fn update_metrics(metrics: &mut Metrics, rank: Option<usize>, ndcg10: f32) {
    metrics.ndcg10_sum += ndcg10;
    if let Some(rank) = rank {
        if rank < 5 {
            metrics.recall5 += 1;
        }
        if rank < 10 {
            metrics.recall10 += 1;
        }
        metrics.reciprocal_rank_sum += 1.0 / (rank as f32 + 1.0);
    }
}

fn normalize_metrics(metrics: Metrics, total: usize) -> (f32, f32, f32, f32) {
    let total = total as f32;
    (
        metrics.recall5 as f32 / total,
        metrics.recall10 as f32 / total,
        metrics.reciprocal_rank_sum / total,
        metrics.ndcg10_sum / total,
    )
}

fn measure(idx: &PluckIndex, cases: &[QueryCase], alpha: Option<f32>) -> Metrics {
    let mut metrics = Metrics::default();

    for case in cases {
        let hits = idx
            .search_hybrid(&case.query, 10, 0.0, alpha)
            .unwrap_or_else(|e| panic!("search {:?}: {e}", case.query));
        update_metrics(
            &mut metrics,
            rank_of(&hits, &case.labels),
            ndcg_at_10(&hits, &case.labels),
        );
    }

    metrics
}

fn language_breakdown(
    idx: &PluckIndex,
    cases: &[QueryCase],
    alpha: Option<f32>,
) -> Vec<LanguageReport> {
    let mut grouped: BTreeMap<String, (usize, Metrics)> = BTreeMap::new();
    for case in cases {
        let lang = language_for_case(case);
        let hits = idx.search_hybrid(&case.query, 10, 0.0, alpha).unwrap();
        let entry = grouped.entry(lang).or_default();
        entry.0 += 1;
        update_metrics(
            &mut entry.1,
            rank_of(&hits, &case.labels),
            ndcg_at_10(&hits, &case.labels),
        );
    }

    grouped
        .into_iter()
        .map(|(language, (queries, metrics))| {
            let (recall5, recall10, mrr, ndcg10) = normalize_metrics(metrics, queries);
            LanguageReport {
                language,
                queries,
                recall5,
                recall10,
                mrr,
                ndcg10,
            }
        })
        .collect()
}

fn dataset_report(
    name: &str,
    cases: &[QueryCase],
    idx: &PluckIndex,
    alpha: Option<f32>,
) -> DatasetReport {
    let metrics = measure(idx, cases, alpha);
    let (recall5, recall10, mrr, ndcg10) = normalize_metrics(metrics, cases.len());
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
            language: language_for_case(case),
            rank: rank.map(|idx| idx + 1),
            reciprocal_rank,
            ndcg10,
            top_hit,
        });
    }

    DatasetReport {
        name: name.to_string(),
        queries: cases.len(),
        recall5,
        recall10,
        mrr,
        ndcg10,
        by_language: language_breakdown(idx, cases, alpha),
        cases: case_reports,
    }
}

fn print_summary_row(report: &DatasetReport) {
    println!(
        "| {} | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
        report.name, report.queries, report.recall5, report.recall10, report.mrr, report.ndcg10
    );
}

fn print_details(name: &str, cases: &[QueryCase], idx: &PluckIndex, alpha: Option<f32>) {
    println!();
    println!("### {name}");
    println!("| Query | Language | Rank | Top hit |");
    println!("|-------|----------|-----:|---------|");
    for case in cases {
        let hits = idx.search_hybrid(&case.query, 10, 0.0, alpha).unwrap();
        let rank = rank_of(&hits, &case.labels);
        let language = language_for_case(case);
        let top = hits
            .first()
            .map(|hit| format!("{}::{}", hit.path, hit.symbol))
            .unwrap_or_else(|| "(none)".to_string());
        println!(
            "| `{}` | {} | {} | `{}` |",
            case.query,
            language,
            fmt_rank(rank),
            top
        );
    }
}

fn print_language_breakdown(reports: &[DatasetReport]) {
    println!();
    println!("### Per-language breakdown");
    println!("| Dataset | Language | Queries | Recall@5 | Recall@10 | MRR | NDCG@10 |");
    println!("|---------|----------|--------:|---------:|----------:|----:|--------:|");
    for report in reports {
        for lang in &report.by_language {
            println!(
                "| {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
                report.name,
                lang.language,
                lang.queries,
                lang.recall5,
                lang.recall10,
                lang.mrr,
                lang.ndcg10
            );
        }
    }
}

fn suite_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/quality/recall.json")
}

fn load_suite() -> Result<QualitySuite> {
    let path = suite_path();
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read quality suite {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse quality suite {}", path.display()))
}

fn dataset_root(dataset: &Dataset) -> PathBuf {
    if let Some(env_name) = &dataset.root_env {
        if let Ok(value) = std::env::var(env_name) {
            return PathBuf::from(value);
        }
    }
    dataset
        .root_default
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/pluck-quality-repo"))
}

fn default_report_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results/recall-quality.json")
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
    let multilingual_idx = PluckIndex::in_ram()?.with_encoder(Arc::clone(&encoder));
    add_synthetic_multilingual(&multilingual_idx)?;

    let mut repo_indices = Vec::new();
    for dataset in &suite.datasets {
        if matches!(dataset.kind, DatasetKind::RepoBacked) {
            let root = dataset_root(dataset);
            let idx = PluckIndex::in_ram()?.with_encoder(Arc::clone(&encoder));
            let available = add_repo(&idx, &root)?;
            repo_indices.push((dataset.name.clone(), idx, available, root));
        }
    }

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
                DatasetKind::SyntheticMultilingual => {
                    let report =
                        dataset_report(&dataset.name, &dataset.cases, &multilingual_idx, *alpha);
                    print_summary_row(&report);
                    reports.push(report);
                }
                DatasetKind::RepoBacked => {
                    let Some((_, idx, available, root)) = repo_indices
                        .iter()
                        .find(|(name, _, _, _)| name == &dataset.name)
                    else {
                        continue;
                    };
                    if *available {
                        let report = dataset_report(&dataset.name, &dataset.cases, idx, *alpha);
                        print_summary_row(&report);
                        reports.push(report);
                    } else {
                        eprintln!(
                            "{} dataset skipped — {} does not exist",
                            dataset.name,
                            root.display()
                        );
                    }
                }
            }
        }

        print_language_breakdown(&reports);

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
            DatasetKind::SyntheticMultilingual => {
                print_details(
                    &dataset.name,
                    &dataset.cases,
                    &multilingual_idx,
                    detail_alpha,
                );
            }
            DatasetKind::RepoBacked => {
                let Some((_, idx, available, _)) = repo_indices
                    .iter()
                    .find(|(name, _, _, _)| name == &dataset.name)
                else {
                    continue;
                };
                if *available {
                    print_details(&dataset.name, &dataset.cases, idx, detail_alpha);
                }
            }
        }
    }
    println!();

    if let Some(report) = &first_report {
        write_report(report)?;
    }

    Ok(())
}
