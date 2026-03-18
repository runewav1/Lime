use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    annotations::Annotation,
    config::EmbeddingConfig,
    index::{ComponentRecord, IndexData},
    search::{MatchType, SearchHit},
};

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

pub trait EmbeddingProvider {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    fn embed_batched(&self, texts: &[String], batch_size: usize) -> Result<Vec<Vec<f32>>> {
        let mut all_vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(batch_size.max(1)) {
            let mut batch = self.embed(chunk)?;
            all_vectors.append(&mut batch);
        }
        Ok(all_vectors)
    }
}

// ---------------------------------------------------------------------------
// Ollama backend
//
// POST {endpoint}/api/embed
// Request:  { "model": "...", "input": ["text1", "text2"] }
// Response: { "model": "...", "embeddings": [[f32...], [f32...]] }
// ---------------------------------------------------------------------------

pub struct OllamaProvider {
    endpoint: String,
    model: String,
    dims: usize,
    timeout: Duration,
}

impl OllamaProvider {
    pub fn from_config(config: &EmbeddingConfig) -> Result<Self> {
        let endpoint = config.effective_endpoint();
        if endpoint.is_empty() {
            bail!("embeddings.endpoint must be set (or use default ollama port)");
        }
        if config.model_id.is_empty() {
            bail!("embeddings.model_id is required for ollama provider");
        }
        Ok(Self {
            endpoint,
            model: config.model_id.clone(),
            dims: config.dimensions,
            timeout: Duration::from_secs(config.timeout_secs),
        })
    }
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl EmbeddingProvider for OllamaProvider {
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dimensions(&self) -> usize {
        self.dims
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.endpoint.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = ureq::post(&url)
            .timeout(self.timeout)
            .send_json(&body)
            .with_context(|| format!("ollama embed request to {url} failed"))?;

        let parsed: OllamaEmbedResponse = resp
            .into_json()
            .context("failed parsing ollama embed response")?;

        if parsed.embeddings.len() != texts.len() {
            bail!(
                "ollama returned {} embeddings for {} inputs",
                parsed.embeddings.len(),
                texts.len()
            );
        }

        Ok(parsed.embeddings)
    }
}

// ---------------------------------------------------------------------------
// llama.cpp server backend
//
// POST {endpoint}/embedding
// Request:  { "content": ["text1", "text2"] }
//   -or- for older versions: { "content": "text" } (single)
// Response: [{ "embedding": [f32...] }, ...]
//   -or- { "embedding": [f32...] } (single)
//
// llama.cpp /v1/embeddings (OpenAI compat) also supported:
// Request:  { "input": ["text1", "text2"], "model": "..." }
// Response: { "data": [{ "embedding": [f32...] }, ...] }
// ---------------------------------------------------------------------------

pub struct LlamaCppProvider {
    endpoint: String,
    model: String,
    dims: usize,
    timeout: Duration,
}

impl LlamaCppProvider {
    pub fn from_config(config: &EmbeddingConfig) -> Result<Self> {
        let endpoint = config.effective_endpoint();
        if endpoint.is_empty() {
            bail!("embeddings.endpoint must be set for llamacpp provider");
        }
        Ok(Self {
            endpoint,
            model: config.model_id.clone(),
            dims: config.dimensions,
            timeout: Duration::from_secs(config.timeout_secs),
        })
    }
}

#[derive(Deserialize)]
struct LlamaCppV1Response {
    data: Vec<LlamaCppV1Item>,
}

#[derive(Deserialize)]
struct LlamaCppV1Item {
    embedding: Vec<f32>,
}

impl EmbeddingProvider for LlamaCppProvider {
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dimensions(&self) -> usize {
        self.dims
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let base = self.endpoint.trim_end_matches('/');
        let url = format!("{base}/v1/embeddings");

        let body = serde_json::json!({
            "input": texts,
            "model": self.model,
        });

        let resp = ureq::post(&url)
            .timeout(self.timeout)
            .send_json(&body)
            .with_context(|| format!("llamacpp embed request to {url} failed"))?;

        let parsed: LlamaCppV1Response = resp
            .into_json()
            .context("failed parsing llamacpp embed response")?;

        if parsed.data.len() != texts.len() {
            bail!(
                "llamacpp returned {} embeddings for {} inputs",
                parsed.data.len(),
                texts.len()
            );
        }

        Ok(parsed.data.into_iter().map(|item| item.embedding).collect())
    }
}

// ---------------------------------------------------------------------------
// Remote (OpenAI-compatible) backend
//
// POST {endpoint}
// Headers:  Authorization: Bearer {api_key}
// Request:  { "input": ["text1", "text2"], "model": "..." }
// Response: { "data": [{ "embedding": [f32...], "index": 0 }, ...] }
// ---------------------------------------------------------------------------

pub struct RemoteEmbeddingProvider {
    endpoint: String,
    model: String,
    dims: usize,
    api_key: Option<String>,
    timeout: Duration,
}

impl RemoteEmbeddingProvider {
    pub fn from_config(config: &EmbeddingConfig) -> Result<Self> {
        if config.endpoint.is_empty() {
            bail!("embeddings.endpoint must be set for remote provider");
        }
        if config.model_id.is_empty() {
            bail!("embeddings.model_id is required for remote provider");
        }
        let api_key = std::env::var("LIME_EMBEDDING_API_KEY").ok();
        Ok(Self {
            endpoint: config.endpoint.clone(),
            model: config.model_id.clone(),
            dims: config.dimensions,
            api_key,
            timeout: Duration::from_secs(config.timeout_secs),
        })
    }
}

#[derive(Deserialize)]
struct OpenAIEmbedResponse {
    data: Vec<OpenAIEmbedItem>,
}

#[derive(Deserialize)]
struct OpenAIEmbedItem {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

impl EmbeddingProvider for RemoteEmbeddingProvider {
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dimensions(&self) -> usize {
        self.dims
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let body = serde_json::json!({
            "input": texts,
            "model": self.model,
        });

        let mut req = ureq::post(&self.endpoint).timeout(self.timeout);
        if let Some(key) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }

        let resp = req
            .send_json(&body)
            .with_context(|| format!("remote embed request to {} failed", self.endpoint))?;

        let parsed: OpenAIEmbedResponse = resp
            .into_json()
            .context("failed parsing remote embed response")?;

        let mut items = parsed.data;
        items.sort_by_key(|item| item.index);

        if items.len() != texts.len() {
            bail!(
                "remote returned {} embeddings for {} inputs",
                items.len(),
                texts.len()
            );
        }

        Ok(items.into_iter().map(|item| item.embedding).collect())
    }
}

// ---------------------------------------------------------------------------
// Provider factory
// ---------------------------------------------------------------------------

pub fn create_provider(config: &EmbeddingConfig) -> Result<Box<dyn EmbeddingProvider>> {
    match config.provider.as_str() {
        "ollama" => Ok(Box::new(OllamaProvider::from_config(config)?)),
        "llamacpp" => Ok(Box::new(LlamaCppProvider::from_config(config)?)),
        "remote" => Ok(Box::new(RemoteEmbeddingProvider::from_config(config)?)),
        other => bail!(
            "unknown embedding provider: {other}; expected one of: ollama, llamacpp, remote"
        ),
    }
}

// ---------------------------------------------------------------------------
// Embedding document construction
// ---------------------------------------------------------------------------

pub fn build_embedding_document(
    component: &ComponentRecord,
    annotation: Option<&Annotation>,
    file_contents: Option<&str>,
) -> String {
    let mut doc = format!(
        "{} {} {} ({})",
        component.language, component.component_type, component.name, component.file,
    );

    if let Some(source) = file_contents {
        let lines: Vec<&str> = source.lines().collect();
        let start = component.start_line.saturating_sub(1);
        let end = component.end_line.min(lines.len());
        if start < end {
            const MAX_SNIPPET_LINES: usize = 30;
            let snippet_end = end.min(start + MAX_SNIPPET_LINES);
            let snippet = lines[start..snippet_end].join("\n");
            doc.push_str("\n\n");
            doc.push_str(&snippet);
        }
    }

    if let Some(ann) = annotation {
        if !ann.content.trim().is_empty() {
            doc.push_str("\n\n");
            doc.push_str(&ann.content);
        }
        for tag in &ann.tags {
            doc.push(' ');
            doc.push_str(tag);
        }
    }

    doc
}

pub fn build_all_documents(
    index: &IndexData,
    annotations: &[Annotation],
    file_contents: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let annotation_map: HashMap<&str, &Annotation> = annotations
        .iter()
        .map(|a| (a.hash_id.as_str(), a))
        .collect();

    index
        .components
        .iter()
        .map(|component| {
            let ann = annotation_map.get(component.id.as_str()).copied();
            let source = file_contents.get(&component.file).map(String::as_str);
            let doc = build_embedding_document(component, ann, source);
            (component.id.clone(), doc)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Vector storage (in-memory representation)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingStore {
    pub model_id: String,
    pub dimensions: usize,
    pub vectors: Vec<StoredVector>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredVector {
    pub component_id: String,
    pub content_hash: String,
    pub vector: Vec<f32>,
}

impl EmbeddingStore {
    pub fn empty(model_id: &str, dimensions: usize) -> Self {
        Self {
            model_id: model_id.to_string(),
            dimensions,
            vectors: Vec::new(),
        }
    }

    pub fn content_hash_map(&self) -> HashMap<String, String> {
        self.vectors
            .iter()
            .map(|v| (v.component_id.clone(), v.content_hash.clone()))
            .collect()
    }

    pub fn upsert(&mut self, entries: Vec<StoredVector>) {
        let new_ids: std::collections::HashSet<&str> =
            entries.iter().map(|e| e.component_id.as_str()).collect();
        self.vectors.retain(|v| !new_ids.contains(v.component_id.as_str()));
        self.vectors.extend(entries);
    }

    pub fn retain_ids(&mut self, valid_ids: &std::collections::HashSet<String>) {
        self.vectors.retain(|v| valid_ids.contains(&v.component_id));
    }
}

fn vectors_dir(root: &Path) -> PathBuf {
    root.join(".lime").join("vectors")
}

fn safe_model_name(model_id: &str) -> String {
    model_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn vdb_path(root: &Path, model_id: &str) -> PathBuf {
    vectors_dir(root).join(format!("{}.vdb", safe_model_name(model_id)))
}

// ---------------------------------------------------------------------------
// Binary VectorDb format
//
// Header:
//   magic:         [u8; 4]  = b"LVDB"
//   version:       u32 LE   = 1
//   dimensions:    u32 LE
//   count:         u32 LE
//   model_id_len:  u16 LE
//   model_id:      [u8; model_id_len]
//
// Per entry (repeated `count` times):
//   component_id_len:  u16 LE
//   component_id:      [u8; component_id_len]
//   content_hash:      [u8; 16]   (hex-encoded ASCII)
//   vector:            [f32 LE; dimensions]
// ---------------------------------------------------------------------------

const VDB_MAGIC: &[u8; 4] = b"LVDB";
const VDB_VERSION: u32 = 1;

pub fn save_embedding_store(root: &Path, store: &EmbeddingStore) -> Result<()> {
    let dir = vectors_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed creating vectors directory: {}", dir.display()))?;

    let path = vdb_path(root, &store.model_id);
    let dims = store.dimensions as u32;
    let count = store.vectors.len() as u32;
    let model_bytes = store.model_id.as_bytes();

    let vector_byte_size = store.dimensions * 4;
    let estimated = 4 + 4 + 4 + 4 + 2 + model_bytes.len()
        + store.vectors.len() * (2 + 40 + 16 + vector_byte_size);
    let mut buf = Vec::with_capacity(estimated);

    buf.extend_from_slice(VDB_MAGIC);
    buf.extend_from_slice(&VDB_VERSION.to_le_bytes());
    buf.extend_from_slice(&dims.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&(model_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(model_bytes);

    for sv in &store.vectors {
        let id_bytes = sv.component_id.as_bytes();
        buf.extend_from_slice(&(id_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(id_bytes);

        let mut hash_buf = [0u8; 16];
        let hash_bytes = sv.content_hash.as_bytes();
        let copy_len = hash_bytes.len().min(16);
        hash_buf[..copy_len].copy_from_slice(&hash_bytes[..copy_len]);
        buf.extend_from_slice(&hash_buf);

        for &val in &sv.vector {
            buf.extend_from_slice(&val.to_le_bytes());
        }
    }

    fs::write(&path, &buf)
        .with_context(|| format!("failed writing vector store: {}", path.display()))?;
    Ok(())
}

pub fn load_embedding_store(root: &Path, model_id: &str) -> Result<Option<EmbeddingStore>> {
    let path = vdb_path(root, model_id);

    if !path.exists() {
        return try_load_legacy_jsonl(root, model_id);
    }

    let data = fs::read(&path)
        .with_context(|| format!("failed reading vector store: {}", path.display()))?;

    if data.len() < 18 {
        bail!("vector store too small: {}", path.display());
    }

    let mut cursor = 0usize;

    let magic = &data[cursor..cursor + 4];
    if magic != VDB_MAGIC {
        bail!("invalid vector store magic in {}", path.display());
    }
    cursor += 4;

    let version = u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap());
    if version != VDB_VERSION {
        bail!("unsupported vector store version {version} in {}", path.display());
    }
    cursor += 4;

    let dimensions = u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
    cursor += 4;

    let count = u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
    cursor += 4;

    let model_id_len = u16::from_le_bytes(data[cursor..cursor + 2].try_into().unwrap()) as usize;
    cursor += 2;

    let stored_model_id = String::from_utf8_lossy(&data[cursor..cursor + model_id_len]).into_owned();
    cursor += model_id_len;

    let mut vectors = Vec::with_capacity(count);

    for _ in 0..count {
        if cursor + 2 > data.len() {
            break;
        }
        let id_len = u16::from_le_bytes(data[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;

        let component_id = String::from_utf8_lossy(&data[cursor..cursor + id_len]).into_owned();
        cursor += id_len;

        let hash_bytes = &data[cursor..cursor + 16];
        let content_hash = String::from_utf8_lossy(hash_bytes).trim_end_matches('\0').to_string();
        cursor += 16;

        let mut vector = Vec::with_capacity(dimensions);
        for _ in 0..dimensions {
            let val = f32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap());
            vector.push(val);
            cursor += 4;
        }

        vectors.push(StoredVector {
            component_id,
            content_hash,
            vector,
        });
    }

    Ok(Some(EmbeddingStore {
        model_id: stored_model_id,
        dimensions,
        vectors,
    }))
}

fn try_load_legacy_jsonl(root: &Path, model_id: &str) -> Result<Option<EmbeddingStore>> {
    let legacy_dir = root.join(".lime").join("embeddings");
    let legacy_path = legacy_dir.join(format!("{}.jsonl", safe_model_name(model_id)));
    if !legacy_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&legacy_path)
        .with_context(|| format!("failed reading legacy store: {}", legacy_path.display()))?;
    let mut lines = content.lines();

    let header_line = lines.next().context("legacy embedding store is empty")?;
    let header: serde_json::Value =
        serde_json::from_str(header_line).context("failed parsing legacy header")?;

    let store_model_id = header["model_id"].as_str().unwrap_or(model_id).to_string();
    let dimensions = header["dimensions"].as_u64().unwrap_or(0) as usize;

    let mut vectors = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let sv: StoredVector =
            serde_json::from_str(line).context("failed parsing legacy stored vector")?;
        vectors.push(sv);
    }

    Ok(Some(EmbeddingStore {
        model_id: store_model_id,
        dimensions,
        vectors,
    }))
}

// ---------------------------------------------------------------------------
// Similarity search (linear scan; ANN can replace later)
// ---------------------------------------------------------------------------

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

pub fn semantic_search(
    store: &EmbeddingStore,
    query_vector: &[f32],
    top_k: usize,
) -> Vec<SearchHit> {
    let mut scored: Vec<(String, f32)> = store
        .vectors
        .iter()
        .map(|sv| {
            let sim = cosine_similarity(&sv.vector, query_vector);
            (sv.component_id.clone(), sim)
        })
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(top_k);

    scored
        .into_iter()
        .map(|(component_id, sim)| SearchHit {
            component_id,
            score: sim as f64,
            match_type: MatchType::Embedding,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Content hashing (for vector invalidation)
// ---------------------------------------------------------------------------

pub fn content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex()[..16].to_string()
}

// ---------------------------------------------------------------------------
// Incremental embedding sync pipeline
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct EmbedSyncResult {
    pub total_components: usize,
    pub embedded: usize,
    pub reused: usize,
    pub removed: usize,
    pub batches_total: usize,
    pub batches_completed: usize,
    pub failed_batches: usize,
}

const MAX_RETRIES_PER_BATCH: usize = 2;
const MAX_FAILED_BATCHES_BEFORE_ABORT: usize = 3;

pub fn sync_embeddings(
    root: &Path,
    config: &crate::config::EmbeddingConfig,
    chunks: &[crate::chunk::Chunk],
    valid_component_ids: &std::collections::HashSet<String>,
) -> Result<EmbedSyncResult> {
    let provider = create_provider(config)?;
    sync_embeddings_with_provider(root, config, chunks, valid_component_ids, provider.as_ref())
}

fn sync_embeddings_with_provider(
    root: &Path,
    config: &crate::config::EmbeddingConfig,
    chunks: &[crate::chunk::Chunk],
    valid_component_ids: &std::collections::HashSet<String>,
    provider: &dyn EmbeddingProvider,
) -> Result<EmbedSyncResult> {
    let model_id = provider.model_id().to_string();
    let batch_size = config.batch_size.max(1);

    let mut store = load_embedding_store(root, &model_id)?
        .unwrap_or_else(|| EmbeddingStore::empty(&model_id, provider.dimensions()));

    let existing_hashes = store.content_hash_map();

    let mut to_embed: Vec<(&crate::chunk::Chunk, String)> = Vec::new();
    let mut reused = 0usize;

    for chunk in chunks {
        if !valid_component_ids.contains(&chunk.component_id) {
            continue;
        }

        let doc = chunk.embedding_document();
        let hash = content_hash(&doc);

        if existing_hashes.get(&chunk.component_id).map(|h| h.as_str()) == Some(hash.as_str()) {
            reused += 1;
            continue;
        }

        to_embed.push((chunk, doc));
    }

    store.retain_ids(valid_component_ids);
    let removed = existing_hashes.len().saturating_sub(store.vectors.len() + reused);

    let total_to_embed = to_embed.len();
    let batches: Vec<Vec<(&crate::chunk::Chunk, String)>> = to_embed
        .into_iter()
        .collect::<Vec<_>>()
        .chunks(batch_size)
        .map(|c| c.to_vec())
        .collect();

    let batches_total = batches.len();
    let mut batches_completed: usize = 0;
    let mut failed_batches: usize = 0;
    let mut embedded: usize = 0;
    let mut abort_remaining = false;

    for batch in &batches {
        if abort_remaining {
            break;
        }

        let docs: Vec<String> = batch.iter().map(|(_, doc)| doc.clone()).collect();
        let mut succeeded = false;

        for attempt in 0..=MAX_RETRIES_PER_BATCH {
            match provider.embed(&docs) {
                Ok(vectors) => {
                    if store.dimensions == 0 && !vectors.is_empty() {
                        store.dimensions = vectors[0].len();
                    }

                    let new_entries: Vec<StoredVector> = batch
                        .iter()
                        .zip(vectors.into_iter())
                        .map(|((chunk, doc), vector)| StoredVector {
                            component_id: chunk.component_id.clone(),
                            content_hash: content_hash(doc),
                            vector,
                        })
                        .collect();

                    store.upsert(new_entries);
                    embedded += batch.len();
                    batches_completed += 1;
                    succeeded = true;
                    break;
                }
                Err(_) if attempt < MAX_RETRIES_PER_BATCH => {
                    std::thread::sleep(Duration::from_millis(500 * (attempt as u64 + 1)));
                    continue;
                }
                Err(_) => {
                    break;
                }
            }
        }

        if !succeeded {
            failed_batches += 1;
            // If we have never completed a batch, a single failed batch is
            // enough to conclude the provider is unavailable for this run.
            // If we already have some progress, abort once failures start to
            // dominate so we don't grind through the whole repo on errors.
            if batches_completed == 0 || failed_batches >= MAX_FAILED_BATCHES_BEFORE_ABORT {
                abort_remaining = true;
            }
        }
    }

    if batches_completed > 0 || total_to_embed == 0 {
        save_embedding_store(root, &store)?;
    }

    if batches_total > 0 && batches_completed == 0 && failed_batches > 0 {
        bail!(
            "embedding failed: all {failed_batches} batch(es) failed; \
             index-only sync completed, embeddings discarded"
        );
    }

    Ok(EmbedSyncResult {
        total_components: valid_component_ids.len(),
        embedded,
        reused,
        removed,
        batches_total,
        batches_completed,
        failed_batches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{DeathEvidence, DeathStatus};

    fn test_component(id: &str, name: &str) -> ComponentRecord {
        ComponentRecord {
            id: id.to_string(),
            language: "rust".to_string(),
            component_type: "fn".to_string(),
            name: name.to_string(),
            file: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 5,
            uses_before: Vec::new(),
            used_by_after: Vec::new(),
            batman: false,
            death_status: DeathStatus::Alive,
            death_evidence: DeathEvidence::default(),
            faults: crate::diagnostics::ComponentFaults::default(),
        }
    }

    #[test]
    fn embedding_document_includes_component_and_annotation() {
        let component = test_component("fn-test", "my_function");
        let ann = Annotation {
            hash_id: "fn-test".to_string(),
            component_type: "fn".to_string(),
            component_name: "my_function".to_string(),
            content: "Important entry point".to_string(),
            tags: vec!["keep".to_string()],
            created_at: String::new(),
            updated_at: String::new(),
        };
        let doc = build_embedding_document(&component, Some(&ann), None);
        assert!(doc.contains("my_function"));
        assert!(doc.contains("Important entry point"));
        assert!(doc.contains("keep"));
    }

    #[test]
    fn cosine_similarity_computes_correctly() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);
    }

    #[test]
    fn semantic_search_ranks_by_similarity() {
        let store = EmbeddingStore {
            model_id: "test".to_string(),
            dimensions: 3,
            vectors: vec![
                StoredVector {
                    component_id: "fn-a".to_string(),
                    content_hash: "abc".to_string(),
                    vector: vec![1.0, 0.0, 0.0],
                },
                StoredVector {
                    component_id: "fn-b".to_string(),
                    content_hash: "def".to_string(),
                    vector: vec![0.7, 0.7, 0.0],
                },
                StoredVector {
                    component_id: "fn-c".to_string(),
                    content_hash: "ghi".to_string(),
                    vector: vec![0.0, 0.0, 1.0],
                },
            ],
        };

        let query = vec![1.0, 0.0, 0.0];
        let results = semantic_search(&store, &query, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].component_id, "fn-a");
        assert_eq!(results[0].match_type, MatchType::Embedding);
    }

    #[test]
    fn content_hash_is_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn provider_factory_creates_correct_types() {
        let ollama_config = EmbeddingConfig {
            enabled: true,
            provider: "ollama".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            model_id: "nomic-embed-text".to_string(),
            dimensions: 768,
            batch_size: 32,
            timeout_secs: 30,
        };
        let provider = create_provider(&ollama_config).unwrap();
        assert_eq!(provider.model_id(), "nomic-embed-text");
        assert_eq!(provider.dimensions(), 768);

        let llamacpp_config = EmbeddingConfig {
            provider: "llamacpp".to_string(),
            endpoint: "http://127.0.0.1:8080".to_string(),
            model_id: "all-minilm".to_string(),
            dimensions: 384,
            ..ollama_config.clone()
        };
        let provider = create_provider(&llamacpp_config).unwrap();
        assert_eq!(provider.model_id(), "all-minilm");

        let remote_config = EmbeddingConfig {
            provider: "remote".to_string(),
            endpoint: "https://api.openai.com/v1/embeddings".to_string(),
            model_id: "text-embedding-3-small".to_string(),
            dimensions: 1536,
            ..ollama_config
        };
        let provider = create_provider(&remote_config).unwrap();
        assert_eq!(provider.model_id(), "text-embedding-3-small");
    }

    #[test]
    fn effective_endpoint_defaults() {
        let mut config = EmbeddingConfig::default();
        config.provider = "ollama".to_string();
        assert_eq!(config.effective_endpoint(), "http://127.0.0.1:11434");

        config.provider = "llamacpp".to_string();
        assert_eq!(config.effective_endpoint(), "http://127.0.0.1:8080");

        config.provider = "remote".to_string();
        assert!(config.effective_endpoint().is_empty());

        config.endpoint = "https://custom.endpoint.com".to_string();
        assert_eq!(config.effective_endpoint(), "https://custom.endpoint.com");
    }

    #[test]
    fn binary_vdb_roundtrip() {
        let dir = std::env::temp_dir().join("lime_vdb_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".lime").join("vectors")).unwrap();

        let store = EmbeddingStore {
            model_id: "test-model".to_string(),
            dimensions: 4,
            vectors: vec![
                StoredVector {
                    component_id: "fn-abc123".to_string(),
                    content_hash: "hash1234567890ab".to_string(),
                    vector: vec![0.1, 0.2, 0.3, 0.4],
                },
                StoredVector {
                    component_id: "struct-def456".to_string(),
                    content_hash: "hash0987654321cd".to_string(),
                    vector: vec![-0.5, 0.6, -0.7, 0.8],
                },
            ],
        };

        save_embedding_store(&dir, &store).unwrap();
        let loaded = load_embedding_store(&dir, "test-model").unwrap().unwrap();

        assert_eq!(loaded.model_id, "test-model");
        assert_eq!(loaded.dimensions, 4);
        assert_eq!(loaded.vectors.len(), 2);
        assert_eq!(loaded.vectors[0].component_id, "fn-abc123");
        assert_eq!(loaded.vectors[0].content_hash, "hash1234567890ab");
        assert!((loaded.vectors[0].vector[0] - 0.1).abs() < 1e-6);
        assert!((loaded.vectors[0].vector[3] - 0.4).abs() < 1e-6);
        assert_eq!(loaded.vectors[1].component_id, "struct-def456");
        assert!((loaded.vectors[1].vector[0] - (-0.5)).abs() < 1e-6);

        let _ = std::fs::remove_dir_all(&dir);
    }

    struct MockProvider {
        model: String,
        dims: usize,
        fail_on_call: std::cell::RefCell<Vec<bool>>,
    }

    impl MockProvider {
        fn always_ok(dims: usize) -> Self {
            Self {
                model: "mock".to_string(),
                dims,
                fail_on_call: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn with_failures(dims: usize, pattern: Vec<bool>) -> Self {
            Self {
                model: "mock".to_string(),
                dims,
                fail_on_call: std::cell::RefCell::new(pattern),
            }
        }
    }

    impl EmbeddingProvider for MockProvider {
        fn model_id(&self) -> &str { &self.model }
        fn dimensions(&self) -> usize { self.dims }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            let mut pattern = self.fail_on_call.borrow_mut();
            if !pattern.is_empty() {
                let should_fail = pattern.remove(0);
                if should_fail {
                    bail!("simulated provider failure");
                }
            }
            Ok(texts.iter().map(|_| vec![0.1; self.dims]).collect())
        }
    }

    fn make_test_chunks(count: usize) -> Vec<crate::chunk::Chunk> {
        (0..count)
            .map(|i| crate::chunk::Chunk {
                component_id: format!("fn-{i}"),
                name: format!("func_{i}"),
                chunk_type: crate::chunk::ChunkKind::Function,
                language: "rust".to_string(),
                file: "src/main.rs".to_string(),
                start_line: i * 10,
                end_line: i * 10 + 5,
                signature: format!("fn func_{i}()"),
                body: format!("let x = {i};"),
                docstring: None,
                content_hash: format!("hash{i}"),
            })
            .collect()
    }

    #[test]
    fn sync_all_batches_succeed() {
        let dir = std::env::temp_dir().join("lime_sync_ok");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".lime").join("vectors")).unwrap();

        let chunks = make_test_chunks(5);
        let valid_ids: std::collections::HashSet<String> =
            chunks.iter().map(|c| c.component_id.clone()).collect();

        let config = EmbeddingConfig {
            enabled: true,
            provider: "ollama".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            model_id: "mock".to_string(),
            dimensions: 4,
            batch_size: 2,
            timeout_secs: 60,
        };

        let provider = MockProvider::always_ok(4);
        let result = sync_embeddings_with_provider(&dir, &config, &chunks, &valid_ids, &provider).unwrap();

        assert_eq!(result.embedded, 5);
        assert_eq!(result.batches_total, 3);
        assert_eq!(result.batches_completed, 3);
        assert_eq!(result.failed_batches, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_partial_batch_failure_continues() {
        let dir = std::env::temp_dir().join("lime_sync_partial");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".lime").join("vectors")).unwrap();

        let chunks = make_test_chunks(4);
        let valid_ids: std::collections::HashSet<String> =
            chunks.iter().map(|c| c.component_id.clone()).collect();

        let config = EmbeddingConfig {
            enabled: true,
            provider: "ollama".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            model_id: "mock".to_string(),
            dimensions: 4,
            batch_size: 2,
            timeout_secs: 60,
        };

        // Batch 1 succeeds, batch 2: fail all retries (3 calls fail), then done
        let provider = MockProvider::with_failures(4, vec![
            false,              // batch 1 attempt 0: ok
            true, true, true,   // batch 2 attempts 0, 1, 2: fail
        ]);
        let result = sync_embeddings_with_provider(&dir, &config, &chunks, &valid_ids, &provider).unwrap();

        assert_eq!(result.batches_completed, 1);
        assert_eq!(result.failed_batches, 1);
        assert_eq!(result.embedded, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_all_batches_fail_returns_error() {
        let dir = std::env::temp_dir().join("lime_sync_allfail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".lime").join("vectors")).unwrap();

        let chunks = make_test_chunks(2);
        let valid_ids: std::collections::HashSet<String> =
            chunks.iter().map(|c| c.component_id.clone()).collect();

        let config = EmbeddingConfig {
            enabled: true,
            provider: "ollama".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            model_id: "mock".to_string(),
            dimensions: 4,
            batch_size: 2,
            timeout_secs: 60,
        };

        // All retries fail
        let provider = MockProvider::with_failures(4, vec![true, true, true]);
        let result = sync_embeddings_with_provider(&dir, &config, &chunks, &valid_ids, &provider);

        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("all"));
        assert!(err_msg.contains("failed"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_retry_recovers_from_transient_failure() {
        let dir = std::env::temp_dir().join("lime_sync_retry");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".lime").join("vectors")).unwrap();

        let chunks = make_test_chunks(2);
        let valid_ids: std::collections::HashSet<String> =
            chunks.iter().map(|c| c.component_id.clone()).collect();

        let config = EmbeddingConfig {
            enabled: true,
            provider: "ollama".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            model_id: "mock".to_string(),
            dimensions: 4,
            batch_size: 2,
            timeout_secs: 60,
        };

        // First attempt fails, retry succeeds
        let provider = MockProvider::with_failures(4, vec![true, false]);
        let result = sync_embeddings_with_provider(&dir, &config, &chunks, &valid_ids, &provider).unwrap();

        assert_eq!(result.batches_completed, 1);
        assert_eq!(result.failed_batches, 0);
        assert_eq!(result.embedded, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_no_chunks_to_embed_succeeds() {
        let dir = std::env::temp_dir().join("lime_sync_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".lime").join("vectors")).unwrap();

        let chunks: Vec<crate::chunk::Chunk> = Vec::new();
        let valid_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        let config = EmbeddingConfig {
            enabled: true,
            provider: "ollama".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            model_id: "mock".to_string(),
            dimensions: 4,
            batch_size: 2,
            timeout_secs: 60,
        };

        let provider = MockProvider::always_ok(4);
        let result = sync_embeddings_with_provider(&dir, &config, &chunks, &valid_ids, &provider).unwrap();

        assert_eq!(result.embedded, 0);
        assert_eq!(result.batches_total, 0);
        assert_eq!(result.batches_completed, 0);
        assert_eq!(result.failed_batches, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedding_store_upsert_and_retain() {
        let mut store = EmbeddingStore::empty("test", 3);
        store.upsert(vec![
            StoredVector {
                component_id: "fn-a".to_string(),
                content_hash: "h1".to_string(),
                vector: vec![1.0, 0.0, 0.0],
            },
            StoredVector {
                component_id: "fn-b".to_string(),
                content_hash: "h2".to_string(),
                vector: vec![0.0, 1.0, 0.0],
            },
        ]);
        assert_eq!(store.vectors.len(), 2);

        store.upsert(vec![StoredVector {
            component_id: "fn-a".to_string(),
            content_hash: "h1_updated".to_string(),
            vector: vec![0.5, 0.5, 0.0],
        }]);
        assert_eq!(store.vectors.len(), 2);
        let a = store.vectors.iter().find(|v| v.component_id == "fn-a").unwrap();
        assert_eq!(a.content_hash, "h1_updated");

        let valid: std::collections::HashSet<String> =
            ["fn-a".to_string()].into_iter().collect();
        store.retain_ids(&valid);
        assert_eq!(store.vectors.len(), 1);
        assert_eq!(store.vectors[0].component_id, "fn-a");
    }
}
