use std::{
    collections::{BTreeMap, HashSet},
    env,
    path::PathBuf,
};

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    annotations,
    config::LimeConfig,
    deps,
    index::{self, ComponentRecord, IndexData},
    search::{self, MatchType},
    storage,
};

/// Executes the CLI command selected by the user.
pub fn run() -> Result<()> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                e.exit();
            }
            let json_mode = std::env::args().any(|a| a == "--json");
            // When no subcommand is given and not in JSON mode, show help and exit cleanly.
            if !json_mode
                && matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                )
            {
                let _ = e.print();
                std::process::exit(0);
            }
            exit_error(&e.to_string(), json_mode);
        }
    };

    let json_mode = cli.json;
    let verbose = cli.verbose;

    let root = match env::current_dir() {
        Ok(dir) => dir,
        Err(e) => exit_error(&format!("failed reading current directory: {e}"), json_mode),
    };

    let result = match cli.command {
        Commands::Sync { files } => handle_sync(root, files),
        Commands::Add { filename } => handle_add(root, filename),
        Commands::Remove { filename } => handle_remove(root, filename),
        Commands::Search { terms, fuzzy } => handle_search(root, terms, fuzzy),
        Commands::List {
            language,
            component_type,
            all,
        } => handle_list(root, language, component_type, all),
        Commands::Deps {
            component_id,
            depth,
        } => handle_deps(root, component_id, depth),
        Commands::Annotate { action } => handle_annotate(root, action),
    };

    match result {
        Ok(mut payload) => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string())
                );
            } else {
                if verbose {
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert("verbose".into(), json!(true));
                    }
                }
                print!("{}", crate::format::render(&payload));
            }
        }
        Err(e) => exit_error(&format!("{e:#}"), json_mode),
    }

    Ok(())
}

fn exit_error(message: &str, json_mode: bool) -> ! {
    if json_mode {
        let payload = json!({"ok": false, "error": message});
        println!(
            "{}",
            serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        eprint!("{}", crate::format::render_error(message));
    }
    std::process::exit(1)
}

/// Lime — language-aware codebase index for AI agents.
///
/// Indexes functions, structs, classes, and more across your project.
/// Run `lime <command> --help` for details on any subcommand.
#[derive(Debug, Parser)]
#[command(
    name = "lime",
    version,
    after_long_help = r#"Examples:
  lime sync                      Rebuild entire index
  lime search run                Find components named "run"
  lime search rust fn run        Search Rust functions
  lime list rust                 Show Rust component counts
  lime list rust --all           List all Rust components
  lime deps fn-61bcc6dabec3f308  Show dependency matrix"#
)]
struct Cli {
    #[arg(long, global = true, help = "Output raw JSON for scripts and agents")]
    json: bool,

    #[arg(short = 'v', long, global = true, help = "Show detailed output")]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Rebuild the index for the whole project or specific files.
    ///
    /// Without arguments, rebuilds from scratch. With file paths,
    /// only those files are re-indexed; the rest stays intact.
    ///
    /// Examples:
    ///   "lime sync"                        rebuild entire index
    ///   "lime sync src/main.rs"            re-index a single file
    ///   "lime sync src/lib.rs src/util.rs" re-index multiple files
    Sync {
        /// Specific files to re-index. Omit to rebuild the entire index.
        files: Vec<String>,
    },
    /// Add a single file to the index.
    ///
    /// Parses the file and adds discovered components to the index.
    /// If already indexed, its entries are refreshed.
    ///
    /// Examples:
    ///   "lime add src/auth.rs"
    ///   "lime add src/utils/mod.rs"
    Add {
        /// File path to index (relative to project root or absolute).
        filename: String,
    },
    /// Remove a file and all its components from the index.
    ///
    /// The file on disk is not modified. Use `lime add` to re-index later.
    ///
    /// Examples:
    ///   "lime remove src/old.rs"
    Remove {
        /// File path to remove (relative to project root or absolute).
        filename: String,
    },
    /// Search indexed components by name, file, or ID.
    ///
    /// Format: lime search [language] [type] <query>
    /// Case-insensitive substring match on names, IDs, and file paths.
    ///
    /// Languages: rust, javascript, typescript, python, go
    /// Types:     fn, struct, enum, trait, impl, class, def, func, and more
    ///
    /// Examples:
    ///   "lime search run"               search all languages
    ///   "lime search rust run"          filter by language
    ///   "lime search rust fn run"       filter by language and type
    ///   "lime search --fuzzy auth"      fuzzy match on tokens and annotations
    Search {
        /// Query terms. Format: [language] [type] <query>
        terms: Vec<String>,
        /// Enable fuzzy matching with token-based and annotation search.
        #[arg(long)]
        fuzzy: bool,
    },
    /// List indexed languages or components.
    ///
    /// Format: lime list [language] [type | --all]
    ///
    /// Languages: rust, javascript, typescript, python, go
    ///
    /// Examples:
    ///   "lime list"              list all indexed languages
    ///   "lime list rust"         show component counts by type
    ///   "lime list rust --all"   list every component with IDs
    ///   "lime list rust fn"      list only functions
    ///   "lime list python class" list only classes
    List {
        /// Language to inspect (e.g. `rust`, `python`).
        language: Option<String>,
        /// Component type to filter by (e.g. `fn`, `struct`, `class`).
        #[arg(allow_hyphen_values = true)]
        component_type: Option<String>,
        /// List all components for the given language (no type grouping).
        #[arg(short = 'a', long = "all")]
        all: bool,
    },
    /// Show the dependency matrix for a component.
    ///
    /// Shows what a component uses and what uses it, up to --depth levels.
    /// Default depth is from `.lime/lime.json` (usually 2). Use --depth 0 for
    /// the component alone. Get IDs from `lime search` or `lime list --all`.
    ///
    /// Examples:
    ///   "lime deps fn-abc123def456"           default depth
    ///   "lime deps fn-abc123def456 --depth 3" traverse 3 levels
    ///   "lime deps fn-abc123def456 --depth 0" component only
    Deps {
        /// Component ID to inspect (from `lime search` or `lime list`).
        component_id: String,
        /// Maximum traversal depth (default: value from .lime/lime.json, usually 2).
        #[arg(long)]
        depth: Option<usize>,
    },
    /// Manage component annotations.
    ///
    /// Attach semantic notes to indexed components. Annotations persist
    /// alongside the index and can be searched with `lime search --fuzzy`.
    ///
    /// Examples:
    ///   "lime annotate add fn-abc123 -m 'Entry point for auth'"
    ///   "lime annotate show fn-abc123"
    ///   "lime annotate list"
    ///   "lime annotate list rust fn"
    ///   "lime annotate remove fn-abc123"
    Annotate {
        #[command(subcommand)]
        action: AnnotateAction,
    },
}

#[derive(Debug, Subcommand)]
enum AnnotateAction {
    /// Add or update an annotation on a component.
    ///
    /// If the component already has an annotation, it is updated
    /// (preserving the original created_at timestamp).
    ///
    /// Examples:
    ///   "lime annotate add fn-abc123 -m 'Entry point for auth flow'"
    ///   "lime annotate add struct-def456 -m 'Primary data model for users'"
    Add {
        /// Component ID (from `lime search` or `lime list --all`).
        component_id: String,
        /// Annotation content (markdown).
        #[arg(short = 'm', long = "message")]
        message: String,
    },
    /// Display an annotation for a component.
    ///
    /// Shows the full annotation content with component metadata.
    ///
    /// Examples:
    ///   "lime annotate show fn-abc123"
    Show {
        /// Component ID.
        component_id: String,
    },
    /// List annotated components.
    ///
    /// Without arguments, lists all annotations. Optionally filter
    /// by language and component type.
    ///
    /// Examples:
    ///   "lime annotate list"              list all annotations
    ///   "lime annotate list rust"         filter by language
    ///   "lime annotate list rust fn"      filter by language and type
    List {
        /// Filter by language (e.g. `rust`, `python`).
        language: Option<String>,
        /// Filter by component type (e.g. `fn`, `struct`).
        component_type: Option<String>,
    },
    /// Remove an annotation from a component.
    ///
    /// The component itself is not affected, only its annotation is deleted.
    ///
    /// Examples:
    ///   "lime annotate remove fn-abc123"
    Remove {
        /// Component ID.
        component_id: String,
    },
}

fn handle_sync(root: PathBuf, files: Vec<String>) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;

    if files.is_empty() {
        let timer = std::time::Instant::now();
        let index = index::rebuild_index(&root, &config)?;
        let elapsed = timer.elapsed();
        storage::save_index(&root, &config, &index)?;
        return Ok(json!({
            "ok": true,
            "command": "sync",
            "scope": "full",
            "elapsed_secs": elapsed.as_secs_f64(),
            "index": summarize_index(&index)
        }));
    }

    let current = storage::load_index_or_empty(&root, &config)?;
    let timer = std::time::Instant::now();
    let (updated, result) = index::sync_files(&root, &config, &files, current)?;
    let elapsed = timer.elapsed();
    storage::save_index(&root, &config, &updated)?;

    Ok(json!({
        "ok": true,
        "command": "sync",
        "scope": "partial",
        "elapsed_secs": elapsed.as_secs_f64(),
        "request": {
            "files": files
        },
        "result": result,
        "index": summarize_index(&updated)
    }))
}

fn handle_add(root: PathBuf, filename: String) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let current = storage::load_index_or_empty(&root, &config)?;
    let timer = std::time::Instant::now();
    let (updated, result) = index::add_file(&root, &config, &filename, current)?;
    let elapsed = timer.elapsed();
    storage::save_index(&root, &config, &updated)?;

    Ok(json!({
        "ok": true,
        "command": "add",
        "elapsed_secs": elapsed.as_secs_f64(),
        "request": {
            "filename": filename
        },
        "result": result,
        "index": summarize_index(&updated)
    }))
}

fn handle_remove(root: PathBuf, filename: String) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let current = storage::load_index_or_empty(&root, &config)?;
    let timer = std::time::Instant::now();
    let (updated, removed) = index::remove_file(&root, &filename, current)?;
    let elapsed = timer.elapsed();
    if removed {
        storage::save_index(&root, &config, &updated)?;
    }

    Ok(json!({
        "ok": true,
        "command": "remove",
        "elapsed_secs": elapsed.as_secs_f64(),
        "request": {
            "filename": filename
        },
        "removed": removed,
        "index": summarize_index(&updated)
    }))
}

fn handle_search(root: PathBuf, terms: Vec<String>, fuzzy: bool) -> Result<Value> {
    if terms.is_empty() {
        bail!("search requires at least one argument");
    }

    let query = parse_search_terms(terms)?;
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let mut index = storage::load_index_or_empty(&root, &config)?;

    let normalized_query = query.query.to_ascii_lowercase();
    if index.search_index.is_none() {
        index.search_index = Some(index.build_search_index());
    }

    let mut results: Vec<&ComponentRecord> = if let Some(search_index) = &index.search_index {
        let mut matched_indices = HashSet::new();

        if let Some(indices) = search_index.get(&normalized_query) {
            for &component_index in indices {
                if let Some(component) = index.components.get(component_index) {
                    if matches_search_filters(component, &query) {
                        matched_indices.insert(component_index);
                    }
                }
            }
        }

        for (name, indices) in search_index {
            if name == &normalized_query || !name.contains(&normalized_query) {
                continue;
            }

            for &component_index in indices {
                if let Some(component) = index.components.get(component_index) {
                    if matches_search_filters(component, &query) {
                        matched_indices.insert(component_index);
                    }
                }
            }
        }

        for (component_index, component) in index.components.iter().enumerate() {
            if !matches_search_filters(component, &query) {
                continue;
            }

            if component
                .id
                .to_ascii_lowercase()
                .contains(&normalized_query)
                || component
                    .file
                    .to_ascii_lowercase()
                    .contains(&normalized_query)
            {
                matched_indices.insert(component_index);
            }
        }

        matched_indices
            .into_iter()
            .filter_map(|component_index| index.components.get(component_index))
            .collect()
    } else {
        index
            .components
            .iter()
            .filter(|component| {
                if !matches_search_filters(component, &query) {
                    return false;
                }

                component
                    .name
                    .to_ascii_lowercase()
                    .contains(&normalized_query)
                    || component
                        .id
                        .to_ascii_lowercase()
                        .contains(&normalized_query)
                    || component
                        .file
                        .to_ascii_lowercase()
                        .contains(&normalized_query)
            })
            .collect()
    };

    results.sort_by(|left, right| {
        (
            left.language.as_str(),
            left.file.as_str(),
            left.start_line,
            left.component_type.as_str(),
            left.name.as_str(),
            left.id.as_str(),
        )
            .cmp(&(
                right.language.as_str(),
                right.file.as_str(),
                right.start_line,
                right.component_type.as_str(),
                right.name.as_str(),
                right.id.as_str(),
            ))
    });

    if !fuzzy {
        let elapsed = timer.elapsed();
        return Ok(json!({
            "ok": true,
            "command": "search",
            "elapsed_secs": elapsed.as_secs_f64(),
            "query": query,
            "fuzzy": false,
            "result_count": results.len(),
            "results": results
        }));
    }

    let exact_ids: HashSet<String> = results.iter().map(|c| c.id.clone()).collect();

    let all_annotations = annotations::list_annotations(&root).unwrap_or_default();
    let token_index = search::build_token_index(&index, &all_annotations);
    let fuzzy_hits = search::fuzzy_search(&token_index, &query.query);

    let annotation_map: std::collections::HashMap<String, &annotations::Annotation> =
        all_annotations
            .iter()
            .map(|a| (a.hash_id.clone(), a))
            .collect();

    let component_map: std::collections::HashMap<&str, &ComponentRecord> = index
        .components
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    #[derive(Serialize)]
    struct FuzzyResult<'a> {
        #[serde(flatten)]
        component: &'a ComponentRecord,
        match_type: &'static str,
        annotation_preview: Option<String>,
        #[serde(skip)]
        sort_score: f64,
        #[serde(skip)]
        sort_match_rank: u8,
    }

    let mut merged: Vec<FuzzyResult> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    for component in &results {
        if seen_ids.insert(component.id.clone()) {
            let preview = annotation_map
                .get(&component.id)
                .map(|a| truncate_preview(&a.content, 80));
            merged.push(FuzzyResult {
                component,
                match_type: MatchType::Exact.as_str(),
                annotation_preview: preview,
                sort_score: 2.0,
                sort_match_rank: 0,
            });
        }
    }

    for hit in &fuzzy_hits {
        if exact_ids.contains(&hit.component_id) {
            continue;
        }
        if !seen_ids.insert(hit.component_id.clone()) {
            continue;
        }
        let Some(component) = component_map.get(hit.component_id.as_str()) else {
            continue;
        };
        if !matches_search_filters(component, &query) {
            continue;
        }
        let preview = annotation_map
            .get(&hit.component_id)
            .map(|a| truncate_preview(&a.content, 80));
        merged.push(FuzzyResult {
            component,
            match_type: hit.match_type.as_str(),
            annotation_preview: preview,
            sort_score: hit.score,
            sort_match_rank: hit.match_type.rank(),
        });
    }

    merged.sort_by(|a, b| {
        a.sort_match_rank
            .cmp(&b.sort_match_rank)
            .then(
                b.sort_score
                    .partial_cmp(&a.sort_score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(
                (
                    a.component.language.as_str(),
                    a.component.file.as_str(),
                    a.component.start_line,
                    a.component.component_type.as_str(),
                    a.component.name.as_str(),
                )
                    .cmp(&(
                        b.component.language.as_str(),
                        b.component.file.as_str(),
                        b.component.start_line,
                        b.component.component_type.as_str(),
                        b.component.name.as_str(),
                    )),
            )
    });

    let elapsed = timer.elapsed();

    let results_json: Vec<Value> = merged
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r.component).unwrap_or(json!({}));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("match_type".to_string(), json!(r.match_type));
                obj.insert(
                    "annotation_preview".to_string(),
                    match &r.annotation_preview {
                        Some(p) => json!(p),
                        None => Value::Null,
                    },
                );
            }
            v
        })
        .collect();

    Ok(json!({
        "ok": true,
        "command": "search",
        "elapsed_secs": elapsed.as_secs_f64(),
        "query": query,
        "fuzzy": true,
        "result_count": results_json.len(),
        "results": results_json
    }))
}

fn matches_search_filters(component: &ComponentRecord, query: &SearchQuery) -> bool {
    if let Some(language) = &query.language {
        if component.language != *language {
            return false;
        }
    }

    if let Some(component_type) = &query.component_type {
        if component.component_type.to_ascii_lowercase() != *component_type {
            return false;
        }
    }

    true
}

fn handle_list(
    root: PathBuf,
    language: Option<String>,
    mut component_type: Option<String>,
    mut all: bool,
) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;

    if matches!(component_type.as_deref(), Some("-all")) {
        all = true;
        component_type = None;
    }

    if all && component_type.is_some() {
        bail!("cannot combine component type filter with --all")
    }

    let Some(language) = language else {
        let elapsed = timer.elapsed();
        return Ok(json!({
            "ok": true,
            "command": "list",
            "mode": "languages",
            "elapsed_secs": elapsed.as_secs_f64(),
            "languages": index.languages
        }));
    };

    let language = normalize_language(&language)?;
    let mut components: Vec<&ComponentRecord> = index
        .components
        .iter()
        .filter(|component| component.language == language)
        .collect();

    if let Some(component_type) = component_type {
        let component_type = component_type.to_ascii_lowercase();
        components
            .retain(|component| component.component_type.to_ascii_lowercase() == component_type);
        let elapsed = timer.elapsed();
        return Ok(json!({
            "ok": true,
            "command": "list",
            "mode": "language_and_type",
            "elapsed_secs": elapsed.as_secs_f64(),
            "language": language,
            "type": component_type,
            "count": components.len(),
            "components": components
        }));
    }

    if all {
        components.sort_by(|left, right| {
            (
                left.file.as_str(),
                left.start_line,
                left.component_type.as_str(),
                left.name.as_str(),
                left.id.as_str(),
            )
                .cmp(&(
                    right.file.as_str(),
                    right.start_line,
                    right.component_type.as_str(),
                    right.name.as_str(),
                    right.id.as_str(),
                ))
        });

        let elapsed = timer.elapsed();
        return Ok(json!({
            "ok": true,
            "command": "list",
            "mode": "language_all",
            "elapsed_secs": elapsed.as_secs_f64(),
            "language": language,
            "count": components.len(),
            "components": components
        }));
    }

    let mut by_type = BTreeMap::<String, usize>::new();
    for component in components {
        *by_type.entry(component.component_type.clone()).or_insert(0) += 1;
    }

    let elapsed = timer.elapsed();
    Ok(json!({
        "ok": true,
        "command": "list",
        "mode": "language_summary",
        "elapsed_secs": elapsed.as_secs_f64(),
        "language": language,
        "component_counts": by_type
    }))
}

fn handle_deps(root: PathBuf, component_id: String, depth: Option<usize>) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;
    let effective_depth = depth.unwrap_or(config.default_dependency_depth);

    let root_component = index
        .components
        .iter()
        .find(|component| component.id == component_id)
        .ok_or_else(|| anyhow!("component not found: {component_id}"))?;

    let matrix = deps::dependency_tree(&index, &component_id, effective_depth)
        .ok_or_else(|| anyhow!("component not found: {component_id}"))?;

    let elapsed = timer.elapsed();

    Ok(json!({
        "ok": true,
        "command": "deps",
        "elapsed_secs": elapsed.as_secs_f64(),
        "component": root_component,
        "dependency_matrix": matrix
    }))
}

fn handle_annotate(root: PathBuf, action: AnnotateAction) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;

    match action {
        AnnotateAction::Add {
            component_id,
            message,
        } => {
            let component = index
                .components
                .iter()
                .find(|c| c.id == component_id)
                .ok_or_else(|| anyhow!("component not found: {component_id}"))?;

            let now = chrono::Utc::now().to_rfc3339();
            let existing = annotations::load_annotation(&root, &component_id)?;
            let created_at = existing
                .as_ref()
                .map(|a| a.created_at.clone())
                .unwrap_or_else(|| now.clone());

            let (comp_type, _) = component_id
                .split_once('-')
                .unwrap_or(("component", &component_id));

            let annotation = annotations::Annotation {
                hash_id: component_id.clone(),
                component_type: comp_type.to_string(),
                component_name: component.name.clone(),
                content: message.clone(),
                created_at,
                updated_at: now,
            };

            annotations::save_annotation(&root, &annotation)?;
            let elapsed = timer.elapsed();

            Ok(json!({
                "ok": true,
                "command": "annotate",
                "action": "add",
                "elapsed_secs": elapsed.as_secs_f64(),
                "component": component,
                "annotation": {
                    "hash_id": annotation.hash_id,
                    "content": annotation.content,
                    "created_at": annotation.created_at,
                    "updated_at": annotation.updated_at
                }
            }))
        }
        AnnotateAction::Show { component_id } => {
            let component = index
                .components
                .iter()
                .find(|c| c.id == component_id)
                .ok_or_else(|| anyhow!("component not found: {component_id}"))?;

            let annotation = annotations::load_annotation(&root, &component_id)?
                .ok_or_else(|| anyhow!("no annotation for component: {component_id}"))?;
            let elapsed = timer.elapsed();

            Ok(json!({
                "ok": true,
                "command": "annotate",
                "action": "show",
                "elapsed_secs": elapsed.as_secs_f64(),
                "component": component,
                "annotation": {
                    "hash_id": annotation.hash_id,
                    "content": annotation.content,
                    "created_at": annotation.created_at,
                    "updated_at": annotation.updated_at
                }
            }))
        }
        AnnotateAction::List {
            language,
            component_type,
        } => {
            let all_annotations = annotations::list_annotations(&root)?;
            let component_map: std::collections::HashMap<&str, &ComponentRecord> = index
                .components
                .iter()
                .map(|c| (c.id.as_str(), c))
                .collect();

            let filtered: Vec<Value> = all_annotations
                .iter()
                .filter_map(|ann| {
                    let comp = component_map.get(ann.hash_id.as_str())?;
                    if let Some(lang) = &language {
                        if comp.language != *lang {
                            return None;
                        }
                    }
                    if let Some(ctype) = &component_type {
                        if comp.component_type.to_ascii_lowercase() != *ctype {
                            return None;
                        }
                    }
                    Some(json!({
                        "component": comp,
                        "annotation": {
                            "hash_id": ann.hash_id,
                            "content": ann.content,
                            "preview": truncate_preview(&ann.content, 80),
                            "created_at": ann.created_at,
                            "updated_at": ann.updated_at
                        }
                    }))
                })
                .collect();

            let elapsed = timer.elapsed();

            Ok(json!({
                "ok": true,
                "command": "annotate",
                "action": "list",
                "elapsed_secs": elapsed.as_secs_f64(),
                "count": filtered.len(),
                "results": filtered
            }))
        }
        AnnotateAction::Remove { component_id } => {
            let removed = annotations::remove_annotation(&root, &component_id)?;
            let elapsed = timer.elapsed();

            Ok(json!({
                "ok": true,
                "command": "annotate",
                "action": "remove",
                "elapsed_secs": elapsed.as_secs_f64(),
                "component_id": component_id,
                "removed": removed
            }))
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SearchQuery {
    language: Option<String>,
    #[serde(rename = "type")]
    component_type: Option<String>,
    query: String,
}

fn parse_search_terms(terms: Vec<String>) -> Result<SearchQuery> {
    match terms.len() {
        1 => Ok(SearchQuery {
            language: None,
            component_type: None,
            query: terms[0].clone(),
        }),
        2 => {
            let language = normalize_language(&terms[0])?;
            Ok(SearchQuery {
                language: Some(language),
                component_type: None,
                query: terms[1].clone(),
            })
        }
        _ => {
            let language = normalize_language(&terms[0])?;
            let candidate_type = terms[1].to_ascii_lowercase();
            let (component_type, query) = if is_component_type_filter(&candidate_type) {
                (Some(candidate_type), terms[2..].join(" "))
            } else {
                (None, terms[1..].join(" "))
            };

            if query.trim().is_empty() {
                bail!("search query cannot be empty");
            }

            Ok(SearchQuery {
                language: Some(language),
                component_type,
                query,
            })
        }
    }
}

fn normalize_language(value: &str) -> Result<String> {
    let normalized = value.to_ascii_lowercase();
    if supported_languages().contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        bail!(
            "unsupported language `{value}`; supported languages: {}",
            supported_languages().join(", ")
        )
    }
}

fn supported_languages() -> Vec<&'static str> {
    vec!["rust", "javascript", "typescript", "python", "go"]
}

fn is_component_type_filter(value: &str) -> bool {
    matches!(
        value,
        "struct"
            | "enum"
            | "fn"
            | "trait"
            | "impl"
            | "mod"
            | "use"
            | "class"
            | "function"
            | "const"
            | "let"
            | "var"
            | "interface"
            | "type"
            | "export"
            | "def"
            | "async def"
            | "async"
            | "import"
            | "from"
            | "func"
    )
}

fn summarize_index(index: &IndexData) -> Value {
    let mut breakdown = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for c in &index.components {
        *breakdown
            .entry(c.language.clone())
            .or_default()
            .entry(c.component_type.clone())
            .or_insert(0) += 1;
    }

    let batman_count = index
        .components
        .iter()
        .filter(|component| component.batman)
        .count();

    json!({
        "version": index.version,
        "generated_at": index.generated_at,
        "languages": index.languages,
        "file_count": index.files.len(),
        "component_count": index.components.len(),
        "batman_count": batman_count,
        "component_breakdown": breakdown
    })
}

fn truncate_preview(content: &str, max_len: usize) -> String {
    let single_line = content.lines().next().unwrap_or("").trim();
    if single_line.len() <= max_len {
        single_line.to_string()
    } else {
        format!("{}...", &single_line[..max_len.saturating_sub(3)])
    }
}
