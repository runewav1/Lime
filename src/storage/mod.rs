use std::{
    collections::HashMap,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use anyhow::{Context, Result};

use crate::{
    config::LimeConfig,
    diagnostics::DiagnosticEntry,
    index::IndexData,
    search::{PersistedTokenIndex, SearchTokenIndex},
};

/// Loads index data if present; otherwise returns an empty index.
pub fn load_index_or_empty(root: &Path, config: &LimeConfig) -> Result<IndexData> {
    let index_path = config.index_path(root);
    if !index_path.exists() {
        return Ok(IndexData::empty(root));
    }

    let content = std::fs::read_to_string(&index_path)
        .with_context(|| format!("failed reading index file: {}", index_path.display()))?;
    let parsed: IndexData = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing index json: {}", index_path.display()))?;
    Ok(parsed)
}

/// Persists index data to configured storage location.
pub fn save_index(root: &Path, config: &LimeConfig, index: &IndexData) -> Result<()> {
    let index_path = config.index_path(root);
    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed creating index directory: {}", parent.display()))?;
    }

    let f = File::create(&index_path).context("failed creating index file")?;
    let mut w = BufWriter::new(f);
    serde_json::to_writer_pretty(&mut w, index).context("failed serializing index json")?;
    w.write_all(b"\n")
        .context("failed writing index file trailing newline")?;
    w.flush()
        .with_context(|| format!("failed writing index file: {}", index_path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Token index persistence
// ---------------------------------------------------------------------------

fn token_index_path(root: &Path) -> std::path::PathBuf {
    root.join(".lime").join("tokens.json")
}

pub fn save_token_index(root: &Path, token_index: &SearchTokenIndex) -> Result<()> {
    let path = token_index_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed creating token index directory: {}", parent.display()))?;
    }

    let persisted = PersistedTokenIndex::from(token_index);
    let f = File::create(&path).context("failed creating token index file")?;
    let mut w = BufWriter::new(f);
    serde_json::to_writer(&mut w, &persisted).context("failed serializing token index")?;
    w.write_all(b"\n")
        .with_context(|| format!("failed writing token index: {}", path.display()))?;
    w.flush()
        .with_context(|| format!("failed writing token index: {}", path.display()))?;
    Ok(())
}

pub fn load_token_index(root: &Path) -> Result<Option<SearchTokenIndex>> {
    let path = token_index_path(root);
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed reading token index: {}", path.display()))?;
    let persisted: PersistedTokenIndex = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing token index: {}", path.display()))?;
    Ok(Some(SearchTokenIndex::from(persisted)))
}

// ---------------------------------------------------------------------------
// Diagnostics cache persistence
// ---------------------------------------------------------------------------

fn diagnostics_cache_path(root: &Path) -> std::path::PathBuf {
    root.join(".lime").join("diagnostics.json")
}

pub fn save_diagnostics_cache(
    root: &Path,
    cache: &HashMap<String, Vec<DiagnosticEntry>>,
) -> Result<()> {
    let path = diagnostics_cache_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed creating diagnostics cache dir: {}", parent.display()))?;
    }
    let f = File::create(&path).context("failed creating diagnostics cache file")?;
    let mut w = BufWriter::new(f);
    serde_json::to_writer(&mut w, cache).context("failed serializing diagnostics cache")?;
    w.write_all(b"\n")
        .with_context(|| format!("failed writing diagnostics cache: {}", path.display()))?;
    w.flush()
        .with_context(|| format!("failed writing diagnostics cache: {}", path.display()))?;
    Ok(())
}

pub fn load_diagnostics_cache(
    root: &Path,
) -> Result<HashMap<String, Vec<DiagnosticEntry>>> {
    let path = diagnostics_cache_path(root);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed reading diagnostics cache: {}", path.display()))?;
    let parsed: HashMap<String, Vec<DiagnosticEntry>> = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing diagnostics cache: {}", path.display()))?;
    Ok(parsed)
}
