use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::{deps, index::IndexData};

#[derive(Debug, Clone, Serialize)]
pub struct BatmanReport {
    pub total_components: usize,
    pub batman_count: usize,
    pub islands_detected: usize,
    pub pass2_rescued_count: usize,
}

pub fn detect_batman(
    index: &mut IndexData,
    file_contents: &HashMap<String, String>,
) -> BatmanReport {
    let total_components = index.components.len();

    for component in &mut index.components {
        component.batman = false;
    }

    if total_components == 0 {
        return BatmanReport {
            total_components: 0,
            batman_count: 0,
            islands_detected: 0,
            pass2_rescued_count: 0,
        };
    }

    // --- Pass 1: Full-depth detachment check ---

    let id_to_idx: HashMap<&str, usize> = index
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();

    let mut adjacency: Vec<HashSet<usize>> = vec![HashSet::new(); total_components];

    for (i, component) in index.components.iter().enumerate() {
        for dep_id in &component.uses_before {
            if let Some(&j) = id_to_idx.get(dep_id.as_str()) {
                adjacency[i].insert(j);
                adjacency[j].insert(i);
            }
        }
        for dep_id in &component.used_by_after {
            if let Some(&j) = id_to_idx.get(dep_id.as_str()) {
                adjacency[i].insert(j);
                adjacency[j].insert(i);
            }
        }
    }

    let mut island_of: Vec<usize> = vec![0; total_components];
    let mut visited = vec![false; total_components];
    let mut islands: Vec<Vec<usize>> = Vec::new();

    for start in 0..total_components {
        if visited[start] {
            continue;
        }

        let island_id = islands.len();
        let mut members = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited[start] = true;

        while let Some(current) = queue.pop_front() {
            island_of[current] = island_id;
            members.push(current);
            for &neighbor in &adjacency[current] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        islands.push(members);
    }

    let islands_detected = islands.len();

    let mut entry_point_islands: HashSet<usize> = HashSet::new();
    for (i, component) in index.components.iter().enumerate() {
        let is_entry_by_name = (component.name == "main" || component.name == "run")
            && component.component_type == "fn";
        let is_entry_by_file = !component.file.contains('/');

        if is_entry_by_name || is_entry_by_file {
            entry_point_islands.insert(island_of[i]);
        }
    }

    let main_islands: HashSet<usize> = if !entry_point_islands.is_empty() {
        entry_point_islands
    } else {
        let largest = islands
            .iter()
            .enumerate()
            .max_by_key(|(_, members)| members.len())
            .map(|(id, _)| id);
        let mut set = HashSet::new();
        if let Some(id) = largest {
            set.insert(id);
        }
        set
    };

    let mut batman_candidates: HashSet<usize> = HashSet::new();

    for (i, component) in index.components.iter().enumerate() {
        if deps::is_import_like_type(&component.component_type) {
            continue;
        }

        let in_main = main_islands.contains(&island_of[i]);
        let has_edges = !component.uses_before.is_empty() || !component.used_by_after.is_empty();

        if !in_main || !has_edges {
            batman_candidates.insert(i);
        }
    }

    // --- Pass 2: Component line reference check ---

    let non_batman_files: HashSet<&str> = index
        .components
        .iter()
        .enumerate()
        .filter(|(i, _)| !batman_candidates.contains(i))
        .map(|(_, c)| c.file.as_str())
        .collect();

    let mut rescued: HashSet<usize> = HashSet::new();

    for &candidate_idx in &batman_candidates {
        let name = &index.components[candidate_idx].name;
        if name.is_empty() {
            continue;
        }

        for &file_path in &non_batman_files {
            if let Some(source) = file_contents.get(file_path) {
                if deps::contains_word(source, name) {
                    rescued.insert(candidate_idx);
                    break;
                }
            }
        }
    }

    for &idx in &rescued {
        batman_candidates.remove(&idx);
    }

    let pass2_rescued_count = rescued.len();

    for &idx in &batman_candidates {
        index.components[idx].batman = true;
    }

    BatmanReport {
        total_components,
        batman_count: batman_candidates.len(),
        islands_detected,
        pass2_rescued_count,
    }
}
