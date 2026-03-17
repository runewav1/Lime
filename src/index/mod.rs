use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use blake3::Hasher;
use chrono::Utc;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    batman,
    config::LimeConfig,
    deps,
    parse::{detect_language, parse_components},
};

/// Persistent codebase index written to storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexData {
    /// Schema version for compatibility.
    pub version: u32,
    /// Root directory where indexing was performed.
    pub root: String,
    /// ISO timestamp for latest index generation.
    pub generated_at: String,
    /// Detected languages across indexed files.
    pub languages: Vec<String>,
    /// Indexed file records.
    pub files: Vec<IndexedFile>,
    /// Indexed component records.
    pub components: Vec<ComponentRecord>,
    /// Optional search index: lowercase component name -> component indices.
    #[serde(skip, default)]
    pub search_index: Option<HashMap<String, Vec<usize>>>,
}

/// File-level index record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedFile {
    /// Repository-relative file path.
    pub path: String,
    /// File language key.
    pub language: String,
    /// Blake3 hash of file content.
    pub file_hash: String,
    /// Component IDs discovered in the file.
    pub component_ids: Vec<String>,
}

/// Component-level index record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentRecord {
    /// Unique prefixed component ID (for example `struct-...`, `fn-...`).
    pub id: String,
    /// Component language key.
    pub language: String,
    /// Component type.
    #[serde(rename = "type")]
    pub component_type: String,
    /// Component name.
    pub name: String,
    /// Repository-relative file path.
    pub file: String,
    /// 1-indexed start line.
    pub start_line: usize,
    /// 1-indexed end line.
    pub end_line: usize,
    /// IDs of components referenced by this component.
    pub uses_before: Vec<String>,
    /// IDs of components that reference this component.
    pub used_by_after: Vec<String>,
    /// Whether this component is flagged as dead code (disconnected from the main graph).
    #[serde(default)]
    pub batman: bool,
}

/// Result metadata for partial file updates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileUpdateResult {
    /// Files successfully indexed or re-indexed.
    pub indexed: Vec<String>,
    /// Files removed from index.
    pub removed: Vec<String>,
    /// Files skipped with reasons.
    pub skipped: Vec<SkippedPath>,
}

/// File skip details for partial update operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedPath {
    /// Input path value.
    pub path: String,
    /// Reason the file was skipped.
    pub reason: String,
}

impl IndexData {
    /// Creates a new empty index data record.
    pub fn empty(root: &Path) -> Self {
        Self {
            version: 1,
            root: normalize_path_string(root),
            generated_at: Utc::now().to_rfc3339(),
            languages: Vec::new(),
            files: Vec::new(),
            components: Vec::new(),
            search_index: None,
        }
    }

    /// Updates metadata fields after structural changes.
    pub fn refresh_metadata(&mut self) {
        let mut languages = BTreeSet::new();
        for file in &self.files {
            languages.insert(file.language.clone());
        }
        self.languages = languages.into_iter().collect();
        self.generated_at = Utc::now().to_rfc3339();

        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        self.components.sort_by(|left, right| {
            (
                left.language.as_str(),
                left.file.as_str(),
                left.start_line,
                left.component_type.as_str(),
                left.name.as_str(),
            )
                .cmp(&(
                    right.language.as_str(),
                    right.file.as_str(),
                    right.start_line,
                    right.component_type.as_str(),
                    right.name.as_str(),
                ))
        });

        self.search_index = Some(self.build_search_index());
    }

    /// Builds a lowercase name -> component index lookup table.
    pub fn build_search_index(&self) -> HashMap<String, Vec<usize>> {
        let mut search_index = HashMap::new();
        for (component_index, component) in self.components.iter().enumerate() {
            search_index
                .entry(component.name.to_ascii_lowercase())
                .or_insert_with(Vec::new)
                .push(component_index);
        }
        search_index
    }
}

struct IndexedFileBuild {
    file: IndexedFile,
    components: Vec<ComponentRecord>,
    content: String,
}

/// Rebuilds the entire codebase index from scratch.
pub fn rebuild_index(root: &Path, config: &LimeConfig) -> Result<IndexData> {
    let files = discover_supported_files(root, config)?;
    let mut index = IndexData::empty(root);
    let mut file_contents = HashMap::new();

    let builds = files
        .par_iter()
        .map(|path| index_single_file(root, path))
        .collect::<Vec<_>>();

    for build in builds {
        if let Some(build) = build? {
            file_contents.insert(build.file.path.clone(), build.content);
            index.files.push(build.file);
            index.components.extend(build.components);
        }
    }

    deps::populate_dependencies(&mut index, &file_contents);
    batman::detect_batman(&mut index, &file_contents);
    index.refresh_metadata();
    Ok(index)
}

/// Adds a single file to the index and refreshes dependencies.
pub fn add_file(
    root: &Path,
    config: &LimeConfig,
    filename: &str,
    index: IndexData,
) -> Result<(IndexData, FileUpdateResult)> {
    let (next, report) = sync_files(root, config, &[filename.to_string()], index)?;

    if report.indexed.is_empty() {
        let reason = report
            .skipped
            .iter()
            .find(|entry| entry.path == filename)
            .map(|entry| entry.reason.clone())
            .unwrap_or_else(|| "file was not indexed".to_string());

        if reason == "file hash unchanged" {
            return Ok((next, report));
        }

        bail!("add failed for `{filename}`: {reason}");
    }

    Ok((next, report))
}

/// Removes a single file from the index and refreshes dependencies.
pub fn remove_file(root: &Path, filename: &str, mut index: IndexData) -> Result<(IndexData, bool)> {
    let relative = relative_from_input(root, filename)?;
    let removed = remove_path_from_index(&mut index, &relative);
    if removed {
        refresh_dependencies_from_disk(root, &mut index)?;
    }
    Ok((index, removed))
}

/// Re-indexes the provided set of files.
pub fn sync_files(
    root: &Path,
    config: &LimeConfig,
    files: &[String],
    mut index: IndexData,
) -> Result<(IndexData, FileUpdateResult)> {
    let mut result = FileUpdateResult::default();
    let matcher = build_ignore_matcher(root, config)?;

    for raw in files {
        let absolute = absolute_from_input(root, raw);
        let relative = relative_from_input(root, raw)?;

        if !absolute.exists() {
            if remove_path_from_index(&mut index, &relative) {
                result.removed.push(relative);
            } else {
                result.skipped.push(SkippedPath {
                    path: raw.clone(),
                    reason: "path does not exist".to_string(),
                });
            }
            continue;
        }

        let metadata = fs::metadata(&absolute)
            .with_context(|| format!("failed reading metadata: {}", absolute.display()))?;
        if metadata.is_dir() {
            result.skipped.push(SkippedPath {
                path: raw.clone(),
                reason: "path is a directory".to_string(),
            });
            continue;
        }

        if is_ignored(root, &absolute, false, &matcher) {
            result.skipped.push(SkippedPath {
                path: raw.clone(),
                reason: "path is ignored by .gitignore or lime config".to_string(),
            });
            continue;
        }

        let Some(language) = detect_path_language(&absolute) else {
            remove_path_from_index(&mut index, &relative);
            result.skipped.push(SkippedPath {
                path: raw.clone(),
                reason: "unsupported file extension".to_string(),
            });
            continue;
        };

        let bytes = fs::read(&absolute)
            .with_context(|| format!("failed reading file: {}", absolute.display()))?;
        let file_hash = hash_bytes(&bytes);

        let unchanged = index
            .files
            .iter()
            .find(|file| file.path == relative)
            .map(|file| file.file_hash == file_hash)
            .unwrap_or(false);

        if unchanged {
            result.skipped.push(SkippedPath {
                path: raw.clone(),
                reason: "file hash unchanged".to_string(),
            });
            continue;
        }

        let build = build_indexed_file(root, &absolute, language, &bytes)?;
        remove_path_from_index(&mut index, &build.file.path);
        result.indexed.push(build.file.path.clone());
        index.files.push(build.file);
        index.components.extend(build.components);
    }

    refresh_dependencies_from_disk(root, &mut index)?;
    Ok((index, result))
}

fn discover_supported_files(root: &Path, config: &LimeConfig) -> Result<Vec<PathBuf>> {
    let matcher = build_ignore_matcher(root, config)?;
    let mut files = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !is_ignored(root, entry.path(), entry.file_type().is_dir(), &matcher)
        });

    for entry in walker {
        let entry = match entry {
            Ok(item) => item,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let extension = path.extension().and_then(|value| value.to_str());

        if let Some(extension) = extension {
            if detect_language(extension).is_some() {
                files.push(path.to_path_buf());
            }
        }
    }

    files.sort();
    Ok(files)
}

fn index_single_file(root: &Path, file_path: &Path) -> Result<Option<IndexedFileBuild>> {
    let Some(language) = detect_path_language(file_path) else {
        return Ok(None);
    };

    let bytes = fs::read(file_path)
        .with_context(|| format!("failed reading file: {}", file_path.display()))?;

    Ok(Some(build_indexed_file(root, file_path, language, &bytes)?))
}

fn build_indexed_file(
    root: &Path,
    file_path: &Path,
    language: &str,
    bytes: &[u8],
) -> Result<IndexedFileBuild> {
    let content = String::from_utf8_lossy(bytes).into_owned();
    let relative = relative_from_path(root, file_path)?;
    let parsed_components = parse_components(language, &content);

    let mut component_records = Vec::with_capacity(parsed_components.len());
    let mut component_ids = Vec::with_capacity(parsed_components.len());

    for component in parsed_components {
        let id = build_component_id(
            language,
            &component.component_type,
            &component.name,
            &relative,
            component.start_line,
        );

        component_ids.push(id.clone());
        component_records.push(ComponentRecord {
            id,
            language: language.to_string(),
            component_type: component.component_type,
            name: component.name,
            file: relative.clone(),
            start_line: component.start_line,
            end_line: component.end_line,
            uses_before: Vec::new(),
            used_by_after: Vec::new(),
            batman: false,
        });
    }

    let file_hash = hash_bytes(bytes);

    Ok(IndexedFileBuild {
        file: IndexedFile {
            path: relative,
            language: language.to_string(),
            file_hash,
            component_ids,
        },
        components: component_records,
        content,
    })
}

fn refresh_dependencies_from_disk(root: &Path, index: &mut IndexData) -> Result<()> {
    let mut file_contents = HashMap::new();
    let mut missing_paths = Vec::new();

    for file in &mut index.files {
        let absolute = root.join(Path::new(&file.path));
        match fs::read(&absolute) {
            Ok(bytes) => {
                file.file_hash = hash_bytes(&bytes);
                let content = String::from_utf8_lossy(&bytes).into_owned();
                file_contents.insert(file.path.clone(), content);
            }
            Err(_) => {
                missing_paths.push(file.path.clone());
            }
        }
    }

    for missing in missing_paths {
        remove_path_from_index(index, &missing);
    }

    deps::populate_dependencies(index, &file_contents);
    batman::detect_batman(index, &file_contents);
    index.refresh_metadata();
    Ok(())
}

fn remove_path_from_index(index: &mut IndexData, relative_path: &str) -> bool {
    let mut removed = false;

    index.files.retain(|file| {
        if file.path == relative_path {
            removed = true;
            false
        } else {
            true
        }
    });

    let original_component_len = index.components.len();
    index
        .components
        .retain(|component| component.file != relative_path);
    removed || original_component_len != index.components.len()
}

fn build_component_id(
    language: &str,
    component_type: &str,
    name: &str,
    file: &str,
    start_line: usize,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(language.as_bytes());
    hasher.update(b"|");
    hasher.update(component_type.as_bytes());
    hasher.update(b"|");
    hasher.update(name.as_bytes());
    hasher.update(b"|");
    hasher.update(file.as_bytes());
    hasher.update(b"|");
    hasher.update(start_line.to_string().as_bytes());

    let digest = hasher.finalize().to_hex().to_string();
    let prefix = component_prefix(component_type);
    format!("{}-{}", prefix, &digest[..16])
}

fn component_prefix(component_type: &str) -> String {
    let mut value = String::new();
    for character in component_type.chars() {
        if character.is_ascii_alphanumeric() {
            value.push(character.to_ascii_lowercase());
        } else if !value.ends_with('_') {
            value.push('_');
        }
    }

    let trimmed = value.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "component".to_string()
    } else {
        trimmed
    }
}

fn build_ignore_matcher(root: &Path, config: &LimeConfig) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    let gitignore_path = root.join(".gitignore");

    if gitignore_path.exists() {
        if let Some(e) = builder.add(&gitignore_path) {
            eprintln!("DEBUG: gitignore add error: {:?}", e);
        }
    }

    for pattern in &config.ignore_patterns {
        builder
            .add_line(None, pattern)
            .with_context(|| format!("invalid ignore pattern in .lime/lime.json: {pattern}"))?;
    }

    builder.build().context("failed building ignore matcher")
}

fn is_ignored(root: &Path, path: &Path, is_dir: bool, matcher: &Gitignore) -> bool {
    if path == root {
        return false;
    }

    let relative = path.strip_prefix(root).unwrap_or(path);
    matcher
        .matched_path_or_any_parents(relative, is_dir)
        .is_ignore()
}

fn absolute_from_input(root: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn relative_from_input(root: &Path, raw: &str) -> Result<String> {
    let absolute = absolute_from_input(root, raw);
    relative_from_path(root, &absolute)
}

fn relative_from_path(root: &Path, path: &Path) -> Result<String> {
    if path.is_absolute() {
        if !path.starts_with(root) {
            bail!("path is outside project root: {}", path.display());
        }
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("failed creating relative path for {}", path.display()))?;
        return Ok(normalize_path_string(relative));
    }

    Ok(normalize_path_string(path))
}

fn normalize_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn detect_path_language(path: &Path) -> Option<&'static str> {
    let extension = path.extension().and_then(|value| value.to_str())?;
    detect_language(extension)
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
