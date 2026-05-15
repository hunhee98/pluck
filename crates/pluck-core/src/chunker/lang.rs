use std::sync::OnceLock;

use tree_sitter::{Language, Query};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    TypeScript,
    JavaScript,
    Rust,
    Python,
    Go,
}

impl Lang {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub fn ts_language(self) -> Language {
        match self {
            Self::TypeScript => tree_sitter_typescript::language_typescript(),
            Self::JavaScript => tree_sitter_javascript::language(),
            Self::Rust => tree_sitter_rust::language(),
            Self::Python => tree_sitter_python::language(),
            Self::Go => tree_sitter_go::language(),
        }
    }

    pub fn query_str(self) -> &'static str {
        match self {
            Self::TypeScript => include_str!("queries/typescript.scm"),
            Self::JavaScript => include_str!("queries/javascript.scm"),
            Self::Rust => include_str!("queries/rust.scm"),
            Self::Python => include_str!("queries/python.scm"),
            Self::Go => include_str!("queries/go.scm"),
        }
    }

    /// Tree-sitter query that captures direct callees inside a chunk —
    /// every node bound to `@callee` is returned as a single callee name.
    pub fn callee_query_str(self) -> &'static str {
        match self {
            Self::TypeScript => include_str!("queries/callees/typescript.scm"),
            Self::JavaScript => include_str!("queries/callees/javascript.scm"),
            Self::Rust => include_str!("queries/callees/rust.scm"),
            Self::Python => include_str!("queries/callees/python.scm"),
            Self::Go => include_str!("queries/callees/go.scm"),
        }
    }

    /// Cached compiled tree-sitter query combining the chunker query (captures
    /// `@*.definition` / `@*.name`) and the callee query (captures `@callee`).
    /// One compilation per language for the lifetime of the process — every
    /// `chunk_source` call walks the tree once with this merged query.
    pub fn compiled_query(self) -> Option<&'static Query> {
        fn build(lang: Lang) -> Option<Query> {
            let ts = lang.ts_language();
            let combined = format!("{}\n{}", lang.query_str(), lang.callee_query_str());
            Query::new(&ts, &combined).ok()
        }
        match self {
            Self::TypeScript => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::TypeScript)).as_ref()
            }
            Self::JavaScript => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::JavaScript)).as_ref()
            }
            Self::Rust => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Rust)).as_ref()
            }
            Self::Python => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Python)).as_ref()
            }
            Self::Go => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Go)).as_ref()
            }
        }
    }
}
