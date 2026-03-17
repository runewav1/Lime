#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub hash_id: String,
    pub component_type: String,
    pub component_name: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnnotationFrontmatter {
    hash_id: String,
    component_type: String,
    component_name: String,
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

pub fn list_annotations_for_type(root: &Path, component_type: &str) -> Result<Vec<Annotation>> {
    let type_dir = annotations_dir(root).join(component_type);
    if !type_dir.exists() {
        return Ok(Vec::new());
    }

    let mut annotations = Vec::new();
    for entry in WalkDir::new(&type_dir)
        .max_depth(1)
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
            left.component_name.as_str(),
            left.hash_id.as_str(),
            left.updated_at.as_str(),
        )
            .cmp(&(
                right.component_name.as_str(),
                right.hash_id.as_str(),
                right.updated_at.as_str(),
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

pub fn annotation_exists(root: &Path, component_id: &str) -> bool {
    let (component_type, hash_id) = parse_component_prefix(component_id);
    annotation_path(root, &component_type, &hash_id).exists()
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
        content,
        created_at: frontmatter.created_at,
        updated_at: frontmatter.updated_at,
    })
}
