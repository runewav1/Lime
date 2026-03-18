use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::LazyLock,
};

use serde::{Deserialize, Serialize};

use crate::{annotations::Annotation, index::IndexData};

/// Static alias map for high-value morphological roots that cannot be
/// reliably derived via suffix stripping alone (e.g. `dead` ↔ `death`,
/// `config` ↔ `configuration`).  Keys are canonical roots; values are
/// related forms that should be treated as siblings during search.
static ROOT_ALIASES: LazyLock<HashMap<&'static str, &'static [&'static str]>> = LazyLock::new(|| {
    HashMap::from([
        ("config", &["configuration", "configure", "configured", "configs"][..]),
        ("dead", &["death", "deaths"]),
        ("delete", &["deletion", "deleted", "deleting"]),
        ("create", &["creation", "created", "creating"]),
        ("execute", &["execution", "executed", "executing"]),
        ("migrate", &["migration", "migrated", "migrating", "migrations"]),
        ("validate", &["validation", "validated", "validating"]),
        ("authenticate", &["authentication", "authenticated", "auth"]),
        ("authorize", &["authorization", "authorized", "auth"]),
        ("connect", &["connection", "connected", "connections"]),
        ("depend", &["dependency", "dependencies", "dependent", "dependence"]),
        ("register", &["registration", "registered"]),
        ("generate", &["generation", "generated", "generator"]),
        ("transform", &["transformation", "transformed"]),
        ("compile", &["compilation", "compiled", "compiler"]),
        ("serialize", &["serialization", "serialized", "serializer"]),
        ("deserialize", &["deserialization", "deserialized"]),
        ("optimize", &["optimization", "optimized", "optimizer"]),
        ("initialize", &["initialization", "initialized", "init"]),
        ("resolve", &["resolution", "resolved", "resolver"]),
        ("define", &["definition", "defined", "definitions"]),
        ("describe", &["description", "described"]),
        ("inject", &["injection", "injected"]),
        ("parse", &["parser", "parsing", "parsed"]),
        ("render", &["renderer", "rendering", "rendered"]),
        ("dispatch", &["dispatcher", "dispatched", "dispatching"]),
        ("subscribe", &["subscription", "subscribed", "subscriber"]),
        ("emit", &["emitter", "emitting", "emitted", "emission"]),
        ("destruct", &["destruction", "destructor"]),
        ("construct", &["construction", "constructor"]),
        ("compose", &["composition", "composed", "composer"]),
        ("assert", &["assertion", "assertions", "asserted"]),
        ("allocate", &["allocation", "allocated", "allocator"]),
        ("encrypt", &["encryption", "encrypted"]),
        ("decrypt", &["decryption", "decrypted"]),
        ("navigate", &["navigation", "navigated"]),
        ("iterate", &["iteration", "iterator", "iterable"]),
        ("mutate", &["mutation", "mutable", "mutated"]),
        ("aggregate", &["aggregation", "aggregated", "aggregator"]),
        ("annotate", &["annotation", "annotated", "annotations"]),
        ("deprecate", &["deprecation", "deprecated"]),
        ("synchronize", &["synchronization", "synchronized", "sync"]),
    ])
});

static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "do", "does",
        "for", "from", "if", "in", "into", "is", "it", "its", "no", "not", "of", "on", "or", "our",
        "out", "over", "per", "so", "such", "than", "that", "the", "their", "them", "there",
        "these", "they", "this", "those", "to", "under", "up", "via", "was", "we", "were", "will",
        "with",
    ])
});

// ---------------------------------------------------------------------------
// Match types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub component_id: String,
    pub score: f64,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    Exact,
    Prefix,
    Substring,
    Stem,
    Fuzzy,
    Annotation,
    Embedding,
}

impl MatchType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchType::Exact => "exact",
            MatchType::Prefix => "prefix",
            MatchType::Substring => "substring",
            MatchType::Stem => "stem",
            MatchType::Fuzzy => "fuzzy",
            MatchType::Annotation => "annotation",
            MatchType::Embedding => "embedding",
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            MatchType::Exact => 0,
            MatchType::Prefix => 1,
            MatchType::Substring => 2,
            MatchType::Stem => 3,
            MatchType::Fuzzy => 4,
            MatchType::Annotation => 5,
            MatchType::Embedding => 6,
        }
    }
}

// ---------------------------------------------------------------------------
// Unified search hit (multi-channel)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchChannel {
    LexicalName,
    TokenFuzzy,
    StemMatch,
    AnnotationText,
    Embedding,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedSearchHit {
    pub component_id: String,
    pub score: f64,
    pub match_type: MatchType,
    pub channels: Vec<SearchChannel>,
}

// ---------------------------------------------------------------------------
// Token index (in-memory, persistable)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchTokenIndex {
    pub token_to_components: HashMap<String, Vec<String>>,
    pub component_tokens: HashMap<String, Vec<String>>,
    #[serde(default)]
    name_token_to_components: HashMap<String, Vec<String>>,
    #[serde(default)]
    annotation_token_to_components: HashMap<String, Vec<String>>,
    #[serde(default)]
    ngram_index: HashMap<String, Vec<String>>,
    /// Stem → set of original tokens that share this stem.
    #[serde(default)]
    stem_index: HashMap<String, Vec<String>>,
}

impl SearchTokenIndex {
    fn all_tokens(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.token_to_components.iter()
    }

    fn contains_component(&self, component_id: &str) -> bool {
        self.component_tokens.contains_key(component_id)
    }

    fn candidate_tokens_for(&self, query_token: &str) -> HashSet<String> {
        let trigrams = generate_trigrams(query_token);
        if trigrams.is_empty() {
            return self.token_to_components.keys().cloned().collect();
        }

        let mut candidates = HashSet::new();
        for trigram in &trigrams {
            if let Some(tokens) = self.ngram_index.get(trigram) {
                for token in tokens {
                    candidates.insert(token.clone());
                }
            }
        }
        candidates
    }

    fn stem_siblings_for(&self, query_token: &str) -> Vec<String> {
        let mut siblings = HashSet::new();

        let query_stem = stem(query_token);
        if let Some(stem_sibs) = self.stem_index.get(&query_stem) {
            for s in stem_sibs {
                if s.as_str() != query_token {
                    siblings.insert(s.clone());
                }
            }
        }

        for (root, aliases) in ROOT_ALIASES.iter() {
            let lower = query_token.to_ascii_lowercase();
            let is_member = *root == lower || aliases.contains(&lower.as_str());
            if !is_member {
                continue;
            }
            if *root != lower {
                if let Some(comps) = self.token_to_components.get(*root) {
                    if !comps.is_empty() {
                        siblings.insert(root.to_string());
                    }
                }
            }
            for alias in *aliases {
                if *alias != lower {
                    if let Some(comps) = self.token_to_components.get(*alias) {
                        if !comps.is_empty() {
                            siblings.insert(alias.to_string());
                        }
                    }
                }
            }
        }

        siblings.into_iter().collect()
    }
}

/// Serializable snapshot for persistence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedTokenIndex {
    pub version: u32,
    pub token_to_components: BTreeMap<String, Vec<String>>,
    pub component_tokens: BTreeMap<String, Vec<String>>,
    pub name_token_to_components: BTreeMap<String, Vec<String>>,
    pub annotation_token_to_components: BTreeMap<String, Vec<String>>,
    pub ngram_index: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub stem_index: BTreeMap<String, Vec<String>>,
}

impl From<&SearchTokenIndex> for PersistedTokenIndex {
    fn from(idx: &SearchTokenIndex) -> Self {
        Self {
            version: 1,
            token_to_components: idx.token_to_components.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            component_tokens: idx.component_tokens.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            name_token_to_components: idx.name_token_to_components.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            annotation_token_to_components: idx.annotation_token_to_components.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            ngram_index: idx.ngram_index.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            stem_index: idx.stem_index.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

impl From<PersistedTokenIndex> for SearchTokenIndex {
    fn from(p: PersistedTokenIndex) -> Self {
        Self {
            token_to_components: p.token_to_components.into_iter().collect(),
            component_tokens: p.component_tokens.into_iter().collect(),
            name_token_to_components: p.name_token_to_components.into_iter().collect(),
            annotation_token_to_components: p.annotation_token_to_components.into_iter().collect(),
            ngram_index: p.ngram_index.into_iter().collect(),
            stem_index: p.stem_index.into_iter().collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Trigram / n-gram generation
// ---------------------------------------------------------------------------

fn generate_trigrams(token: &str) -> Vec<String> {
    if token.len() < 3 {
        return Vec::new();
    }
    let bytes = token.as_bytes();
    let mut trigrams = Vec::with_capacity(bytes.len().saturating_sub(2));
    for window in bytes.windows(3) {
        trigrams.push(String::from_utf8_lossy(window).into_owned());
    }
    trigrams.sort();
    trigrams.dedup();
    trigrams
}

// ---------------------------------------------------------------------------
// Lightweight English suffix stemmer
// ---------------------------------------------------------------------------

pub fn stem(word: &str) -> String {
    if word.len() < 4 {
        return word.to_string();
    }

    let mut s = word.to_string();

    // Plural / verb forms: highest specificity first
    if s.ends_with("nesses") {
        s.truncate(s.len() - 6);
    } else if s.ends_with("ities") {
        s.truncate(s.len() - 5);
        s.push_str("ity");
    } else if s.ends_with("ments") {
        s.truncate(s.len() - 5);
        s.push_str("ment");
    } else if s.ends_with("ations") {
        s.truncate(s.len() - 6);
        s.push_str("ation");
    } else if s.ends_with("ences") {
        s.truncate(s.len() - 5);
        s.push_str("ence");
    } else if s.ends_with("ances") {
        s.truncate(s.len() - 5);
        s.push_str("ance");
    } else if s.ends_with("ables") {
        s.truncate(s.len() - 5);
        s.push_str("able");
    } else if s.ends_with("ibles") {
        s.truncate(s.len() - 5);
        s.push_str("ible");
    } else if s.ends_with("iers") {
        s.truncate(s.len() - 4);
        s.push_str("ier");
    } else if s.ends_with("ious") || s.ends_with("eous") {
        // keep as-is (precious, gorgeous)
    } else if s.ends_with("ies") && s.len() > 4 {
        s.truncate(s.len() - 3);
        s.push('y');
    } else if s.ends_with("ness") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("ment") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("able") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("ible") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("ation") && s.len() > 6 {
        s.truncate(s.len() - 5);
    } else if s.ends_with("tion") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("sion") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("ence") || s.ends_with("ance") {
        // keep: these are root forms (dependence, performance)
    } else if s.ends_with("ings") && s.len() > 5 {
        s.truncate(s.len() - 4);
    } else if s.ends_with("ers") && s.len() > 4 {
        s.truncate(s.len() - 3);
    } else if s.ends_with("ing") && s.len() > 4 {
        s.truncate(s.len() - 3);
        // handle doubling: "running" → "runn" → just check trailing double
        let bytes = s.as_bytes();
        if bytes.len() >= 2 && bytes[bytes.len() - 1] == bytes[bytes.len() - 2] {
            s.pop();
        }
    } else if s.ends_with("ed") && s.len() > 4 && !s.ends_with("eed") {
        s.truncate(s.len() - 2);
        let bytes = s.as_bytes();
        if bytes.len() >= 2 && bytes[bytes.len() - 1] == bytes[bytes.len() - 2] {
            s.pop();
        }
    } else if s.ends_with("er") && s.len() > 4 && !s.ends_with("eer") {
        s.truncate(s.len() - 2);
    } else if s.ends_with("ly") && s.len() > 4 {
        s.truncate(s.len() - 2);
    } else if s.ends_with("es") && s.len() > 4 {
        // dependencies → dependenci → apply 'ies' rule won't catch it since we
        // already went past that branch; handle the common case
        s.truncate(s.len() - 2);
        if s.ends_with("ci") {
            s.truncate(s.len() - 1);
            s.push('y');
        } else if s.ends_with("ss") || s.ends_with("sh") || s.ends_with("ch") || s.ends_with("x") {
            // buses, crashes, etc — put the 'es' back (it's the root form w/ es)
            s.push_str("es");
        }
    } else if s.ends_with('s') && s.len() > 4 && !s.ends_with("ss") {
        s.pop();
    }

    if s.ends_with("th") && s.len() > 4 {
        s.truncate(s.len() - 2);
    }

    if s.ends_with('e') && s.len() > 4 {
        s.pop();
    }

    s
}

// ---------------------------------------------------------------------------
// Edit distance (Damerau-Levenshtein, bounded)
// ---------------------------------------------------------------------------

fn edit_distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let a_len = a.len();
    let b_len = b.len();
    if a_len.abs_diff(b_len) > max {
        return None;
    }

    let mut prev = vec![0usize; b_len + 1];
    let mut curr = vec![0usize; b_len + 1];

    for j in 0..=b_len {
        prev[j] = j;
    }

    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    for i in 1..=a_len {
        curr[0] = i;
        let mut min_in_row = curr[0];
        for j in 1..=b_len {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
            min_in_row = min_in_row.min(curr[j]);
        }
        if min_in_row > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    let distance = prev[b_len];
    if distance <= max {
        Some(distance)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Match candidate
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct MatchCandidate {
    score: f64,
    match_type: MatchType,
}

// ---------------------------------------------------------------------------
// Tokenization
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Token index construction
// ---------------------------------------------------------------------------

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

        let mut ann_tokens = tokenize_content(&annotation.content);
        for tag in &annotation.tags {
            let tag_lower = tag.to_ascii_lowercase();
            if !ann_tokens.contains(&tag_lower) {
                ann_tokens.push(tag_lower);
            }
        }

        add_tokens(
            &annotation.hash_id,
            ann_tokens,
            &mut token_to_components,
            &mut component_tokens,
            &mut annotation_token_to_components,
        );
    }

    let finalized_ttc = finalize_index(token_to_components);

    let ngram_index = build_ngram_index(&finalized_ttc);
    let stem_index = build_stem_index(&finalized_ttc);

    SearchTokenIndex {
        token_to_components: finalized_ttc,
        component_tokens: finalize_index(component_tokens),
        name_token_to_components: finalize_index(name_token_to_components),
        annotation_token_to_components: finalize_index(annotation_token_to_components),
        ngram_index,
        stem_index,
    }
}

fn build_ngram_index(token_to_components: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
    let mut ngram_map: HashMap<String, BTreeSet<String>> = HashMap::new();
    for token in token_to_components.keys() {
        for trigram in generate_trigrams(token) {
            ngram_map.entry(trigram).or_default().insert(token.clone());
        }
    }
    ngram_map
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

fn build_stem_index(token_to_components: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
    let mut stem_map: HashMap<String, BTreeSet<String>> = HashMap::new();
    for token in token_to_components.keys() {
        if token.len() >= 3 {
            let s = stem(token);
            stem_map.entry(s).or_default().insert(token.clone());
        }
    }

    for (root, aliases) in ROOT_ALIASES.iter() {
        let shared_stem = stem(root);
        let group = stem_map.entry(shared_stem).or_default();
        if token_to_components.contains_key(*root) {
            group.insert(root.to_string());
        }
        for alias in *aliases {
            if token_to_components.contains_key(*alias) {
                group.insert(alias.to_string());
            }
        }
    }

    stem_map
        .into_iter()
        .filter(|(_, tokens)| !tokens.is_empty())
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

// ---------------------------------------------------------------------------
// Fuzzy search (ngram-accelerated)
// ---------------------------------------------------------------------------

pub fn fuzzy_search(token_index: &SearchTokenIndex, query: &str) -> Vec<SearchHit> {
    let query_tokens = tokenize_name(query);
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let mut total_scores = HashMap::<String, f64>::new();
    let mut best_match_types = HashMap::<String, MatchCandidate>::new();

    for query_token in query_tokens {
        let mut token_matches = HashMap::<String, MatchCandidate>::new();

        let candidates = token_index.candidate_tokens_for(&query_token);
        let scan_tokens: Box<dyn Iterator<Item = (&String, &Vec<String>)>> = if candidates.is_empty() {
            Box::new(token_index.all_tokens())
        } else {
            Box::new(
                candidates.iter().filter_map(|tok| {
                    token_index
                        .token_to_components
                        .get_key_value(tok)
                        .map(|(k, v)| (k, v))
                })
            )
        };

        for (index_token, component_ids) in scan_tokens {
            let name_candidate = name_match_with_fuzzy(index_token, &query_token);
            let annotation_candidate = if index_token.contains(query_token.as_str()) {
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

        // Stem-based matching: find sibling tokens that share the same stem
        let stem_siblings = token_index.stem_siblings_for(&query_token);
        for sibling_token in &stem_siblings {
            if let Some(component_ids) = token_index.token_to_components.get(sibling_token) {
                let stem_score = stem_similarity_score(&query_token, sibling_token);
                let candidate = MatchCandidate {
                    score: stem_score,
                    match_type: MatchType::Stem,
                };
                for component_id in component_ids {
                    if !token_index.contains_component(component_id) {
                        continue;
                    }
                    if token_has_component(
                        &token_index.name_token_to_components,
                        sibling_token,
                        component_id,
                    ) || token_has_component(
                        &token_index.annotation_token_to_components,
                        sibling_token,
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

// ---------------------------------------------------------------------------
// Merge multiple search channels into unified hits
// ---------------------------------------------------------------------------

pub fn merge_search_hits(
    lexical: &[SearchHit],
    fuzzy: &[SearchHit],
    embedding: &[SearchHit],
) -> Vec<UnifiedSearchHit> {
    let mut map: HashMap<String, (f64, MatchType, Vec<SearchChannel>)> = HashMap::new();

    for hit in lexical {
        let entry = map.entry(hit.component_id.clone()).or_insert((0.0, MatchType::Exact, Vec::new()));
        entry.0 += hit.score * 2.0;
        if hit.match_type.rank() < entry.1.rank() {
            entry.1 = hit.match_type;
        }
        if !entry.2.contains(&SearchChannel::LexicalName) {
            entry.2.push(SearchChannel::LexicalName);
        }
    }

    for hit in fuzzy {
        let entry = map.entry(hit.component_id.clone()).or_insert((0.0, hit.match_type, Vec::new()));
        entry.0 += hit.score;
        if hit.match_type.rank() < entry.1.rank() {
            entry.1 = hit.match_type;
        }
        let channel = match hit.match_type {
            MatchType::Annotation => SearchChannel::AnnotationText,
            MatchType::Stem => SearchChannel::StemMatch,
            _ => SearchChannel::TokenFuzzy,
        };
        if !entry.2.contains(&channel) {
            entry.2.push(channel);
        }
    }

    for hit in embedding {
        let entry = map.entry(hit.component_id.clone()).or_insert((0.0, hit.match_type, Vec::new()));
        entry.0 += hit.score;
        if hit.match_type.rank() < entry.1.rank() {
            entry.1 = hit.match_type;
        }
        if !entry.2.contains(&SearchChannel::Embedding) {
            entry.2.push(SearchChannel::Embedding);
        }
    }

    let mut results: Vec<UnifiedSearchHit> = map
        .into_iter()
        .map(|(component_id, (score, match_type, channels))| UnifiedSearchHit {
            component_id,
            score,
            match_type,
            channels,
        })
        .collect();

    results.sort_by(|a, b| {
        a.match_type
            .rank()
            .cmp(&b.match_type.rank())
            .then(b.score.total_cmp(&a.score))
            .then(a.component_id.cmp(&b.component_id))
    });

    results
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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

fn name_match_with_fuzzy(index_token: &str, query_token: &str) -> Option<MatchCandidate> {
    if let Some(candidate) = name_match(index_token, query_token) {
        return Some(candidate);
    }

    let max_dist = match query_token.len() {
        0..=2 => return None,
        3..=5 => 1,
        _ => 2,
    };

    if let Some(dist) = edit_distance(index_token, query_token, max_dist) {
        let max_len = index_token.len().max(query_token.len()) as f64;
        let similarity = 1.0 - (dist as f64 / max_len);
        Some(MatchCandidate {
            score: 0.2 * similarity,
            match_type: MatchType::Fuzzy,
        })
    } else {
        None
    }
}

fn stem_similarity_score(query_token: &str, sibling_token: &str) -> f64 {
    let is_alias_pair = is_alias_connected(query_token, sibling_token);

    let query_stem = stem(query_token);
    let sibling_stem = stem(sibling_token);
    if query_stem != sibling_stem && !is_alias_pair {
        return 0.0;
    }

    let shared_prefix = query_token
        .chars()
        .zip(sibling_token.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let max_len = query_token.len().max(sibling_token.len());
    let prefix_ratio = shared_prefix as f64 / max_len as f64;

    let base = if is_alias_pair { 0.45 } else { 0.35 };
    base + (0.20 * prefix_ratio)
}

fn is_alias_connected(a: &str, b: &str) -> bool {
    let la = a.to_ascii_lowercase();
    let lb = b.to_ascii_lowercase();
    for (root, aliases) in ROOT_ALIASES.iter() {
        let a_in = *root == la || aliases.contains(&la.as_str());
        let b_in = *root == lb || aliases.contains(&lb.as_str());
        if a_in && b_in {
            return true;
        }
    }
    false
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        build_token_index, edit_distance, fuzzy_search, generate_trigrams, name_match_with_fuzzy,
        tokenize_content, tokenize_name, MatchType, PersistedTokenIndex,
    };
    use crate::{
        annotations::Annotation,
        index::{ComponentRecord, DeathEvidence, DeathStatus, IndexData},
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
            tags: Vec::new(),
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

        assert!(!token_index.ngram_index.is_empty());
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
            tags: Vec::new(),
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
            tags: Vec::new(),
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

    #[test]
    fn trigram_generation_produces_expected_ngrams() {
        let trigrams = generate_trigrams("parse");
        assert!(trigrams.contains(&"par".to_string()));
        assert!(trigrams.contains(&"ars".to_string()));
        assert!(trigrams.contains(&"rse".to_string()));
        assert_eq!(trigrams.len(), 3);

        assert!(generate_trigrams("ab").is_empty());
    }

    #[test]
    fn edit_distance_computes_correctly() {
        assert_eq!(edit_distance("kitten", "sitting", 3), Some(3));
        assert_eq!(edit_distance("abc", "abc", 0), Some(0));
        assert_eq!(edit_distance("abc", "abd", 1), Some(1));
        assert_eq!(edit_distance("abc", "xyz", 2), None);
    }

    #[test]
    fn fuzzy_match_type_catches_typos() {
        let candidate = name_match_with_fuzzy("authenticate", "authentcate");
        assert!(candidate.is_some());
        let c = candidate.unwrap();
        assert_eq!(c.match_type, MatchType::Fuzzy);
        assert!(c.score > 0.0);
    }

    #[test]
    fn token_index_roundtrips_through_persistence() {
        let index = sample_index(vec![component("fn-a", "hello")]);
        let token_index = build_token_index(&index, &[]);
        let persisted = PersistedTokenIndex::from(&token_index);
        let json = serde_json::to_string(&persisted).unwrap();
        let restored: PersistedTokenIndex = serde_json::from_str(&json).unwrap();
        let rehydrated = super::SearchTokenIndex::from(restored);
        assert_eq!(
            token_index.token_to_components.get("hello"),
            rehydrated.token_to_components.get("hello")
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
            death_status: DeathStatus::Alive,
            death_evidence: DeathEvidence::default(),
            faults: crate::diagnostics::ComponentFaults::default(),
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn stem_reduces_common_suffixes() {
        use super::stem;
        assert_eq!(stem("dependencies"), stem("dependency"));
        assert_eq!(stem("parsing"), stem("parse"));
        assert_eq!(stem("parsers"), stem("parser"));
        assert_eq!(stem("components"), stem("component"));
        assert_eq!(stem("running"), stem("run"));
        assert_eq!(stem("authenticated"), stem("authenticat"));
        assert_eq!(stem("searchable"), stem("search"));
        assert_eq!(stem("abilities"), stem("ability"));
        assert_eq!(stem("performances"), stem("performance"));
    }

    #[test]
    fn stem_index_groups_related_tokens() {
        let index = sample_index(vec![
            component("fn-dep", "dependency"),
            component("fn-deps", "dependencies"),
            component("fn-parse", "parse"),
            component("fn-parser", "parser"),
        ]);
        let token_index = build_token_index(&index, &[]);

        let dep_stem = super::stem("dependency");
        let siblings = token_index.stem_index.get(&dep_stem);
        assert!(siblings.is_some(), "stem index should contain '{dep_stem}'");
        let sibs = siblings.unwrap();
        assert!(sibs.contains(&"dependency".to_string()));
        assert!(sibs.contains(&"dependencies".to_string()));
    }

    #[test]
    fn alias_dead_finds_death() {
        let index = sample_index(vec![
            component("fn-die", "death_handler"),
            component("fn-alive", "keepAlive"),
        ]);
        let token_index = build_token_index(&index, &[]);
        let hits = fuzzy_search(&token_index, "dead");
        let ids: Vec<&str> = hits.iter().map(|h| h.component_id.as_str()).collect();
        assert!(ids.contains(&"fn-die"), "searching 'dead' should find 'death_handler' via alias, got: {ids:?}");
    }

    #[test]
    fn alias_config_finds_configuration() {
        let index = sample_index(vec![
            component("fn-cfg", "loadConfiguration"),
            component("fn-set", "configureApp"),
            component("fn-misc", "unrelated"),
        ]);
        let token_index = build_token_index(&index, &[]);
        let hits = fuzzy_search(&token_index, "config");
        let ids: Vec<&str> = hits.iter().map(|h| h.component_id.as_str()).collect();
        assert!(ids.contains(&"fn-cfg"), "searching 'config' should find 'loadConfiguration', got: {ids:?}");
        assert!(ids.contains(&"fn-set"), "searching 'config' should find 'configureApp', got: {ids:?}");
    }

    #[test]
    fn alias_configuration_finds_config() {
        let index = sample_index(vec![
            component("fn-cfg", "config_loader"),
            component("fn-misc", "unrelated"),
        ]);
        let token_index = build_token_index(&index, &[]);
        let hits = fuzzy_search(&token_index, "configuration");
        let ids: Vec<&str> = hits.iter().map(|h| h.component_id.as_str()).collect();
        assert!(ids.contains(&"fn-cfg"), "searching 'configuration' should find 'config_loader', got: {ids:?}");
    }

    #[test]
    fn alias_does_not_overpower_exact() {
        let index = sample_index(vec![
            component("fn-exact", "dead"),
            component("fn-alias", "death_handler"),
        ]);
        let token_index = build_token_index(&index, &[]);
        let hits = fuzzy_search(&token_index, "dead");
        assert!(hits.len() >= 2);
        assert_eq!(hits[0].component_id, "fn-exact", "exact match should rank first");
        assert!(hits[0].score > hits[1].score, "exact score should be higher than alias score");
    }

    #[test]
    fn stem_tion_strips_suffix() {
        use super::stem;
        let s_config = stem("configuration");
        let s_delete = stem("deletion");
        let s_create = stem("creation");
        let s_execute = stem("execution");
        assert!(!s_config.contains("tion"), "configuration stem should strip -ation: {s_config}");
        assert!(!s_delete.contains("tion"), "deletion stem should strip -tion: {s_delete}");
        assert!(!s_create.contains("tion"), "creation stem should strip -ation: {s_create}");
        assert!(!s_execute.contains("tion"), "execution stem should strip -tion: {s_execute}");
    }

    #[test]
    fn stem_th_strips_suffix() {
        use super::stem;
        assert_eq!(stem("death"), "dea");
        assert_eq!(stem("growth"), "grow");
        assert_eq!(stem("health"), "heal");
    }

    #[test]
    fn stem_search_finds_morphological_variants() {
        let index = sample_index(vec![
            component("fn-dep", "resolveDependency"),
            component("fn-deps", "loadDependencies"),
            component("fn-unrelated", "handleAuth"),
        ]);
        let token_index = build_token_index(&index, &[]);

        let hits = fuzzy_search(&token_index, "dependency");
        let hit_ids: Vec<&str> = hits.iter().map(|h| h.component_id.as_str()).collect();
        assert!(hit_ids.contains(&"fn-dep"), "should find resolveDependency");
        assert!(hit_ids.contains(&"fn-deps"), "should find loadDependencies via stem");

        let deps_hit = hits.iter().find(|h| h.component_id == "fn-deps").unwrap();
        assert!(
            deps_hit.match_type == MatchType::Stem || deps_hit.match_type == MatchType::Prefix
                || deps_hit.match_type == MatchType::Substring,
            "match for loadDependencies should be stem, prefix, or substring"
        );
    }
}
