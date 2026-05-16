//! Custom tantivy tokenizer for code identifiers.
//!
//! The stock `SimpleTokenizer` (a) drops everything outside ASCII, so
//! Hangul / CJK / European-accented identifiers and comments never make
//! it into the index, and (b) does not split `camelCase` / `snake_case`,
//! so the agent can't partial-match `Handler` inside `HandlerStack`.
//!
//! `PluckTokenizer` fixes both:
//!
//!   1. Identifier spans match the regex `[\p{L}_][\p{L}\p{N}_]*` — any
//!      Unicode letter, plus digits and underscores, so `의존성_그래프`,
//!      `getTokenCount`, and `cors_middleware` all survive.
//!   2. For each identifier longer than one part, the whole token *and*
//!      its split-by-case-or-underscore parts are emitted at the same
//!      input offset. Searching `handler` finds `HandlerStack`.
//!
//! Output tokens are lowercased — BM25 matching is case-insensitive in
//! every code-search tool the agent expects to behave consistently with.

use std::sync::OnceLock;

use regex::Regex;
use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

/// Tantivy tokenizer name. Set on every field that participates in
/// BM25 scoring (symbol / signature / content) so QueryParser and the
/// index agree on tokenization.
pub const TOKENIZER_NAME: &str = "pluck";

const BM25_STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "being", "but", "by", "can", "could", "did", "do", "does", "doing", "done", "during",
    "else", "for", "from", "had", "has", "have", "having", "he", "her", "here", "hers", "him",
    "his", "how", "i", "if", "in", "into", "is", "it", "its", "may", "might", "must", "my", "of",
    "on", "onto", "or", "our", "ours", "over", "she", "should", "so", "such", "than", "that",
    "the", "their", "theirs", "them", "then", "there", "these", "they", "this", "those", "to",
    "under", "until", "up", "was", "we", "were", "what", "when", "where", "which", "while", "who",
    "whom", "whose", "why", "will", "with", "without", "would", "you", "your", "yours",
];

fn ident_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[\p{L}_][\p{L}\p{N}_]*").expect("compile IDENT_RE"))
}

#[derive(Clone, Default)]
pub struct PluckTokenizer;

pub struct PluckTokenStream {
    tokens: Vec<Token>,
    cursor: i64, // -1 = before first token
}

impl Tokenizer for PluckTokenizer {
    type TokenStream<'a> = PluckTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        let mut tokens: Vec<Token> = Vec::new();
        for mat in ident_re().find_iter(text) {
            let span = mat.as_str();
            let start = mat.start();
            let end = mat.end();

            let lowered = span.to_lowercase();
            push_token(&mut tokens, &lowered, start, end);

            // Emit camelCase / snake_case parts as separate tokens at the
            // same offset. Searching `handler` therefore matches
            // `HandlerStack`, `my_func` is indexed as both itself + `my`
            // + `func`, etc.
            for part in split_identifier(span) {
                if part == lowered {
                    continue;
                }
                push_token(&mut tokens, &part, start, end);
            }
        }
        PluckTokenStream { tokens, cursor: -1 }
    }
}

impl TokenStream for PluckTokenStream {
    fn advance(&mut self) -> bool {
        self.cursor += 1;
        (self.cursor as usize) < self.tokens.len()
    }

    fn token(&self) -> &Token {
        &self.tokens[self.cursor as usize]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.cursor as usize]
    }
}

fn push_token(tokens: &mut Vec<Token>, text: &str, offset_from: usize, offset_to: usize) {
    if text.is_empty() {
        return;
    }
    let position = tokens.len();
    tokens.push(Token {
        offset_from,
        offset_to,
        position,
        text: text.to_string(),
        position_length: 1,
    });
}

/// Split a single identifier into its parts:
///
///   - `my_func` → `[my, func]`
///   - `HandlerStack` → `[handler, stack]`
///   - `getURLPath` → `[get, url, path]` (acronym handling)
///   - `parse2Int` → `[parse, 2, int]`
///   - `simple` → `[]` (single-part — caller already emits the whole token)
///   - `의존성_그래프` → `[의존성, 그래프]`
///
/// Returned parts are lowercased.
pub fn split_identifier(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }

    // Snake_case is the easy path — split on `_` and trust the result.
    if token.contains('_') {
        let parts: Vec<String> = token
            .split('_')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect();
        return if parts.len() >= 2 { parts } else { Vec::new() };
    }

    // Otherwise treat the token as camel / Pascal / acronym-friendly.
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let curr = chars[i];
        let next = chars.get(i + 1).copied();

        let split = (prev.is_lowercase() && curr.is_uppercase())
            // Acronym boundary: `URLPath` → break before the `P`.
            || (prev.is_uppercase()
                && curr.is_uppercase()
                && next.map(|c| c.is_lowercase()).unwrap_or(false))
            // Letter <-> digit boundaries.
            || (prev.is_alphabetic() && curr.is_ascii_digit())
            || (prev.is_ascii_digit() && curr.is_alphabetic());

        if split {
            let chunk: String = chars[start..i].iter().collect();
            if !chunk.is_empty() {
                parts.push(chunk.to_lowercase());
            }
            start = i;
        }
    }
    if start < chars.len() {
        let chunk: String = chars[start..].iter().collect();
        if !chunk.is_empty() {
            parts.push(chunk.to_lowercase());
        }
    }

    if parts.len() >= 2 {
        parts
    } else {
        Vec::new()
    }
}

/// Tokenize a natural-language BM25 query and drop high-frequency
/// function words. This is deliberately query-side only: source code
/// can use words like `for`, `in`, or `use` as meaningful lexical
/// evidence, so the index keeps every token intact.
pub fn bm25_query_terms(query: &str) -> Vec<String> {
    let mut tokenizer = PluckTokenizer;
    let mut stream = tokenizer.token_stream(query);
    let mut terms = Vec::new();
    while stream.advance() {
        let token = &stream.token().text;
        if is_bm25_stopword(token) || terms.iter().any(|existing| existing == token) {
            continue;
        }
        terms.push(token.clone());
    }
    terms
}

pub fn is_bm25_stopword(token: &str) -> bool {
    BM25_STOPWORDS.binary_search(&token).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_of(text: &str) -> Vec<String> {
        let mut t = PluckTokenizer;
        let mut stream = t.token_stream(text);
        let mut out = Vec::new();
        while stream.advance() {
            out.push(stream.token().text.clone());
        }
        out
    }

    #[test]
    fn snake_case_splits() {
        let parts = split_identifier("my_handler_func");
        assert_eq!(parts, vec!["my", "handler", "func"]);
    }

    #[test]
    fn camel_case_splits() {
        let parts = split_identifier("HandlerStack");
        assert_eq!(parts, vec!["handler", "stack"]);
    }

    #[test]
    fn acronym_boundary() {
        // `getURLPath` should split at the lower→upper boundary AND at
        // the upper→upper→lower acronym boundary.
        let parts = split_identifier("getURLPath");
        assert_eq!(parts, vec!["get", "url", "path"]);
    }

    #[test]
    fn digit_boundary() {
        let parts = split_identifier("parse2Int");
        assert_eq!(parts, vec!["parse", "2", "int"]);
    }

    #[test]
    fn single_part_returns_empty() {
        // Caller already emits the whole token, so an unsplittable
        // identifier returns an empty parts list rather than echoing the
        // whole thing.
        let parts = split_identifier("simple");
        assert!(parts.is_empty());
    }

    #[test]
    fn unicode_snake() {
        // Non-ASCII letters + underscore — the snake-case branch handles
        // it because `_` is the splitter, the letters are just text.
        let parts = split_identifier("의존성_그래프");
        assert_eq!(parts, vec!["의존성", "그래프"]);
    }

    #[test]
    fn tokenizer_emits_whole_plus_parts_at_same_offset() {
        let toks = tokens_of("HandlerStack");
        assert!(toks.contains(&"handlerstack".to_string()));
        assert!(toks.contains(&"handler".to_string()));
        assert!(toks.contains(&"stack".to_string()));
    }

    #[test]
    fn tokenizer_preserves_unicode_identifiers() {
        let toks = tokens_of("Tree-sitter AST 기반 청킹");
        assert!(toks.contains(&"tree".to_string()));
        assert!(toks.contains(&"sitter".to_string()));
        assert!(toks.contains(&"ast".to_string()));
        assert!(toks.contains(&"기반".to_string()));
        assert!(toks.contains(&"청킹".to_string()));
    }

    #[test]
    fn tokenizer_handles_european_letters() {
        let toks = tokens_of("Müller naïve façade");
        assert!(toks.contains(&"müller".to_string()));
        assert!(toks.contains(&"naïve".to_string()));
        assert!(toks.contains(&"façade".to_string()));
    }

    #[test]
    fn tokenizer_drops_pure_punctuation() {
        let toks = tokens_of("() {} ;; --");
        assert!(toks.is_empty());
    }

    #[test]
    fn bm25_query_terms_drop_stopwords_and_dedupe() {
        let terms = bm25_query_terms("How do I validate the user token for the user?");
        assert_eq!(terms, vec!["validate", "user", "token"]);
    }

    #[test]
    fn bm25_query_terms_keep_identifier_parts() {
        let terms = bm25_query_terms("find AuthTokenExpiry in handleLogin");
        assert_eq!(
            terms,
            vec![
                "find",
                "authtokenexpiry",
                "auth",
                "token",
                "expiry",
                "handlelogin",
                "handle",
                "login"
            ]
        );
    }
}
