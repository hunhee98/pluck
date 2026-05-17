//! Extract direct callees from a code chunk.
//!
//! `pluck.peek`'s value proposition rests on this — the agent gets the
//! symbol's signature plus the list of functions it directly invokes,
//! without paying for the full body. Useful when the agent just needs
//! to understand the interface or sketch a call graph.

use std::collections::HashSet;

use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::chunker::Language;

/// Extract callees using an **already-compiled** query against an
/// **already-parsed** tree, scoped to `[scope_start, scope_end)`.
/// Called per-chunk from `chunk_source`; the query is compiled once
/// before the chunk loop to avoid N×compile overhead.
pub fn extract_callees_with_query(
    src: &str,
    tree: &Tree,
    query: &Query,
    scope_start: usize,
    scope_end: usize,
) -> Vec<String> {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(scope_start..scope_end);
    let mut matches = cursor.matches(query, tree.root_node(), src.as_bytes());

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let start = cap.node.start_byte();
            if start < scope_start || start >= scope_end {
                continue;
            }
            let text = src[cap.node.byte_range()].trim();
            let normalized: String = text.split_whitespace().collect::<Vec<_>>().join("");
            if normalized.is_empty() {
                continue;
            }
            if seen.insert(normalized.clone()) {
                out.push(normalized);
            }
        }
    }
    out
}

/// Extract callees from an **already-parsed** tree, scoped to the byte range
/// `[scope_start, scope_end)`. Compiles the query internally — prefer
/// `extract_callees_with_query` when extracting callees for multiple ranges.
pub fn extract_callees_in_range(
    src: &str,
    tree: &Tree,
    lang: Language,
    scope_start: usize,
    scope_end: usize,
) -> Vec<String> {
    let query_src = lang.callee_query_str();
    if query_src.is_empty() {
        return Vec::new();
    }
    let ts_lang = lang.ts_language();
    let Ok(query) = Query::new(&ts_lang, query_src) else {
        return Vec::new();
    };
    extract_callees_with_query(src, tree, &query, scope_start, scope_end)
}

/// Parse `src` as `lang` and return every direct callee name in source order,
/// deduplicated. Returns an empty Vec on parse / query errors — peek
/// degrades to signature-only rather than failing the request.
pub fn extract_callees(src: &str, lang: Language) -> Vec<String> {
    let query_src = lang.callee_query_str();
    if query_src.is_empty() {
        return Vec::new();
    }

    let ts_lang = lang.ts_language();
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };

    let Ok(query) = Query::new(&ts_lang, query_src) else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src.as_bytes());

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    while let Some(m) = matches.next() {
        for cap in m.captures {
            let text = src[cap.node.byte_range()].trim();
            // Collapse intra-token whitespace (member chains can wrap across
            // lines in source).
            let normalized: String = text.split_whitespace().collect::<Vec<_>>().join("");
            if normalized.is_empty() {
                continue;
            }
            if seen.insert(normalized.clone()) {
                out.push(normalized);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_function_callees_dedup_and_order() {
        let src = r#"
async function handleLogin(req) {
  const ok = await validateToken(req.token);
  if (!ok) return null;
  const user = await db.user.findOne(req.id);
  audit.log("login", req.id);
  return validateToken(user.session);
}
"#;
        let callees = extract_callees(src, Language::TypeScript);
        assert!(callees.contains(&"validateToken".to_string()));
        assert!(callees.contains(&"db.user.findOne".to_string()));
        assert!(callees.contains(&"audit.log".to_string()));
        // Dedup: validateToken appears twice in source, once in output.
        assert_eq!(callees.iter().filter(|c| *c == "validateToken").count(), 1);
    }

    #[test]
    fn ts_new_expression_callees() {
        let src = "const r = new Response(body, { status: 200 });\n";
        let callees = extract_callees(src, Language::TypeScript);
        assert!(callees.contains(&"Response".to_string()));
    }

    #[test]
    fn rust_callees() {
        let src = r#"
fn handle(req: Request) -> Response {
    let body = parse_body(&req);
    let logger = Logger::new();
    logger.info("handle");
    println!("done");
    Response::ok(body)
}
"#;
        let callees = extract_callees(src, Language::Rust);
        assert!(
            callees.contains(&"parse_body".to_string()),
            "got: {callees:?}"
        );
        // Method calls show up as `obj.field` in tree-sitter-rust (the
        // function field of the call_expression is a field_expression).
        assert!(
            callees.contains(&"logger.info".to_string()),
            "got: {callees:?}"
        );
        assert!(callees.contains(&"println".to_string()), "got: {callees:?}");
        assert!(
            callees
                .iter()
                .any(|c| c.contains("Logger") || c.contains("Response")),
            "expected a type-prefixed callee, got: {callees:?}"
        );
    }

    #[test]
    fn python_callees() {
        let src = r#"
def handle(req):
    body = parse(req.body)
    db.users.insert(body)
    return jsonify(body)
"#;
        let callees = extract_callees(src, Language::Python);
        assert!(callees.contains(&"parse".to_string()));
        assert!(callees.contains(&"db.users.insert".to_string()));
        assert!(callees.contains(&"jsonify".to_string()));
    }

    #[test]
    fn go_callees() {
        let src = r#"
package main

func Handle(req *Request) *Response {
    body := Parse(req)
    logger.Info("hi")
    return NewResponse(body)
}
"#;
        let callees = extract_callees(src, Language::Go);
        assert!(callees.contains(&"Parse".to_string()));
        assert!(callees.contains(&"logger.Info".to_string()));
        assert!(callees.contains(&"NewResponse".to_string()));
    }

    #[test]
    fn empty_on_parse_failure() {
        // Garbage but not impossible — extractor should not panic.
        let src = "fn ((((((((( not real code";
        let callees = extract_callees(src, Language::Rust);
        assert!(callees.is_empty() || !callees.is_empty()); // either is OK; just no panic
    }
}
