//! Component link paths stored in `.lime/component_links.json`, independent of annotations.
//! Optional `.lime/link_catalog.json` for titles and sort_key overrides.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    annotations::{self, Annotation},
    index::IndexData,
};

/// Maximum length of a single link path string (chars).
pub const MAX_LINK_PATH_LEN: usize = 256;
/// Maximum number of distinct link paths per component.
pub const MAX_PATHS_PER_COMPONENT: usize = 128;

pub fn component_links_path(root: &Path) -> PathBuf {
    root.join(".lime").join("component_links.json")
}

pub fn link_catalog_path(root: &Path) -> PathBuf {
    root.join(".lime").join("link_catalog.json")
}

/// Persisted membership: component ID -> link paths (`auth`, `auth/login`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentLinksFile {
    pub version: u32,
    pub updated_at: String,
    /// Component ID -> link paths (validated on write).
    #[serde(default)]
    pub memberships: HashMap<String, Vec<String>>,
}

impl Default for ComponentLinksFile {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            memberships: HashMap::new(),
        }
    }
}

/// Optional metadata per canonical link path (path string is the map key).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkCatalogEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_path: Option<String>,
    /// When set, used for ordering before falling back to lexicographic path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkCatalogFile {
    pub version: u32,
    #[serde(default)]
    pub entries: HashMap<String, LinkCatalogEntry>,
}

/// Validate and normalize a link path: non-empty segments, `/` delimiter, no `//`, trimmed.
pub fn validate_link_path(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() {
        bail!("link path must not be empty");
    }
    if s.len() > MAX_LINK_PATH_LEN {
        bail!("link path exceeds max length ({MAX_LINK_PATH_LEN})");
    }
    if s.starts_with('/') || s.ends_with('/') {
        bail!("link path must not start or end with '/'");
    }
    if s.contains("//") {
        bail!("link path must not contain empty segments ('//')");
    }
    for seg in s.split('/') {
        if seg.trim().is_empty() {
            bail!("link path segments must be non-empty");
        }
    }
    Ok(s.to_string())
}

pub fn load_component_links(root: &Path) -> Result<ComponentLinksFile> {
    let path = component_links_path(root);
    if !path.exists() {
        return Ok(ComponentLinksFile::default());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed reading {}", path.display()))?;
    let parsed: ComponentLinksFile = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing {}", path.display()))?;
    Ok(parsed)
}

pub fn save_component_links(root: &Path, file: &ComponentLinksFile) -> Result<()> {
    let path = component_links_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let mut out = file.clone();
    out.updated_at = Utc::now().to_rfc3339();
    let json = serde_json::to_string_pretty(&out).context("failed serializing component_links")?;
    fs::write(&path, format!("{json}\n")).with_context(|| format!("failed writing {}", path.display()))?;
    Ok(())
}

pub fn load_link_catalog(root: &Path) -> Result<LinkCatalogFile> {
    let path = link_catalog_path(root);
    if !path.exists() {
        return Ok(LinkCatalogFile {
            version: 1,
            entries: HashMap::new(),
        });
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed reading {}", path.display()))?;
    let parsed: LinkCatalogFile = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing {}", path.display()))?;
    Ok(parsed)
}

/// Writes `.lime/link_catalog.json` (optional metadata; agents may edit by hand).
#[allow(dead_code)]
pub fn save_link_catalog(root: &Path, catalog: &LinkCatalogFile) -> Result<()> {
    let path = link_catalog_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(catalog).context("failed serializing link_catalog")?;
    fs::write(&path, format!("{json}\n")).with_context(|| format!("failed writing {}", path.display()))?;
    Ok(())
}

/// Returns true if `path` matches query: exact (case-insensitive) or child path under `query/`.
pub fn path_matches_link_query(path: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    let p = path.trim();
    if p.eq_ignore_ascii_case(q) {
        return true;
    }
    let prefix = format!("{q}/");
    p.len() > prefix.len() && p[..prefix.len()].eq_ignore_ascii_case(&prefix)
}

fn dedupe_paths(paths: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    for p in paths {
        let key = p.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(p);
        }
    }
    if out.len() > MAX_PATHS_PER_COMPONENT {
        out.truncate(MAX_PATHS_PER_COMPONENT);
    }
    out
}

/// Merge paths from `component_links.json` and from annotations resolved to each component.
pub fn merged_link_paths_by_component(
    root: &Path,
    index: &IndexData,
    annotations: &[Annotation],
) -> HashMap<String, Vec<String>> {
    let mut acc: HashMap<String, Vec<String>> = HashMap::new();

    let store = load_component_links(root).unwrap_or_default();
    for (cid, paths) in store.memberships {
        let mut v = Vec::new();
        for p in paths {
            if let Ok(n) = validate_link_path(&p) {
                v.push(n);
            }
        }
        v = dedupe_paths(v.into_iter());
        if !v.is_empty() {
            acc.insert(cid, v);
        }
    }

    for ann in annotations {
        let Some(comp) = annotations::resolve_component_for_annotation(index, ann) else {
            continue;
        };
        let entry = acc.entry(comp.id.clone()).or_default();
        let mut combined: Vec<String> = entry.clone();
        for l in &ann.links {
            if let Ok(n) = validate_link_path(l) {
                combined.push(n);
            }
        }
        *entry = dedupe_paths(combined.into_iter());
    }

    acc
}

/// Paths for one component (store + annotation), deduped.
#[allow(dead_code)]
pub fn merged_paths_for_component(
    root: &Path,
    index: &IndexData,
    annotations: &[Annotation],
    component_id: &str,
) -> Vec<String> {
    merged_link_paths_by_component(root, index, annotations)
        .remove(component_id)
        .unwrap_or_default()
}

pub fn add_membership(root: &Path, component_id: &str, path: &str) -> Result<()> {
    let normalized = validate_link_path(path)?;
    let mut store = load_component_links(root)?;
    let list = store
        .memberships
        .entry(component_id.to_string())
        .or_default();
    if !list.iter().any(|p| p.eq_ignore_ascii_case(&normalized)) {
        if list.len() >= MAX_PATHS_PER_COMPONENT {
            bail!("component already has max number of link paths ({MAX_PATHS_PER_COMPONENT})");
        }
        list.push(normalized);
    }
    list.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    save_component_links(root, &store)
}

pub fn remove_membership(root: &Path, component_id: &str, path: &str) -> Result<bool> {
    let q = path.trim();
    if q.is_empty() {
        bail!("path must not be empty");
    }
    let mut store = load_component_links(root)?;
    let Some(list) = store.memberships.get_mut(component_id) else {
        return Ok(false);
    };
    let before = list.len();
    list.retain(|p| !p.eq_ignore_ascii_case(q));
    let removed = list.len() < before;
    if list.is_empty() {
        store.memberships.remove(component_id);
    }
    save_component_links(root, &store)?;
    Ok(removed)
}

/// Sort link paths for display: by catalog `sort_key` then lexicographic path (case-insensitive).
pub fn sort_paths_for_display(paths: &[String], catalog: &LinkCatalogFile) -> Vec<String> {
    let mut v: Vec<String> = paths.to_vec();
    v.sort_by(|a, b| {
        let sa = catalog
            .entries
            .get(a)
            .and_then(|e| e.sort_key.as_deref())
            .unwrap_or("");
        let sb = catalog
            .entries
            .get(b)
            .and_then(|e| e.sort_key.as_deref())
            .unwrap_or("");
        sa.cmp(sb)
            .then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
    });
    v
}

/// Distinct paths across all memberships and annotations, optional prefix filter (case-insensitive).
pub fn distinct_merged_paths(
    root: &Path,
    index: &IndexData,
    annotations: &[Annotation],
    prefix: Option<&str>,
) -> Vec<String> {
    let merged = merged_link_paths_by_component(root, index, annotations);
    let mut set: HashSet<String> = HashSet::new();
    for paths in merged.values() {
        for p in paths {
            set.insert(p.clone());
        }
    }
    for ann in annotations {
        for l in &ann.links {
            if let Ok(n) = validate_link_path(l) {
                set.insert(n);
            }
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    if let Some(pre) = prefix {
        let pre = pre.trim();
        if !pre.is_empty() {
            out.retain(|p| {
                p.eq_ignore_ascii_case(pre)
                    || p
                        .to_ascii_lowercase()
                        .starts_with(&format!("{}/", pre.to_ascii_lowercase()))
            });
        }
    }
    let catalog = load_link_catalog(root).unwrap_or_default();
    sort_paths_for_display(&out, &catalog)
}

/// Remove link lines from annotation frontmatter when the same path exists in the store for that component.
pub fn compact_annotation_links(root: &Path, index: &IndexData) -> Result<usize> {
    let store = load_component_links(root)?;
    let mut updated = 0usize;
    for mut ann in annotations::list_annotations(root)? {
        let Some(comp) = annotations::resolve_component_for_annotation(index, &ann) else {
            continue;
        };
        let store_paths: HashSet<String> = store
            .memberships
            .get(&comp.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_ascii_lowercase())
            .collect();
        if store_paths.is_empty() || ann.links.is_empty() {
            continue;
        }
        let before = ann.links.len();
        ann.links.retain(|l| {
            !store_paths.contains(&l.trim().to_ascii_lowercase())
        });
        if ann.links.len() != before {
            annotations::save_annotation(root, &ann)?;
            updated += 1;
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_link_path_accepts_hierarchical() {
        assert_eq!(validate_link_path("auth/login").unwrap(), "auth/login");
    }

    #[test]
    fn validate_link_path_rejects_empty_segment() {
        assert!(validate_link_path("auth//x").is_err());
    }

    #[test]
    fn path_matches_query_prefix() {
        assert!(path_matches_link_query("auth/login", "auth"));
        assert!(path_matches_link_query("auth", "auth"));
        assert!(!path_matches_link_query("oauth", "auth"));
    }
}
