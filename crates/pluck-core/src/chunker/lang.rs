use std::{path::Path, sync::OnceLock};
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
    Kotlin,
    Sql,
    Hcl,
    C,
    Cpp,
    Html,
    Css,
    Scss,
    Markdown,
    Mdx,
    Swift,
    Ruby,
    Php,
    Svelte,
    GraphQl,
    Json,
    Yaml,
    Toml,
    Dockerfile,
    Shell,
}

impl Lang {
    pub fn from_path(path: &Path) -> Option<Self> {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let lower_name = file_name.to_ascii_lowercase();
        if lower_name == "dockerfile"
            || lower_name.starts_with("dockerfile.")
            || lower_name == "containerfile"
            || lower_name.starts_with("containerfile.")
        {
            return Some(Self::Dockerfile);
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("dockerfile") {
            return Some(Self::Dockerfile);
        }

        if is_shell_filename(&lower_name) {
            return Some(Self::Shell);
        }
        Self::from_extension(ext)
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "java" => Some(Self::Java),
            "kt" | "kts" => Some(Self::Kotlin),
            "sql" | "ddl" | "dml" => Some(Self::Sql),
            "tf" | "tfvars" | "hcl" => Some(Self::Hcl),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hxx" | "h++" | "tcc" => Some(Self::Cpp),
            "html" | "htm" => Some(Self::Html),
            "css" => Some(Self::Css),
            "scss" => Some(Self::Scss),
            "md" | "markdown" => Some(Self::Markdown),
            "mdx" => Some(Self::Mdx),
            "swift" => Some(Self::Swift),
            "rb" => Some(Self::Ruby),
            "php" => Some(Self::Php),
            "svelte" => Some(Self::Svelte),
            "graphql" | "gql" => Some(Self::GraphQl),
            "json" => Some(Self::Json),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            "dockerfile" => Some(Self::Dockerfile),
            "sh" | "bash" | "zsh" | "ksh" | "bats" => Some(Self::Shell),
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
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Self::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Self::Hcl => tree_sitter_hcl::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Html => tree_sitter_html::LANGUAGE.into(),
            Self::Css => tree_sitter_css::LANGUAGE.into(),
            Self::Scss => tree_sitter_scss::language(),
            Self::Markdown | Self::Mdx => tree_sitter_md_025::LANGUAGE.into(),
            Self::Swift => tree_sitter_swift::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::Svelte => tree_sitter_svelte_ng::LANGUAGE.into(),
            Self::GraphQl => tree_sitter_graphql::LANGUAGE.into(),
            Self::Json | Self::Yaml | Self::Toml | Self::Dockerfile | Self::Shell => {
                unreachable!("custom formats do not use tree-sitter")
            }
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
            Self::Kotlin => include_str!("queries/kotlin.scm"),
            Self::Sql => include_str!("queries/sql.scm"),
            Self::Hcl => include_str!("queries/hcl.scm"),
            Self::C => include_str!("queries/c.scm"),
            Self::Cpp => include_str!("queries/cpp.scm"),
            Self::Html => include_str!("queries/html.scm"),
            Self::Css => include_str!("queries/css.scm"),
            Self::Scss => include_str!("queries/scss.scm"),
            Self::Markdown | Self::Mdx => include_str!("queries/markdown.scm"),
            Self::Swift => include_str!("queries/swift.scm"),
            Self::Ruby => include_str!("queries/ruby.scm"),
            Self::Php => include_str!("queries/php.scm"),
            Self::Svelte => include_str!("queries/svelte.scm"),
            Self::GraphQl => include_str!("queries/graphql.scm"),
            Self::Json | Self::Yaml | Self::Toml | Self::Dockerfile | Self::Shell => "",
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
            Self::Kotlin => include_str!("queries/callees/kotlin.scm"),
            Self::Sql => include_str!("queries/callees/sql.scm"),
            Self::Hcl => include_str!("queries/callees/hcl.scm"),
            Self::C => include_str!("queries/callees/c.scm"),
            Self::Cpp => include_str!("queries/callees/cpp.scm"),
            Self::Html => include_str!("queries/callees/html.scm"),
            Self::Css => include_str!("queries/callees/css.scm"),
            Self::Scss => include_str!("queries/callees/scss.scm"),
            Self::Markdown | Self::Mdx => include_str!("queries/callees/markdown.scm"),
            Self::Swift => include_str!("queries/callees/swift.scm"),
            Self::Ruby => include_str!("queries/callees/ruby.scm"),
            Self::Php => include_str!("queries/callees/php.scm"),
            Self::Svelte => include_str!("queries/callees/svelte.scm"),
            Self::GraphQl => include_str!("queries/callees/graphql.scm"),
            Self::Json | Self::Yaml | Self::Toml | Self::Dockerfile | Self::Shell => "",
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
            Self::Kotlin => include_str!("queries/imports/kotlin.scm"),
            Self::Sql => include_str!("queries/imports/sql.scm"),
            Self::Hcl => include_str!("queries/imports/hcl.scm"),
            Self::C => include_str!("queries/imports/c.scm"),
            Self::Cpp => include_str!("queries/imports/cpp.scm"),
            Self::Html => include_str!("queries/imports/html.scm"),
            Self::Css => include_str!("queries/imports/css.scm"),
            Self::Scss => include_str!("queries/imports/scss.scm"),
            Self::Markdown | Self::Mdx => include_str!("queries/imports/markdown.scm"),
            Self::Swift => include_str!("queries/imports/swift.scm"),
            Self::Ruby => include_str!("queries/imports/ruby.scm"),
            Self::Php => include_str!("queries/imports/php.scm"),
            Self::Svelte => include_str!("queries/imports/svelte.scm"),
            Self::GraphQl => include_str!("queries/imports/graphql.scm"),
            Self::Json | Self::Yaml | Self::Toml | Self::Dockerfile | Self::Shell => "",
        }
    }

    pub fn is_config_format(self) -> bool {
        matches!(self, Self::Json | Self::Yaml | Self::Toml)
    }

    /// Cached compiled tree-sitter query combining the chunker query (captures
    /// `@*.definition` / `@*.name`), the callee query (captures `@callee`),
    /// and the import query (captures `@import`). One compilation per language
    /// for the lifetime of the process — every `chunk_source` call walks the
    /// tree once with this merged query.
    pub fn compiled_query(self) -> Option<&'static Query> {
        fn build(lang: Lang) -> Option<Query> {
            if lang.is_config_format() {
                return None;
            }
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
            Self::Kotlin => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Kotlin)).as_ref()
            }
            Self::Sql => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Sql)).as_ref()
            }
            Self::Hcl => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Hcl)).as_ref()
            }
            Self::C => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::C)).as_ref()
            }
            Self::Cpp => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Cpp)).as_ref()
            }
            Self::Html => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Html)).as_ref()
            }
            Self::Css => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Css)).as_ref()
            }
            Self::Scss => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Scss)).as_ref()
            }
            Self::Markdown => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Markdown)).as_ref()
            }
            Self::Mdx => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Mdx)).as_ref()
            }
            Self::Swift => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Swift)).as_ref()
            }
            Self::Ruby => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Ruby)).as_ref()
            }
            Self::Php => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Php)).as_ref()
            }
            Self::Svelte => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::Svelte)).as_ref()
            }
            Self::GraphQl => {
                static Q: OnceLock<Option<Query>> = OnceLock::new();
                Q.get_or_init(|| build(Self::GraphQl)).as_ref()
            }
            Self::Json | Self::Yaml | Self::Toml | Self::Dockerfile | Self::Shell => None,
        }
    }
}

fn is_shell_filename(name: &str) -> bool {
    matches!(
        name,
        ".bashrc"
            | ".bash_profile"
            | ".bash_login"
            | ".profile"
            | "profile"
            | ".zshrc"
            | ".zprofile"
            | ".zshenv"
            | ".zlogin"
            | ".kshrc"
            | ".envrc"
            | "pkgbuild"
    )
}
