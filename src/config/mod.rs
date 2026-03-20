use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = ".lime/lime.json";
const GLOBAL_CONFIG_SUBPATH: &str = "lime/lime.json";

fn default_index_pretty() -> bool {
    true
}

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
    /// When true (default), `index.json` is written pretty-printed. Set false for smaller, faster writes.
    #[serde(default = "default_index_pretty")]
    pub index_pretty: bool,
    /// Death detection seed overrides.
    #[serde(default)]
    pub death_seeds: DeathSeedConfig,
    /// Diagnostics / static analysis configuration.
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
    /// Empty `lime sync` (no file args): use git dirty paths vs full rebuild.
    #[serde(default)]
    pub git_partial_sync: GitPartialSyncConfig,
}

/// When `empty_sync_uses_git` is true, `lime sync` with no file arguments runs partial sync
/// on paths reported by `git status --porcelain` (unless overridden by `--no-git` / `--git`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitPartialSyncConfig {
    /// If true, `lime sync` with no paths uses git working-tree changes for partial indexing.
    pub empty_sync_uses_git: bool,
}

impl Default for GitPartialSyncConfig {
    fn default() -> Self {
        Self {
            empty_sync_uses_git: false,
        }
    }
}

/// Controls which components are treated as alive seeds in death detection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeathSeedConfig {
    /// File path patterns whose components are always alive seeds.
    pub seed_files: Vec<String>,
    /// Component name patterns that are always alive seeds (exact match).
    pub seed_names: Vec<String>,
    /// Component types that are always alive seeds.
    pub seed_types: Vec<String>,
}

/// Diagnostics / static analysis configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticsConfig {
    /// Whether diagnostics integration is enabled.
    pub enabled: bool,
    /// Timeout in seconds per analyzer invocation.
    pub timeout_secs: u64,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: 120,
        }
    }
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
            death_seeds: DeathSeedConfig::default(),
            diagnostics: DiagnosticsConfig::default(),
            git_partial_sync: GitPartialSyncConfig::default(),
            index_storage: ".lime/index.json".to_string(),
            index_pretty: true,
        }
    }
}

impl LimeConfig {
    /// Loads config from `.lime/lime.json`.
    ///
    /// When no project config exists yet (first `lime init` / `lime sync`), the
    /// **global config** (`~/.config/lime/lime.json`) is used as the starting
    /// template so user preferences carry over automatically.  If no global
    /// config exists either, hard-coded defaults are used.
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

        // New project — seed from global config, fall back to compiled defaults.
        let mut config = Self::load_global().unwrap_or_default();
        // Always reset the storage path to the project-relative default so a
        // global `index_storage` override from one repo doesn't pollute another.
        config.index_storage = ".lime/index.json".to_string();
        config.ensure_default_ignores();
        config.save(root)?;
        Ok(config)
    }

    /// Returns the path to the global config file (`~/.config/lime/lime.json`).
    ///
    /// Returns `None` when the platform config directory cannot be determined.
    pub fn global_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join(GLOBAL_CONFIG_SUBPATH))
    }

    /// Loads the global config.
    ///
    /// Returns `LimeConfig::default()` (silently) when the file does not exist
    /// or the platform config dir is unavailable.
    pub fn load_global() -> Result<Self> {
        let Some(path) = Self::global_config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed reading global config: {}", path.display()))?;
        let mut parsed: Self = serde_json::from_str(&content)
            .with_context(|| format!("failed parsing global config JSON: {}", path.display()))?;
        parsed.ensure_default_ignores();
        Ok(parsed)
    }

    /// Persists `self` as the global config (`~/.config/lime/lime.json`).
    pub fn save_global(&self) -> Result<()> {
        let path = Self::global_config_path()
            .context("cannot determine platform config directory for global config")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed creating global config directory: {}",
                    parent.display()
                )
            })?;
        }
        let serialized =
            serde_json::to_string_pretty(self).context("failed serializing global lime config")?;
        fs::write(&path, serialized)
            .with_context(|| format!("failed writing global config: {}", path.display()))?;
        Ok(())
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

    /// Returns path to the project config file.
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
