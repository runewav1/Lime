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
    config::LimeConfig,
    deps,
    index::{self, ComponentRecord, IndexData},
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
        Commands::Search { terms } => handle_search(root, terms),
        Commands::List {
            language,
            component_type,
            all,
        } => handle_list(root, language, component_type, all),
        Commands::Deps {
            component_id,
            depth,
        } => handle_deps(root, component_id, depth),
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
    ///   "lime search python class Auth" filter by language and type
    Search {
        /// Query terms. Format: [language] [type] <query>
        terms: Vec<String>,
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

fn handle_search(root: PathBuf, terms: Vec<String>) -> Result<Value> {
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

    let elapsed = timer.elapsed();

    Ok(json!({
        "ok": true,
        "command": "search",
        "elapsed_secs": elapsed.as_secs_f64(),
        "query": query,
        "result_count": results.len(),
        "results": results
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

    json!({
        "version": index.version,
        "generated_at": index.generated_at,
        "languages": index.languages,
        "file_count": index.files.len(),
        "component_count": index.components.len(),
        "component_breakdown": breakdown
    })
}
