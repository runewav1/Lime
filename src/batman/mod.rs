use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::{
    annotations::Annotation,
    config::DeathSeedConfig,
    deps,
    index::{DeathEvidence, DeathReason, DeathStatus, IndexData},
};

/// Undirected adjacency for death-detection reachability — same edges as `deps::populate_dependencies` materializes into `uses_before` / `used_by_after`.
pub(crate) fn build_component_adjacency(index: &IndexData) -> Vec<BTreeSet<usize>> {
    let total = index.components.len();
    let id_to_idx: HashMap<&str, usize> = index
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();

    let mut adjacency: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); total];

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

    adjacency
}

#[derive(Debug, Clone, Serialize)]
pub struct DeathReport {
    pub total_components: usize,
    pub definitely_dead: usize,
    pub probably_dead: usize,
    pub maybe_dead: usize,
    pub alive: usize,
    pub islands_detected: usize,
    pub pass2_rescued_count: usize,
    pub pass3_rescued_count: usize,
}

/// Multi-pass deterministic death classification.
///
/// Pass 1 — graph reachability from seed (entrypoint) components.
/// Pass 2 — strict symbol reference validation across non-candidate files.
/// Pass 3 — local-scope line-by-line validation within candidate files.
/// Pass 4 — annotation-based retention (keep tags).
pub fn detect_batman_full(
    index: &mut IndexData,
    file_contents: &HashMap<String, String>,
    seed_config: &DeathSeedConfig,
    annotations: &[Annotation],
) -> DeathReport {
    let total_components = index.components.len();

    for component in &mut index.components {
        component.batman = false;
        component.death_status = DeathStatus::Alive;
        component.death_evidence = DeathEvidence::default();
    }

    if total_components == 0 {
        return DeathReport {
            total_components: 0,
            definitely_dead: 0,
            probably_dead: 0,
            maybe_dead: 0,
            alive: 0,
            islands_detected: 0,
            pass2_rescued_count: 0,
            pass3_rescued_count: 0,
        };
    }

    // --- Build deterministic adjacency graph (mirrors `uses_before` / `used_by_after`) ---

    let adjacency = build_component_adjacency(index);

    let id_to_idx: HashMap<&str, usize> = index
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();

    // --- Pass 1: deterministic BFS island detection ---

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

    // Deterministic seed selection with config overrides
    let mut entry_point_islands = BTreeSet::new();
    for (i, component) in index.components.iter().enumerate() {
        let is_entry_by_name = (component.name == "main" || component.name == "run")
            && component.component_type == "fn";
        let is_entry_by_file = !component.file.contains('/');

        let is_config_seed_file = seed_config
            .seed_files
            .iter()
            .any(|pat| component.file.contains(pat.as_str()));
        let is_config_seed_name = seed_config.seed_names.contains(&component.name);
        let is_config_seed_type = seed_config
            .seed_types
            .contains(&component.component_type);

        if is_entry_by_name
            || is_entry_by_file
            || is_config_seed_file
            || is_config_seed_name
            || is_config_seed_type
        {
            entry_point_islands.insert(island_of[i]);
        }
    }

    let main_islands: BTreeSet<usize> = if !entry_point_islands.is_empty() {
        entry_point_islands
    } else {
        let largest = islands
            .iter()
            .enumerate()
            .max_by_key(|(_, members)| members.len())
            .map(|(id, _)| id);
        let mut set = BTreeSet::new();
        if let Some(id) = largest {
            set.insert(id);
        }
        set
    };

    // Collect pass-1 candidates deterministically (sorted indices)
    let mut pass1_candidates: Vec<usize> = Vec::new();
    let mut pass1_evidence: HashMap<usize, Vec<DeathReason>> = HashMap::new();

    for (i, component) in index.components.iter().enumerate() {
        if deps::is_import_like_type(&component.component_type) {
            continue;
        }

        let in_main = main_islands.contains(&island_of[i]);
        let has_edges = !component.uses_before.is_empty() || !component.used_by_after.is_empty();

        if !in_main || !has_edges {
            let mut reasons = Vec::new();
            if !in_main {
                reasons.push(DeathReason::NotReachableFromSeeds);
            }
            if !has_edges {
                reasons.push(DeathReason::NoDependencyEdges);
            }
            // Check if all dependency parents are also candidates
            let all_parents_candidate = component.used_by_after.is_empty()
                || component.used_by_after.iter().all(|parent_id| {
                    id_to_idx
                        .get(parent_id.as_str())
                        .is_none_or(|&pi| !main_islands.contains(&island_of[pi]))
                });
            if all_parents_candidate && !component.used_by_after.is_empty() {
                reasons.push(DeathReason::AllParentsDeadCandidates);
            }
            pass1_evidence.insert(i, reasons);
            pass1_candidates.push(i);
        }
    }

    let candidate_set: HashSet<usize> = pass1_candidates.iter().copied().collect();

    // --- Pass 2: strict symbol reference validation ---

    let non_candidate_files: BTreeSet<&str> = index
        .components
        .iter()
        .enumerate()
        .filter(|(i, _)| !candidate_set.contains(i))
        .map(|(_, c)| c.file.as_str())
        .collect();

    let mut pass2_rescued: HashSet<usize> = HashSet::new();

    for &candidate_idx in &pass1_candidates {
        let component = &index.components[candidate_idx];
        let name = &component.name;
        if name.is_empty() {
            continue;
        }

        for &file_path in &non_candidate_files {
            if let Some(source) = file_contents.get(file_path) {
                if deps::contains_word(source, name) {
                    pass2_rescued.insert(candidate_idx);
                    if let Some(reasons) = pass1_evidence.get_mut(&candidate_idx) {
                        *reasons = vec![DeathReason::FoundExternalRef {
                            file: file_path.to_string(),
                            count: 1,
                        }];
                    }
                    break;
                }
            }
        }
    }

    let pass2_rescued_count = pass2_rescued.len();

    // --- Pass 3: local-scope line-by-line validation ---
    // For remaining candidates, scan their own file outside their definition range.

    const MAX_SCAN_BYTES: usize = 512_000;

    let mut pass3_rescued: HashSet<usize> = HashSet::new();
    let mut pass3_capped: HashSet<usize> = HashSet::new();

    for &candidate_idx in &pass1_candidates {
        if pass2_rescued.contains(&candidate_idx) {
            continue;
        }

        let component = &index.components[candidate_idx];
        let name = &component.name;
        if name.is_empty() {
            continue;
        }

        let Some(source) = file_contents.get(&component.file) else {
            continue;
        };

        if source.len() > MAX_SCAN_BYTES {
            pass3_capped.insert(candidate_idx);
            if let Some(reasons) = pass1_evidence.get_mut(&candidate_idx) {
                reasons.push(DeathReason::ScanCapped);
            }
            continue;
        }

        let lines: Vec<&str> = source.lines().collect();
        let def_start = component.start_line.saturating_sub(1);
        let def_end = component.end_line.min(lines.len());

        let mut found = false;
        for (line_idx, line) in lines.iter().enumerate() {
            if line_idx >= def_start && line_idx < def_end {
                continue;
            }
            if deps::contains_word(line, name) {
                found = true;
                break;
            }
        }

        if found {
            pass3_rescued.insert(candidate_idx);
            if let Some(reasons) = pass1_evidence.get_mut(&candidate_idx) {
                *reasons = vec![DeathReason::FoundExternalRef {
                    file: component.file.clone(),
                    count: 1,
                }];
            }
        } else if let Some(reasons) = pass1_evidence.get_mut(&candidate_idx) {
            reasons.push(DeathReason::NoLocalScopeReferences);
        }
    }

    let pass3_rescued_count = pass3_rescued.len();

    // --- Pass 4: annotation-based retention (keep tags) ---

    let mut pass4_rescued: HashSet<usize> = HashSet::new();
    for &candidate_idx in &pass1_candidates {
        if pass2_rescued.contains(&candidate_idx) || pass3_rescued.contains(&candidate_idx) {
            continue;
        }
        let component = &index.components[candidate_idx];
        let kept = annotations.iter().filter(|a| a.has_keep_tag()).any(|a| {
            a.hash_id == component.id || crate::annotations::annotation_applies_to_component(a, component)
        });
        if kept {
            pass4_rescued.insert(candidate_idx);
        }
    }

    // --- Assign final tiered status ---

    let mut definitely_dead = 0usize;
    let mut probably_dead = 0usize;
    let mut maybe_dead = 0usize;

    for &candidate_idx in &pass1_candidates {
        if pass2_rescued.contains(&candidate_idx)
            || pass3_rescued.contains(&candidate_idx)
        {
            continue;
        }

        if pass4_rescued.contains(&candidate_idx) {
            let component = &mut index.components[candidate_idx];
            component.death_status = DeathStatus::Alive;
            component.death_evidence = DeathEvidence {
                reasons: vec![DeathReason::AnnotatedKeep],
            };
            component.batman = false;
            continue;
        }

        let reasons = pass1_evidence
            .remove(&candidate_idx)
            .unwrap_or_default();

        let capped = pass3_capped.contains(&candidate_idx);

        let status = if capped {
            // Scan was incomplete — cannot be definite
            maybe_dead += 1;
            DeathStatus::MaybeDead
        } else if reasons.iter().any(|r| matches!(r, DeathReason::NoLocalScopeReferences)) {
            // Passed all validation with no references found
            definitely_dead += 1;
            DeathStatus::DefinitelyDead
        } else {
            probably_dead += 1;
            DeathStatus::ProbablyDead
        };

        let component = &mut index.components[candidate_idx];
        component.death_status = status;
        component.death_evidence = DeathEvidence { reasons };
        component.batman = status.is_dead();
    }

    let alive = total_components - definitely_dead - probably_dead - maybe_dead;

    DeathReport {
        total_components,
        definitely_dead,
        probably_dead,
        maybe_dead,
        alive,
        islands_detected,
        pass2_rescued_count,
        pass3_rescued_count,
    }
}
