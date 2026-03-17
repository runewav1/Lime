use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::{
    config::LimeConfig,
    index::IndexData,
    search::{PersistedTokenIndex, SearchTokenIndex},
};

/// Loads index data if present; otherwise returns an empty index.
pub fn load_index_or_empty(root: &Path, config: &LimeConfig) -> Result<IndexData> {
    let index_path = config.index_path(root);
    if !index_path.exists() {
        return Ok(IndexData::empty(root));
    }

    let content = fs::read_to_string(&index_path)
        .with_context(|| format!("failed reading index file: {}", index_path.display()))?;
    let parsed: IndexData = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing index json: {}", index_path.display()))?;
    Ok(parsed)
}

/// Persists index data to configured storage location.
pub fn save_index(root: &Path, config: &LimeConfig, index: &IndexData) -> Result<()> {
    let index_path = config.index_path(root);
    if let Some(parent) = index_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating index directory: {}", parent.display()))?;
    }

    let payload = serde_json::to_string_pretty(index).context("failed serializing index json")?;
    fs::write(&index_path, payload)
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
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating token index directory: {}", parent.display()))?;
    }

    let persisted = PersistedTokenIndex::from(token_index);
    let payload = serde_json::to_string(&persisted).context("failed serializing token index")?;
    fs::write(&path, payload)
        .with_context(|| format!("failed writing token index: {}", path.display()))?;
    Ok(())
}

pub fn load_token_index(root: &Path) -> Result<Option<SearchTokenIndex>> {
    let path = token_index_path(root);
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed reading token index: {}", path.display()))?;
    let persisted: PersistedTokenIndex = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing token index: {}", path.display()))?;
    Ok(Some(SearchTokenIndex::from(persisted)))
}
