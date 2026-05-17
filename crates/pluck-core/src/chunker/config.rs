use super::{Chunk, ChunkKind, ChunkResult, Language};

const MAX_CONFIG_CHUNKS: usize = 512;

pub(super) fn chunk_config_source(src: &str, lang: Language) -> ChunkResult {
    match lang {
        Language::Json => chunk_json(src),
        Language::Yaml => chunk_yaml(src),
        Language::Toml => chunk_toml(src),
        _ => ChunkResult::default(),
    }
}

fn chunk_json(src: &str) -> ChunkResult {
    let mut parser = JsonParser::new(src);
    let parsed = parser.parse();
    let parse_errors = !parsed || serde_json::from_str::<serde_json::Value>(src).is_err();
    ChunkResult {
        chunks: parser.chunks,
        imports: Vec::new(),
        parse_errors,
    }
}

fn chunk_yaml(src: &str) -> ChunkResult {
    let parse_errors = serde_yaml::from_str::<serde_yaml::Value>(src).is_err();
    let lines = source_lines(src);
    let line_starts = line_starts(src);
    let mut chunks = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let Some((indent, body)) = yaml_body(line.text) else {
            continue;
        };
        let Some((key, rest)) = split_yaml_key(body) else {
            continue;
        };
        let key = clean_path_segment(key);
        if key.is_empty() {
            continue;
        }

        while stack
            .last()
            .map(|(stack_indent, _)| *stack_indent >= indent)
            .unwrap_or(false)
        {
            stack.pop();
        }

        let symbol = join_parent_path(stack.last().map(|(_, path)| path.as_str()), &key);
        let end_byte = yaml_chunk_end(&lines, idx, indent, src.len());
        push_config_chunk(
            &mut chunks,
            src,
            &line_starts,
            symbol.clone(),
            line.start,
            end_byte,
        );

        let value = rest.trim();
        if value.is_empty() || value.starts_with('#') {
            stack.push((indent, symbol));
        }
    }

    ChunkResult {
        chunks,
        imports: Vec::new(),
        parse_errors,
    }
}

fn chunk_toml(src: &str) -> ChunkResult {
    let parse_errors = src.parse::<toml_edit::DocumentMut>().is_err();
    let lines = source_lines(src);
    let line_starts = line_starts(src);
    let mut chunks = Vec::new();
    let mut section: Vec<String> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(parts) = parse_toml_header(trimmed) {
            if !parts.is_empty() {
                section = parts;
                let symbol = join_path(&section);
                let end_byte = toml_section_end(&lines, idx, src.len());
                push_config_chunk(&mut chunks, src, &line_starts, symbol, line.start, end_byte);
            }
            continue;
        }

        let Some(eq_idx) = find_toml_key_separator(trimmed) else {
            continue;
        };
        let key = &trimmed[..eq_idx];
        let mut key_parts = split_toml_path(key);
        if key_parts.is_empty() {
            continue;
        }

        let mut path = section.clone();
        path.append(&mut key_parts);
        let symbol = join_path(&path);
        let end_byte = toml_key_end(&lines, idx, src.len());
        push_config_chunk(&mut chunks, src, &line_starts, symbol, line.start, end_byte);
    }

    ChunkResult {
        chunks,
        imports: Vec::new(),
        parse_errors,
    }
}

struct JsonParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line_starts: Vec<usize>,
    chunks: Vec<Chunk>,
    failed: bool,
}

impl<'a> JsonParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line_starts: line_starts(src),
            chunks: Vec::new(),
            failed: false,
        }
    }

    fn parse(&mut self) -> bool {
        let mut path = Vec::new();
        self.skip_ws();
        let ok = self.parse_value(&mut path).is_some();
        self.skip_ws();
        ok && self.pos == self.bytes.len() && !self.failed
    }

    fn parse_value(&mut self, path: &mut Vec<String>) -> Option<(usize, usize)> {
        self.skip_ws();
        let start = self.pos;
        let byte = *self.bytes.get(self.pos)?;
        match byte {
            b'{' => self.parse_object(path),
            b'[' => self.parse_array(path),
            b'"' => {
                self.parse_string_token()?;
                Some((start, self.pos))
            }
            b'-' | b'0'..=b'9' => {
                self.parse_number();
                Some((start, self.pos))
            }
            b't' => self.parse_literal(b"true").map(|_| (start, self.pos)),
            b'f' => self.parse_literal(b"false").map(|_| (start, self.pos)),
            b'n' => self.parse_literal(b"null").map(|_| (start, self.pos)),
            _ => {
                self.failed = true;
                None
            }
        }
    }

    fn parse_object(&mut self, path: &mut Vec<String>) -> Option<(usize, usize)> {
        let start = self.pos;
        self.pos += 1;
        self.skip_ws();
        if self.consume(b'}') {
            return Some((start, self.pos));
        }

        loop {
            self.skip_ws();
            let (key_start, _, key) = self.parse_string_token()?;
            self.skip_ws();
            if !self.consume(b':') {
                self.failed = true;
                return None;
            }

            path.push(key);
            let (_, value_end) = self.parse_value(path)?;
            let symbol = json_path_symbol(path);
            self.push_chunk(symbol, key_start, value_end);
            path.pop();

            self.skip_ws();
            if self.consume(b',') {
                continue;
            }
            if self.consume(b'}') {
                return Some((start, self.pos));
            }
            self.failed = true;
            return None;
        }
    }

    fn parse_array(&mut self, path: &mut Vec<String>) -> Option<(usize, usize)> {
        let start = self.pos;
        self.pos += 1;
        self.skip_ws();
        if self.consume(b']') {
            return Some((start, self.pos));
        }

        let mut index = 0usize;
        loop {
            path.push(format!("[{index}]"));
            self.parse_value(path)?;
            path.pop();
            index += 1;

            self.skip_ws();
            if self.consume(b',') {
                continue;
            }
            if self.consume(b']') {
                return Some((start, self.pos));
            }
            self.failed = true;
            return None;
        }
    }

    fn parse_string_token(&mut self) -> Option<(usize, usize, String)> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            self.failed = true;
            return None;
        }
        let start = self.pos;
        self.pos += 1;

        let mut escaped = false;
        while let Some(&byte) = self.bytes.get(self.pos) {
            if escaped {
                escaped = false;
                self.pos += 1;
                continue;
            }
            match byte {
                b'\\' => {
                    escaped = true;
                    self.pos += 1;
                }
                b'"' => {
                    self.pos += 1;
                    let raw = &self.src[start..self.pos];
                    let value = serde_json::from_str::<String>(raw)
                        .unwrap_or_else(|_| raw.trim_matches('"').to_string());
                    return Some((start, self.pos, value));
                }
                _ => self.pos += 1,
            }
        }

        self.failed = true;
        None
    }

    fn parse_number(&mut self) {
        while matches!(
            self.bytes.get(self.pos),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.pos += 1;
        }
    }

    fn parse_literal(&mut self, literal: &[u8]) -> Option<()> {
        if self.bytes.get(self.pos..self.pos + literal.len()) == Some(literal) {
            self.pos += literal.len();
            Some(())
        } else {
            self.failed = true;
            None
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn push_chunk(&mut self, symbol: String, start_byte: usize, end_byte: usize) {
        push_config_chunk(
            &mut self.chunks,
            self.src,
            &self.line_starts,
            symbol,
            start_byte,
            end_byte,
        );
    }
}

#[derive(Debug)]
struct SourceLine<'a> {
    start: usize,
    text: &'a str,
}

fn source_lines(src: &str) -> Vec<SourceLine<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;

    for line in src.split_inclusive('\n') {
        let end = start + line.len();
        let text = line.trim_end_matches(['\r', '\n']);
        out.push(SourceLine { start, text });
        start = end;
    }

    if start < src.len() {
        out.push(SourceLine {
            start,
            text: &src[start..],
        });
    }

    out
}

fn push_config_chunk(
    chunks: &mut Vec<Chunk>,
    src: &str,
    line_starts: &[usize],
    symbol: String,
    start_byte: usize,
    end_byte: usize,
) {
    if chunks.len() >= MAX_CONFIG_CHUNKS || symbol.trim().is_empty() {
        return;
    }

    let start = start_byte.min(src.len());
    let mut end = end_byte.min(src.len());
    while end > start && src.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end <= start {
        return;
    }

    let content = src[start..end].to_string();
    let start_line = line_for_byte(line_starts, start) as u32 + 1;
    let end_line = line_for_byte(line_starts, end.saturating_sub(1)) as u32 + 1;
    let signature = content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| collapse_ascii_ws(line.trim_end()))
        .unwrap_or_default();

    chunks.push(Chunk {
        symbol,
        kind: ChunkKind::Module,
        start_line,
        end_line,
        start_byte: start as u32,
        end_byte: end as u32,
        doc_comment: String::new(),
        content,
        signature,
        callees: Vec::new(),
    });
}

fn line_starts(src: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, byte) in src.bytes().enumerate() {
        if byte == b'\n' && idx + 1 < src.len() {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_for_byte(line_starts: &[usize], byte: usize) -> usize {
    match line_starts.binary_search(&byte) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    }
}

fn yaml_body(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed == "---"
        || trimmed == "..."
        || trimmed.starts_with("%YAML")
    {
        return None;
    }

    let indent = line.len() - trimmed.len();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some((indent + 2, rest.trim_start()));
    }
    if trimmed == "-" {
        return None;
    }

    Some((indent, trimmed))
}

fn split_yaml_key(body: &str) -> Option<(&str, &str)> {
    let idx = find_yaml_key_separator(body)?;
    Some((body[..idx].trim(), body[idx + 1..].trim_start()))
}

fn find_yaml_key_separator(body: &str) -> Option<usize> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    for (idx, ch) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if double => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ':' if !single && !double => {
                let next = body[idx + ch.len_utf8()..].chars().next();
                if next.map(|c| c.is_whitespace()).unwrap_or(true) {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }

    None
}

fn yaml_chunk_end(
    lines: &[SourceLine<'_>],
    start_idx: usize,
    indent: usize,
    src_len: usize,
) -> usize {
    for line in lines.iter().skip(start_idx + 1) {
        let Some((next_indent, _)) = yaml_body(line.text) else {
            continue;
        };
        if next_indent <= indent {
            return line.start;
        }
    }
    src_len
}

fn parse_toml_header(trimmed: &str) -> Option<Vec<String>> {
    let array = trimmed.starts_with("[[");
    let body = if array {
        trimmed.strip_prefix("[[")?.split("]]").next()?
    } else {
        trimmed.strip_prefix('[')?.split(']').next()?
    };
    let mut parts = split_toml_path(body);
    if array {
        if let Some(last) = parts.last_mut() {
            last.push_str("[]");
        }
    }
    Some(parts)
}

fn toml_section_end(lines: &[SourceLine<'_>], start_idx: usize, src_len: usize) -> usize {
    for line in lines.iter().skip(start_idx + 1) {
        if parse_toml_header(line.text.trim_start()).is_some() {
            return line.start;
        }
    }
    src_len
}

fn toml_key_end(lines: &[SourceLine<'_>], start_idx: usize, src_len: usize) -> usize {
    let mut balance = toml_bracket_balance(lines[start_idx].text);
    for line in lines.iter().skip(start_idx + 1) {
        let trimmed = line.text.trim_start();
        if balance <= 0
            && (parse_toml_header(trimmed).is_some() || find_toml_key_separator(trimmed).is_some())
        {
            return line.start;
        }
        balance += toml_bracket_balance(line.text);
    }
    src_len
}

fn find_toml_key_separator(input: &str) -> Option<usize> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if double => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '=' if !single && !double => return Some(idx),
            _ => {}
        }
    }

    None
}

fn split_toml_path(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if double => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '.' if !single && !double => {
                push_clean_part(&mut parts, &input[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    push_clean_part(&mut parts, &input[start..]);
    parts
}

fn push_clean_part(parts: &mut Vec<String>, raw: &str) {
    let part = clean_path_segment(raw);
    if !part.is_empty() {
        parts.push(part);
    }
}

fn toml_bracket_balance(input: &str) -> i32 {
    let mut balance = 0i32;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if double => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' | '{' if !single && !double => balance += 1,
            ']' | '}' if !single && !double => balance -= 1,
            _ => {}
        }
    }

    balance
}

fn clean_path_segment(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .trim();
    collapse_ascii_ws(trimmed)
}

fn join_parent_path(parent: Option<&str>, key: &str) -> String {
    match parent {
        Some(parent) if !parent.is_empty() => format!("{parent}.{key}"),
        _ => key.to_string(),
    }
}

fn join_path(parts: &[String]) -> String {
    parts.join(".")
}

fn json_path_symbol(path: &[String]) -> String {
    let mut out = String::new();
    for part in path {
        if part.starts_with('[') {
            out.push_str(part);
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(part);
        }
    }
    out
}

fn collapse_ascii_ws(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
