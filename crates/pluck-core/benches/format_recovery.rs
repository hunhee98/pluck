//! Deterministic v0.4 format chunk recovery bench.
//!
//! This is a functional gate, not a latency benchmark. It loads the public
//! fixture suite from `benchmarks/quality/format-chunk-recovery.json`, chunks
//! each file through the same path-based language detection used by indexing,
//! and reports the percentage of expected chunk properties recovered.

use std::path::Path;

use pluck_core::chunker::{chunk_source_with_meta_for_path, Chunk, ChunkKind, Language};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Suite {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    path: String,
    source: Vec<String>,
    expected: Vec<ExpectedChunk>,
    #[serde(default)]
    min_chunks: usize,
    #[serde(default)]
    allow_parse_errors: bool,
}

#[derive(Debug, Deserialize)]
struct ExpectedChunk {
    symbol: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    contains: Option<String>,
    #[serde(default)]
    doc_contains: Option<String>,
    #[serde(default)]
    signature_contains: Option<String>,
}

fn main() {
    let suite: Suite = serde_json::from_str(include_str!(
        "../../../benchmarks/quality/format-chunk-recovery.json"
    ))
    .expect("format chunk recovery fixture parses");

    println!();
    println!(
        "Format chunk recovery bench — {} fixtures.",
        suite.cases.len()
    );
    println!();
    println!("| Fixture | Path | Language | Chunks | Expected | Recovered | Recovery |");
    println!("|---------|------|----------|-------:|---------:|----------:|---------:|");

    let mut total_expected = 0usize;
    let mut total_recovered = 0usize;
    let mut misses: Vec<String> = Vec::new();

    for case in &suite.cases {
        let path = Path::new(&case.path);
        let lang = Language::from_path(path).expect("fixture path has supported language");
        let source = case.source.join("\n");
        let result = chunk_source_with_meta_for_path(&source, lang, path)
            .expect("fixture source chunks without fatal error");

        let mut case_expected = case.expected.len();
        let mut case_recovered = 0usize;

        for expected in &case.expected {
            if result.chunks.iter().any(|chunk| expected.matches(chunk)) {
                case_recovered += 1;
            } else {
                misses.push(format!("{}: missing `{}`", case.name, expected.symbol));
            }
        }

        case_expected += 1;
        if result.parse_errors && !case.allow_parse_errors {
            misses.push(format!("{}: parse errors reported", case.name));
        } else {
            case_recovered += 1;
        }

        case_expected += 1;
        if result.chunks.len() < case.min_chunks {
            misses.push(format!(
                "{}: only {} chunks, expected at least {}",
                case.name,
                result.chunks.len(),
                case.min_chunks
            ));
        } else {
            case_recovered += 1;
        }

        total_expected += case_expected;
        total_recovered += case_recovered;

        let case_pct = recovery_pct(case_recovered, case_expected);
        println!(
            "| `{}` | `{}` | {:?} | {} | {} | {} | {:.1}% |",
            case.name,
            case.path,
            lang,
            result.chunks.len(),
            case_expected,
            case_recovered,
            case_pct
        );
    }

    let pct = recovery_pct(total_recovered, total_expected);

    println!();
    println!("Format chunk recovery: **{pct:.1}%**  (gated metric: format_chunk_recovery_pct)");
    println!("Recovered {total_recovered}/{total_expected} expected chunk properties.");
    if !misses.is_empty() {
        println!();
        println!("Misses:");
        for miss in misses {
            println!("- {miss}");
        }
    }
    println!();
}

impl ExpectedChunk {
    fn matches(&self, chunk: &Chunk) -> bool {
        if chunk.symbol != self.symbol {
            return false;
        }
        if let Some(kind) = &self.kind {
            if !kind_matches(&chunk.kind, kind) {
                return false;
            }
        }
        if let Some(needle) = &self.contains {
            if !chunk.content.contains(needle) {
                return false;
            }
        }
        if let Some(needle) = &self.doc_contains {
            if !chunk.doc_comment.contains(needle) {
                return false;
            }
        }
        if let Some(needle) = &self.signature_contains {
            if !chunk.signature.contains(needle) {
                return false;
            }
        }
        true
    }
}

fn kind_matches(actual: &ChunkKind, expected: &str) -> bool {
    matches!(
        (actual, expected),
        (ChunkKind::Function, "Function")
            | (ChunkKind::Method, "Method")
            | (ChunkKind::Class, "Class")
            | (ChunkKind::Struct, "Struct")
            | (ChunkKind::Enum, "Enum")
            | (ChunkKind::Impl, "Impl")
            | (ChunkKind::Trait, "Trait")
            | (ChunkKind::Module, "Module")
    )
}

fn recovery_pct(recovered: usize, expected: usize) -> f64 {
    if expected == 0 {
        100.0
    } else {
        recovered as f64 / expected as f64 * 100.0
    }
}
