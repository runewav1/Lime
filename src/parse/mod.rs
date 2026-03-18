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

// ---- Zig ----

static ZIG_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub\s+|export\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static ZIG_CONST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?::|=)").unwrap()
});
static ZIG_VAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub\s+)?var\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?::|=)").unwrap()
});
static ZIG_TEST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*test\s+"([^"]+)""#).unwrap());
static ZIG_STRUCT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:packed\s+|extern\s+)?struct\b").unwrap()
});
static ZIG_ENUM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*enum\b").unwrap()
});
static ZIG_UNION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:pub\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:packed\s+|extern\s+)?union\b").unwrap()
});
static ZIG_IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\s*(?:pub\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*@import\("#).unwrap()
});

// ---- C ----

static C_FUNC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(?:static\s+|inline\s+|extern\s+)*(?:(?:unsigned|signed|long|short|const|volatile|struct|enum)\s+)*[A-Za-z_][A-Za-z0-9_*\s]*\s+\**([A-Za-z_][A-Za-z0-9_]*)\s*\([^)]*\)\s*\{"
    ).unwrap()
});
static C_STRUCT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:typedef\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static C_ENUM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:typedef\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static C_TYPEDEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*typedef\s+[^;{]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*;").unwrap()
});
static C_DEFINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^#\s*define\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static C_INCLUDE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^#\s*include\s+([<"][^>"]+[>"])"#).unwrap()
});

// ---- C++ (extends C) ----

static CPP_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:template\s*<[^>]*>\s*)?class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static CPP_NAMESPACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*namespace\s+([A-Za-z_][A-Za-z0-9_:]*)").unwrap()
});
static CPP_TEMPLATE_FUNC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*template\s*<[^>]*>\s*(?:static\s+|inline\s+|constexpr\s+)*[A-Za-z_][A-Za-z0-9_*&\s<>:]*\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
});
static CPP_USING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*using\s+([A-Za-z_][A-Za-z0-9_]*)\s*=").unwrap()
});

// ---- Go ----

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
        "zig" => Some("zig"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
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
        "zig" => parse_zig(content, &mut components),
        "c" => parse_c(content, &mut components),
        "cpp" => parse_cpp(content, &mut components),
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

fn parse_zig(content: &str, out: &mut Vec<ParsedComponent>) {
    collect_single_group(content, &ZIG_STRUCT_RE, "struct", out);
    collect_single_group(content, &ZIG_ENUM_RE, "enum", out);
    collect_single_group(content, &ZIG_UNION_RE, "union", out);
    collect_single_group(content, &ZIG_IMPORT_RE, "import", out);
    collect_single_group(content, &ZIG_TEST_RE, "test", out);

    // fn/const/var — but skip entries already captured as struct/enum/union/import
    let type_names: std::collections::HashSet<String> =
        out.iter().map(|c| c.name.clone()).collect();

    for captures in ZIG_FN_RE.captures_iter(content) {
        if let (Some(matched), Some(name)) = (captures.get(0), captures.get(1)) {
            let n = name.as_str().trim();
            if !type_names.contains(n) {
                out.push(ParsedComponent::new("fn", n, matched.start()));
            }
        }
    }
    for captures in ZIG_CONST_RE.captures_iter(content) {
        if let (Some(matched), Some(name)) = (captures.get(0), captures.get(1)) {
            let n = name.as_str().trim();
            if !type_names.contains(n) {
                out.push(ParsedComponent::new("const", n, matched.start()));
            }
        }
    }
    for captures in ZIG_VAR_RE.captures_iter(content) {
        if let (Some(matched), Some(name)) = (captures.get(0), captures.get(1)) {
            let n = name.as_str().trim();
            if !type_names.contains(n) {
                out.push(ParsedComponent::new("var", n, matched.start()));
            }
        }
    }
}

fn parse_c(content: &str, out: &mut Vec<ParsedComponent>) {
    collect_single_group(content, &C_STRUCT_RE, "struct", out);
    collect_single_group(content, &C_ENUM_RE, "enum", out);
    collect_single_group(content, &C_TYPEDEF_RE, "typedef", out);
    collect_single_group(content, &C_DEFINE_RE, "define", out);
    collect_single_group(content, &C_INCLUDE_RE, "include", out);
    collect_single_group(content, &C_FUNC_RE, "fn", out);
}

fn parse_cpp(content: &str, out: &mut Vec<ParsedComponent>) {
    parse_c(content, out);
    collect_single_group(content, &CPP_CLASS_RE, "class", out);
    collect_single_group(content, &CPP_NAMESPACE_RE, "namespace", out);
    collect_single_group(content, &CPP_TEMPLATE_FUNC_RE, "fn", out);
    collect_single_group(content, &CPP_USING_RE, "using", out);
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- detect_language ----

    #[test]
    fn detect_language_existing_languages() {
        assert_eq!(detect_language("rs"), Some("rust"));
        assert_eq!(detect_language("py"), Some("python"));
        assert_eq!(detect_language("go"), Some("go"));
        assert_eq!(detect_language("js"), Some("javascript"));
        assert_eq!(detect_language("ts"), Some("typescript"));
    }

    #[test]
    fn detect_language_zig() {
        assert_eq!(detect_language("zig"), Some("zig"));
    }

    #[test]
    fn detect_language_c() {
        assert_eq!(detect_language("c"), Some("c"));
        assert_eq!(detect_language("h"), Some("c"));
    }

    #[test]
    fn detect_language_cpp() {
        assert_eq!(detect_language("cpp"), Some("cpp"));
        assert_eq!(detect_language("cc"), Some("cpp"));
        assert_eq!(detect_language("cxx"), Some("cpp"));
        assert_eq!(detect_language("hpp"), Some("cpp"));
        assert_eq!(detect_language("hh"), Some("cpp"));
        assert_eq!(detect_language("hxx"), Some("cpp"));
    }

    #[test]
    fn detect_language_unknown() {
        assert_eq!(detect_language("txt"), None);
        assert_eq!(detect_language("md"), None);
    }

    // ---- Zig parsing ----

    #[test]
    fn zig_functions() {
        let src = r#"
pub fn init() void {
}

fn helper(x: u32) u32 {
    return x + 1;
}

export fn entry() void {
}
"#;
        let components = parse_components("zig", src);
        let names: Vec<&str> = components.iter()
            .filter(|c| c.component_type == "fn")
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"init"), "expected 'init' fn, got: {names:?}");
        assert!(names.contains(&"helper"), "expected 'helper' fn, got: {names:?}");
        assert!(names.contains(&"entry"), "expected 'entry' fn, got: {names:?}");
    }

    #[test]
    fn zig_structs_enums_unions() {
        let src = r#"
const Point = struct {
    x: f32,
    y: f32,
};

pub const Color = enum {
    red,
    green,
    blue,
};

const Payload = union {
    int: i64,
    float: f64,
};
"#;
        let components = parse_components("zig", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("struct", "Point")), "expected struct Point, got: {types:?}");
        assert!(types.contains(&("enum", "Color")), "expected enum Color, got: {types:?}");
        assert!(types.contains(&("union", "Payload")), "expected union Payload, got: {types:?}");
    }

    #[test]
    fn zig_consts_vars_imports_tests() {
        let src = r#"
const std = @import("std");
const max_items = 100;
var counter: u32 = 0;
test "basic addition" {
}
"#;
        let components = parse_components("zig", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("import", "std")), "expected import std, got: {types:?}");
        assert!(types.contains(&("const", "max_items")), "expected const max_items, got: {types:?}");
        assert!(types.contains(&("var", "counter")), "expected var counter, got: {types:?}");
        assert!(types.contains(&("test", "basic addition")), "expected test, got: {types:?}");
    }

    // ---- C parsing ----

    #[test]
    fn c_functions() {
        let src = r#"
int main(int argc, char *argv[]) {
    return 0;
}

static void helper(void) {
}
"#;
        let components = parse_components("c", src);
        let fns: Vec<&str> = components.iter()
            .filter(|c| c.component_type == "fn")
            .map(|c| c.name.as_str())
            .collect();
        assert!(fns.contains(&"main"), "expected 'main', got: {fns:?}");
        assert!(fns.contains(&"helper"), "expected 'helper', got: {fns:?}");
    }

    #[test]
    fn c_structs_enums_typedefs() {
        let src = r#"
struct Point {
    int x;
    int y;
};

enum Color { RED, GREEN, BLUE };

typedef unsigned long ulong;
"#;
        let components = parse_components("c", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("struct", "Point")), "expected struct Point, got: {types:?}");
        assert!(types.contains(&("enum", "Color")), "expected enum Color, got: {types:?}");
        assert!(types.contains(&("typedef", "ulong")), "expected typedef ulong, got: {types:?}");
    }

    #[test]
    fn c_defines_and_includes() {
        let src = r#"
#include <stdio.h>
#include "myheader.h"
#define MAX_SIZE 1024
#define MIN(a, b) ((a) < (b) ? (a) : (b))
"#;
        let components = parse_components("c", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("define", "MAX_SIZE")), "expected define MAX_SIZE, got: {types:?}");
        assert!(types.contains(&("define", "MIN")), "expected define MIN, got: {types:?}");
        let includes: Vec<&str> = components.iter()
            .filter(|c| c.component_type == "include")
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(includes.len(), 2, "expected 2 includes, got: {includes:?}");
    }

    // ---- C++ parsing ----

    #[test]
    fn cpp_classes_and_namespaces() {
        let src = r#"
namespace mylib {

class Widget {
public:
    void draw();
};

}
"#;
        let components = parse_components("cpp", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("namespace", "mylib")), "expected namespace mylib, got: {types:?}");
        assert!(types.contains(&("class", "Widget")), "expected class Widget, got: {types:?}");
    }

    #[test]
    fn cpp_template_functions_and_using() {
        let src = r#"
template<typename T>
T max_val(T a, T b) {
    return a > b ? a : b;
}

using StringVec = std::vector<std::string>;
"#;
        let components = parse_components("cpp", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("fn", "max_val")), "expected fn max_val, got: {types:?}");
        assert!(types.contains(&("using", "StringVec")), "expected using StringVec, got: {types:?}");
    }

    #[test]
    fn cpp_inherits_c_parsing() {
        let src = r#"
#include <iostream>
#define PI 3.14159

struct Vec2 {
    float x, y;
};

int main() {
}
"#;
        let components = parse_components("cpp", src);
        let types: Vec<(&str, &str)> = components.iter()
            .map(|c| (c.component_type.as_str(), c.name.as_str()))
            .collect();
        assert!(types.contains(&("include", "<iostream>")), "expected include <iostream>, got: {types:?}");
        assert!(types.contains(&("define", "PI")), "expected define PI, got: {types:?}");
        assert!(types.contains(&("struct", "Vec2")), "expected struct Vec2, got: {types:?}");
    }
}
