use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

const REGISTRY_SUBPATH: &str = ".lime/projects.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectsRegistry {
    pub version: u32,
    pub projects: BTreeMap<String, ProjectRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRegistration {
    pub root: String,
    pub updated_at: String,
}

impl Default for ProjectsRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            projects: BTreeMap::new(),
        }
    }
}

pub fn registry_path() -> Result<PathBuf> {
    let home =
        dirs::home_dir().context("cannot determine home directory for projects registry path")?;
    Ok(home.join(REGISTRY_SUBPATH))
}

pub fn load_registry() -> Result<ProjectsRegistry> {
    let path = registry_path()?;
    if !path.exists() {
        return Ok(ProjectsRegistry::default());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed reading projects registry: {}", path.display()))?;
    let parsed: ProjectsRegistry = serde_json::from_str(&content)
        .with_context(|| format!("failed parsing projects registry JSON: {}", path.display()))?;
    Ok(parsed)
}

pub fn save_registry(registry: &ProjectsRegistry) -> Result<()> {
    let path = registry_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed creating projects registry directory: {}",
                parent.display()
            )
        })?;
    }
    let body =
        serde_json::to_string_pretty(registry).context("failed serializing projects registry")?;
    fs::write(&path, body)
        .with_context(|| format!("failed writing projects registry: {}", path.display()))?;
    Ok(())
}

pub fn default_project_id_for_root(root: &Path) -> Result<String> {
    let canonical = fs::canonicalize(root)
        .with_context(|| format!("failed resolving repository path: {}", root.display()))?;
    let Some(name) = canonical.file_name() else {
        return Err(anyhow!(
            "cannot derive project id from repository path: {}",
            canonical.display()
        ));
    };
    Ok(name.to_string_lossy().to_string())
}

pub fn register_project(project_id: Option<&str>, root: &Path) -> Result<(String, PathBuf)> {
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed resolving repository path: {}", root.display()))?;
    if !canonical_root.exists() {
        return Err(anyhow!(
            "repository path does not exist: {}",
            canonical_root.display()
        ));
    }

    let mut registry = load_registry()?;
    let id = match project_id {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => default_project_id_for_root(&canonical_root)?,
    };
    let canonical_root_str = canonical_root.display().to_string();

    if let Some(existing) = registry.projects.get(&id) {
        ensure_id_not_conflicting(&id, &existing.root, &canonical_root_str)?;
        return Ok((id, canonical_root));
    }

    registry.projects.insert(
        id.clone(),
        ProjectRegistration {
            root: canonical_root_str,
            updated_at: Utc::now().to_rfc3339(),
        },
    );
    save_registry(&registry)?;
    Ok((id, canonical_root))
}

fn ensure_id_not_conflicting(project_id: &str, existing_root: &str, new_root: &str) -> Result<()> {
    if existing_root == new_root {
        return Ok(());
    }
    Err(anyhow!(
        "project id '{}' is already registered to '{}'; use --id to choose a unique id",
        project_id,
        existing_root
    ))
}

pub fn unregister_project(project_id: &str) -> Result<bool> {
    let mut registry = load_registry()?;
    let removed = registry.projects.remove(project_id).is_some();
    if removed {
        save_registry(&registry)?;
    }
    Ok(removed)
}

pub fn resolve_project_root(project_id: &str) -> Result<PathBuf> {
    let registry = load_registry()?;
    let Some(reg) = registry.projects.get(project_id) else {
        return Err(anyhow!(
            "unknown project id: {} (use `lime registry list` to inspect registered roots)",
            project_id
        ));
    };
    let root = PathBuf::from(&reg.root);
    if !root.exists() {
        return Err(anyhow!(
            "registered project root no longer exists for '{}': {}",
            project_id,
            root.display()
        ));
    }
    fs::canonicalize(&root)
        .with_context(|| format!("failed resolving registered root path: {}", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roundtrip_json() {
        let mut registry = ProjectsRegistry::default();
        registry.projects.insert(
            "demo".to_string(),
            ProjectRegistration {
                root: "C:/tmp/demo".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        );
        let json = serde_json::to_string(&registry).unwrap();
        let parsed: ProjectsRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.projects.get("demo").unwrap().root, "C:/tmp/demo");
        assert_eq!(parsed.version, 1);
    }

    #[test]
    fn rejects_duplicate_id_for_different_root() {
        let result = ensure_id_not_conflicting("demo", "C:/a/demo", "C:/b/demo");
        assert!(result.is_err());
        let message = format!("{:#}", result.err().unwrap());
        assert!(message.contains("already registered"));
    }

    #[test]
    fn allows_duplicate_id_for_same_root() {
        let result = ensure_id_not_conflicting("demo", "C:/a/demo", "C:/a/demo");
        assert!(result.is_ok());
    }
}
