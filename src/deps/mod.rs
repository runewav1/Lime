use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::index::{ComponentRecord, IndexData};

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
}

/// Populates dependency links (`uses_before`, `used_by_after`) for all components.
pub fn populate_dependencies(index: &mut IndexData, file_contents: &HashMap<String, String>) {
    for component in &mut index.components {
        component.uses_before.clear();
        component.used_by_after.clear();
    }

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
            let tokens = scan_identifiers(&component.name)
                .into_iter()
                .filter(|token| !is_import_noise(token))
                .collect::<HashSet<_>>();
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

    let mut uses_map: HashMap<String, HashSet<String>> = HashMap::new();

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
        let mut local_uses = HashSet::new();

        if let Some(named_components) = file_component_names.get(&component.file) {
            for (target_id, target_name) in named_components {
                if target_id == &component.id {
                    continue;
                }

                if contains_word(&scope, target_name) {
                    local_uses.insert(target_id.clone());
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
                    let candidates = target_ids
                        .iter()
                        .filter(|target_id| *target_id != &component.id)
                        .collect::<Vec<_>>();

                    if candidates.len() == 1 {
                        local_uses.insert(candidates[0].clone());
                    }
                }
            }

            let qualified_tokens = scan_qualified_identifiers(&scope);
            for token in qualified_tokens {
                if let Some(target_ids) = identifiers.get(&token) {
                    let candidates = target_ids
                        .iter()
                        .filter(|target_id| *target_id != &component.id)
                        .collect::<Vec<_>>();

                    if candidates.len() == 1 {
                        local_uses.insert(candidates[0].clone());
                    }
                }
            }
        }

        uses_map.insert(component.id.clone(), local_uses);
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
        component.uses_before = before;
        component.used_by_after = after;
    }
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

fn contains_word(scope: &str, word: &str) -> bool {
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
    )
}

fn is_import_like_type(component_type: &str) -> bool {
    matches!(component_type, "use" | "import" | "from")
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
