use std::collections::HashMap;

use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Chunk representation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Chunk {
    pub component_id: String,
    pub name: String,
    pub chunk_type: ChunkKind,
    pub language: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: String,
    pub body: String,
    pub docstring: Option<String>,
    pub content_hash: String,
}

impl Chunk {
    pub fn embedding_document(&self) -> String {
        let mut doc = format!(
            "{} {} {} ({})",
            self.language,
            self.chunk_type.as_str(),
            self.name,
            self.file,
        );

        if let Some(ds) = &self.docstring {
            if !ds.is_empty() {
                doc.push_str("\n\n");
                doc.push_str(ds);
            }
        }

        if !self.signature.is_empty() {
            doc.push_str("\n\n");
            doc.push_str(&self.signature);
        }

        if !self.body.is_empty() {
            const MAX_BODY_CHARS: usize = 2048;
            doc.push_str("\n\n");
            if self.body.len() <= MAX_BODY_CHARS {
                doc.push_str(&self.body);
            } else {
                doc.push_str(&self.body[..MAX_BODY_CHARS]);
                doc.push_str("\n...");
            }
        }

        doc
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Impl,
    Module,
    TypeAlias,
}

impl ChunkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChunkKind::Function => "fn",
            ChunkKind::Method => "method",
            ChunkKind::Class => "class",
            ChunkKind::Struct => "struct",
            ChunkKind::Enum => "enum",
            ChunkKind::Trait => "trait",
            ChunkKind::Interface => "interface",
            ChunkKind::Impl => "impl",
            ChunkKind::Module => "mod",
            ChunkKind::TypeAlias => "type",
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level API
// ---------------------------------------------------------------------------

pub fn extract_chunks(
    language: &str,
    file: &str,
    source: &str,
    component_ids: &HashMap<(String, String, usize), String>,
) -> Vec<Chunk> {
    let Some(mut parser) = create_parser(language) else {
        return Vec::new();
    };

    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    let mut chunks = Vec::new();
    collect_chunks(
        root,
        source_bytes,
        language,
        file,
        component_ids,
        &mut chunks,
    );
    chunks
}

fn create_parser(language: &str) -> Option<Parser> {
    let mut parser = Parser::new();
    let lang = match language {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    };
    parser.set_language(&lang).ok()?;
    Some(parser)
}

// ---------------------------------------------------------------------------
// AST walker
// ---------------------------------------------------------------------------

fn collect_chunks(
    node: Node,
    source: &[u8],
    language: &str,
    file: &str,
    component_ids: &HashMap<(String, String, usize), String>,
    chunks: &mut Vec<Chunk>,
) {
    if let Some((kind, name)) = classify_node(node, source, language) {
        let start_line = node.start_position().row + 1;
        let end_line = node.end_position().row + 1;
        let body = node_text(node, source);
        let signature = extract_signature(node, source, language);
        let docstring = extract_docstring(node, source, language);

        let lookup_type = kind.as_str().to_string();
        let comp_id = component_ids
            .get(&(lookup_type.clone(), name.clone(), start_line))
            .or_else(|| {
                component_ids
                    .iter()
                    .find(|((t, n, l), _)| {
                        n == &name && (t == &lookup_type || compatible_types(t, &lookup_type))
                            && l.abs_diff(start_line) <= 2
                    })
                    .map(|(_, id)| id)
            })
            .cloned()
            .unwrap_or_default();

        let full_text = format!("{}\n{}\n{}", signature, docstring.as_deref().unwrap_or(""), body);
        let content_hash = blake3::hash(full_text.as_bytes()).to_hex()[..16].to_string();

        if !comp_id.is_empty() {
            chunks.push(Chunk {
                component_id: comp_id,
                name,
                chunk_type: kind,
                language: language.to_string(),
                file: file.to_string(),
                start_line,
                end_line,
                signature,
                body,
                docstring,
                content_hash,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_chunks(child, source, language, file, component_ids, chunks);
    }
}

fn compatible_types(index_type: &str, chunk_type: &str) -> bool {
    matches!(
        (index_type, chunk_type),
        ("fn", "method") | ("method", "fn")
            | ("function", "fn") | ("fn", "function")
            | ("func", "fn") | ("fn", "func")
            | ("func", "method") | ("method", "func")
            | ("def", "fn") | ("fn", "def")
            | ("def", "method") | ("method", "def")
            | ("async_def", "fn") | ("fn", "async_def")
            | ("async_def", "method") | ("method", "async_def")
            | ("class", "struct") | ("struct", "class")
            | ("const", "fn") | ("let", "fn") | ("var", "fn")
            | ("type ... struct", "struct") | ("struct", "type ... struct")
            | ("type ... interface", "interface") | ("interface", "type ... interface")
            | ("impl", "impl") | ("impl for", "impl")
    )
}

// ---------------------------------------------------------------------------
// Node classification per language
// ---------------------------------------------------------------------------

fn classify_node<'a>(node: Node<'a>, source: &[u8], language: &str) -> Option<(ChunkKind, String)> {
    match language {
        "rust" => classify_rust(node, source),
        "javascript" | "typescript" => classify_js_ts(node, source),
        "python" => classify_python(node, source),
        "go" => classify_go(node, source),
        _ => None,
    }
}

fn classify_rust(node: Node, source: &[u8]) -> Option<(ChunkKind, String)> {
    match node.kind() {
        "function_item" => {
            let name = child_field_text(node, "name", source)?;
            let kind = if has_ancestor(node, "impl_item") {
                ChunkKind::Method
            } else {
                ChunkKind::Function
            };
            Some((kind, name))
        }
        "struct_item" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Struct, name))
        }
        "enum_item" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Enum, name))
        }
        "trait_item" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Trait, name))
        }
        "impl_item" => {
            let name = impl_name(node, source)?;
            Some((ChunkKind::Impl, name))
        }
        "mod_item" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Module, name))
        }
        _ => None,
    }
}

fn classify_js_ts(node: Node, source: &[u8]) -> Option<(ChunkKind, String)> {
    match node.kind() {
        "function_declaration" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Function, name))
        }
        "class_declaration" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Class, name))
        }
        "method_definition" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Method, name))
        }
        "interface_declaration" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Interface, name))
        }
        "type_alias_declaration" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::TypeAlias, name))
        }
        "lexical_declaration" | "variable_declaration" => {
            let name = extract_var_fn_name(node, source)?;
            Some((ChunkKind::Function, name))
        }
        "export_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(result) = classify_js_ts(child, source) {
                    return Some(result);
                }
            }
            None
        }
        _ => None,
    }
}

fn classify_python(node: Node, source: &[u8]) -> Option<(ChunkKind, String)> {
    match node.kind() {
        "function_definition" => {
            let name = child_field_text(node, "name", source)?;
            let kind = if has_ancestor(node, "class_definition") {
                ChunkKind::Method
            } else {
                ChunkKind::Function
            };
            Some((kind, name))
        }
        "class_definition" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Class, name))
        }
        "decorated_definition" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(result) = classify_python(child, source) {
                    return Some(result);
                }
            }
            None
        }
        _ => None,
    }
}

fn classify_go(node: Node, source: &[u8]) -> Option<(ChunkKind, String)> {
    match node.kind() {
        "function_declaration" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Function, name))
        }
        "method_declaration" => {
            let name = child_field_text(node, "name", source)?;
            Some((ChunkKind::Method, name))
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_spec" {
                    let name = child_field_text(child, "name", source)?;
                    let type_node = child.child_by_field_name("type")?;
                    let kind = match type_node.kind() {
                        "struct_type" => ChunkKind::Struct,
                        "interface_type" => ChunkKind::Interface,
                        _ => ChunkKind::TypeAlias,
                    };
                    return Some((kind, name));
                }
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Signature extraction
// ---------------------------------------------------------------------------

fn extract_signature(node: Node, source: &[u8], language: &str) -> String {
    match language {
        "rust" => extract_rust_signature(node, source),
        "javascript" | "typescript" => extract_js_signature(node, source),
        "python" => extract_python_signature(node, source),
        "go" => extract_go_signature(node, source),
        _ => first_line(node, source),
    }
}

fn extract_rust_signature(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "function_item" => {
            if let Some(params) = node.child_by_field_name("parameters") {
                let end = params.end_byte();
                let ret_end = node
                    .child_by_field_name("return_type")
                    .map(|r| r.end_byte())
                    .unwrap_or(end);
                let sig_bytes = &source[node.start_byte()..ret_end];
                return String::from_utf8_lossy(sig_bytes).trim().to_string();
            }
            first_line(node, source)
        }
        _ => first_line(node, source),
    }
}

fn extract_js_signature(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "function_declaration" | "method_definition" => {
            if let Some(params) = node.child_by_field_name("parameters") {
                let end = params.end_byte();
                let sig_bytes = &source[node.start_byte()..end];
                return format!("{})", String::from_utf8_lossy(sig_bytes).trim());
            }
            first_line(node, source)
        }
        _ => first_line(node, source),
    }
}

fn extract_python_signature(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "function_definition" => {
            if let Some(params) = node.child_by_field_name("parameters") {
                let end = params.end_byte();
                let ret_end = node
                    .child_by_field_name("return_type")
                    .map(|r| r.end_byte())
                    .unwrap_or(end);
                let sig_bytes = &source[node.start_byte()..ret_end];
                return String::from_utf8_lossy(sig_bytes).trim().to_string();
            }
            first_line(node, source)
        }
        _ => first_line(node, source),
    }
}

fn extract_go_signature(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "function_declaration" | "method_declaration" => {
            if let Some(params) = node.child_by_field_name("parameters") {
                let end = params.end_byte();
                let result_end = node
                    .child_by_field_name("result")
                    .map(|r| r.end_byte())
                    .unwrap_or(end);
                let sig_bytes = &source[node.start_byte()..result_end];
                return String::from_utf8_lossy(sig_bytes).trim().to_string();
            }
            first_line(node, source)
        }
        _ => first_line(node, source),
    }
}

// ---------------------------------------------------------------------------
// Docstring / leading comment extraction
// ---------------------------------------------------------------------------

fn extract_docstring(node: Node, source: &[u8], language: &str) -> Option<String> {
    match language {
        "python" => extract_python_docstring(node, source),
        _ => extract_leading_comments(node, source),
    }
}

fn extract_python_docstring(node: Node, source: &[u8]) -> Option<String> {
    if let Some(comment) = extract_leading_comments(node, source) {
        return Some(comment);
    }

    if node.kind() != "function_definition" && node.kind() != "class_definition" {
        return None;
    }

    let body = node.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let first_stmt = body.children(&mut cursor).next()?;

    if first_stmt.kind() == "expression_statement" {
        let mut inner_cursor = first_stmt.walk();
        let expr = first_stmt.children(&mut inner_cursor).next()?;
        if expr.kind() == "string" {
            let text = node_text(expr, source);
            let trimmed = text
                .trim_start_matches("\"\"\"")
                .trim_start_matches("'''")
                .trim_end_matches("\"\"\"")
                .trim_end_matches("'''")
                .trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

fn extract_leading_comments(node: Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut sibling = node.prev_sibling();

    while let Some(prev) = sibling {
        match prev.kind() {
            "line_comment" | "comment" | "block_comment" => {
                let text = node_text(prev, source);
                comments.push(clean_comment(&text));

                let gap = node_start_line(node) as i64
                    - (prev.end_position().row as i64 + 1);
                sibling = prev.prev_sibling();
                if gap > 1 {
                    break;
                }
            }
            _ => break,
        }
    }

    if comments.is_empty() {
        return None;
    }

    comments.reverse();
    Some(comments.join("\n"))
}

fn clean_comment(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            let stripped = trimmed
                .strip_prefix("///")
                .or_else(|| trimmed.strip_prefix("//!"))
                .or_else(|| trimmed.strip_prefix("//"))
                .or_else(|| trimmed.strip_prefix('#'))
                .or_else(|| trimmed.strip_prefix("/*"))
                .or_else(|| trimmed.strip_prefix("/**"))
                .or_else(|| trimmed.strip_prefix('*'))
                .unwrap_or(trimmed);
            stripped.strip_suffix("*/").unwrap_or(stripped).trim()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn has_ancestor(node: Node, kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == kind {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn child_field_text(node: Node, field: &str, source: &[u8]) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    Some(node_text(child, source))
}

fn node_text(node: Node, source: &[u8]) -> String {
    let bytes = &source[node.start_byte()..node.end_byte()];
    String::from_utf8_lossy(bytes).into_owned()
}

fn first_line(node: Node, source: &[u8]) -> String {
    let text = node_text(node, source);
    text.lines().next().unwrap_or("").trim().to_string()
}

fn node_start_line(node: Node) -> usize {
    node.start_position().row + 1
}

fn impl_name(node: Node, source: &[u8]) -> Option<String> {
    let text = node_text(node, source);
    let first = text.lines().next()?;
    let after_impl = first.strip_prefix("impl")?.trim_start();
    let name = after_impl
        .split(|c: char| c == '{' || c == '<' || c.is_whitespace())
        .next()?
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_var_fn_name(node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let value = child.child_by_field_name("value")?;
            if matches!(value.kind(), "arrow_function" | "function") {
                return child_field_text(child, "name", source);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Bulk extraction for an indexed codebase
// ---------------------------------------------------------------------------

pub fn extract_all_chunks(
    index: &crate::index::IndexData,
    file_contents: &HashMap<String, String>,
) -> Vec<Chunk> {
    let mut component_id_map: HashMap<String, HashMap<(String, String, usize), String>> =
        HashMap::new();

    for comp in &index.components {
        component_id_map
            .entry(comp.file.clone())
            .or_default()
            .insert(
                (comp.component_type.clone(), comp.name.clone(), comp.start_line),
                comp.id.clone(),
            );
    }

    let mut all_chunks = Vec::new();
    for file in &index.files {
        let Some(source) = file_contents.get(&file.path) else {
            continue;
        };
        let id_map = component_id_map
            .get(&file.path)
            .cloned()
            .unwrap_or_default();
        let file_chunks = extract_chunks(&file.language, &file.path, source, &id_map);
        all_chunks.extend(file_chunks);
    }

    all_chunks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_function_chunking() {
        let source = r#"
/// Computes the sum of two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

struct Config {
    name: String,
    value: u32,
}

impl Config {
    fn new(name: &str) -> Self {
        Config {
            name: name.to_string(),
            value: 0,
        }
    }
}
"#;
        let ids: HashMap<(String, String, usize), String> = [
            (("fn".into(), "add".into(), 3), "fn-add".into()),
            (("struct".into(), "Config".into(), 7), "struct-cfg".into()),
            (("impl".into(), "Config".into(), 12), "impl-cfg".into()),
            (("fn".into(), "new".into(), 13), "fn-new".into()),
        ]
        .into();

        let chunks = extract_chunks("rust", "src/lib.rs", source, &ids);

        let add_chunk = chunks.iter().find(|c| c.name == "add");
        assert!(add_chunk.is_some(), "should find add function");
        let add = add_chunk.unwrap();
        assert_eq!(add.chunk_type, ChunkKind::Function);
        assert!(add.signature.contains("pub fn add(a: i32, b: i32) -> i32"));
        assert!(add.docstring.as_deref().unwrap_or("").contains("sum of two numbers"));

        let cfg_chunk = chunks.iter().find(|c| c.name == "Config" && c.chunk_type == ChunkKind::Struct);
        assert!(cfg_chunk.is_some(), "should find Config struct");

        let new_chunk = chunks.iter().find(|c| c.name == "new");
        assert!(new_chunk.is_some(), "should find new method");
        assert_eq!(new_chunk.unwrap().chunk_type, ChunkKind::Method);
    }

    #[test]
    fn python_function_chunking() {
        let source = r#"
class MyService:
    def process(self, data):
        """Process the incoming data."""
        return data

def standalone():
    pass
"#;
        let ids: HashMap<(String, String, usize), String> = [
            (("class".into(), "MyService".into(), 2), "cls-svc".into()),
            (("def".into(), "process".into(), 3), "fn-proc".into()),
            (("def".into(), "standalone".into(), 7), "fn-stand".into()),
        ]
        .into();

        let chunks = extract_chunks("python", "app.py", source, &ids);

        let proc_chunk = chunks.iter().find(|c| c.name == "process");
        assert!(proc_chunk.is_some(), "should find process method");
        let proc = proc_chunk.unwrap();
        assert!(proc.docstring.as_deref().unwrap_or("").contains("Process the incoming data"));

        let stand = chunks.iter().find(|c| c.name == "standalone");
        assert!(stand.is_some(), "should find standalone function");
        assert_eq!(stand.unwrap().chunk_type, ChunkKind::Function);
    }

    #[test]
    fn javascript_function_chunking() {
        let source = r#"
// Fetches data from the API
function fetchData(url) {
    return fetch(url);
}

class DataService {
    async process(data) {
        return data;
    }
}

const helper = (x) => x * 2;
"#;
        let ids: HashMap<(String, String, usize), String> = [
            (("function".into(), "fetchData".into(), 3), "fn-fetch".into()),
            (("class".into(), "DataService".into(), 7), "cls-ds".into()),
            (("method".into(), "process".into(), 8), "fn-proc".into()),
            (("const".into(), "helper".into(), 12), "fn-helper".into()),
        ]
        .into();

        let chunks = extract_chunks("javascript", "app.js", source, &ids);

        let fetch_chunk = chunks.iter().find(|c| c.name == "fetchData");
        assert!(fetch_chunk.is_some(), "should find fetchData function");
        assert!(fetch_chunk.unwrap().docstring.as_deref().unwrap_or("").contains("Fetches data"));

        let ds_chunk = chunks.iter().find(|c| c.name == "DataService");
        assert!(ds_chunk.is_some(), "should find DataService class");
    }

    #[test]
    fn go_function_chunking() {
        let source = r#"package main

// ServeHTTP handles incoming HTTP requests.
func ServeHTTP(w http.ResponseWriter, r *http.Request) {
	fmt.Fprintln(w, "hello")
}

type Config struct {
	Name  string
	Value int
}
"#;
        let ids: HashMap<(String, String, usize), String> = [
            (("func".into(), "ServeHTTP".into(), 4), "fn-serve".into()),
            (("type ... struct".into(), "Config".into(), 8), "struct-cfg".into()),
        ]
        .into();

        let chunks = extract_chunks("go", "main.go", source, &ids);

        let serve = chunks.iter().find(|c| c.name == "ServeHTTP");
        assert!(serve.is_some(), "should find ServeHTTP");
        assert!(serve.unwrap().docstring.as_deref().unwrap_or("").contains("handles incoming"));
    }

    #[test]
    fn embedding_document_format() {
        let chunk = Chunk {
            component_id: "fn-test".into(),
            name: "process".into(),
            chunk_type: ChunkKind::Function,
            language: "rust".into(),
            file: "src/lib.rs".into(),
            start_line: 10,
            end_line: 20,
            signature: "pub fn process(data: &[u8]) -> Result<()>".into(),
            body: "    let parsed = parse(data)?;\n    Ok(())".into(),
            docstring: Some("Processes raw data bytes.".into()),
            content_hash: "abcdef".into(),
        };

        let doc = chunk.embedding_document();
        assert!(doc.contains("rust fn process"));
        assert!(doc.contains("Processes raw data bytes"));
        assert!(doc.contains("pub fn process"));
        assert!(doc.contains("let parsed = parse"));
    }
}
