use super::{Chunk, ChunkKind, ChunkResult};

const MAX_SHELL_CHUNKS: usize = 512;

pub(super) fn chunk_shell_source(src: &str) -> ChunkResult {
    let lines = source_lines(src);
    let line_starts = line_starts(src);
    let mut chunks = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if let Some(title) = section_heading(line.text) {
            let end_byte = next_section_start(&lines, idx + 1).unwrap_or(src.len());
            push_shell_chunk(
                &mut chunks,
                src,
                &line_starts,
                format!("section: {title}"),
                ChunkKind::Module,
                line.start,
                end_byte,
                String::new(),
            );
        }

        if let Some(name) = function_name(line.text) {
            let end_byte = function_end(&lines, idx).unwrap_or(line.end);
            let doc_comment = leading_shell_doc(&lines, idx);
            push_shell_chunk(
                &mut chunks,
                src,
                &line_starts,
                name,
                ChunkKind::Function,
                line.start,
                end_byte,
                doc_comment,
            );
        }
    }

    for case_block in case_blocks(&lines) {
        for arm in case_arms(&lines, &case_block) {
            push_shell_chunk(
                &mut chunks,
                src,
                &line_starts,
                format!(
                    "case: {}",
                    normalize_case_pattern(lines[arm.start_idx].text)
                ),
                ChunkKind::Module,
                lines[arm.start_idx].start,
                arm.end_byte,
                String::new(),
            );
        }
    }

    ChunkResult {
        chunks,
        imports: Vec::new(),
        parse_errors: false,
    }
}

#[derive(Debug)]
struct SourceLine<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

#[derive(Debug)]
struct CaseBlock {
    start_idx: usize,
    end_idx: usize,
}

#[derive(Debug)]
struct CaseArm {
    start_idx: usize,
    end_byte: usize,
}

fn section_heading(line: &str) -> Option<String> {
    if !line.starts_with('#') || line.starts_with("#!") {
        return None;
    }

    let marker_count = line.chars().take_while(|c| *c == '#').count();
    let raw = line[marker_count..].trim();
    let decorated = raw.starts_with("---")
        || raw.starts_with("===")
        || raw.starts_with("***")
        || raw.starts_with("___")
        || raw.ends_with("---")
        || raw.ends_with("===")
        || raw.ends_with("***")
        || raw.ends_with("___");
    if marker_count < 2 && !decorated {
        return None;
    }

    let mut text = raw;
    text = text.trim_matches(|c: char| matches!(c, '-' | '=' | '*' | '_' | ' ' | '\t'));
    let title = collapse_ascii_ws(text);
    if title.len() < 3 || title.contains('=') {
        return None;
    }
    Some(title)
}

fn next_section_start(lines: &[SourceLine<'_>], start_idx: usize) -> Option<usize> {
    lines
        .iter()
        .skip(start_idx)
        .find(|line| section_heading(line.text).is_some())
        .map(|line| line.start)
}

fn function_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("if ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("case ")
    {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        let (name, rest) = split_shell_word(rest)?;
        let rest = rest.trim_start();
        if rest.starts_with("()") || rest.starts_with('{') || rest.is_empty() {
            return Some(clean_name(name));
        }
        return None;
    }

    let (name, rest) = split_shell_word(trimmed)?;
    let rest = rest.trim_start();
    if let Some(after_open) = rest.strip_prefix('(') {
        let after_open = after_open.trim_start();
        if after_open.starts_with(')') {
            return Some(clean_name(name));
        }
    }
    None
}

fn split_shell_word(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let split_at = trimmed
        .char_indices()
        .find_map(|(idx, c)| (c.is_whitespace() || c == '(' || c == '{').then_some(idx))
        .unwrap_or(trimmed.len());
    let word = &trimmed[..split_at];
    if !is_valid_shell_name(word) {
        return None;
    }
    Some((word, &trimmed[split_at..]))
}

fn is_valid_shell_name(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '-') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
}

fn clean_name(name: &str) -> String {
    name.trim_matches(['"', '\'']).to_string()
}

fn function_end(lines: &[SourceLine<'_>], start_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut saw_body = false;

    for line in lines.iter().skip(start_idx) {
        let code = strip_comment(line.text);
        for ch in code.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    saw_body = true;
                }
                '}' if saw_body => {
                    depth -= 1;
                    if depth <= 0 {
                        return Some(line.end);
                    }
                }
                _ => {}
            }
        }
    }

    saw_body.then(|| lines.last().map(|line| line.end).unwrap_or(0))
}

fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn leading_shell_doc(lines: &[SourceLine<'_>], start_idx: usize) -> String {
    let mut row = start_idx;
    let mut out = Vec::new();

    while row > 0 {
        let prev = lines[row - 1].text.trim();
        if prev.is_empty() {
            break;
        }
        if let Some(text) = prev.strip_prefix('#') {
            if prev.starts_with("#!") {
                break;
            }
            out.push(text.trim().to_string());
            row -= 1;
            continue;
        }
        break;
    }

    out.reverse();
    out.into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn case_blocks(lines: &[SourceLine<'_>]) -> Vec<CaseBlock> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let code = strip_comment(line.text).trim();
        if is_case_start(code) {
            stack.push(idx);
        }
        if code == "esac" || code.ends_with(" esac") {
            if let Some(start_idx) = stack.pop() {
                out.push(CaseBlock {
                    start_idx,
                    end_idx: idx,
                });
            }
        }
    }

    out
}

fn is_case_start(code: &str) -> bool {
    code.starts_with("case ") && (code.ends_with(" in") || code.contains(" in "))
}

fn case_arms(lines: &[SourceLine<'_>], block: &CaseBlock) -> Vec<CaseArm> {
    let mut arm_starts = Vec::new();
    for idx in block.start_idx + 1..block.end_idx {
        if is_case_arm_line(lines[idx].text) {
            arm_starts.push(idx);
        }
    }

    let mut out = Vec::new();
    for (pos, start_idx) in arm_starts.iter().enumerate() {
        let end_byte = arm_starts
            .get(pos + 1)
            .map(|idx| lines[*idx].start)
            .unwrap_or(lines[block.end_idx].start);
        out.push(CaseArm {
            start_idx: *start_idx,
            end_byte,
        });
    }
    out
}

fn is_case_arm_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("case ")
        || trimmed == "esac"
        || trimmed.starts_with(";;")
        || trimmed.starts_with(";&")
        || trimmed.starts_with(";;&")
    {
        return false;
    }

    let Some(idx) = trimmed.find(')') else {
        return false;
    };
    let pattern = trimmed[..idx].trim();
    if pattern.is_empty() || pattern.contains("()") || pattern.contains(" (") {
        return false;
    }
    !pattern.contains(char::is_whitespace)
}

fn normalize_case_pattern(line: &str) -> String {
    let trimmed = line.trim_start();
    let pattern = trimmed.split(')').next().unwrap_or(trimmed).trim();
    collapse_ascii_ws(pattern)
}

fn push_shell_chunk(
    chunks: &mut Vec<Chunk>,
    src: &str,
    line_starts: &[usize],
    symbol: String,
    kind: ChunkKind,
    start_byte: usize,
    end_byte: usize,
    doc_comment: String,
) {
    if chunks.len() >= MAX_SHELL_CHUNKS || symbol.trim().is_empty() {
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
        kind,
        start_line,
        end_line,
        start_byte: start as u32,
        end_byte: end as u32,
        doc_comment,
        content,
        signature,
        callees: Vec::new(),
    });
}

fn source_lines(src: &str) -> Vec<SourceLine<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;

    for line in src.split_inclusive('\n') {
        let end = start + line.len();
        let text = line.trim_end_matches(['\r', '\n']);
        out.push(SourceLine { start, end, text });
        start = end;
    }

    if start < src.len() {
        out.push(SourceLine {
            start,
            end: src.len(),
            text: &src[start..],
        });
    }

    out
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

fn collapse_ascii_ws(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}
