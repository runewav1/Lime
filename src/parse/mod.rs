use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::Regex;

/// Parsed high-level component discovered in a source file.
#[derive(Debug, Clone)]
pub struct ParsedComponent {
    /// Component category (for example `struct`, `fn`, `class`).
    pub component_type: String,
    /// Component identifier as declared in source.
    pub name: String,
    /// 1-indexed start line in source file.
    pub start_line: usize,
    /// 1-indexed end line in source file.
    pub end_line: usize,
    start_offset: usize,
}

impl ParsedComponent {
    fn new(
        component_type: impl Into<String>,
        name: impl Into<String>,
        start_offset: usize,
    ) -> Self {
        Self {
            component_type: component_type.into(),
            name: name.into(),
            start_line: 1,
            end_line: 1,
            start_offset,
        }
    }
}

static RUST_STRUCT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RUST_ENUM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RUST_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub(?:\([^\)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")
        .unwrap()
});
static RUST_TRAIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RUST_IMPL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*impl(?:<[^\n>]+>\s*)?([^\{\n]+)").unwrap());
static RUST_MOD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RUST_USE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*use\s+([^;]+);").unwrap());

static JS_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:export\s+)?(?:default\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap()
});
static JS_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)",
    )
    .unwrap()
});
static JS_VARIABLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:export\s+)?(const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=").unwrap()
});
static TS_INTERFACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:export\s+)?interface\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap()
});
static TS_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=").unwrap()
});
static JS_EXPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*export\s*\{\s*([^}]+)\s*\}").unwrap());

static PY_CLASS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());
static PY_ASYNC_DEF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*async\s+def\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());
static PY_DEF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());
static PY_IMPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*import\s+([^\n#]+)").unwrap());
static PY_FROM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*from\s+([A-Za-z0-9_\.]+)\s+import\s+([^\n#]+)").unwrap());

static GO_STRUCT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+struct\b").unwrap());
static GO_INTERFACE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+interface\b").unwrap());
static GO_FUNC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*func\s*(?:\([^\)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
});
static GO_TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+").unwrap());
static GO_CONST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*const\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());
static GO_VAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*var\s+([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());

/// Detects the language key used by Lime for a file extension.
pub fn detect_language(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "py" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

/// Parses components from a source file content for the given language.
pub fn parse_components(language: &str, content: &str) -> Vec<ParsedComponent> {
    let mut components = Vec::new();

    match language {
        "rust" => parse_rust(content, &mut components),
        "javascript" | "typescript" => parse_js_ts(content, language, &mut components),
        "python" => parse_python(content, &mut components),
        "go" => parse_go(content, &mut components),
        _ => {}
    }

    finalize_line_spans(content, components)
}

fn parse_rust(content: &str, out: &mut Vec<ParsedComponent>) {
    collect_single_group(content, &RUST_STRUCT_RE, "struct", out);
    collect_single_group(content, &RUST_ENUM_RE, "enum", out);
    collect_single_group(content, &RUST_FN_RE, "fn", out);
    collect_single_group(content, &RUST_TRAIT_RE, "trait", out);
    collect_single_group(content, &RUST_MOD_RE, "mod", out);
    collect_single_group(content, &RUST_USE_RE, "use", out);

    for captures in RUST_IMPL_RE.captures_iter(content) {
        if let (Some(matched), Some(name)) = (captures.get(0), captures.get(1)) {
            let impl_target = name.as_str().trim().replace("\n", " ");
            out.push(ParsedComponent::new("impl", impl_target, matched.start()));
        }
    }
}

fn parse_js_ts(content: &str, language: &str, out: &mut Vec<ParsedComponent>) {
    collect_single_group(content, &JS_CLASS_RE, "class", out);
    collect_single_group(content, &JS_FUNCTION_RE, "function", out);

    for captures in JS_VARIABLE_RE.captures_iter(content) {
        if let (Some(matched), Some(kind), Some(name)) =
            (captures.get(0), captures.get(1), captures.get(2))
        {
            out.push(ParsedComponent::new(
                kind.as_str().trim(),
                name.as_str().trim(),
                matched.start(),
            ));
        }
    }

    if language == "typescript" {
        collect_single_group(content, &TS_INTERFACE_RE, "interface", out);
        collect_single_group(content, &TS_TYPE_RE, "type", out);
    }

    for captures in JS_EXPORT_RE.captures_iter(content) {
        if let (Some(matched), Some(list)) = (captures.get(0), captures.get(1)) {
            for part in list.as_str().split(',') {
                let raw = part.trim();
                if raw.is_empty() {
                    continue;
                }

                let name = raw.split_whitespace().next().unwrap_or(raw);
                out.push(ParsedComponent::new("export", name, matched.start()));
            }
        }
    }
}

fn parse_python(content: &str, out: &mut Vec<ParsedComponent>) {
    collect_single_group(content, &PY_CLASS_RE, "class", out);
    collect_single_group(content, &PY_ASYNC_DEF_RE, "async def", out);
    collect_single_group(content, &PY_DEF_RE, "def", out);
    collect_single_group(content, &PY_IMPORT_RE, "import", out);

    for captures in PY_FROM_RE.captures_iter(content) {
        if let (Some(matched), Some(module), Some(targets)) =
            (captures.get(0), captures.get(1), captures.get(2))
        {
            let value = format!("{} -> {}", module.as_str().trim(), targets.as_str().trim());
            out.push(ParsedComponent::new("from", value, matched.start()));
        }
    }
}

fn parse_go(content: &str, out: &mut Vec<ParsedComponent>) {
    collect_single_group(content, &GO_STRUCT_RE, "struct", out);
    collect_single_group(content, &GO_INTERFACE_RE, "interface", out);
    collect_single_group(content, &GO_FUNC_RE, "func", out);
    collect_single_group(content, &GO_CONST_RE, "const", out);
    collect_single_group(content, &GO_VAR_RE, "var", out);

    for captures in GO_TYPE_RE.captures_iter(content) {
        if let (Some(matched), Some(name_match)) = (captures.get(0), captures.get(1)) {
            let line = extract_line(content, matched.start());
            if line.contains(" struct") || line.contains(" interface") {
                continue;
            }

            out.push(ParsedComponent::new(
                "type",
                name_match.as_str().trim(),
                matched.start(),
            ));
        }
    }
}

fn collect_single_group(
    content: &str,
    regex: &Regex,
    component_type: &str,
    out: &mut Vec<ParsedComponent>,
) {
    for captures in regex.captures_iter(content) {
        if let (Some(matched), Some(name)) = (captures.get(0), captures.get(1)) {
            out.push(ParsedComponent::new(
                component_type,
                name.as_str().trim(),
                matched.start(),
            ));
        }
    }
}

fn finalize_line_spans(
    content: &str,
    mut components: Vec<ParsedComponent>,
) -> Vec<ParsedComponent> {
    if components.is_empty() {
        return components;
    }

    components.sort_by(|left, right| {
        if left.start_offset == right.start_offset {
            return left.component_type.cmp(&right.component_type);
        }

        if left.start_offset < right.start_offset {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    });

    let line_starts = line_start_offsets(content);
    let total_lines = line_starts.len().max(1);

    for component in &mut components {
        component.start_line = line_for_offset(component.start_offset, &line_starts);
    }

    for index in 0..components.len() {
        let current_line = components[index].start_line;
        let next_line = components
            .get(index + 1)
            .map(|next| next.start_line.saturating_sub(1))
            .unwrap_or(total_lines);

        let detected_end_line =
            detect_component_end_line(content, components[index].start_offset, &line_starts)
                .unwrap_or(next_line);
        let bounded_end = detected_end_line.min(next_line).max(current_line);
        let end_line = bounded_end.max(current_line);
        components[index].end_line = end_line;
    }

    dedupe_components(components)
}

fn detect_component_end_line(
    content: &str,
    start_offset: usize,
    line_starts: &[usize],
) -> Option<usize> {
    let scope = content.get(start_offset..)?;
    let mut line = line_for_offset(start_offset, line_starts);

    let mut seen_open_brace = false;
    let mut brace_depth = 0usize;

    let mut chars = scope.chars().peekable();
    let mut in_string: Option<char> = None;
    let mut escape = false;
    let mut in_block_comment = false;
    let mut in_line_comment = false;

    while let Some(character) = chars.next() {
        if character == '\n' {
            line += 1;
            in_line_comment = false;
            escape = false;
            continue;
        }

        if in_line_comment {
            continue;
        }

        if in_block_comment {
            if character == '*' && matches!(chars.peek(), Some('/')) {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }

        if let Some(delimiter) = in_string {
            if escape {
                escape = false;
                continue;
            }

            if character == '\\' {
                escape = true;
                continue;
            }

            if character == delimiter {
                in_string = None;
            }
            continue;
        }

        if character == '/' && matches!(chars.peek(), Some('/')) {
            chars.next();
            in_line_comment = true;
            continue;
        }

        if character == '/' && matches!(chars.peek(), Some('*')) {
            chars.next();
            in_block_comment = true;
            continue;
        }

        match character {
            '"' | '\'' | '`' => {
                in_string = Some(character);
            }
            '{' => {
                seen_open_brace = true;
                brace_depth += 1;
            }
            '}' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    if seen_open_brace && brace_depth == 0 {
                        return Some(line);
                    }
                }
            }
            ';' => {
                if !seen_open_brace || brace_depth == 0 {
                    return Some(line);
                }
            }
            _ => {}
        }
    }

    None
}

fn dedupe_components(components: Vec<ParsedComponent>) -> Vec<ParsedComponent> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(components.len());

    for component in components {
        let key = format!(
            "{}|{}|{}|{}",
            component.component_type, component.name, component.start_line, component.end_line
        );
        if seen.insert(key) {
            result.push(component);
        }
    }

    result
}

fn line_start_offsets(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn line_for_offset(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(index) => index + 1,
        Err(insert_at) => insert_at,
    }
}

fn extract_line(content: &str, start_offset: usize) -> &str {
    let line_start = content[..start_offset]
        .rfind('\n')
        .map(|position| position + 1)
        .unwrap_or(0);
    let line_end = content[start_offset..]
        .find('\n')
        .map(|relative| start_offset + relative)
        .unwrap_or(content.len());
    &content[line_start..line_end]
}
