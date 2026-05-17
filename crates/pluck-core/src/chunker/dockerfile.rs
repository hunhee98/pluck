use super::{Chunk, ChunkKind, ChunkResult};

const MAX_DOCKERFILE_CHUNKS: usize = 512;

pub(super) fn chunk_dockerfile_source(src: &str) -> ChunkResult {
    let lines = source_lines(src);
    let line_starts = line_starts(src);
    let instructions = parse_instructions(&lines);
    let mut chunks = Vec::new();

    let mut stage_ordinal = 0usize;
    for (idx, instruction) in instructions.iter().enumerate() {
        if instruction.keyword == "FROM" {
            stage_ordinal += 1;
            let end_byte = instructions
                .iter()
                .skip(idx + 1)
                .find(|candidate| candidate.keyword == "FROM")
                .map(|candidate| candidate.start_byte)
                .unwrap_or(src.len());
            push_dockerfile_chunk(
                &mut chunks,
                src,
                &line_starts,
                stage_symbol(instruction, stage_ordinal),
                instruction.start_byte,
                end_byte,
            );
        }

        push_dockerfile_chunk(
            &mut chunks,
            src,
            &line_starts,
            instruction_symbol(instruction),
            instruction.start_byte,
            instruction.end_byte,
        );
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
struct Instruction {
    keyword: String,
    body: String,
    start_byte: usize,
    end_byte: usize,
}

fn parse_instructions(lines: &[SourceLine<'_>]) -> Vec<Instruction> {
    let mut out = Vec::new();
    let mut idx = 0usize;

    while idx < lines.len() {
        let line = &lines[idx];
        let trimmed = line.text.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            idx += 1;
            continue;
        }

        let start_byte = line.start;
        let mut end_byte = line.end;
        let mut logical = line.text.to_string();
        let mut last = idx;

        while has_line_continuation(lines[last].text) && last + 1 < lines.len() {
            last += 1;
            end_byte = lines[last].end;
            logical.push('\n');
            logical.push_str(lines[last].text);
        }

        let delims = heredoc_delimiters(&logical);
        for delimiter in delims {
            while last + 1 < lines.len() {
                last += 1;
                end_byte = lines[last].end;
                logical.push('\n');
                logical.push_str(lines[last].text);
                if lines[last].text.trim() == delimiter {
                    break;
                }
            }
        }

        if let Some((keyword, body)) = split_instruction_header(&logical) {
            out.push(Instruction {
                keyword,
                body,
                start_byte,
                end_byte,
            });
        }

        idx = last + 1;
    }

    out
}

fn split_instruction_header(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim_start();
    let (first, rest) = split_first_word(trimmed)?;
    let first = first.to_ascii_uppercase();

    if first == "ONBUILD" && !rest.is_empty() {
        let (trigger, body) = split_first_word(rest)?;
        let trigger = trigger.to_ascii_uppercase();
        return Some((format!("ONBUILD {trigger}"), body.to_string()));
    }

    Some((first, rest.to_string()))
}

fn split_first_word(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let split_at = trimmed
        .char_indices()
        .find_map(|(idx, c)| c.is_whitespace().then_some(idx))
        .unwrap_or(trimmed.len());
    let first = &trimmed[..split_at];
    let rest = trimmed[split_at..].trim_start();
    Some((first, rest))
}

fn stage_symbol(instruction: &Instruction, ordinal: usize) -> String {
    if let Some(alias) = from_alias(&instruction.body) {
        return format!("stage: {alias}");
    }

    let image = instruction
        .body
        .split_whitespace()
        .next()
        .unwrap_or("scratch")
        .trim();
    if image.is_empty() {
        format!("stage {ordinal}")
    } else {
        format!("stage {ordinal}: {image}")
    }
}

fn from_alias(body: &str) -> Option<String> {
    let parts: Vec<&str> = body.split_whitespace().collect();
    for window in parts.windows(2) {
        if window[0].eq_ignore_ascii_case("AS") {
            return Some(clean_symbol_token(window[1]));
        }
    }
    None
}

fn instruction_symbol(instruction: &Instruction) -> String {
    if instruction.keyword == "FROM" {
        return from_alias(&instruction.body)
            .map(|alias| format!("FROM {alias}"))
            .unwrap_or_else(|| short_instruction_symbol(&instruction.keyword, &instruction.body));
    }

    if instruction.keyword == "RUN" {
        if let Some(kind) = install_block_kind(&instruction.body) {
            return format!("install: {kind}");
        }
    }

    if matches!(instruction.keyword.as_str(), "COPY" | "ADD")
        && is_dependency_manifest_copy(&instruction.body)
    {
        return format!("deps: {}", short_body(&instruction.body, 8));
    }

    short_instruction_symbol(&instruction.keyword, &instruction.body)
}

fn short_instruction_symbol(keyword: &str, body: &str) -> String {
    let body = short_body(body, 8);
    if body.is_empty() {
        keyword.to_string()
    } else {
        format!("{keyword} {body}")
    }
}

fn short_body(body: &str, max_words: usize) -> String {
    collapse_ascii_ws(body)
        .split_whitespace()
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ")
}

fn install_block_kind(body: &str) -> Option<&'static str> {
    let lower = body.to_ascii_lowercase();
    let patterns = [
        ("apt-get install", "apt-get install"),
        ("apt install", "apt install"),
        ("apk add", "apk add"),
        ("dnf install", "dnf install"),
        ("yum install", "yum install"),
        ("microdnf install", "microdnf install"),
        ("npm ci", "npm ci"),
        ("npm install", "npm install"),
        ("pnpm install", "pnpm install"),
        ("yarn install", "yarn install"),
        ("bun install", "bun install"),
        ("pip install", "pip install"),
        ("pip3 install", "pip install"),
        ("poetry install", "poetry install"),
        ("bundle install", "bundle install"),
        ("cargo chef cook", "cargo chef cook"),
        ("cargo fetch", "cargo fetch"),
        ("go mod download", "go mod download"),
    ];

    patterns
        .iter()
        .find_map(|(needle, label)| lower.contains(needle).then_some(*label))
}

fn is_dependency_manifest_copy(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    [
        "package.json",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lockb",
        "cargo.toml",
        "cargo.lock",
        "go.mod",
        "go.sum",
        "requirements.txt",
        "pyproject.toml",
        "poetry.lock",
        "gemfile",
        "gemfile.lock",
        "pom.xml",
        "build.gradle",
        "gradle.lockfile",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn heredoc_delimiters(logical: &str) -> Vec<String> {
    logical
        .split_whitespace()
        .filter_map(|token| {
            let marker = token
                .strip_prefix("<<-")
                .or_else(|| token.strip_prefix("<<"))?;
            let delimiter = marker
                .trim_matches(['"', '\'', '`'])
                .trim_matches(|c: char| c == ';' || c == '&' || c == '|');
            (!delimiter.is_empty()).then(|| delimiter.to_string())
        })
        .collect()
}

fn has_line_continuation(line: &str) -> bool {
    let trimmed = line.trim_end();
    let slash_count = trimmed.chars().rev().take_while(|c| *c == '\\').count();
    slash_count % 2 == 1
}

fn clean_symbol_token(token: &str) -> String {
    token
        .trim_matches(['"', '\'', '`'])
        .trim_matches(|c: char| c == ',' || c == ';')
        .to_string()
}

fn push_dockerfile_chunk(
    chunks: &mut Vec<Chunk>,
    src: &str,
    line_starts: &[usize],
    symbol: String,
    start_byte: usize,
    end_byte: usize,
) {
    if chunks.len() >= MAX_DOCKERFILE_CHUNKS || symbol.trim().is_empty() {
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
