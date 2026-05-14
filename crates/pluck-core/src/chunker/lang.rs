use tree_sitter::Language;

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
            "py" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub fn ts_language(self) -> Language {
        match self {
            Self::TypeScript | Self::JavaScript => {
                tree_sitter_typescript::language_typescript()
            }
            Self::Rust => tree_sitter_rust::language(),
            Self::Python => tree_sitter_python::language(),
            Self::Go => tree_sitter_go::language(),
        }
    }

    pub fn query_str(self) -> &'static str {
        match self {
            Self::TypeScript | Self::JavaScript => {
                include_str!("queries/typescript.scm")
            }
            _ => "",
        }
    }
}
