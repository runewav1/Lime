#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::index::{ComponentRecord, IndexData};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub hash_id: String,
    pub component_type: String,
    pub component_name: String,
    /// Repo-relative path of the defining file (forward slashes). Persisted for stable matching when component IDs change.
    #[serde(default)]
    pub file: Option<String>,
    /// Language key from the index (e.g. `rust`). Optional for legacy annotation files.
    #[serde(default)]
    pub language: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Link paths (`/` segments); merged with `.lime/component_links.json` for `lime link` / search.
    #[serde(default)]
    pub links: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Annotation {
    pub fn has_keep_tag(&self) -> bool {
        self.tags
            .iter()
            .any(|t| matches!(t.as_str(), "keep" | "public_api" | "entrypoint"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnnotationFrontmatter {
    hash_id: String,
    component_type: String,
    component_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    links: Vec<String>,
    created_at: String,
    updated_at: String,
}

pub fn annotations_dir(root: &Path) -> PathBuf {
    root.join(".lime").join("annotations")
}

pub fn save_annotation(root: &Path, annotation: &Annotation) -> Result<()> {
    let (component_type, hash_id) = parse_component_prefix(&annotation.hash_id);
    let path = annotation_path(root, &component_type, &hash_id);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed creating annotation directory: {}", parent.display())
        })?;
    }

    let now = Utc::now().to_rfc3339();
    let created_at = if annotation.created_at.trim().is_empty() {
        now.clone()
    } else {
        annotation.created_at.clone()
    };
    let updated_at = if annotation.updated_at.trim().is_empty() {
        now
    } else {
        annotation.updated_at.clone()
    };

    let frontmatter = AnnotationFrontmatter {
        hash_id,
        component_type,
        component_name: annotation.component_name.clone(),
        file: annotation.file.clone(),
        language: annotation.language.clone(),
        tags: annotation.tags.clone(),
        links: annotation.links.clone(),
        created_at,
        updated_at,
    };

    let frontmatter_toml =
        toml::to_string(&frontmatter).context("failed serializing annotation frontmatter")?;
    let body = annotation.content.trim_end_matches('\n');
    let payload = format!("---\n{}---\n{}\n", frontmatter_toml, body);

    fs::write(&path, payload)
        .with_context(|| format!("failed writing annotation file: {}", path.display()))?;

    Ok(())
}

pub fn load_annotation(root: &Path, component_id: &str) -> Result<Option<Annotation>> {
    let (component_type, hash_id) = parse_component_prefix(component_id);
    let path = annotation_path(root, &component_type, &hash_id);

    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed reading annotation file: {}", path.display()))?;
    parse_annotation_file(&raw).map(Some)
}

pub fn list_annotations(root: &Path) -> Result<Vec<Annotation>> {
    let base = annotations_dir(root);
    if !base.exists() {
        return Ok(Vec::new());
    }

    let mut annotations = Vec::new();
    for entry in WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        if entry.path().extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }

        let raw = fs::read_to_string(entry.path()).with_context(|| {
            format!("failed reading annotation file: {}", entry.path().display())
        })?;
        let parsed = parse_annotation_file(&raw).with_context(|| {
            format!("failed parsing annotation file: {}", entry.path().display())
        })?;
        annotations.push(parsed);
    }

    annotations.sort_by(|left, right| {
        (
            left.component_type.as_str(),
            left.component_name.as_str(),
            left.hash_id.as_str(),
        )
            .cmp(&(
                right.component_type.as_str(),
                right.component_name.as_str(),
                right.hash_id.as_str(),
            ))
    });

    Ok(annotations)
}

pub fn remove_annotation(root: &Path, component_id: &str) -> Result<bool> {
    let (component_type, hash_id) = parse_component_prefix(component_id);
    let path = annotation_path(root, &component_type, &hash_id);

    if !path.exists() {
        return Ok(false);
    }

    fs::remove_file(&path)
        .with_context(|| format!("failed removing annotation file: {}", path.display()))?;

    Ok(true)
}

pub fn parse_component_prefix(component_id: &str) -> (String, String) {
    if component_id.trim().is_empty() {
        return ("component".to_string(), component_id.to_string());
    }

    if let Some((prefix, _)) = component_id.split_once('-') {
        if !prefix.is_empty() {
            return (prefix.to_string(), component_id.to_string());
        }
    }

    ("component".to_string(), component_id.to_string())
}

fn annotation_path(root: &Path, component_type: &str, hash_id: &str) -> PathBuf {
    annotations_dir(root)
        .join(component_type)
        .join(format!("{hash_id}.md"))
}

fn parse_annotation_file(raw: &str) -> Result<Annotation> {
    let mut lines = raw.lines();

    let Some(first_line) = lines.next() else {
        bail!("annotation file is empty");
    };

    if first_line.trim() != "---" {
        bail!("annotation file missing opening frontmatter delimiter");
    }

    let mut frontmatter_lines = Vec::new();
    let mut found_closing = false;

    for line in &mut lines {
        if line.trim() == "---" {
            found_closing = true;
            break;
        }
        frontmatter_lines.push(line);
    }

    if !found_closing {
        bail!("annotation file missing closing frontmatter delimiter");
    }

    let frontmatter_raw = frontmatter_lines.join("\n");
    let frontmatter: AnnotationFrontmatter =
        toml::from_str(&frontmatter_raw).context("failed parsing annotation frontmatter")?;

    let content = lines.collect::<Vec<_>>().join("\n");

    Ok(Annotation {
        hash_id: frontmatter.hash_id,
        component_type: frontmatter.component_type,
        component_name: frontmatter.component_name,
        file: frontmatter.file,
        language: frontmatter.language,
        content,
        tags: frontmatter.tags,
        links: frontmatter.links,
        created_at: frontmatter.created_at,
        updated_at: frontmatter.updated_at,
    })
}

fn normalize_rel_path(s: &str) -> String {
    s.replace('\\', "/")
}

/// True if this annotation belongs to the given component (by ID or by stable file/name/type).
pub fn annotation_applies_to_component(annotation: &Annotation, component: &ComponentRecord) -> bool {
    if annotation.hash_id == component.id {
        return true;
    }
    if let Some(ref f) = annotation.file {
        if normalize_rel_path(f) != normalize_rel_path(&component.file) {
            return false;
        }
        let lang_ok = match &annotation.language {
            Some(l) => l == &component.language,
            None => true,
        };
        return lang_ok
            && annotation.component_name == component.name
            && annotation.component_type == component.component_type;
    }
    false
}

/// Maps an on-disk annotation to the current index component, if any.
pub fn resolve_component_for_annotation<'a>(
    index: &'a IndexData,
    annotation: &Annotation,
) -> Option<&'a ComponentRecord> {
    if let Some(c) = index
        .components
        .iter()
        .find(|c| c.id == annotation.hash_id)
    {
        return Some(c);
    }

    if let Some(ref f) = annotation.file {
        let nf = normalize_rel_path(f);
        let matches: Vec<_> = index
            .components
            .iter()
            .filter(|c| {
                normalize_rel_path(&c.file) == nf
                    && c.name == annotation.component_name
                    && c.component_type == annotation.component_type
            })
            .filter(|c| {
                annotation
                    .language
                    .as_ref()
                    .map(|l| l == &c.language)
                    .unwrap_or(true)
            })
            .collect();
        if matches.len() == 1 {
            return Some(matches[0]);
        }
        return None;
    }

    // Legacy: no file in frontmatter — only match when (type, name) is unique in the index.
    let matches: Vec<_> = index
        .components
        .iter()
        .filter(|c| {
            c.name == annotation.component_name && c.component_type == annotation.component_type
        })
        .collect();
    if matches.len() == 1 {
        return Some(matches[0]);
    }
    None
}

/// Loads an annotation for a component, including legacy files keyed by an outdated `hash_id`.
pub fn find_annotation_for_component(
    root: &Path,
    index: &IndexData,
    component: &ComponentRecord,
) -> Result<Option<Annotation>> {
    if let Some(a) = load_annotation(root, &component.id)? {
        return Ok(Some(a));
    }

    for ann in list_annotations(root)? {
        if let Some(c) = resolve_component_for_annotation(index, &ann) {
            if c.id == component.id {
                return Ok(Some(ann));
            }
        }
    }
    Ok(None)
}

/// After indexing, rewrites annotation files so `hash_id` matches the current component ID and
/// [`Annotation::file`] / [`Annotation::language`] are populated.
///
/// Returns the up-to-date annotation list (single disk walk) for callers that build derived indexes.
pub fn reconcile_annotations_with_index(root: &Path, index: &IndexData) -> Result<Vec<Annotation>> {
    let mut batch = list_annotations(root)?;
    for ann in batch.iter_mut() {
        let old_path = {
            let (t, h) = parse_component_prefix(&ann.hash_id);
            annotation_path(root, &t, &h)
        };

        let Some(component) = resolve_component_for_annotation(index, ann) else {
            continue;
        };

        if ann.hash_id == component.id
            && ann.file.as_deref() == Some(component.file.as_str())
            && ann.language.as_deref() == Some(component.language.as_str())
        {
            continue;
        }

        let mut updated = ann.clone();
        updated.hash_id = component.id.clone();
        updated.file = Some(component.file.clone());
        updated.language = Some(component.language.clone());

        save_annotation(root, &updated)?;

        let new_path = {
            let (t, h) = parse_component_prefix(&updated.hash_id);
            annotation_path(root, &t, &h)
        };

        if old_path != new_path && old_path.exists() {
            let _ = fs::remove_file(&old_path);
        }

        *ann = updated;
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::diagnostics::ComponentFaults;
    use crate::index::{DeathStatus, IndexData};

    fn sample_component(id: &str, file: &str, name: &str) -> ComponentRecord {
        ComponentRecord {
            id: id.to_string(),
            language: "rust".to_string(),
            component_type: "fn".to_string(),
            name: name.to_string(),
            file: file.to_string(),
            start_line: 1,
            end_line: 5,
            uses_before: vec![],
            used_by_after: vec![],
            batman: false,
            death_status: DeathStatus::Alive,
            death_evidence: Default::default(),
            faults: ComponentFaults::default(),
            display_path: String::new(),
        }
    }

    #[test]
    fn resolve_by_file_name_type_when_hash_id_differs() {
        let mut idx = IndexData::empty(Path::new("."));
        idx.components.push(sample_component("fn-newid", "src/a.rs", "foo"));

        let ann = Annotation {
            hash_id: "fn-oldid".to_string(),
            component_type: "fn".to_string(),
            component_name: "foo".to_string(),
            file: Some("src/a.rs".to_string()),
            language: Some("rust".to_string()),
            content: "x".to_string(),
            tags: vec![],
            links: vec![],
            created_at: String::new(),
            updated_at: String::new(),
        };

        let c = resolve_component_for_annotation(&idx, &ann).expect("resolves");
        assert_eq!(c.id, "fn-newid");
    }

    #[test]
    fn legacy_unique_name_resolves_without_file() {
        let mut idx = IndexData::empty(Path::new("."));
        idx.components.push(sample_component("fn-only", "src/b.rs", "bar"));

        let ann = Annotation {
            hash_id: "fn-stale".to_string(),
            component_type: "fn".to_string(),
            component_name: "bar".to_string(),
            file: None,
            language: None,
            content: "y".to_string(),
            tags: vec![],
            links: vec![],
            created_at: String::new(),
            updated_at: String::new(),
        };

        let c = resolve_component_for_annotation(&idx, &ann).expect("resolves");
        assert_eq!(c.id, "fn-only");
    }

    #[test]
    fn parse_annotation_with_links_in_frontmatter() {
        let raw = r#"---
hash_id = "fn-a"
component_type = "fn"
component_name = "foo"
tags = []
links = ["auth", "billing"]
created_at = "2026-01-01T00:00:00Z"
updated_at = "2026-01-01T00:00:00Z"
---

Note body
"#;
        let a = parse_annotation_file(raw).expect("parse");
        assert_eq!(a.links, vec!["auth".to_string(), "billing".to_string()]);
        assert_eq!(a.content.trim(), "Note body");
    }
}
