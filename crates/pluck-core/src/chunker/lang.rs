use std::sync::OnceLock;

use tree_sitter::{Language, Query};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
    Python,
    Go,
    Java,
    Html,
}

impl Lang {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "html" | "htm" => Some(Self::Html),
            _ => None,
        }
    }

    pub fn ts_language(self) -> Language {
        match self {
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::Html => tree_sitter_html::LANGUAGE.into(),
        }
    }

    pub fn query_str(self) -> &'static str {
        match self {
            Self::TypeScript => include_str!("queries/typescript.scm"),
            Self::Tsx => include_str!("queries/typescript.scm"),
            Self::JavaScript => include_str!("queries/javascript.scm"),
            Self::Rust => include_str!("queries/rust.scm"),
            Self::Python => include_str!("queries/python.scm"),
            Self::Go => include_str!("queries/go.scm"),
            Self::Java => include_str!("queries/java.scm"),
            Self::Html => include_str!("queries/html.scm"),
        }
    }

    /// Tree-sitter query that captures direct callees inside a chunk —
    /// every node bound to `@callee` is returned as a single callee name.
    pub fn callee_query_str(self) -> &'static str {
        match self {
            Self::TypeScript => include_str!("queries/callees/typescript.scm"),
            Self::Tsx => include_str!("queries/callees/typescript.scm"),
            Self::JavaScript => include_str!("queries/callees/javascript.scm"),
            Self::Rust => include_str!("queries/callees/rust.scm"),
            Self::Python => include_str!("queries/callees/python.scm"),
            Self::Go => include_str!("queries/callees/go.scm"),
            Self::Java => include_str!("queries/callees/java.scm"),
            Self::Html => include_str!("queries/callees/html.scm"),
        }
    }

    /// Tree-sitter query that captures file-level import statements.
    /// Nodes bound to `@import` are emitted as the imported module path
    /// (raw form — string literal contents for JS/TS/Go, scoped path for
    /// Rust/Python).
    pub fn import_query_str(self) -> &'static str {
        match self {
            Self::TypeScript => include_str!("queries/imports/typescript.scm"),
            Self::Tsx => include_str!("queries/imports/typescript.scm"),
            Self::JavaScript => include_str!("queries/imports/javascript.scm"),
            Self::Rust => include_str!("queries/imports/rust.scm"),
            Self::Python => include_str!("queries/imports/python.scm"),
            Self::Go => include_str!("queries/imports/go.scm"),
            Self::Java => include_str!("queries/imports/java.scm"),
            Self::Html => include_str!("queries/imports/html.scm"),
        }
    }

    /// Cached compiled tree-sitter query combining the chunker query (captures
    /// `@*.definition` / `@*.name`), the callee query (captures `@callee`),
    /// and the import query (captures `@import`). One compilation per language
    /// for the lifetime of the process — every `chunk_source` call walks the
    /// tree once with this merged query.
    pub fn compiled_query(self) -> Option<&'static Query> {
        fn build(lang: Lang) -> Option<Query> {
            let ts = lang.ts_language();
            let combined = format!(
                "{}\n{}\n{}",
                lang.query_str(),
                lang.callee_query_str(),
                lang.import_query_str(),
            );
            Query::new(&ts, &combined).ok()
        }
        match self {
            Self::TypeScript => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::TypeScript)).as_ref()
            }
            Self::Tsx => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Tsx)).as_ref()
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
            Self::Java => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Java)).as_ref()
            }
            Self::Html => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Html)).as_ref()
            }
        }
    }
}
