use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::LazyLock,
};

use serde::Serialize;

use crate::{annotations::Annotation, index::IndexData};

static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "do", "does",
        "for", "from", "if", "in", "into", "is", "it", "its", "no", "not", "of", "on", "or", "our",
        "out", "over", "per", "so", "such", "than", "that", "the", "their", "them", "there",
        "these", "they", "this", "those", "to", "under", "up", "via", "was", "we", "were", "will",
        "with",
    ])
});

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub component_id: String,
    pub score: f64,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    Exact,
    Prefix,
    Substring,
    Annotation,
}

impl MatchType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchType::Exact => "exact",
            MatchType::Prefix => "prefix",
            MatchType::Substring => "substring",
            MatchType::Annotation => "annotation",
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            MatchType::Exact => 0,
            MatchType::Prefix => 1,
            MatchType::Substring => 2,
            MatchType::Annotation => 3,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchTokenIndex {
    pub token_to_components: HashMap<String, Vec<String>>,
    pub component_tokens: HashMap<String, Vec<String>>,
    name_token_to_components: HashMap<String, Vec<String>>,
    annotation_token_to_components: HashMap<String, Vec<String>>,
}

impl SearchTokenIndex {
    fn all_tokens(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.token_to_components.iter()
    }

    fn contains_component(&self, component_id: &str) -> bool {
        self.component_tokens.contains_key(component_id)
    }
}

#[derive(Debug, Clone, Copy)]
struct MatchCandidate {
    score: f64,
    match_type: MatchType,
}

pub fn tokenize_name(name: &str) -> Vec<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut tokens = Vec::new();
    let mut chunk = String::new();

    for character in trimmed.chars() {
        if character.is_ascii_alphanumeric() {
            chunk.push(character);
        } else if !chunk.is_empty() {
            push_name_chunk(&chunk, &mut tokens);
            chunk.clear();
        }
    }

    if !chunk.is_empty() {
        push_name_chunk(&chunk, &mut tokens);
    }

    push_unique(&mut tokens, trimmed.to_ascii_lowercase(), true);
    tokens
}

pub fn tokenize_content(content: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chunk = String::new();

    for character in content.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            chunk.push(character);
        } else if !chunk.is_empty() {
            push_content_chunk(&chunk, &mut tokens);
            chunk.clear();
        }
    }

    if !chunk.is_empty() {
        push_content_chunk(&chunk, &mut tokens);
    }

    tokens
}

pub fn build_token_index(index: &IndexData, annotations: &[Annotation]) -> SearchTokenIndex {
    let valid_component_ids = index
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect::<HashSet<_>>();

    let mut token_to_components = HashMap::<String, BTreeSet<String>>::new();
    let mut component_tokens = HashMap::<String, BTreeSet<String>>::new();
    let mut name_token_to_components = HashMap::<String, BTreeSet<String>>::new();
    let mut annotation_token_to_components = HashMap::<String, BTreeSet<String>>::new();

    for component in &index.components {
        add_tokens(
            &component.id,
            tokenize_name(&component.name),
            &mut token_to_components,
            &mut component_tokens,
            &mut name_token_to_components,
        );
    }

    for annotation in annotations {
        if !valid_component_ids.contains(annotation.hash_id.as_str()) {
            continue;
        }

        add_tokens(
            &annotation.hash_id,
            tokenize_content(&annotation.content),
            &mut token_to_components,
            &mut component_tokens,
            &mut annotation_token_to_components,
        );
    }

    SearchTokenIndex {
        token_to_components: finalize_index(token_to_components),
        component_tokens: finalize_index(component_tokens),
        name_token_to_components: finalize_index(name_token_to_components),
        annotation_token_to_components: finalize_index(annotation_token_to_components),
    }
}

pub fn fuzzy_search(token_index: &SearchTokenIndex, query: &str) -> Vec<SearchHit> {
    let query_tokens = tokenize_name(query);
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let mut total_scores = HashMap::<String, f64>::new();
    let mut best_match_types = HashMap::<String, MatchCandidate>::new();

    for query_token in query_tokens {
        let mut token_matches = HashMap::<String, MatchCandidate>::new();

        for (index_token, component_ids) in token_index.all_tokens() {
            let name_candidate = name_match(index_token, &query_token);
            let annotation_candidate = if index_token.contains(&query_token) {
                Some(MatchCandidate {
                    score: 0.3,
                    match_type: MatchType::Annotation,
                })
            } else {
                None
            };

            for component_id in component_ids {
                if !token_index.contains_component(component_id) {
                    continue;
                }

                if let Some(candidate) = name_candidate {
                    if token_has_component(
                        &token_index.name_token_to_components,
                        index_token,
                        component_id,
                    ) {
                        update_best_match(&mut token_matches, component_id, candidate);
                    }
                }

                if let Some(candidate) = annotation_candidate {
                    if token_has_component(
                        &token_index.annotation_token_to_components,
                        index_token,
                        component_id,
                    ) {
                        update_best_match(&mut token_matches, component_id, candidate);
                    }
                }
            }
        }

        for (component_id, candidate) in token_matches {
            *total_scores.entry(component_id.clone()).or_insert(0.0) += candidate.score;
            update_best_match(&mut best_match_types, &component_id, candidate);
        }
    }

    let mut hits = total_scores
        .into_iter()
        .map(|(component_id, score)| SearchHit {
            match_type: best_match_types
                .get(&component_id)
                .map(|candidate| candidate.match_type)
                .unwrap_or(MatchType::Substring),
            component_id,
            score,
        })
        .collect::<Vec<_>>();

    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.component_id.cmp(&right.component_id))
    });

    hits
}

fn push_name_chunk(chunk: &str, tokens: &mut Vec<String>) {
    for part in split_identifier_chunk(chunk) {
        push_unique(tokens, part.to_ascii_lowercase(), false);
    }
}

fn push_content_chunk(chunk: &str, tokens: &mut Vec<String>) {
    for token in tokenize_name(chunk) {
        if token.len() < 2 || STOP_WORDS.contains(token.as_str()) {
            continue;
        }

        push_unique(tokens, token, true);
    }
}

fn split_identifier_chunk(chunk: &str) -> Vec<&str> {
    if chunk.is_empty() {
        return Vec::new();
    }

    let bytes = chunk.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;

    for index in 1..bytes.len() {
        let previous = bytes[index - 1] as char;
        let current = bytes[index] as char;
        let next = bytes.get(index + 1).copied().map(char::from);

        if is_name_boundary(previous, current, next) {
            parts.push(&chunk[start..index]);
            start = index;
        }
    }

    parts.push(&chunk[start..]);
    parts
}

fn is_name_boundary(previous: char, current: char, next: Option<char>) -> bool {
    (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
        || (previous.is_ascii_alphabetic() && current.is_ascii_digit())
        || (previous.is_ascii_digit() && current.is_ascii_alphabetic())
        || (previous.is_ascii_uppercase()
            && current.is_ascii_uppercase()
            && next
                .map(|character| character.is_ascii_lowercase())
                .unwrap_or(false))
}

fn push_unique(tokens: &mut Vec<String>, token: String, always_keep: bool) {
    if token.is_empty() {
        return;
    }

    if !always_keep
        && token.len() == 1
        && !token.chars().all(|character| character.is_ascii_digit())
    {
        return;
    }

    if !tokens.iter().any(|existing| existing == &token) {
        tokens.push(token);
    }
}

fn add_tokens(
    component_id: &str,
    tokens: Vec<String>,
    token_to_components: &mut HashMap<String, BTreeSet<String>>,
    component_tokens: &mut HashMap<String, BTreeSet<String>>,
    source_token_to_components: &mut HashMap<String, BTreeSet<String>>,
) {
    let component_id = component_id.to_string();

    for token in tokens {
        token_to_components
            .entry(token.clone())
            .or_default()
            .insert(component_id.clone());
        component_tokens
            .entry(component_id.clone())
            .or_default()
            .insert(token.clone());
        source_token_to_components
            .entry(token)
            .or_default()
            .insert(component_id.clone());
    }
}

fn finalize_index(index: HashMap<String, BTreeSet<String>>) -> HashMap<String, Vec<String>> {
    index
        .into_iter()
        .map(|(token, values)| (token, values.into_iter().collect()))
        .collect()
}

fn name_match(index_token: &str, query_token: &str) -> Option<MatchCandidate> {
    if index_token == query_token {
        Some(MatchCandidate {
            score: 1.0,
            match_type: MatchType::Exact,
        })
    } else if index_token.starts_with(query_token) {
        Some(MatchCandidate {
            score: 0.7,
            match_type: MatchType::Prefix,
        })
    } else if index_token.contains(query_token) {
        Some(MatchCandidate {
            score: 0.4,
            match_type: MatchType::Substring,
        })
    } else {
        None
    }
}

fn update_best_match(
    matches: &mut HashMap<String, MatchCandidate>,
    component_id: &str,
    candidate: MatchCandidate,
) {
    match matches.get(component_id) {
        Some(existing)
            if existing.score > candidate.score
                || (existing.score == candidate.score
                    && existing.match_type.rank() <= candidate.match_type.rank()) => {}
        _ => {
            matches.insert(component_id.to_string(), candidate);
        }
    }
}

fn token_has_component(
    token_index: &HashMap<String, Vec<String>>,
    token: &str,
    component_id: &str,
) -> bool {
    token_index
        .get(token)
        .map(|components| components.iter().any(|value| value == component_id))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{build_token_index, fuzzy_search, tokenize_content, tokenize_name, MatchType};
    use crate::{
        annotations::Annotation,
        index::{ComponentRecord, IndexData},
    };

    #[test]
    fn tokenize_name_splits_identifier_styles() {
        let camel = tokenize_name("parseComponents");
        assert!(camel.contains(&"parse".to_string()));
        assert!(camel.contains(&"components".to_string()));
        assert!(camel.contains(&"parsecomponents".to_string()));

        let snake = tokenize_name("file_hash");
        assert!(snake.contains(&"file".to_string()));
        assert!(snake.contains(&"hash".to_string()));
        assert!(snake.contains(&"file_hash".to_string()));

        let pascal = tokenize_name("IndexData");
        assert!(pascal.contains(&"index".to_string()));
        assert!(pascal.contains(&"data".to_string()));

        let numeric = tokenize_name("blake3Hash");
        assert!(numeric.contains(&"blake".to_string()));
        assert!(numeric.contains(&"3".to_string()));
        assert!(numeric.contains(&"hash".to_string()));

        let simple = tokenize_name("run");
        assert_eq!(simple, vec!["run".to_string()]);
    }

    #[test]
    fn tokenize_content_strips_markdown_and_stop_words() {
        let tokens =
            tokenize_content("# The *Quick* `BrownFox` jumps on the lazy dog in `IndexData`.");

        assert!(tokens.contains(&"quick".to_string()));
        assert!(tokens.contains(&"brown".to_string()));
        assert!(tokens.contains(&"fox".to_string()));
        assert!(tokens.contains(&"lazy".to_string()));
        assert!(tokens.contains(&"index".to_string()));
        assert!(tokens.contains(&"data".to_string()));
        assert!(!tokens.contains(&"the".to_string()));
        assert!(!tokens.contains(&"on".to_string()));
        assert!(tokens
            .iter()
            .all(|token| token == &token.to_ascii_lowercase()));
        assert!(tokens.iter().all(|token| token.len() >= 2));
    }

    #[test]
    fn build_token_index_includes_component_and_annotation_tokens() {
        let index = sample_index(vec![
            component("fn-parse", "parseComponents"),
            component("fn-file", "file_hash"),
        ]);
        let annotations = vec![Annotation {
            hash_id: "fn-file".to_string(),
            component_type: "fn".to_string(),
            component_name: "file_hash".to_string(),
            content: "Searchable **annotation** details".to_string(),
            created_at: "2026-03-17T00:00:00Z".to_string(),
            updated_at: "2026-03-17T00:00:00Z".to_string(),
        }];

        let token_index = build_token_index(&index, &annotations);

        assert_eq!(
            token_index.token_to_components.get("parse"),
            Some(&vec!["fn-parse".to_string()])
        );
        assert_eq!(
            token_index.token_to_components.get("annotation"),
            Some(&vec!["fn-file".to_string()])
        );

        let file_tokens = token_index
            .component_tokens
            .get("fn-file")
            .expect("component tokens should exist");
        assert!(file_tokens.contains(&"file".to_string()));
        assert!(file_tokens.contains(&"hash".to_string()));
        assert!(file_tokens.contains(&"searchable".to_string()));
        assert!(file_tokens.contains(&"annotation".to_string()));
    }

    #[test]
    fn fuzzy_search_scores_exact_prefix_substring_and_annotation_matches() {
        let index = sample_index(vec![
            component("fn-run", "run"),
            component("fn-auth", "authenticateUser"),
            component("fn-component", "componentRegistry"),
            component("fn-notes", "notes"),
        ]);
        let annotations = vec![Annotation {
            hash_id: "fn-notes".to_string(),
            component_type: "fn".to_string(),
            component_name: "notes".to_string(),
            content: "OAuth callback handling details".to_string(),
            created_at: "2026-03-17T00:00:00Z".to_string(),
            updated_at: "2026-03-17T00:00:00Z".to_string(),
        }];
        let token_index = build_token_index(&index, &annotations);

        let exact_hits = fuzzy_search(&token_index, "run");
        assert_eq!(exact_hits[0].component_id, "fn-run");
        assert_eq!(exact_hits[0].match_type, MatchType::Exact);
        assert_close(exact_hits[0].score, 1.0);

        let prefix_hits = fuzzy_search(&token_index, "auth");
        assert_eq!(prefix_hits[0].component_id, "fn-auth");
        assert_eq!(prefix_hits[0].match_type, MatchType::Prefix);
        assert_close(prefix_hits[0].score, 0.7);

        let substring_hits = fuzzy_search(&token_index, "onent");
        assert_eq!(substring_hits[0].component_id, "fn-component");
        assert_eq!(substring_hits[0].match_type, MatchType::Substring);
        assert_close(substring_hits[0].score, 0.4);

        let annotation_hits = fuzzy_search(&token_index, "oauth");
        assert_eq!(annotation_hits[0].component_id, "fn-notes");
        assert_eq!(annotation_hits[0].match_type, MatchType::Annotation);
        assert_close(annotation_hits[0].score, 0.3);
    }

    #[test]
    fn fuzzy_search_sorts_by_score_and_deduplicates_components() {
        let index = sample_index(vec![
            component("fn-parser", "parseComponents"),
            component("fn-parse", "parse"),
            component("fn-notes", "notes"),
        ]);
        let annotations = vec![Annotation {
            hash_id: "fn-notes".to_string(),
            component_type: "fn".to_string(),
            component_name: "notes".to_string(),
            content: "Parse behavior overview".to_string(),
            created_at: "2026-03-17T00:00:00Z".to_string(),
            updated_at: "2026-03-17T00:00:00Z".to_string(),
        }];
        let token_index = build_token_index(&index, &annotations);
        let hits = fuzzy_search(&token_index, "parse components");

        assert_eq!(hits[0].component_id, "fn-parser");
        assert!(hits[0].score > hits[1].score);
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits.iter()
                .filter(|hit| hit.component_id == "fn-parser")
                .count(),
            1
        );
    }

    fn sample_index(components: Vec<ComponentRecord>) -> IndexData {
        IndexData {
            version: 1,
            root: ".".to_string(),
            generated_at: "2026-03-17T00:00:00Z".to_string(),
            languages: vec!["rust".to_string()],
            files: Vec::new(),
            components,
            search_index: None,
        }
    }

    fn component(id: &str, name: &str) -> ComponentRecord {
        ComponentRecord {
            id: id.to_string(),
            language: "rust".to_string(),
            component_type: "fn".to_string(),
            name: name.to_string(),
            file: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 1,
            uses_before: Vec::new(),
            used_by_after: Vec::new(),
            batman: false,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }
}
