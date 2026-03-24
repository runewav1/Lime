use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    annotations,
    config::LimeConfig,
    deps, diagnostics, git_staleness,
    index::{self, ComponentRecord, DeathStatus, FileUpdateResult, IndexData},
    links, projects_registry,
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

    let root = match env::current_dir() {
        Ok(dir) => dir,
        Err(e) => exit_error(&format!("failed reading current directory: {e}"), json_mode),
    };

    let external = cli.external.clone();
    let result = match cli.command {
        Commands::Sync {
            files,
            diagnostics,
            verbose,
            git,
            no_git,
        } => {
            ensure_external_disabled("sync", external.as_deref())?;
            handle_sync(root, files, diagnostics, verbose, git, no_git)
        }
        Commands::Add { filename } => {
            ensure_external_disabled("add", external.as_deref())?;
            handle_add(root, filename)
        }
        Commands::Remove { filename } => {
            ensure_external_disabled("remove", external.as_deref())?;
            handle_remove(root, filename)
        }
        Commands::Search { terms, fuzzy } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_search(target.effective_root.clone(), terms, fuzzy)
                .map(|p| attach_target(p, &target))
        }
        Commands::Links { action } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_links(target.effective_root.clone(), action).map(|p| attach_target(p, &target))
        }
        Commands::Sum { top_links } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_sum(target.effective_root.clone(), top_links).map(|p| attach_target(p, &target))
        }
        Commands::List {
            language,
            component_type,
            all,
            dead,
            fault,
        } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_list(
                target.effective_root.clone(),
                language,
                component_type,
                all,
                dead,
                fault,
            )
            .map(|p| attach_target(p, &target))
        }
        Commands::Show { component_id } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_show(target.effective_root.clone(), component_id)
                .map(|p| attach_target(p, &target))
        }
        Commands::Deps {
            component_id,
            depth,
        } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_deps(target.effective_root.clone(), component_id, depth)
                .map(|p| attach_target(p, &target))
        }
        Commands::Annotate { action } => {
            let target = resolve_command_target(&root, external.clone())?;
            handle_annotate(target, action)
        }
        Commands::Config { action, global } => {
            ensure_external_disabled("config", external.as_deref())?;
            handle_config(root, action, global)
        }
        Commands::Registry { action } => {
            ensure_external_disabled("registry", external.as_deref())?;
            handle_registry(root.clone(), action)
        }
    };

    match result {
        Ok(payload) => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string())
                );
            } else {
                print!("{}", crate::format::render(&payload));
            }
        }
        Err(e) => exit_error(&format!("{e:#}"), json_mode),
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct CommandTarget {
    cwd: PathBuf,
    effective_root: PathBuf,
    project_id: Option<String>,
}

impl CommandTarget {
    fn local(cwd: PathBuf) -> Self {
        Self {
            cwd: cwd.clone(),
            effective_root: cwd,
            project_id: None,
        }
    }

    fn is_foreign(&self) -> bool {
        canonical_for_compare(&self.cwd) != canonical_for_compare(&self.effective_root)
    }
}

fn canonical_for_compare(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn ensure_external_disabled(scope: &str, external: Option<&str>) -> Result<()> {
    if let Some(project_id) = external {
        bail!(
            "`--external {}` is not supported for {}; run this command from the target repository",
            project_id,
            scope
        );
    }
    Ok(())
}

fn resolve_command_target(cwd: &Path, external: Option<String>) -> Result<CommandTarget> {
    if let Some(project_id) = external {
        let effective_root = projects_registry::resolve_project_root(&project_id)?;
        Ok(CommandTarget {
            cwd: cwd.to_path_buf(),
            effective_root,
            project_id: Some(project_id),
        })
    } else {
        Ok(CommandTarget::local(cwd.to_path_buf()))
    }
}

fn attach_target(mut payload: Value, target: &CommandTarget) -> Value {
    let target_json = if let Some(project_id) = &target.project_id {
        json!({
            "mode": "external",
            "project_id": project_id,
            "root": target.effective_root.display().to_string(),
        })
    } else {
        json!({
            "mode": "local",
            "root": target.effective_root.display().to_string(),
        })
    };
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("target".to_string(), target_json);
    }
    payload
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
    about = "Language-aware codebase index, search, deps, annotations, and links for agents.",
    long_about = "Lime builds a component index under .lime/, supports git-partial sync when configured, and optional --json on every command for machine output. Use lime registry add + --external <id> to query another registered repo without copying its index.",
    after_long_help = r#"Examples:
  lime sync                      Full or git-partial empty sync (see config); first run is full if index empty
  lime sync --git                Partial sync on git dirty paths (no file args)
  lime sync src/main.rs          Re-index only these files
  lime search run                Find components named "run"
  lime search rust fn run        Search Rust functions
  lime list rust                 Show Rust component counts by type
  lime list rust --all           List all Rust components with IDs
  lime deps fn-61bcc6dabec3f308  Dependency matrix (depth from config or --depth)
  lime links show auth           Components on link path auth (merged store + annotations)
  lime links show @peer/topic    Scoped path (registered project + topic)
  lime --external r links add …  Add/remove/compact links in that repo's .lime/
  lime links list                All link paths
  lime registry add              Register cwd in ~/.lime/projects.json
  lime registry add --id tokio ../tokio   Register with explicit id
  lime --external tokio show fn-abc123    Read from another registered root
  lime sum                       Overview: counts, top links, staleness

Global flags: --json (machine output), --external <id> (route index/IO to another registered root: show, search, list, deps, sum, links, annotate; links add/remove/compact write that repo's `.lime/`)."#
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Print machine-readable JSON (stable for scripts and agents)"
    )]
    json: bool,
    #[arg(
        long,
        global = true,
        value_name = "PROJECT_ID",
        help = "Registered project id → use that repo root for commands that support it (see `lime --help`; includes links add/remove/compact and annotate add)"
    )]
    external: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Rebuild the index for the whole project or specific files.
    ///
    /// **With file paths:** partial re-index of only those files; `--git` / `--no-git` are ignored.
    ///
    /// **With no paths:** full rebuild, git-partial on dirty paths, or noop — see `lime config git-partial-sync`.
    /// If git-partial is enabled, the first sync still does a **full** index when the index has no components yet.
    ///
    /// Examples:
    ///   "lime sync"                        full index, or git partial if configured (only after index exists)
    ///   "lime sync --git"                  partial sync on git dirty paths
    ///   "lime sync --no-git"               full rebuild even if config uses git partial
    ///   "lime sync src/main.rs"            re-index a single file (git flags ignored)
    Sync {
        /// Files to re-index (project-relative or absolute). If empty, whole-project sync per config/flags.
        #[arg(value_name = "FILE")]
        files: Vec<String>,
        /// Run static analyzers during sync and attach fault data to components (`diagnostics.*` in config).
        #[arg(long)]
        diagnostics: bool,
        /// Verbose human output: per-file / component breakdown where supported.
        #[arg(short = 'v', long)]
        verbose: bool,
        /// Empty sync only: use `git status` dirty paths (overrides `git_partial_sync` default for this run).
        #[arg(long, conflicts_with = "no_git")]
        git: bool,
        /// Empty sync only: force full rebuild (overrides `git_partial_sync.empty_sync_uses_git`).
        #[arg(long, conflicts_with = "git")]
        no_git: bool,
    },
    /// Add a single file to the index.
    ///
    /// Parses the file and adds discovered components to the index.
    /// If already indexed, its entries are refreshed. Run from the project root (or use paths relative to it).
    ///
    /// Examples:
    ///   "lime add src/auth.rs"
    ///   "lime add src/utils/mod.rs"
    Add {
        /// File path to index (relative to project root or absolute).
        #[arg(value_name = "PATH")]
        filename: String,
    },
    /// Remove a file and all its components from the index.
    ///
    /// The file on disk is not modified. Use `lime add` to re-index later.
    ///
    /// Examples:
    ///   "lime remove src/old.rs"
    Remove {
        /// File path to drop from the index only; disk file is unchanged.
        #[arg(value_name = "PATH")]
        filename: String,
    },
    /// Search indexed components by name, file, or ID.
    ///
    /// Format: lime search [language] [type] <query>
    /// Case-insensitive substring match on names, IDs, and file paths.
    ///
    /// Languages: rust (alias: rs), python (alias: py), javascript (alias: js), typescript (alias: ts), go, zig, c, cpp, swift
    /// Types:     fn, struct, enum, trait, impl, class, def, func, and more
    ///
    /// Examples:
    ///   "lime search run"               search all languages
    ///   "lime search rust run"          filter by language
    ///   "lime search js run"            javascript via alias
    ///   "lime search rust fn run"       filter by language and type
    ///   "lime search --fuzzy auth"      fuzzy match on tokens and annotations
    Search {
        /// Terms: `[language] [type] <query>` — language aliases: rs, py, js, ts. Multi-word query after language/type.
        #[arg(value_name = "TERMS")]
        terms: Vec<String>,
        /// Fuzzy match on tokens, stems, and annotation text (slower, broader).
        #[arg(long)]
        fuzzy: bool,
    },
    /// Link paths: query members, list paths, or edit `.lime/component_links.json`.
    ///
    /// All link operations use one subcommand group:
    /// - **show** — components whose merged paths match a path or prefix (`auth` matches `auth/login`). Scoped: `@<registered_id>/<topic>`; optional **`--peer-resolve`** loads the peer index and matches local paths to the topic tail.
    /// - **list** — distinct paths (`--tree` indents by `/` depth).
    /// - **add** / **remove** — membership without editing annotation prose (local or scoped paths; scoped ids must be in `lime registry list`).
    /// - **compact** — drop duplicate `links` lines from annotations when the store already has the path.
    ///
    /// Membership is merged from `.lime/component_links.json` and annotation frontmatter `links`.
    ///
    /// Examples:
    ///   "lime links show auth"
    ///   "lime links show @myapp/topic --peer-resolve"
    ///   "lime links show auth/login --notes"
    ///   "lime links list"
    ///   "lime links list auth --tree"
    ///   "lime links add fn-abc123 auth/login"
    ///   "lime --external other links add fn-x @app/feature"
    ///   "lime links remove fn-abc123 auth/login"
    ///   "lime links compact"
    Links {
        #[command(subcommand)]
        action: LinksAction,
    },
    /// Bounded index overview for agents (counts, top link labels, staleness; no component list).
    ///
    /// Examples:
    ///   "lime sum"
    ///   "lime sum --top-links 16"
    Sum {
        /// How many of the most frequent link paths to include in `links_top` (1–256 clamped in handler).
        #[arg(long, default_value_t = 32, value_name = "N")]
        top_links: usize,
    },
    /// List indexed languages or components.
    ///
    /// Format: lime list [language] [type | --all]
    ///
    /// Languages: rust (alias: rs), python (alias: py), javascript (alias: js), typescript (alias: ts), go, zig, c, cpp, swift
    ///
    /// Examples:
    ///   "lime list"              list all indexed languages
    ///   "lime list rust"         show component counts by type
    ///   "lime list rust --all"   list every component with IDs
    ///   "lime list rust fn"      list only functions
    ///   "lime list python class" list only classes
    ///   "lime list ts --all"     typescript components (alias)
    ///
    /// `--dead` / `--fault` without `--all` still lists all matching components for the language (full list mode).
    List {
        /// Language (`rs`/`py`/`js`/`ts` → rust/python/javascript/typescript). Omit: list indexed languages only.
        #[arg(value_name = "LANG")]
        language: Option<String>,
        /// Component type (e.g. `fn`, `struct`, `async def`). Incompatible with `--all`.
        #[arg(allow_hyphen_values = true, value_name = "TYPE")]
        component_type: Option<String>,
        /// List every component with IDs for the language (no per-type summary).
        #[arg(short = 'a', long = "all")]
        all: bool,
        /// Restrict to dead-code tier (combine with `--fault` for union).
        #[arg(long)]
        dead: bool,
        /// Restrict to components with diagnostic faults (combine with `--dead` for union).
        #[arg(long)]
        fault: bool,
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
    ///
    /// Traversal depth limits the **display** graph only; death detection uses the full stored edge set.
    Deps {
        /// Component ID (`type-hexhash` from `lime list --all` or `lime search`).
        #[arg(value_name = "COMPONENT_ID")]
        component_id: String,
        /// Max hops in each direction (uses / used-by). Default: `default_dependency_depth` in config. `0` = no neighbors in matrix.
        #[arg(long, value_name = "N")]
        depth: Option<usize>,
    },
    /// Manage component annotations.
    ///
    /// Attach semantic notes to indexed components. Annotations persist
    /// alongside the index and can be searched with `lime search --fuzzy`.
    /// With `--external`, `add`/`show`/`list` target the foreign index; valid `-l` paths also update that repo’s link store; `remove` is not allowed (see error text).
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
    /// Show a component's source code with line numbers and inline diagnostics.
    ///
    /// Reads the source file from disk and prints the component's line range
    /// with syntax highlighting, diagnostic markers, and the annotation
    /// (if any). Use component IDs from `lime list --all` or `lime search`.
    ///
    /// Examples:
    ///   "lime show fn-abc123def456"
    ///   "lime --external tokio show fn-abc123"
    Show {
        /// Component ID (`type-hexhash`).
        #[arg(value_name = "COMPONENT_ID")]
        component_id: String,
    },

    /// Inspect and update Lime configuration.
    ///
    /// By default this reads/writes the **project** config (`.lime/lime.json`).
    /// Pass **`--global` before the subcommand** to operate on the **global** template
    /// (`~/.config/lime/lime.json` on Linux/macOS, `%APPDATA%\lime\lime.json` on Windows).
    ///
    /// The global config is used as a template when initialising new project
    /// configs, so preferences set globally carry over automatically.
    ///
    /// Examples:
    ///   "lime config show"
    ///   "lime config --global show"
    ///   "lime config --global git-partial-sync --git-empty-sync true"
    ///   "lime config diagnostics --enabled true --timeout 120"
    ///   "lime config dependency-depth --depth 3"
    ///   "lime config ignores --add vendor/"
    ///   "lime config index-storage --path .lime/index.json"
    ///   "lime config death-seeds --seed-file src/main.rs --seed-name main --seed-type fn"
    Config {
        #[command(subcommand)]
        action: ConfigAction,
        /// Apply to the user-wide template file instead of `.lime/lime.json` (place `--global` right after `lime config`).
        #[arg(long, global = true)]
        global: bool,
    },
    /// Global router: register repository roots in `~/.lime/projects.json` (no `.lime` init required).
    ///
    /// Maps `projectID` → absolute root so `lime --external <projectID> …` can read (and safely annotate-add)
    /// another checkout without copying indexes.
    ///
    /// Examples:
    ///   "lime registry list"
    ///   "lime registry add"                    # current directory
    ///   "lime registry add ../tokio"
    ///   "lime registry add --id ruststd C:/dev/rust/library"
    ///   "lime registry remove ruststd"
    Registry {
        #[command(subcommand)]
        action: RegistryAction,
    },
}

#[derive(Debug, Subcommand)]
enum RegistryAction {
    /// List all registered repository roots.
    List,
    /// Register a repository root (defaults to current directory if path omitted).
    Add {
        /// Project id for `--external <id>` (default: basename of registered path).
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Root directory to register (default: current working directory).
        #[arg(value_name = "ROOT")]
        path: Option<PathBuf>,
    },
    /// Remove a registered project id from the router file.
    Remove {
        /// Id as shown in `lime registry list`.
        #[arg(value_name = "PROJECT_ID")]
        project_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum LinksAction {
    /// List components whose merged link paths match this path or prefix (store + annotations).
    ///
    /// Prefix match: `auth` matches `auth` and `auth/login`. Use `--notes` for full annotation bodies.
    ///
    /// Examples:
    ///   "lime links show auth"
    ///   "lime links show auth/oauth --notes"
    Show {
        /// Path or prefix (`auth` matches `auth` and `auth/login`). Scoped: `@<registered_id>/<topic>`.
        #[arg(value_name = "PATH_OR_PREFIX")]
        path: String,
        /// Include full annotation body per hit (JSON/human).
        #[arg(long)]
        notes: bool,
        /// With a scoped query `@peer/topic`, also load that peer repo and include components whose **local** paths match `topic` (extra index load).
        #[arg(long = "peer-resolve")]
        peer_resolve: bool,
    },
    /// List distinct link paths across the project (merged store + annotations).
    List {
        /// Restrict to paths under this prefix (case-insensitive). Omit: all paths.
        #[arg(value_name = "PREFIX")]
        prefix: Option<String>,
        /// Tree-style indentation by `/` depth (human output).
        #[arg(long)]
        tree: bool,
    },
    /// Add a link path for a component (writes `.lime/component_links.json`).
    Add {
        /// Component ID.
        #[arg(value_name = "COMPONENT_ID")]
        component_id: String,
        /// Hierarchical path (`/` segments, e.g. `auth/login`).
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Remove a link path from the link store for a component.
    Remove {
        #[arg(value_name = "COMPONENT_ID")]
        component_id: String,
        #[arg(value_name = "PATH")]
        path: String,
    },
    /// Remove redundant `links:` lines from annotation front matter when the link store already has the path.
    Compact,
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
    ///   "lime annotate add fn-x --file docs/fn-x.md --link auth/login"
    ///   "lime annotate add fn-x --link auth"   (updates links only if annotation exists; dual-writes link store)
    Add {
        /// Component ID to annotate.
        #[arg(value_name = "COMPONENT_ID")]
        component_id: String,
        /// Inline markdown body (cannot be used with `--file`).
        #[arg(short = 'm', long = "message", conflicts_with = "body_file")]
        message: Option<String>,
        /// Read annotation body from this file (repo-relative or absolute; cannot be used with `-m`).
        #[arg(long = "file", conflicts_with = "message")]
        body_file: Option<PathBuf>,
        /// Tags for the annotation (e.g. keep, public_api, entrypoint).
        #[arg(short = 't', long = "tag")]
        tags: Vec<String>,
        /// Link paths (`/` segments); dual-written to `.lime/component_links.json` and annotation frontmatter.
        #[arg(short = 'l', long = "link")]
        links: Vec<String>,
    },
    /// Display an annotation for a component.
    ///
    /// Shows the full annotation content with component metadata.
    ///
    /// Examples:
    ///   "lime annotate show fn-abc123"
    Show {
        #[arg(value_name = "COMPONENT_ID")]
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
    ///   "lime annotate list ts fn"        typescript (alias) + type
    ///   "lime annotate list rust fn"      filter by language and type
    List {
        /// Filter by language (aliases: rs, py, js, ts).
        #[arg(value_name = "LANG")]
        language: Option<String>,
        /// Filter by component type (e.g. `fn`, `async def`).
        #[arg(allow_hyphen_values = true, value_name = "TYPE")]
        component_type: Option<String>,
    },
    /// Remove an annotation from a component.
    ///
    /// The component itself is not affected, only its annotation is deleted.
    ///
    /// Examples:
    ///   "lime annotate remove fn-abc123"
    Remove {
        #[arg(value_name = "COMPONENT_ID")]
        component_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Show the current configuration from `.lime/lime.json`.
    ///
    /// This prints the full JSON so it can be inspected or piped to tools.
    Show,

    /// Update diagnostics configuration.
    ///
    /// Controls whether static analysis runs during `lime sync --diagnostics`,
    /// and sets per-language tool timeout.
    ///
    /// Examples:
    ///   "lime config diagnostics --enabled true"
    ///   "lime config diagnostics --timeout 60"
    Diagnostics {
        /// Enable or disable diagnostics integration.
        #[arg(long)]
        enabled: Option<bool>,
        /// Timeout in seconds for each analyzer invocation.
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Update death seed configuration for the component death algorithm.
    ///
    /// Seed files / names / types are treated as always-alive roots.
    /// Use the `--seed-*` flags to add new entries, and the `--clear-*`
    /// flags to wipe existing lists entirely before adding.
    ///
    /// Examples:
    ///   "lime config death-seeds --seed-file src/main.rs --seed-name main --seed-type fn"
    ///   "lime config death-seeds --clear-seed-files --seed-file cmd/**"
    DeathSeeds {
        /// Add file path patterns whose components are always alive seeds.
        #[arg(long = "seed-file")]
        seed_files: Vec<String>,
        /// Add component name patterns that are always alive seeds (exact match).
        #[arg(long = "seed-name")]
        seed_names: Vec<String>,
        /// Add component types that are always alive seeds (e.g. "fn", "class").
        #[arg(long = "seed-type")]
        seed_types: Vec<String>,
        /// Clear all existing seed file patterns before applying additions.
        #[arg(long)]
        clear_seed_files: bool,
        /// Clear all existing seed name patterns before applying additions.
        #[arg(long)]
        clear_seed_names: bool,
        /// Clear all existing seed type patterns before applying additions.
        #[arg(long)]
        clear_seed_types: bool,
    },

    /// Index file serialization (`index.json`).
    ///
    /// Examples:
    ///   "lime config index"              show current `index_pretty`
    ///   "lime config index --pretty false"  write compact JSON on next sync/save
    Index {
        /// Pretty-print index JSON when saving (omit to only show current value).
        #[arg(long)]
        pretty: Option<bool>,
    },

    /// Git-assisted partial sync for bare `lime sync` (no file args), after the index has components.
    ///
    /// Examples:
    ///   "lime config git-partial-sync"
    ///   "lime config git-partial-sync --use-git-for-empty-sync true"
    ///   "lime config git-partial-sync --git-empty-sync true"
    GitPartialSync {
        /// Use `git status` dirty paths for empty `lime sync` (omit to only show current value).
        #[arg(long = "use-git-for-empty-sync", visible_alias = "git-empty-sync")]
        use_git_for_empty_sync: Option<bool>,
    },

    /// Default maximum depth for `lime deps` when `--depth` is omitted.
    ///
    /// Examples:
    ///   "lime config dependency-depth"           show current value
    ///   "lime config dependency-depth --depth 4" set value
    DependencyDepth {
        /// Default hops for `lime deps` when `--depth` is omitted (≥ 0).
        #[arg(long, value_name = "N")]
        depth: Option<usize>,
    },

    /// Add or remove extra ignore patterns (in addition to `.gitignore`).
    ///
    /// Required defaults (`.git/`, `node_modules/`, etc.) are re-applied on load; you can remove
    /// user-added entries with `--remove`.
    ///
    /// Examples:
    ///   "lime config ignores"                       show patterns
    ///   "lime config ignores --add dist/"
    ///   "lime config ignores --add build/ --remove old_tmp/"
    Ignores {
        /// Append a glob-style ignore pattern (repeatable).
        #[arg(long)]
        add: Vec<String>,
        /// Remove a pattern if present (exact match; repeatable).
        #[arg(long)]
        remove: Vec<String>,
    },

    /// Set or show the index JSON path (`index_storage` in config).
    ///
    /// Examples:
    ///   "lime config index-storage"                    show current path
    ///   "lime config index-storage --path .lime/index.json"
    IndexStorage {
        /// Relative or absolute path for `index.json` (stored as `index_storage` in config).
        #[arg(long, value_name = "PATH")]
        path: Option<String>,
    },
}

fn handle_registry(cwd: PathBuf, action: RegistryAction) -> Result<Value> {
    match action {
        RegistryAction::List => {
            let registry = projects_registry::load_registry()?;
            let path = projects_registry::registry_path()?;
            let projects: Vec<Value> = registry
                .projects
                .iter()
                .map(|(project_id, reg)| {
                    let project_root = PathBuf::from(&reg.root);
                    let languages: Vec<String> = LimeConfig::load_or_create(&project_root)
                        .and_then(|config| storage::load_index_or_empty(&project_root, &config))
                        .map(|index| index.languages)
                        .unwrap_or_default();
                    json!({
                        "project_id": project_id,
                        "root": reg.root,
                        "updated_at": reg.updated_at,
                        "languages": languages,
                    })
                })
                .collect();
            Ok(json!({
                "ok": true,
                "command": "registry",
                "action": "list",
                "registry_path": path.display().to_string(),
                "count": projects.len(),
                "projects": projects,
            }))
        }
        RegistryAction::Add { id, path } => {
            let path = path.unwrap_or(cwd);
            let (project_id, canonical_root) =
                projects_registry::register_project(id.as_deref(), &path)?;
            let registry_path = projects_registry::registry_path()?;
            Ok(json!({
                "ok": true,
                "command": "registry",
                "action": "add",
                "registry_path": registry_path.display().to_string(),
                "project_id": project_id,
                "root": canonical_root.display().to_string(),
                "updated": true,
            }))
        }
        RegistryAction::Remove { project_id } => {
            let removed = projects_registry::unregister_project(&project_id)?;
            let registry_path = projects_registry::registry_path()?;
            Ok(json!({
                "ok": true,
                "command": "registry",
                "action": "remove",
                "registry_path": registry_path.display().to_string(),
                "project_id": project_id,
                "removed": removed,
            }))
        }
    }
}

fn handle_sum(root: PathBuf, top_links: usize) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;
    let index_staleness =
        serde_json::to_value(git_staleness::evaluate(&root, &index)).unwrap_or(json!({}));

    let all_annotations = annotations::list_annotations(&root).unwrap_or_default();
    let diag_cache = storage::load_diagnostics_cache(&root).unwrap_or_default();

    let top = top_links.clamp(1, 256);
    let merged_links = links::merged_link_paths_by_component(&root, &index, &all_annotations);
    let mut link_counts: HashMap<String, usize> = HashMap::new();
    for paths in merged_links.values() {
        for p in paths {
            let key = p.to_ascii_lowercase();
            *link_counts.entry(key).or_insert(0) += 1;
        }
    }
    let mut pairs: Vec<(String, usize)> = link_counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    pairs.truncate(top);
    let links_top: Vec<Value> = pairs
        .into_iter()
        .map(|(link, count)| json!({ "link": link, "count": count }))
        .collect();

    let cache_entry_total: usize = diag_cache.values().map(|v| v.len()).sum();
    let indexed_with_faults = index
        .components
        .iter()
        .filter(|c| c.faults.total() > 0)
        .count();

    let elapsed = timer.elapsed();
    Ok(json!({
        "ok": true,
        "command": "sum",
        "elapsed_secs": elapsed.as_secs_f64(),
        "languages": index.languages,
        "component_count": index.components.len(),
        "file_count": index.files.len(),
        "annotation_count": all_annotations.len(),
        "links_top": links_top,
        "index_staleness": index_staleness,
        "diagnostics_summary": {
            "cache_component_count": diag_cache.len(),
            "cache_entry_total": cache_entry_total,
            "indexed_components_with_faults": indexed_with_faults,
        },
        "index_pretty": config.index_pretty,
    }))
}

/// Per-sync diff for git partial (and agents): new vs re-indexed files, component IDs added/removed.
fn sync_delta_json(
    ids_before: &HashSet<String>,
    paths_before: &HashSet<String>,
    after: &IndexData,
    result: &FileUpdateResult,
) -> Value {
    let ids_after: HashSet<String> = after.components.iter().map(|c| c.id.clone()).collect();
    let mut components_added: Vec<String> = ids_after.difference(ids_before).cloned().collect();
    let mut components_removed: Vec<String> = ids_before.difference(&ids_after).cloned().collect();
    components_added.sort();
    components_removed.sort();
    let mut files_new_to_index: Vec<String> = Vec::new();
    let mut files_reindexed: Vec<String> = Vec::new();
    for p in &result.indexed {
        if paths_before.contains(p) {
            files_reindexed.push(p.clone());
        } else {
            files_new_to_index.push(p.clone());
        }
    }
    files_new_to_index.sort();
    files_reindexed.sort();

    json!({
        "components_added": components_added,
        "components_removed": components_removed,
        "components_added_count": components_added.len(),
        "components_removed_count": components_removed.len(),
        "files_new_to_index": files_new_to_index,
        "files_reindexed": files_reindexed,
        "files_touched_count": result.indexed.len(),
    })
}

fn handle_sync(
    root: PathBuf,
    files: Vec<String>,
    run_diagnostics: bool,
    verbose: bool,
    git: bool,
    no_git: bool,
) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let do_diag = run_diagnostics || config.diagnostics.enabled;

    if !files.is_empty() {
        let current = storage::load_index_or_empty(&root, &config)?;
        let timer = std::time::Instant::now();
        let (mut updated, result) = index::sync_files(&root, &config, &files, current)?;

        let diag_result = run_and_attach_diagnostics(&root, &mut updated, do_diag);

        let elapsed = timer.elapsed();
        git_staleness::refresh_git_head_at_sync(&mut updated, &root);
        storage::save_index(&root, &config, &updated)?;
        persist_token_index(&root, &updated);

        let mut response = json!({
            "ok": true,
            "command": "sync",
            "scope": "partial",
            "sync_mode": "partial",
            "elapsed_secs": elapsed.as_secs_f64(),
            "request": {
                "files": files
            },
            "result": result,
            "index": index_payload_with_staleness(&root, &updated)
        });
        if verbose {
            if let Some(obj) = response.as_object_mut() {
                obj.insert("verbose".into(), json!(true));
            }
        }
        response["diagnostics"] = diag_result;
        return Ok(response);
    }

    let mut use_git_partial = if git {
        true
    } else if no_git {
        false
    } else {
        config.git_partial_sync.empty_sync_uses_git
    };

    // Git-partial only applies once the index has components; otherwise a dirty-path-only pass
    // would miss most of the tree on the first run.
    let mut loaded_for_git: Option<IndexData> = None;
    let mut git_partial_skip_reason: Option<&'static str> = None;
    if use_git_partial {
        let current = storage::load_index_or_empty(&root, &config)?;
        if current.components.is_empty() {
            use_git_partial = false;
            git_partial_skip_reason = Some("empty_index");
        } else {
            loaded_for_git = Some(current);
        }
    }

    if !use_git_partial {
        let timer = std::time::Instant::now();
        let mut index = index::rebuild_index(&root, &config)?;

        let diag_result = run_and_attach_diagnostics(&root, &mut index, do_diag);

        let elapsed = timer.elapsed();
        git_staleness::refresh_git_head_at_sync(&mut index, &root);
        storage::save_index(&root, &config, &index)?;
        persist_token_index(&root, &index);

        let mut response = json!({
            "ok": true,
            "command": "sync",
            "scope": "full",
            "sync_mode": "full",
            "elapsed_secs": elapsed.as_secs_f64(),
            "index": index_payload_with_staleness(&root, &index)
        });
        if let Some(reason) = git_partial_skip_reason {
            if let Some(obj) = response.as_object_mut() {
                obj.insert("git_partial_skip_reason".into(), json!(reason));
            }
        }
        if verbose {
            if let Some(obj) = response.as_object_mut() {
                obj.insert("verbose".into(), json!(true));
            }
        }
        response["diagnostics"] = diag_result;
        return Ok(response);
    }

    let timer = std::time::Instant::now();

    if !git_staleness::is_inside_git_work_tree(&root) {
        let mut index = index::rebuild_index(&root, &config)?;
        let diag_result = run_and_attach_diagnostics(&root, &mut index, do_diag);
        let elapsed = timer.elapsed();
        git_staleness::refresh_git_head_at_sync(&mut index, &root);
        storage::save_index(&root, &config, &index)?;
        persist_token_index(&root, &index);

        let mut response = json!({
            "ok": true,
            "command": "sync",
            "scope": "full",
            "sync_mode": "full",
            "git_partial_fallback": "not_a_git_repository",
            "elapsed_secs": elapsed.as_secs_f64(),
            "index": index_payload_with_staleness(&root, &index)
        });
        if verbose {
            if let Some(obj) = response.as_object_mut() {
                obj.insert("verbose".into(), json!(true));
            }
        }
        response["diagnostics"] = diag_result;
        return Ok(response);
    }

    let current = loaded_for_git.expect("loaded when git partial path is active");
    let dirty = match git_staleness::worktree_changed_paths(&root) {
        Ok(d) => d,
        Err(e) => {
            let mut index = index::rebuild_index(&root, &config)?;
            let diag_result = run_and_attach_diagnostics(&root, &mut index, do_diag);
            let elapsed = timer.elapsed();
            git_staleness::refresh_git_head_at_sync(&mut index, &root);
            storage::save_index(&root, &config, &index)?;
            persist_token_index(&root, &index);

            let mut response = json!({
                "ok": true,
                "command": "sync",
                "scope": "full",
                "sync_mode": "full",
                "git_partial_fallback": "git_status_failed",
                "git_partial_error": e,
                "elapsed_secs": elapsed.as_secs_f64(),
                "index": index_payload_with_staleness(&root, &index)
            });
            if verbose {
                if let Some(obj) = response.as_object_mut() {
                    obj.insert("verbose".into(), json!(true));
                }
            }
            response["diagnostics"] = diag_result;
            return Ok(response);
        }
    };

    let candidate_count = dirty.len();
    let to_sync = index::filter_worktree_paths_for_sync(&root, &config, &current, &dirty)?;

    if to_sync.is_empty() {
        let mut index = current;
        let diag_result = if do_diag {
            json!({
                "enabled": true,
                "status": "skipped",
                "reason": "git_partial_noop_no_indexable_dirty_paths"
            })
        } else {
            json!({ "enabled": false, "status": "skipped" })
        };
        git_staleness::refresh_git_head_at_sync(&mut index, &root);
        storage::save_index(&root, &config, &index)?;
        persist_token_index(&root, &index);
        let elapsed = timer.elapsed();

        let mut response = json!({
            "ok": true,
            "command": "sync",
            "scope": "noop",
            "sync_mode": "noop",
            "elapsed_secs": elapsed.as_secs_f64(),
            "git_partial": {
                "candidates": candidate_count,
                "sync_paths": [],
            },
            "index": index_payload_with_staleness(&root, &index)
        });
        if verbose {
            if let Some(obj) = response.as_object_mut() {
                obj.insert("verbose".into(), json!(true));
            }
        }
        response["diagnostics"] = diag_result;
        return Ok(response);
    }

    let ids_before: HashSet<String> = current.components.iter().map(|c| c.id.clone()).collect();
    let paths_before: HashSet<String> = current.files.iter().map(|f| f.path.clone()).collect();
    let (mut updated, result) = index::sync_files(&root, &config, &to_sync, current)?;
    let sync_delta = sync_delta_json(&ids_before, &paths_before, &updated, &result);
    let diag_result = run_and_attach_diagnostics(&root, &mut updated, do_diag);
    let elapsed = timer.elapsed();
    git_staleness::refresh_git_head_at_sync(&mut updated, &root);
    storage::save_index(&root, &config, &updated)?;
    persist_token_index(&root, &updated);

    let mut response = json!({
        "ok": true,
        "command": "sync",
        "scope": "partial",
        "sync_mode": "git_partial",
        "elapsed_secs": elapsed.as_secs_f64(),
        "git_partial": {
            "candidates": candidate_count,
            "sync_paths": to_sync,
        },
        "sync_delta": sync_delta,
        "result": result,
        "index": index_payload_with_staleness(&root, &updated)
    });
    if verbose {
        if let Some(obj) = response.as_object_mut() {
            obj.insert("verbose".into(), json!(true));
        }
    }
    response["diagnostics"] = diag_result;
    Ok(response)
}

fn run_and_attach_diagnostics(
    root: &std::path::Path,
    index: &mut IndexData,
    enabled: bool,
) -> Value {
    if !enabled {
        let _ = storage::save_diagnostics_cache(root, &std::collections::HashMap::new());
        return json!({ "enabled": false, "status": "skipped" });
    }

    let results = diagnostics::run_diagnostics(root, index);
    let faults_map = diagnostics::map_diagnostics_to_components(root, &results, &index.components);
    let entries_map =
        diagnostics::build_component_diagnostics_map(root, &results, &index.components);

    for component in &mut index.components {
        if let Some(faults) = faults_map.get(&component.id) {
            component.faults = faults.clone();
        }
    }

    let _ = storage::save_diagnostics_cache(root, &entries_map);

    let total_errors: usize = results
        .iter()
        .flat_map(|r| &r.entries)
        .filter(|e| e.severity == diagnostics::DiagSeverity::Error)
        .count();
    let total_warnings: usize = results
        .iter()
        .flat_map(|r| &r.entries)
        .filter(|e| e.severity == diagnostics::DiagSeverity::Warning)
        .count();
    let faulty = faults_map.len();

    let analyzer_info: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "language": r.language,
                "tool": r.tool,
                "tool_found": r.tool_found,
                "tool_failed": r.tool_failed,
                "entry_count": r.entries.len(),
            })
        })
        .collect();

    json!({
        "enabled": true,
        "status": "ok",
        "total_errors": total_errors,
        "total_warnings": total_warnings,
        "faulty_components": faulty,
        "analyzers": analyzer_info,
    })
}

fn handle_add(root: PathBuf, filename: String) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let current = storage::load_index_or_empty(&root, &config)?;
    let timer = std::time::Instant::now();
    let (mut updated, result) = index::add_file(&root, &config, &filename, current)?;
    let elapsed = timer.elapsed();
    git_staleness::refresh_git_head_at_sync(&mut updated, &root);
    storage::save_index(&root, &config, &updated)?;
    persist_token_index(&root, &updated);

    Ok(json!({
        "ok": true,
        "command": "add",
        "elapsed_secs": elapsed.as_secs_f64(),
        "request": {
            "filename": filename
        },
        "result": result,
        "index": index_payload_with_staleness(&root, &updated)
    }))
}

fn handle_remove(root: PathBuf, filename: String) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let current = storage::load_index_or_empty(&root, &config)?;
    let timer = std::time::Instant::now();
    let (mut updated, removed) = index::remove_file(&root, &config, &filename, current)?;
    let elapsed = timer.elapsed();
    if removed {
        git_staleness::refresh_git_head_at_sync(&mut updated, &root);
        storage::save_index(&root, &config, &updated)?;
        persist_token_index(&root, &updated);
    }

    Ok(json!({
        "ok": true,
        "command": "remove",
        "elapsed_secs": elapsed.as_secs_f64(),
        "request": {
            "filename": filename
        },
        "removed": removed,
        "index": index_payload_with_staleness(&root, &updated)
    }))
}

fn handle_config(root: PathBuf, action: ConfigAction, global: bool) -> Result<Value> {
    // Helper: load either global or project config.
    let load = |root: &std::path::Path| -> Result<LimeConfig> {
        if global {
            LimeConfig::load_global()
        } else {
            LimeConfig::load_or_create(root)
        }
    };
    // Helper: save either global or project config; returns the path string for output.
    let save = |cfg: &LimeConfig, root: &std::path::Path| -> Result<String> {
        if global {
            cfg.save_global()?;
            Ok(LimeConfig::global_config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unknown)".to_string()))
        } else {
            cfg.save(root)?;
            Ok(LimeConfig::config_path(root).display().to_string())
        }
    };

    match action {
        ConfigAction::Show => {
            let config = load(&root)?;
            let config_path = if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            Ok(json!({
                "ok": true,
                "command": "config_show",
                "global": global,
                "config_path": config_path,
                "config": config,
            }))
        }
        ConfigAction::Diagnostics { enabled, timeout } => {
            let mut config = load(&root)?;
            let changed = enabled.is_some() || timeout.is_some();
            if let Some(value) = enabled {
                config.diagnostics.enabled = value;
            }
            if let Some(value) = timeout {
                config.diagnostics.timeout_secs = value;
            }
            let config_path = if changed {
                save(&config, &root)?
            } else if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            let diag = config.diagnostics.clone();
            Ok(json!({
                "ok": true,
                "command": "config_diagnostics",
                "global": global,
                "config_path": config_path,
                "diagnostics": diag,
                "updated": changed,
            }))
        }
        ConfigAction::DeathSeeds {
            seed_files,
            seed_names,
            seed_types,
            clear_seed_files,
            clear_seed_names,
            clear_seed_types,
        } => {
            let mut config = load(&root)?;

            let changed = clear_seed_files
                || clear_seed_names
                || clear_seed_types
                || !seed_files.is_empty()
                || !seed_names.is_empty()
                || !seed_types.is_empty();

            if clear_seed_files {
                config.death_seeds.seed_files.clear();
            }
            if clear_seed_names {
                config.death_seeds.seed_names.clear();
            }
            if clear_seed_types {
                config.death_seeds.seed_types.clear();
            }

            if !seed_files.is_empty() {
                for pattern in seed_files {
                    if !config.death_seeds.seed_files.contains(&pattern) {
                        config.death_seeds.seed_files.push(pattern);
                    }
                }
            }
            if !seed_names.is_empty() {
                for name in seed_names {
                    if !config.death_seeds.seed_names.contains(&name) {
                        config.death_seeds.seed_names.push(name);
                    }
                }
            }
            if !seed_types.is_empty() {
                for t in seed_types {
                    if !config.death_seeds.seed_types.contains(&t) {
                        config.death_seeds.seed_types.push(t);
                    }
                }
            }

            let config_path = if changed {
                save(&config, &root)?
            } else if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            let seeds = config.death_seeds.clone();

            Ok(json!({
                "ok": true,
                "command": "config_death_seeds",
                "global": global,
                "config_path": config_path,
                "death_seeds": seeds,
                "updated": changed,
            }))
        }
        ConfigAction::Index { pretty } => {
            let mut config = load(&root)?;
            let updated = pretty.is_some();
            if let Some(value) = pretty {
                config.index_pretty = value;
                let _ = save(&config, &root)?;
            }
            let config_path = if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            Ok(json!({
                "ok": true,
                "command": "config_index",
                "global": global,
                "config_path": config_path,
                "index_pretty": config.index_pretty,
                "updated": updated,
            }))
        }
        ConfigAction::GitPartialSync {
            use_git_for_empty_sync,
        } => {
            let mut config = load(&root)?;
            let updated = use_git_for_empty_sync.is_some();
            if let Some(value) = use_git_for_empty_sync {
                config.git_partial_sync.empty_sync_uses_git = value;
                let _ = save(&config, &root)?;
            }
            let config_path = if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            Ok(json!({
                "ok": true,
                "command": "config_git_partial_sync",
                "global": global,
                "config_path": config_path,
                "empty_sync_uses_git": config.git_partial_sync.empty_sync_uses_git,
                "updated": updated,
            }))
        }
        ConfigAction::DependencyDepth { depth } => {
            let mut config = load(&root)?;
            let updated = depth.is_some();
            if let Some(d) = depth {
                config.default_dependency_depth = d;
                let _ = save(&config, &root)?;
            }
            let config_path = if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            Ok(json!({
                "ok": true,
                "command": "config_dependency_depth",
                "global": global,
                "config_path": config_path,
                "default_dependency_depth": config.default_dependency_depth,
                "updated": updated,
            }))
        }
        ConfigAction::Ignores { add, remove } => {
            let mut config = load(&root)?;
            let mut changed = false;
            for pattern in add {
                let p = pattern.trim().to_string();
                if p.is_empty() {
                    continue;
                }
                if !config.ignore_patterns.contains(&p) {
                    config.ignore_patterns.push(p);
                    changed = true;
                }
            }
            for pattern in remove {
                let p = pattern.trim();
                if p.is_empty() {
                    continue;
                }
                let before = config.ignore_patterns.len();
                config.ignore_patterns.retain(|x| x != p);
                if config.ignore_patterns.len() != before {
                    changed = true;
                }
            }
            config.ensure_default_ignores();
            let config_path = if changed {
                save(&config, &root)?
            } else if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            Ok(json!({
                "ok": true,
                "command": "config_ignores",
                "global": global,
                "config_path": config_path,
                "ignore_patterns": config.ignore_patterns,
                "updated": changed,
            }))
        }
        ConfigAction::IndexStorage { path } => {
            let mut config = load(&root)?;
            let updated = path.is_some();
            if let Some(p) = path {
                let trimmed = p.trim();
                if trimmed.is_empty() {
                    bail!("index-storage --path must not be empty");
                }
                config.index_storage = trimmed.to_string();
                let _ = save(&config, &root)?;
            }
            let config_path = if global {
                LimeConfig::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string())
            } else {
                LimeConfig::config_path(&root).display().to_string()
            };
            Ok(json!({
                "ok": true,
                "command": "config_index_storage",
                "global": global,
                "config_path": config_path,
                "index_storage": config.index_storage,
                "updated": updated,
            }))
        }
    }
}

fn handle_search(root: PathBuf, terms: Vec<String>, fuzzy: bool) -> Result<Value> {
    if terms.is_empty() {
        bail!("search requires at least one argument");
    }

    let query = parse_search_terms(terms)?;
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let mut index = storage::load_index_or_empty(&root, &config)?;
    let index_staleness =
        serde_json::to_value(git_staleness::evaluate(&root, &index)).unwrap_or(json!({}));

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
            "results": results,
            "index_staleness": index_staleness
        }));
    }

    let exact_ids: HashSet<String> = results.iter().map(|c| c.id.clone()).collect();

    let all_annotations = annotations::list_annotations(&root).unwrap_or_default();
    let link_map = links::merged_link_paths_by_component(&root, &index, &all_annotations);
    let token_index = storage::load_token_index(&root)?
        .unwrap_or_else(|| search::build_token_index(&index, &all_annotations, &link_map));
    let fuzzy_hits = if fuzzy {
        search::fuzzy_search(&token_index, &query.query)
    } else {
        Vec::new()
    };

    let mut annotation_map: std::collections::HashMap<String, &annotations::Annotation> =
        std::collections::HashMap::new();
    for a in &all_annotations {
        if let Some(c) = annotations::resolve_component_for_annotation(&index, a) {
            annotation_map.insert(c.id.clone(), a);
        }
    }

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

    let all_extra_hits: Vec<&search::SearchHit> = fuzzy_hits.iter().collect();

    for hit in all_extra_hits {
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
        "fuzzy": fuzzy,
        "result_count": results_json.len(),
        "results": results_json,
        "index_staleness": index_staleness
    }))
}

fn canonical_link_path_for_match(raw: &str) -> Option<String> {
    links::validate_link_path(raw)
        .ok()
        .or_else(|| links::normalize_link_path_for_merge(raw))
}

fn annotation_links_match_query(annotation: &annotations::Annotation, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    annotation.links.iter().any(|l| {
        let t = l.trim();
        if t.is_empty() {
            return false;
        }
        canonical_link_path_for_match(t)
            .map(|p| links::path_matches_link_query(&p, q))
            .unwrap_or_else(|| t.eq_ignore_ascii_case(q))
    })
}

fn link_path_display_depth(path: &str) -> usize {
    path.matches('/').count()
}

#[derive(Debug, Clone)]
struct LinkResultMeta {
    match_source: &'static str,
    peer_project_id: Option<String>,
    peer_repo_root: Option<String>,
    matched_local_paths: Option<Vec<String>>,
}

fn json_for_link_result(
    comp: &ComponentRecord,
    merged_paths: &[String],
    ann: Option<&annotations::Annotation>,
    notes: bool,
    meta: Option<&LinkResultMeta>,
) -> Value {
    let tags = ann.map(|a| a.tags.clone()).unwrap_or_default();
    let links_json: Vec<&str> = merged_paths.iter().map(String::as_str).collect();
    let mut base = if notes {
        if let Some(a) = ann {
            json!({
                "component": comp,
                "links": links_json,
                "tags": tags,
                "annotation": {
                    "hash_id": a.hash_id,
                    "content": a.content,
                    "tags": a.tags,
                    "links": a.links,
                    "created_at": a.created_at,
                    "updated_at": a.updated_at,
                }
            })
        } else {
            json!({
                "component": comp,
                "links": links_json,
                "tags": tags,
                "annotation": Value::Null,
            })
        }
    } else {
        let preview = ann
            .map(|a| truncate_preview(&a.content, 120))
            .unwrap_or_default();
        json!({
            "component": comp,
            "links": links_json,
            "tags": tags,
            "annotation_preview": preview,
        })
    };
    if let Some(m) = meta {
        if let Value::Object(ref mut map) = base {
            map.insert("match_source".into(), json!(m.match_source));
            if let Some(ref pid) = m.peer_project_id {
                map.insert("peer_project_id".into(), json!(pid));
            }
            if let Some(ref pr) = m.peer_repo_root {
                map.insert("peer_repo_root".into(), json!(pr));
            }
            if let Some(ref mp) = m.matched_local_paths {
                map.insert("matched_local_paths".into(), json!(mp));
            }
        }
    }
    base
}

fn handle_links(root: PathBuf, action: LinksAction) -> Result<Value> {
    match action {
        LinksAction::Show {
            path,
            notes,
            peer_resolve,
        } => handle_link(root, path, notes, peer_resolve),
        LinksAction::Add { component_id, path } => {
            let config = LimeConfig::load_or_create(&root)?;
            let timer = std::time::Instant::now();
            let index = storage::load_index_or_empty(&root, &config)?;
            index
                .components
                .iter()
                .find(|c| c.id == component_id)
                .ok_or_else(|| anyhow!("component not found: {component_id}"))?;
            links::add_membership(&root, &component_id, &path)?;
            persist_token_index(&root, &index);
            let mut out = json!({
                "ok": true,
                "command": "links",
                "action": "add",
                "component_id": component_id,
                "path": path.trim(),
            });
            if let Some(obj) = out.as_object_mut() {
                obj.insert("elapsed_secs".into(), json!(timer.elapsed().as_secs_f64()));
            }
            Ok(out)
        }
        LinksAction::Remove { component_id, path } => {
            let config = LimeConfig::load_or_create(&root)?;
            let timer = std::time::Instant::now();
            let index = storage::load_index_or_empty(&root, &config)?;
            let removed = links::remove_membership(&root, &component_id, &path)?;
            persist_token_index(&root, &index);
            let mut out = json!({
                "ok": true,
                "command": "links",
                "action": "remove",
                "component_id": component_id,
                "path": path.trim(),
                "removed": removed,
            });
            if let Some(obj) = out.as_object_mut() {
                obj.insert("elapsed_secs".into(), json!(timer.elapsed().as_secs_f64()));
            }
            Ok(out)
        }
        LinksAction::List { prefix, tree } => {
            let config = LimeConfig::load_or_create(&root)?;
            let timer = std::time::Instant::now();
            let index = storage::load_index_or_empty(&root, &config)?;
            let all_annotations = annotations::list_annotations(&root).unwrap_or_default();
            let paths =
                links::distinct_merged_paths(&root, &index, &all_annotations, prefix.as_deref());
            let mut out = json!({
                "ok": true,
                "command": "links",
                "action": "list",
                "tree": tree,
                "prefix": prefix,
                "path_count": paths.len(),
                "paths": paths,
            });
            if let Some(obj) = out.as_object_mut() {
                obj.insert("elapsed_secs".into(), json!(timer.elapsed().as_secs_f64()));
            }
            Ok(out)
        }
        LinksAction::Compact => {
            let config = LimeConfig::load_or_create(&root)?;
            let timer = std::time::Instant::now();
            let index = storage::load_index_or_empty(&root, &config)?;
            let updated = links::compact_annotation_links(&root, &index)?;
            persist_token_index(&root, &index);
            let mut out = json!({
                "ok": true,
                "command": "links",
                "action": "compact",
                "annotations_updated": updated,
            });
            if let Some(obj) = out.as_object_mut() {
                obj.insert("elapsed_secs".into(), json!(timer.elapsed().as_secs_f64()));
            }
            Ok(out)
        }
    }
}

fn handle_link(root: PathBuf, label: String, notes: bool, peer_resolve: bool) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;
    let index_staleness =
        serde_json::to_value(git_staleness::evaluate(&root, &index)).unwrap_or(json!({}));

    let q = label.trim();
    if q.is_empty() {
        bail!("link path query must not be empty");
    }

    if peer_resolve && !links::link_query_is_scoped(q) {
        bail!("--peer-resolve requires a scoped query like @myapp/auth/login (registered project id + topic)");
    }

    let all_annotations = annotations::list_annotations(&root)?;
    let merged = links::merged_link_paths_by_component(&root, &index, &all_annotations);
    let catalog = links::load_link_catalog(&root).unwrap_or_default();

    let component_by_id: HashMap<&str, &ComponentRecord> = index
        .components
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    let mut ann_by_id: HashMap<&str, &annotations::Annotation> = HashMap::new();
    for ann in &all_annotations {
        if let Some(comp) = annotations::resolve_component_for_annotation(&index, ann) {
            ann_by_id.insert(comp.id.as_str(), ann);
        }
    }

    let scoped_meta = if peer_resolve {
        Some(LinkResultMeta {
            match_source: "scoped",
            peer_project_id: None,
            peer_repo_root: None,
            matched_local_paths: None,
        })
    } else {
        None
    };

    let mut path_buckets: HashMap<String, HashSet<String>> = HashMap::new();
    for (cid, paths) in &merged {
        for p in paths {
            if links::path_matches_link_query(p, q) {
                path_buckets
                    .entry(p.clone())
                    .or_default()
                    .insert(cid.clone());
            }
        }
    }

    let mut sorted_paths: Vec<String> = path_buckets.keys().cloned().collect();
    sorted_paths = links::sort_paths_for_display(&sorted_paths, &catalog);

    let mut path_groups: Vec<Value> = Vec::new();
    let mut seen_result_ids: HashSet<String> = HashSet::new();
    let mut results: Vec<Value> = Vec::new();

    for path in &sorted_paths {
        let Some(cids) = path_buckets.get(path) else {
            continue;
        };
        let mut comps: Vec<&ComponentRecord> = cids
            .iter()
            .filter_map(|id| component_by_id.get(id.as_str()).copied())
            .collect();
        comps.sort_by(|a, b| {
            (
                a.file.as_str(),
                a.start_line,
                a.name.as_str(),
                a.id.as_str(),
            )
                .cmp(&(
                    b.file.as_str(),
                    b.start_line,
                    b.name.as_str(),
                    b.id.as_str(),
                ))
        });

        let group_components: Vec<Value> = comps
            .iter()
            .map(|comp| {
                let mpaths = merged.get(comp.id.as_str()).cloned().unwrap_or_default();
                json_for_link_result(
                    comp,
                    &mpaths,
                    ann_by_id.get(comp.id.as_str()).copied(),
                    notes,
                    scoped_meta.as_ref(),
                )
            })
            .collect();

        let mut pg = json!({
            "path": path,
            "depth": link_path_display_depth(path),
            "components": group_components,
        });
        if peer_resolve {
            if let Value::Object(ref mut m) = pg {
                m.insert("match_source".into(), json!("scoped"));
            }
        }
        path_groups.push(pg);

        for comp in comps {
            if seen_result_ids.insert(format!("scoped:{}", comp.id)) {
                let mpaths = merged.get(comp.id.as_str()).cloned().unwrap_or_default();
                results.push(json_for_link_result(
                    comp,
                    &mpaths,
                    ann_by_id.get(comp.id.as_str()).copied(),
                    notes,
                    scoped_meta.as_ref(),
                ));
            }
        }
    }

    if peer_resolve {
        if let Some((peer_id, tail)) = links::split_scoped_query(q) {
            let peer_root = projects_registry::resolve_project_root(&peer_id)?;
            let peer_root_str = peer_root.display().to_string();
            let peer_matches = links::peer_local_link_matches(&peer_root, &tail)?;
            let peer_config = LimeConfig::load_or_create(&peer_root)?;
            let peer_index = storage::load_index_or_empty(&peer_root, &peer_config)?;
            let peer_annotations = annotations::list_annotations(&peer_root).unwrap_or_default();
            let peer_merged =
                links::merged_link_paths_by_component(&peer_root, &peer_index, &peer_annotations);

            let mut peer_ann_by_id: HashMap<&str, &annotations::Annotation> = HashMap::new();
            for ann in &peer_annotations {
                if let Some(comp) = annotations::resolve_component_for_annotation(&peer_index, ann)
                {
                    peer_ann_by_id.insert(comp.id.as_str(), ann);
                }
            }

            let mut matched_by_id: HashMap<String, Vec<String>> = HashMap::new();
            for (comp, paths) in &peer_matches {
                matched_by_id.insert(comp.id.clone(), paths.clone());
            }

            let mut peer_buckets: HashMap<String, HashSet<String>> = HashMap::new();
            for (comp, matched_paths) in &peer_matches {
                for mp in matched_paths {
                    peer_buckets
                        .entry(mp.clone())
                        .or_default()
                        .insert(comp.id.clone());
                }
            }

            let mut peer_sorted_paths: Vec<String> = peer_buckets.keys().cloned().collect();
            peer_sorted_paths.sort_by_key(|a| a.to_ascii_lowercase());

            let peer_component_by_id: HashMap<&str, &ComponentRecord> = peer_index
                .components
                .iter()
                .map(|c| (c.id.as_str(), c))
                .collect();

            for path in &peer_sorted_paths {
                let Some(cids) = peer_buckets.get(path) else {
                    continue;
                };
                let mut comps: Vec<&ComponentRecord> = cids
                    .iter()
                    .filter_map(|id| peer_component_by_id.get(id.as_str()).copied())
                    .collect();
                comps.sort_by(|a, b| {
                    (
                        a.file.as_str(),
                        a.start_line,
                        a.name.as_str(),
                        a.id.as_str(),
                    )
                        .cmp(&(
                            b.file.as_str(),
                            b.start_line,
                            b.name.as_str(),
                            b.id.as_str(),
                        ))
                });

                let group_components: Vec<Value> = comps
                    .iter()
                    .map(|comp| {
                        let mpaths = peer_merged
                            .get(comp.id.as_str())
                            .cloned()
                            .unwrap_or_default();
                        let matched_local = matched_by_id
                            .get(comp.id.as_str())
                            .cloned()
                            .unwrap_or_default();
                        let meta = LinkResultMeta {
                            match_source: "peer_local",
                            peer_project_id: Some(peer_id.clone()),
                            peer_repo_root: Some(peer_root_str.clone()),
                            matched_local_paths: Some(matched_local),
                        };
                        json_for_link_result(
                            comp,
                            &mpaths,
                            peer_ann_by_id.get(comp.id.as_str()).copied(),
                            notes,
                            Some(&meta),
                        )
                    })
                    .collect();

                path_groups.push(json!({
                    "path": path,
                    "depth": link_path_display_depth(path),
                    "match_source": "peer_local",
                    "peer_project_id": peer_id,
                    "peer_repo_root": peer_root_str,
                    "components": group_components,
                }));

                for comp in comps {
                    if seen_result_ids.insert(format!("peer_local:{}", comp.id)) {
                        let mpaths = peer_merged
                            .get(comp.id.as_str())
                            .cloned()
                            .unwrap_or_default();
                        let matched_local = matched_by_id
                            .get(comp.id.as_str())
                            .cloned()
                            .unwrap_or_default();
                        let meta = LinkResultMeta {
                            match_source: "peer_local",
                            peer_project_id: Some(peer_id.clone()),
                            peer_repo_root: Some(peer_root_str.clone()),
                            matched_local_paths: Some(matched_local),
                        };
                        results.push(json_for_link_result(
                            comp,
                            &mpaths,
                            peer_ann_by_id.get(comp.id.as_str()).copied(),
                            notes,
                            Some(&meta),
                        ));
                    }
                }
            }
        }
    }

    results.sort_by(|a, b| {
        let ca = a.get("component").and_then(Value::as_object);
        let cb = b.get("component").and_then(Value::as_object);
        let fa = ca
            .and_then(|o| o.get("file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let fb = cb
            .and_then(|o| o.get("file"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let la = ca
            .and_then(|o| o.get("start_line"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let lb = cb
            .and_then(|o| o.get("start_line"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        (fa, la).cmp(&(fb, lb))
    });

    let store = links::load_component_links(&root).unwrap_or_default();
    let mut orphan_memberships: Vec<Value> = Vec::new();
    for (cid, paths) in &store.memberships {
        if component_by_id.contains_key(cid.as_str()) {
            continue;
        }
        if !paths.iter().any(|p| links::path_matches_link_query(p, q)) {
            continue;
        }
        orphan_memberships.push(json!({
            "kind": "link_store",
            "component_id": cid,
            "paths": paths,
        }));
    }

    let mut orphans: Vec<Value> = Vec::new();
    for ann in &all_annotations {
        if !annotation_links_match_query(ann, q) {
            continue;
        }
        if annotations::resolve_component_for_annotation(&index, ann).is_some() {
            continue;
        }
        orphans.push(json!({
            "kind": "annotation",
            "hash_id": ann.hash_id,
            "component_name": ann.component_name,
            "component_type": ann.component_type,
            "file": ann.file,
            "language": ann.language,
            "links": ann.links,
            "tags": ann.tags,
            "annotation_preview": truncate_preview(&ann.content, 120),
        }));
    }

    let elapsed = timer.elapsed();
    let orphan_count = orphans.len() + orphan_memberships.len();
    Ok(json!({
        "ok": true,
        "command": "links",
        "action": "show",
        "link": q,
        "notes": notes,
        "peer_resolve": peer_resolve,
        "result_count": results.len(),
        "results": results,
        "path_groups": path_groups,
        "orphan_count": orphan_count,
        "orphans": orphans,
        "orphan_memberships": orphan_memberships,
        "elapsed_secs": elapsed.as_secs_f64(),
        "index_staleness": index_staleness
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
    dead: bool,
    fault: bool,
) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;
    let index_staleness =
        serde_json::to_value(git_staleness::evaluate(&root, &index)).unwrap_or(json!({}));

    if matches!(component_type.as_deref(), Some("-all")) {
        all = true;
        component_type = None;
    }

    if all && component_type.is_some() {
        bail!("cannot combine component type filter with --all")
    }

    // If a filter is requested, default to listing mode rather than summary mode.
    if (dead || fault) && !all && component_type.is_none() {
        all = true;
    }

    let Some(language) = language else {
        let elapsed = timer.elapsed();
        return Ok(json!({
            "ok": true,
            "command": "list",
            "mode": "languages",
            "elapsed_secs": elapsed.as_secs_f64(),
            "languages": index.languages,
            "index_staleness": index_staleness
        }));
    };

    let language = normalize_language(&language)?;
    let mut components: Vec<&ComponentRecord> = index
        .components
        .iter()
        .filter(|component| component.language == language)
        .collect();

    if dead || fault {
        components.retain(|c| {
            let is_dead = c.death_status.is_dead();
            let is_faulty = c.faults.total() > 0;
            match (dead, fault) {
                (true, true) => is_dead || is_faulty,
                (true, false) => is_dead,
                (false, true) => is_faulty,
                (false, false) => true,
            }
        });
    }

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
            "components": components,
            "index_staleness": index_staleness
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
            "components": components,
            "index_staleness": index_staleness
        }));
    }

    let dead_count = components
        .iter()
        .filter(|c| c.death_status.is_dead())
        .count();
    let faulty_count = components.iter().filter(|c| c.faults.total() > 0).count();

    let mut by_type = BTreeMap::<String, usize>::new();
    for component in &components {
        *by_type.entry(component.component_type.clone()).or_insert(0) += 1;
    }

    let elapsed = timer.elapsed();
    Ok(json!({
        "ok": true,
        "command": "list",
        "mode": "language_summary",
        "elapsed_secs": elapsed.as_secs_f64(),
        "language": language,
        "total": components.len(),
        "dead": dead_count,
        "faulty": faulty_count,
        "component_counts": by_type,
        "index_staleness": index_staleness
    }))
}

fn handle_show(root: PathBuf, component_id: String) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let index = storage::load_index_or_empty(&root, &config)?;
    let index_staleness =
        serde_json::to_value(git_staleness::evaluate(&root, &index)).unwrap_or(json!({}));

    let component = index
        .components
        .iter()
        .find(|c| c.id == component_id)
        .ok_or_else(|| anyhow!("component not found: {component_id}"))?
        .clone();

    let file_path = root.join(&component.file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("failed reading source file: {}", file_path.display()))?;

    let file_hash_current = {
        let mut hasher = blake3::Hasher::new();
        hasher.update(source.as_bytes());
        hasher.finalize().to_hex().to_string()
    };
    let stored_hash = index
        .files
        .iter()
        .find(|f| f.path == component.file)
        .map(|f| f.file_hash.clone())
        .unwrap_or_default();
    let file_changed = !stored_hash.is_empty() && stored_hash != file_hash_current;

    let lines: Vec<&str> = source.lines().collect();
    let start = component.start_line.saturating_sub(1);
    let end = component.end_line.min(lines.len());
    let component_lines: Vec<&str> = lines[start..end].to_vec();

    let diag_cache = storage::load_diagnostics_cache(&root).unwrap_or_default();
    let diag_entries = diag_cache.get(&component_id).cloned().unwrap_or_default();

    let annotation = annotations::find_annotation_for_component(&root, &index, &component)?;

    let mut source_line_data: Vec<Value> = Vec::new();
    for (i, line_text) in component_lines.iter().enumerate() {
        let line_num = component.start_line + i;
        let line_diags: Vec<Value> = diag_entries
            .iter()
            .filter(|e| e.line == line_num)
            .map(|e| {
                json!({
                    "severity": match e.severity {
                        diagnostics::DiagSeverity::Error => "error",
                        diagnostics::DiagSeverity::Warning => "warning",
                        diagnostics::DiagSeverity::Note => "note",
                    },
                    "code": e.code,
                    "message": e.message,
                })
            })
            .collect();

        source_line_data.push(json!({
            "line": line_num,
            "code": line_text,
            "diagnostics": line_diags,
        }));
    }

    Ok(json!({
        "ok": true,
        "command": "show",
        "component": {
            "id": component.id,
            "language": component.language,
            "type": component.component_type,
            "name": component.name,
            "file": component.file,
            "start_line": component.start_line,
            "end_line": component.end_line,
            "death_status": component.death_status,
            "faults": component.faults,
        },
        "file_changed": file_changed,
        "source_lines": source_line_data,
        "annotation": annotation,
        "index_staleness": index_staleness
    }))
}

fn handle_deps(root: PathBuf, component_id: String, depth: Option<usize>) -> Result<Value> {
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;
    let index_staleness =
        serde_json::to_value(git_staleness::evaluate(&root, &index)).unwrap_or(json!({}));
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
        "dependency_matrix": matrix,
        "index_staleness": index_staleness
    }))
}

fn handle_annotate(target: CommandTarget, action: AnnotateAction) -> Result<Value> {
    let root = target.effective_root.clone();
    let config = LimeConfig::load_or_create(&root)?;
    let timer = std::time::Instant::now();
    let index = storage::load_index_or_empty(&root, &config)?;

    match action {
        AnnotateAction::Add {
            component_id,
            message,
            body_file,
            tags,
            links,
        } => {
            let component = index
                .components
                .iter()
                .find(|c| c.id == component_id)
                .ok_or_else(|| anyhow!("component not found: {component_id}"))?;

            let now = chrono::Utc::now().to_rfc3339();
            let existing = annotations::find_annotation_for_component(&root, &index, component)?;
            let created_at = existing
                .as_ref()
                .map(|a| a.created_at.clone())
                .unwrap_or_else(|| now.clone());
            let merged_tags = if tags.is_empty() {
                existing
                    .as_ref()
                    .map(|a| a.tags.clone())
                    .unwrap_or_default()
            } else {
                tags
            };
            let merged_links = if links.is_empty() {
                existing
                    .as_ref()
                    .map(|a| a.links.clone())
                    .unwrap_or_default()
            } else {
                links
            };

            for l in &merged_links {
                crate::links::validate_link_path(l)
                    .with_context(|| format!("invalid link path in annotation: {l}"))?;
            }

            let body_path = body_file.as_ref().map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    target.effective_root.join(p)
                }
            });

            let content = if let Some(msg) = message {
                msg
            } else if let Some(ref p) = body_path {
                fs::read_to_string(p).with_context(|| {
                    format!("failed reading annotation body file: {}", p.display())
                })?
            } else if let Some(ref e) = existing {
                e.content.clone()
            } else {
                bail!(
                    "annotate add requires --message, --file, or an existing annotation (to update links/tags only)"
                );
            };

            let (comp_type, _) = component_id
                .split_once('-')
                .unwrap_or(("component", &component_id));

            let annotation = annotations::Annotation {
                hash_id: component.id.clone(),
                component_type: comp_type.to_string(),
                component_name: component.name.clone(),
                file: Some(component.file.clone()),
                language: Some(component.language.clone()),
                content,
                tags: merged_tags,
                links: merged_links,
                created_at,
                updated_at: now,
            };

            annotations::save_annotation(&root, &annotation)?;
            for link in &annotation.links {
                if crate::links::validate_link_path(link).is_ok() {
                    let _ = crate::links::add_membership(&root, &component_id, link);
                }
            }
            persist_token_index(&root, &index);
            let elapsed = timer.elapsed();

            Ok(attach_target(
                json!({
                    "ok": true,
                    "command": "annotate",
                    "action": "add",
                    "elapsed_secs": elapsed.as_secs_f64(),
                    "component": component,
                    "annotation": {
                        "hash_id": annotation.hash_id,
                        "content": annotation.content,
                        "tags": annotation.tags,
                        "links": annotation.links,
                        "created_at": annotation.created_at,
                        "updated_at": annotation.updated_at
                    },
                    "external_write_policy": if target.is_foreign() { "annotation_and_links" } else { "local_full" }
                }),
                &target,
            ))
        }
        AnnotateAction::Show { component_id } => {
            let component = index
                .components
                .iter()
                .find(|c| c.id == component_id)
                .ok_or_else(|| anyhow!("component not found: {component_id}"))?;

            let annotation = annotations::find_annotation_for_component(&root, &index, component)?
                .ok_or_else(|| anyhow!("no annotation for component: {component_id}"))?;
            let elapsed = timer.elapsed();

            Ok(attach_target(
                json!({
                    "ok": true,
                    "command": "annotate",
                    "action": "show",
                    "elapsed_secs": elapsed.as_secs_f64(),
                    "component": component,
                    "annotation": {
                        "hash_id": annotation.hash_id,
                        "content": annotation.content,
                        "tags": annotation.tags,
                        "links": annotation.links,
                        "created_at": annotation.created_at,
                        "updated_at": annotation.updated_at
                    }
                }),
                &target,
            ))
        }
        AnnotateAction::List {
            language,
            component_type,
        } => {
            let language = match &language {
                Some(l) => Some(normalize_language(l)?),
                None => None,
            };
            let all_annotations = annotations::list_annotations(&root)?;

            let filtered: Vec<Value> = all_annotations
                .iter()
                .filter_map(|ann| {
                    let comp = annotations::resolve_component_for_annotation(&index, ann)?;
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
                            "tags": ann.tags,
                            "links": ann.links,
                            "created_at": ann.created_at,
                            "updated_at": ann.updated_at
                        }
                    }))
                })
                .collect();

            let elapsed = timer.elapsed();

            Ok(attach_target(
                json!({
                    "ok": true,
                    "command": "annotate",
                    "action": "list",
                    "elapsed_secs": elapsed.as_secs_f64(),
                    "count": filtered.len(),
                    "results": filtered
                }),
                &target,
            ))
        }
        AnnotateAction::Remove { component_id } => {
            if target.is_foreign() {
                bail!(
                    "annotate remove is not allowed with --external for foreign repositories (annotation add is permitted)"
                );
            }
            let removed = annotations::remove_annotation(&root, &component_id)?;
            if removed {
                persist_token_index(&root, &index);
            }
            let elapsed = timer.elapsed();

            Ok(attach_target(
                json!({
                    "ok": true,
                    "command": "annotate",
                    "action": "remove",
                    "elapsed_secs": elapsed.as_secs_f64(),
                    "component_id": component_id,
                    "removed": removed
                }),
                &target,
            ))
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
    let lower = value.trim().to_ascii_lowercase();
    let canonical: &str = match lower.as_str() {
        "js" => "javascript",
        "ts" => "typescript",
        "rs" => "rust",
        "py" => "python",
        other => other,
    };
    if supported_languages().contains(&canonical) {
        Ok(canonical.to_string())
    } else {
        bail!(
            "unsupported language `{value}`; supported: {} (aliases: rs→rust, py→python, js→javascript, ts→typescript)",
            supported_languages().join(", ")
        )
    }
}

fn supported_languages() -> Vec<&'static str> {
    vec![
        "rust",
        "javascript",
        "typescript",
        "python",
        "go",
        "zig",
        "c",
        "cpp",
        "swift",
    ]
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
            | "protocol"
            | "actor"
            | "extension"
            | "init"
            | "deinit"
            | "typealias"
            | "subscript"
    )
}

fn index_payload_with_staleness(root: &Path, index: &IndexData) -> Value {
    let mut summary = summarize_index(index);
    if let Some(obj) = summary.as_object_mut() {
        obj.insert(
            "index_staleness".to_string(),
            serde_json::to_value(git_staleness::evaluate(root, index)).unwrap_or(json!({})),
        );
    }
    summary
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

    let definitely_dead = index
        .components
        .iter()
        .filter(|c| c.death_status == DeathStatus::DefinitelyDead)
        .count();
    let probably_dead = index
        .components
        .iter()
        .filter(|c| c.death_status == DeathStatus::ProbablyDead)
        .count();
    let maybe_dead = index
        .components
        .iter()
        .filter(|c| c.death_status == DeathStatus::MaybeDead)
        .count();

    json!({
        "version": index.version,
        "generated_at": index.generated_at,
        "languages": index.languages,
        "file_count": index.files.len(),
        "component_count": index.components.len(),
        "batman_count": batman_count,
        "death_summary": {
            "definitely_dead": definitely_dead,
            "probably_dead": probably_dead,
            "maybe_dead": maybe_dead,
            "alive": index.components.len() - definitely_dead - probably_dead - maybe_dead
        },
        "component_breakdown": breakdown
    })
}

fn persist_token_index(root: &std::path::Path, index: &IndexData) {
    let all_annotations = annotations::reconcile_annotations_with_index(root, index)
        .unwrap_or_else(|_| annotations::list_annotations(root).unwrap_or_default());
    let link_map = links::merged_link_paths_by_component(root, index, &all_annotations);
    let token_index = search::build_token_index(index, &all_annotations, &link_map);
    let _ = storage::save_token_index(root, &token_index);
}

fn truncate_preview(content: &str, max_len: usize) -> String {
    let single_line = content.lines().next().unwrap_or("").trim();
    if single_line.len() <= max_len {
        single_line.to_string()
    } else {
        format!("{}...", &single_line[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn attach_target_marks_external_mode() {
        let payload = json!({"ok": true, "command": "show"});
        let target = CommandTarget {
            cwd: PathBuf::from("C:/repo-a"),
            effective_root: PathBuf::from("C:/repo-b"),
            project_id: Some("repo-b".to_string()),
        };
        let result = attach_target(payload, &target);
        assert_eq!(
            result
                .get("target")
                .and_then(|t| t.get("mode"))
                .and_then(Value::as_str),
            Some("external")
        );
        assert_eq!(
            result
                .get("target")
                .and_then(|t| t.get("project_id"))
                .and_then(Value::as_str),
            Some("repo-b")
        );
    }

    #[test]
    fn reject_external_for_forbidden_command() {
        let err = ensure_external_disabled("sync", Some("demo")).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("not supported"));
    }

    #[test]
    fn show_payload_can_be_tagged_external_target() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lime-external-show-{suffix}"));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src").join("demo.rs"),
            "pub fn external_demo() -> i32 { 42 }\n",
        )
        .unwrap();

        let config = LimeConfig::load_or_create(&root).unwrap();
        let index = index::rebuild_index(&root, &config).unwrap();
        storage::save_index(&root, &config, &index).unwrap();
        let component_id = index
            .components
            .iter()
            .find(|c| c.name == "external_demo")
            .map(|c| c.id.clone())
            .expect("component id must exist");

        let payload = handle_show(root.clone(), component_id).unwrap();
        let tagged = attach_target(
            payload,
            &CommandTarget {
                cwd: root.join("other-cwd"),
                effective_root: root.clone(),
                project_id: Some("demo".to_string()),
            },
        );
        assert_eq!(
            tagged
                .get("target")
                .and_then(|t| t.get("mode"))
                .and_then(Value::as_str),
            Some("external")
        );
        assert_eq!(
            tagged
                .get("component")
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str),
            Some("external_demo")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_language_aliases() {
        assert_eq!(normalize_language("rs").unwrap(), "rust");
        assert_eq!(normalize_language("RS").unwrap(), "rust");
        assert_eq!(normalize_language("py").unwrap(), "python");
        assert_eq!(normalize_language("PY").unwrap(), "python");
        assert_eq!(normalize_language("js").unwrap(), "javascript");
        assert_eq!(normalize_language("JS").unwrap(), "javascript");
        assert_eq!(normalize_language("ts").unwrap(), "typescript");
        assert_eq!(normalize_language("TS").unwrap(), "typescript");
        assert_eq!(normalize_language("rust").unwrap(), "rust");
        assert_eq!(normalize_language("python").unwrap(), "python");
        assert_eq!(normalize_language("javascript").unwrap(), "javascript");
        assert_eq!(normalize_language("typescript").unwrap(), "typescript");
    }

    #[test]
    fn parse_search_terms_accepts_language_aliases() {
        let q = parse_search_terms(vec!["js".into(), "run".into()]).unwrap();
        assert_eq!(q.language.as_deref(), Some("javascript"));
        assert_eq!(q.query, "run");
        let q = parse_search_terms(vec!["rs".into(), "main".into()]).unwrap();
        assert_eq!(q.language.as_deref(), Some("rust"));
        assert_eq!(q.query, "main");
    }

    #[test]
    fn git_partial_deferred_until_index_nonempty() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lime-git-partial-{suffix}"));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("a.rs"), "pub fn a() {}\n").unwrap();

        assert!(
            Command::new("git")
                .args(["init"])
                .current_dir(&root)
                .status()
                .expect("git")
                .success(),
            "git init failed — install git for this test"
        );
        assert!(Command::new("git")
            .args(["add", "."])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args([
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-m",
                "init",
            ])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());

        let mut config = LimeConfig::load_or_create(&root).unwrap();
        config.git_partial_sync.empty_sync_uses_git = true;
        config.save(&root).unwrap();

        let first = handle_sync(root.clone(), Vec::new(), false, false, false, false).unwrap();
        assert_eq!(first.get("sync_mode").and_then(Value::as_str), Some("full"));
        assert_eq!(
            first.get("git_partial_skip_reason").and_then(Value::as_str),
            Some("empty_index")
        );

        let second = handle_sync(root.clone(), Vec::new(), false, false, false, false).unwrap();
        assert_eq!(
            second.get("sync_mode").and_then(Value::as_str),
            Some("noop")
        );

        std::fs::write(root.join("src").join("a.rs"), "pub fn a() -> i32 { 42 }\n").unwrap();
        let third = handle_sync(root.clone(), Vec::new(), false, false, false, false).unwrap();
        assert_eq!(
            third.get("sync_mode").and_then(Value::as_str),
            Some("git_partial")
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
