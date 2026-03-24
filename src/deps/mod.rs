use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::index::{ComponentRecord, DeathStatus, DepEdge, DepEdgeKind, IndexData};

/// Dependency tree output from `lime deps`.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyTree {
    /// Requested root component ID.
    pub component_id: String,
    /// Actual depth used for traversal.
    pub depth: usize,
    /// Components this root depends on (before).
    pub before: Vec<DependencyNode>,
    /// Components depending on this root (after).
    pub after: Vec<DependencyNode>,
}

/// Node in dependency output graph.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyNode {
    /// Component ID.
    pub id: String,
    /// Language key.
    pub language: String,
    /// Component type.
    #[serde(rename = "type")]
    pub component_type: String,
    /// Component name.
    pub name: String,
    /// File path.
    pub file: String,
    /// Starting line.
    pub start_line: usize,
    /// Relationship distance from root.
    pub depth: usize,
    /// Whether this component is flagged as dead code.
    pub batman: bool,
    /// Tiered death classification.
    pub death_status: DeathStatus,
}

/// Populates dependency links (`uses_before`, `used_by_after`, `dep_edges`) for all components.
pub fn populate_dependencies(index: &mut IndexData, file_contents: &HashMap<String, String>) {
    for component in &mut index.components {
        component.uses_before.clear();
        component.used_by_after.clear();
        component.dep_edges.clear();
    }

    let id_to_file: HashMap<String, String> = index
        .components
        .iter()
        .map(|c| (c.id.clone(), c.file.clone()))
        .collect();

    let mut file_component_names: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut file_imported_tokens: HashMap<String, HashSet<String>> = HashMap::new();
    for component in &index.components {
        if is_dependency_target_type(&component.component_type) {
            file_component_names
                .entry(component.file.clone())
                .or_default()
                .push((component.id.clone(), component.name.clone()));
        }

        if is_import_like_type(&component.component_type) {
            let tokens = extract_import_tokens(&component.name);
            file_imported_tokens
                .entry(component.file.clone())
                .or_default()
                .extend(tokens);
        }
    }

    let mut language_identifier_index: HashMap<String, HashMap<String, Vec<String>>> =
        HashMap::new();
    for component in &index.components {
        if is_dependency_target_type(&component.component_type)
            && is_identifier_name(&component.name)
        {
            language_identifier_index
                .entry(component.language.clone())
                .or_default()
                .entry(component.name.clone())
                .or_default()
                .push(component.id.clone());
        }
    }

    // Deterministic order for duplicate names in the global index.
    for ids in language_identifier_index.values_mut() {
        for v in ids.values_mut() {
            v.sort();
        }
    }

    let mut uses_map: HashMap<String, HashSet<String>> = HashMap::new();
    let mut edge_kinds_map: HashMap<String, HashMap<String, DepEdgeKind>> = HashMap::new();

    for component in &index.components {
        let content = file_contents
            .get(&component.file)
            .map(String::as_str)
            .unwrap_or_default();

        let lines: Vec<&str> = content.lines().collect();
        let start = component.start_line.saturating_sub(1);
        let end = component.end_line.min(lines.len());
        if start >= end || lines.is_empty() {
            continue;
        }

        let scope = lines[start..end].join("\n");
        let scope = sanitize_scope_for_deps(&component.language, &scope);
        let mut local_uses: HashSet<String> = HashSet::new();
        let mut local_kinds: HashMap<String, DepEdgeKind> = HashMap::new();

        if let Some(named_components) = file_component_names.get(&component.file) {
            for (target_id, target_name) in named_components {
                if target_id == &component.id {
                    continue;
                }

                if contains_word(&scope, target_name) {
                    local_uses.insert(target_id.clone());
                    merge_edge_kind(
                        &mut local_kinds,
                        target_id.clone(),
                        DepEdgeKind::SameFile,
                    );
                }
            }
        }

        if let Some(identifiers) = language_identifier_index.get(&component.language) {
            let imported_tokens = file_imported_tokens.get(&component.file);
            let tokens = scan_identifiers(&scope);
            for token in tokens {
                if !imported_tokens
                    .map(|entries| entries.contains(&token))
                    .unwrap_or(false)
                {
                    continue;
                }

                if let Some(target_ids) = identifiers.get(&token) {
                    let candidates: Vec<String> = target_ids
                        .iter()
                        .filter(|target_id| *target_id != &component.id)
                        .cloned()
                        .collect();

                    if let Some(chosen) =
                        pick_deterministic_candidate(&candidates, &component.file, &id_to_file)
                    {
                        let kind = if candidates.len() == 1 {
                            DepEdgeKind::ImportResolved
                        } else {
                            DepEdgeKind::ImportDisambiguated
                        };
                        local_uses.insert(chosen.clone());
                        merge_edge_kind(&mut local_kinds, chosen, kind);
                    }
                }
            }

            let qualified_tokens = scan_qualified_identifiers(&scope);
            for token in qualified_tokens {
                if let Some(target_ids) = identifiers.get(&token) {
                    let candidates: Vec<String> = target_ids
                        .iter()
                        .filter(|target_id| *target_id != &component.id)
                        .cloned()
                        .collect();

                    if let Some(chosen) =
                        pick_deterministic_candidate(&candidates, &component.file, &id_to_file)
                    {
                        let kind = if candidates.len() == 1 {
                            DepEdgeKind::Qualified
                        } else {
                            DepEdgeKind::ImportDisambiguated
                        };
                        local_uses.insert(chosen.clone());
                        merge_edge_kind(&mut local_kinds, chosen, kind);
                    }
                }
            }
        }

        uses_map.insert(component.id.clone(), local_uses);
        edge_kinds_map.insert(component.id.clone(), local_kinds);
    }

    let mut used_by_map: HashMap<String, HashSet<String>> = HashMap::new();
    for (source, targets) in &uses_map {
        for target in targets {
            used_by_map
                .entry(target.clone())
                .or_default()
                .insert(source.clone());
        }
    }

    for component in &mut index.components {
        let mut before = uses_map
            .remove(&component.id)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        let mut after = used_by_map
            .remove(&component.id)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        before.sort();
        after.sort();

        if let Some(kinds) = edge_kinds_map.remove(&component.id) {
            let mut edges: Vec<DepEdge> = before
                .iter()
                .map(|tid| DepEdge {
                    target: tid.clone(),
                    kind: kinds.get(tid).copied().unwrap_or(DepEdgeKind::SameFile),
                })
                .collect();
            edges.sort_by(|a, b| a.target.cmp(&b.target));
            component.dep_edges = edges;
        } else {
            component.dep_edges = Vec::new();
        }

        component.uses_before = before;
        component.used_by_after = after;
    }
}

fn merge_edge_kind(map: &mut HashMap<String, DepEdgeKind>, target: String, kind: DepEdgeKind) {
    map.entry(target)
        .and_modify(|e| *e = (*e).max_rank(kind))
        .or_insert(kind);
}

/// Deterministic pick when multiple components share an identifier name.
/// Preference order: same file as source, same parent directory, shorter path, then id.
fn pick_deterministic_candidate(
    candidates: &[String],
    source_file: &str,
    id_to_file: &HashMap<String, String>,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }

    let source_parent = parent_dir_key(source_file);
    let mut sorted: Vec<&String> = candidates.iter().collect();
    sorted.sort_by(|a, b| {
        let fa = id_to_file.get(*a).map(|s| s.as_str()).unwrap_or("");
        let fb = id_to_file.get(*b).map(|s| s.as_str()).unwrap_or("");
        rank_key(fa, a.as_str(), source_file, source_parent)
            .cmp(&rank_key(fb, b.as_str(), source_file, source_parent))
    });
    sorted.first().map(|s| (*s).clone())
}

fn parent_dir_key(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

fn rank_key(
    target_file: &str,
    target_id: &str,
    source_file: &str,
    source_parent: &str,
) -> (bool, bool, usize, String) {
    let same_file = target_file == source_file;
    let same_parent = parent_dir_key(target_file) == source_parent;
    (
        !same_file,
        !same_parent,
        target_file.len(),
        target_id.to_string(),
    )
}

/// Tokens imported or re-exported from a parsed import/`use` line (best-effort).
pub fn extract_import_tokens(name: &str) -> HashSet<String> {
    let mut out = scan_identifiers(name);
    for token in raw_brace_list_tokens(name) {
        if !is_import_noise(&token) {
            out.insert(token);
        }
    }
    out
}

/// Inside `{ ... }`, split on commas and handle `Foo as Bar` (take `Bar`).
fn raw_brace_list_tokens(name: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(start) = name.find('{') else {
        return out;
    };
    let Some(end) = name.rfind('}') else {
        return out;
    };
    if start >= end {
        return out;
    }
    let inner = &name[start + 1..end];
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let segment = if let Some(pos) = part.rfind(" as ") {
            part[pos + 4..].trim()
        } else {
            part
        };
        for t in scan_identifiers(segment) {
            if !is_import_noise(&t) {
                out.insert(t);
            }
        }
    }
    out
}

/// Remove comments and string literals so dependency matching ignores mentions inside them.
pub fn sanitize_scope_for_deps(language: &str, scope: &str) -> String {
    let after_blocks = strip_block_comments_slash_star(scope);
    let after_line = strip_line_comments_slash_slash(&after_blocks);
    let after_hash = if language == "python" {
        strip_python_hash_lines_str(&after_line)
    } else {
        after_line
    };
    strip_quoted_strings_best_effort(&after_hash)
}

fn strip_block_comments_slash_star(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            } else {
                break;
            }
            continue;
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn strip_line_comments_slash_slash(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let mut line_out = String::new();
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                break;
            }
            let ch = line[i..].chars().next().unwrap();
            line_out.push(ch);
            i += ch.len_utf8();
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line_out);
    }
    out
}

fn strip_python_hash_lines_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(strip_python_line_comment(line));
    }
    out
}

fn strip_python_line_comment(line: &str) -> &str {
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0usize;
    while i < line.len() {
        let c = line[i..].chars().next().unwrap();
        match c {
            '"' if !in_sq => in_dq = !in_dq,
            '\'' if !in_dq => in_sq = !in_sq,
            '#' if !in_dq && !in_sq => return line[..i].trim_end(),
            _ => {}
        }
        i += c.len_utf8();
    }
    line
}

/// Removes `"..."` strings with `\\` and `\"` escapes (best-effort).
fn strip_quoted_strings_best_effort(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(' ');
            continue;
        }
        if bytes[i] == b'\'' && language_allows_single_quote_strings(bytes, i) {
            // Skip a single-quoted run (char literal or short string).
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'\'' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(' ');
            continue;
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn language_allows_single_quote_strings(bytes: &[u8], at: usize) -> bool {
    // Avoid treating Rust lifetimes `'a` as strings.
    if at > 0 {
        let prev = bytes[at - 1];
        if prev == b'_' || prev.is_ascii_alphanumeric() {
            return false;
        }
    }
    true
}

/// Builds dependency matrix for a component with depth limiting.
pub fn dependency_tree(
    index: &IndexData,
    component_id: &str,
    depth: usize,
) -> Option<DependencyTree> {
    let by_id: HashMap<&str, &ComponentRecord> = index
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect();

    let root = by_id.get(component_id)?;

    let before = walk_direction(&by_id, root, depth, Direction::Before);
    let after = walk_direction(&by_id, root, depth, Direction::After);

    Some(DependencyTree {
        component_id: component_id.to_string(),
        depth,
        before,
        after,
    })
}

#[derive(Copy, Clone)]
enum Direction {
    Before,
    After,
}

fn walk_direction(
    by_id: &HashMap<&str, &ComponentRecord>,
    root: &ComponentRecord,
    max_depth: usize,
    direction: Direction,
) -> Vec<DependencyNode> {
    let mut nodes = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    let initial = match direction {
        Direction::Before => &root.uses_before,
        Direction::After => &root.used_by_after,
    };

    for dependency_id in initial {
        queue.push_back((dependency_id.clone(), 1usize));
    }

    while let Some((current_id, current_depth)) = queue.pop_front() {
        if current_depth > max_depth {
            continue;
        }

        if !visited.insert(current_id.clone()) {
            continue;
        }

        if let Some(component) = by_id.get(current_id.as_str()) {
            nodes.push(DependencyNode {
                id: component.id.clone(),
                language: component.language.clone(),
                component_type: component.component_type.clone(),
                name: component.name.clone(),
                file: component.file.clone(),
                start_line: component.start_line,
                depth: current_depth,
                batman: component.batman,
                death_status: component.death_status,
            });

            let next = match direction {
                Direction::Before => &component.uses_before,
                Direction::After => &component.used_by_after,
            };

            for next_id in next {
                queue.push_back((next_id.clone(), current_depth + 1));
            }
        }
    }

    nodes.sort_by(|left, right| {
        (
            left.depth,
            left.file.as_str(),
            left.start_line,
            left.component_type.as_str(),
            left.name.as_str(),
            left.id.as_str(),
        )
            .cmp(&(
                right.depth,
                right.file.as_str(),
                right.start_line,
                right.component_type.as_str(),
                right.name.as_str(),
                right.id.as_str(),
            ))
    });

    if nodes.is_empty() {
        return nodes;
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for node in nodes {
        if seen.insert(node.id.clone()) {
            deduped.push(node);
        }
    }

    deduped
}

pub fn contains_word(scope: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }

    let mut index = 0;
    while let Some(found_at) = scope[index..].find(word) {
        let absolute = index + found_at;
        let start_ok = absolute == 0
            || !scope[..absolute]
                .chars()
                .next_back()
                .map(is_identifier_char)
                .unwrap_or(false);

        let end_index = absolute + word.len();
        let end_ok = end_index >= scope.len()
            || !scope[end_index..]
                .chars()
                .next()
                .map(is_identifier_char)
                .unwrap_or(false);

        if start_ok && end_ok {
            return true;
        }

        index = end_index;
        if index >= scope.len() {
            break;
        }
    }

    false
}

fn scan_identifiers(scope: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let mut current = String::new();

    for character in scope.chars() {
        if is_identifier_char(character) {
            current.push(character);
        } else if !current.is_empty() {
            if is_identifier_name(&current) {
                tokens.insert(current.clone());
            }
            current.clear();
        }
    }

    if is_identifier_name(&current) {
        tokens.insert(current);
    }

    tokens
}

fn scan_qualified_identifiers(scope: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let bytes = scope.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] == b':' && bytes[index + 1] == b':' {
            index += 2;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }

            let start = index;
            while index < bytes.len() {
                let character = bytes[index] as char;
                if is_identifier_char(character) {
                    index += 1;
                } else {
                    break;
                }
            }

            if start < index {
                let token = &scope[start..index];
                if is_identifier_name(token) {
                    tokens.insert(token.to_string());
                }
            }
        } else {
            index += 1;
        }
    }

    tokens
}

fn is_identifier_name(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    characters.all(is_identifier_char)
}

fn is_dependency_target_type(component_type: &str) -> bool {
    matches!(
        component_type,
        "struct"
            | "enum"
            | "fn"
            | "trait"
            | "class"
            | "function"
            | "interface"
            | "type"
            | "const"
            | "let"
            | "var"
            | "def"
            | "async def"
            | "func"
            | "typedef"
            | "define"
            | "namespace"
            | "union"
            | "test"
            | "using"
    )
}

pub fn is_import_like_type(component_type: &str) -> bool {
    matches!(component_type, "use" | "import" | "from" | "include")
}

fn is_import_noise(token: &str) -> bool {
    matches!(
        token,
        "crate" | "self" | "super" | "as" | "mod" | "pub" | "from" | "import" | "use" | "default"
    )
}

fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_comment_mentions() {
        let scope = "fn x() {\n// FooBar\n let _ = other;\n}";
        let clean = sanitize_scope_for_deps("rust", scope);
        assert!(!contains_word(&clean, "FooBar"));
    }

    #[test]
    fn sanitize_keeps_real_identifier() {
        let scope = "fn x() { call_helper(); }";
        let clean = sanitize_scope_for_deps("rust", scope);
        assert!(contains_word(&clean, "call_helper"));
    }

    #[test]
    fn pick_prefers_same_file() {
        let mut m = HashMap::new();
        m.insert("a".into(), "src/lib.rs".into());
        m.insert("b".into(), "src/other.rs".into());
        let cands = vec!["a".into(), "b".into()];
        let p = pick_deterministic_candidate(&cands, "src/lib.rs", &m);
        assert_eq!(p.as_deref(), Some("a"));
    }

    #[test]
    fn extract_import_tokens_brace_list() {
        let s = "use crate::foo::{Bar, Baz as Qux};";
        let t = extract_import_tokens(s);
        assert!(t.contains("Bar"));
        assert!(t.contains("Qux"));
    }
}
