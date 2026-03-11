use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = ".lime/lime.json";

/// Runtime configuration for Lime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimeConfig {
    /// Default max traversal depth for dependency lookups.
    pub default_dependency_depth: usize,
    /// Ignore patterns applied in addition to `.gitignore`.
    pub ignore_patterns: Vec<String>,
    /// Relative or absolute path to the index JSON file.
    pub index_storage: String,
}

impl Default for LimeConfig {
    fn default() -> Self {
        Self {
            default_dependency_depth: 2,
            ignore_patterns: vec![
                ".git/".to_string(),
                "node_modules/".to_string(),
                "target/".to_string(),
                ".lime/".to_string(),
                ".lemon/".to_string(),
            ],
            index_storage: ".lime/index.json".to_string(),
        }
    }
}

impl LimeConfig {
    /// Loads config from `.lime/lime.json`, creating it with defaults if absent.
    pub fn load_or_create(root: &Path) -> Result<Self> {
        let path = Self::config_path(root);

        if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed reading config: {}", path.display()))?;
            let mut parsed: Self = serde_json::from_str(&content)
                .with_context(|| format!("failed parsing config JSON: {}", path.display()))?;
            parsed.ensure_default_ignores();
            return Ok(parsed);
        }

        let mut config = Self::default();
        config.ensure_default_ignores();
        config.save(root)?;
        Ok(config)
    }

    /// Persists current configuration to `.lime/lime.json`.
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::config_path(root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed creating config directory: {}", parent.display())
            })?;
        }
        let serialized =
            serde_json::to_string_pretty(self).context("failed serializing default lime config")?;
        fs::write(&path, serialized)
            .with_context(|| format!("failed writing config: {}", path.display()))?;
        Ok(())
    }

    /// Returns path to the config file.
    pub fn config_path(root: &Path) -> PathBuf {
        root.join(CONFIG_FILE)
    }

    /// Resolves the index file location from config.
    pub fn index_path(&self, root: &Path) -> PathBuf {
        let configured = PathBuf::from(&self.index_storage);
        if configured.is_absolute() {
            configured
        } else {
            root.join(configured)
        }
    }

    fn ensure_default_ignores(&mut self) {
        for required in [".git/", "node_modules/", "target/", ".lime/", ".lemon/"] {
            if !self.ignore_patterns.iter().any(|value| value == required) {
                self.ignore_patterns.push(required.to_string());
            }
        }
    }
}
