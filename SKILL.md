---
name: lime
description: codebase indexation for most languages; high-level component retrieval, annotation, linking, and more.
---

Language-aware codebase index for AI agents. Indexes functions, structs, classes, and more. All commands support `--json` (global) for raw JSON output — always use it.

## Quick Start

```
lime --json sync                          # full index (default) or git partial if configured
lime --json search run                    # find components named "run"
lime --json list rust --all               # list all rust components with IDs
lime --json show fn-61bcc6dabec3f308      # show component source
lime --json deps fn-61bcc6dabec3f308      # dependency matrix
```

## Commands

### sync [files...] [--diagnostics] [-v] [--git | --no-git]
Rebuild index. **With file paths:** partial re-index of only those files (`--git` / `--no-git` ignored).

**With no file arguments:** behavior depends on config and flags:

| Resolution | Effect |
|------------|--------|
| `--git` | Partial sync on paths from `git status --porcelain` (working tree + untracked). |
| `--no-git` | Full rebuild (overrides `git_partial_sync.empty_sync_uses_git`). |
| Neither | If `git_partial_sync.empty_sync_uses_git` is **true**, same as `--git`; else **full** rebuild. |

If git mode is selected but the project is **not** a git repo (or `git status` fails), Lime **falls back to a full rebuild** and sets JSON `git_partial_fallback` (`not_a_git_repository` / `git_status_failed`). If the work tree is clean or every dirty path is ignored / non-indexable, JSON has `scope: "noop"`, `sync_mode: "noop"` (index refreshed; no file re-parse).

There is **no background watcher**—run `lime sync` when you want the index updated.

```bash
lime --json sync
lime --json sync --git
lime --json sync --no-git
lime --json sync src/main.rs src/lib.rs
lime --json sync --diagnostics            # attach static-analyzer faults
lime --json sync -v                       # verbose breakdown
```

**JSON (selected):** `sync_mode`: `full` | `git_partial` | `noop` | `partial`; optional `git_partial: { candidates, sync_paths }`, `git_partial_fallback`. When `sync_mode` is **`git_partial`**, responses include **`sync_delta`**: `components_added` / `components_removed` (IDs), counts, `files_new_to_index` vs `files_reindexed`, and `files_touched_count`. A content-only refresh can yield **zero net component IDs** while files still appear under `result.indexed` / `files_reindexed`—human output reports *“no additional components found”* for that case.

### add <filepath>
Add/refresh a single file. Returns `{ok, command:"add", elapsed_secs, request:{filename}, result, index}`.

```bash
lime --json add src/auth.rs
```

### remove <filepath>
Remove file from index (disk untouched). Returns `{ok, command:"remove", elapsed_secs, removed, index}`.

```bash
lime --json remove src/old.rs
```

### search [language] [type] <query> [--fuzzy]
Case-insensitive substring match on names, IDs, file paths.

**Language aliases:** `rs` → rust, `py` → python, `js` → javascript, `ts` → typescript (same for `list`, `annotate list`, etc.).

```bash
lime --json search run                    # all languages
lime --json search rust run               # filter language
lime --json search rs main               # rust (alias)
lime --json search py class              # python (alias)
lime --json search js run                 # javascript (alias)
lime --json search rust fn run            # filter language + type
lime --json search --fuzzy auth           # fuzzy: token + annotation search
```

**JSON:** `{ok, command:"search", query, fuzzy, result_count, results:[{id, name, type, file, start_line, language, death_status, match_type?, annotation_preview?}], elapsed_secs}`

`--fuzzy` adds `match_type`: `exact`, `prefix`, `substring`, `annotation`. Annotation matches include `annotation_preview` (first 80 chars).

### list [language] [type] [-a|--all] [--dead] [--fault]
```bash
lime --json list                          # list indexed languages
lime --json list rust                     # component counts by type
lime --json list rs -a                    # all rust components (alias)
lime --json list py -a                    # all python components (alias)
lime --json list ts -a                    # all typescript components (alias)
lime --json list rust -a                  # all components with IDs
lime --json list rust fn                  # only functions
lime --json list rust --dead              # dead components only
lime --json list rust --fault             # faulty components only
lime --json list rust --dead --fault      # combined filter
```

**JSON modes:** `languages` → `{ok, languages:[...]}` · `language_summary` → `{ok, language, total, dead, faulty, component_counts:{type:count}}` · `language_all` / `language_and_type` → `{ok, language, count, components:[{id, name, type, file, start_line, language, death_status, faults}]}`

### show <component_id>
Source code + line numbers + inline diagnostics + annotation.

```bash
lime --json show fn-61bcc6dabec3f308
```

**JSON:** `{ok, command:"show", component:{id, language, type, name, file, start_line, end_line, death_status, faults}, file_changed, source_lines:[{line, code, diagnostics:[]}], annotation, index_staleness}`

### deps <component_id> [--depth <n>]
Dependency matrix (uses / used-by). Default depth from config (usually 2).

```bash
lime --json deps fn-61bcc6dabec3f308
lime --json deps fn-61bcc6dabec3f308 --depth 3
lime --json deps fn-61bcc6dabec3f308 --depth 0   # component only; obsolete
```

**JSON:** `{ok, command:"deps", component, dependency_matrix, elapsed_secs}`

### annotate {add|show|list|remove}

```bash
lime --json annotate add <id> -m "description"        # inline message
lime --json annotate add <id> --file notes/id.md      # from file
lime --json annotate add <id> -m "x" -t keep -t api   # with tags
lime --json annotate add <id> -m "x" -l auth          # with link label
lime --json annotate show <id>
lime --json annotate list [language] [type]   # e.g. `rs fn`, `py class`, `ts fn`
lime --json annotate remove <id>
```

Flags: `-m/--message` (inline body), `--file` (body from path; exclusive with `-m`), `-t/--tag` (repeatable), `-l/--link` (repeatable link paths; **dual-written** to `.lime/component_links.json` and the annotation file).

**JSON:** `{ok, command:"annotate", action, elapsed_secs, annotation:{hash_id, content, tags, links, created_at, updated_at}}` (list → `results:[{component, annotation:{...preview}}]`, remove → `{removed}`)

### Link paths (naming)

- **Delimiter:** `/` only. Segments are non-empty; no leading/trailing `/`, no `//`.
- **Hierarchy:** prefix = ancestry (`auth` is parent of `auth/login`).
- **Order (default):** Unicode **lexicographic** sort on the full path. Encode timelines with zero-padded segments, e.g. `auth/01-discovery`, `auth/02-design`, or date buckets `auth/2026-03/01-login`.
- **Matching:** `lime links show <query>` matches a path **equal** to `query` or any path **under** `query/` (case-insensitive).
- **Limits:** path length and count per component are capped (see code: `MAX_LINK_PATH_LEN`, `MAX_PATHS_PER_COMPONENT`).

Optional **`.lime/link_catalog.json`**: map path → `{ title, description, parent_path, sort_key }`. When present, `sort_key` orders paths before lexicographic path (used by `lime links list` and `lime links show` grouping).

### links {show|list|add|remove|compact}

Single command for all link workflows (merged **`.lime/component_links.json`** + annotation `links`).

```bash
lime --json links show auth               # components on path auth or under auth/…
lime --json links show auth/login --notes # include full annotation bodies
lime --json links list                    # all distinct paths
lime --json links list auth --tree        # indent by / depth
lime --json links add fn-abc123 auth/login
lime --json links remove fn-abc123 auth/login
lime --json links compact                 # drop duplicate link lines from annotations when store has path
```

**JSON:** `{ok, command:"links", action, elapsed_secs, ...}`

- **show** — `action:"show"`, plus `link`, `notes`, `result_count`, `results`, `path_groups`, `orphan_count`, `orphans`, `orphan_memberships`, `index_staleness`, …
- **list** — `paths`, `path_count`, `tree`, `prefix`
- **remove** — `removed`
- **compact** — `annotations_updated`

### sum [--top-links N]

Bounded overview; **`links_top`** counts components per path using the **same merged** membership as `lime links show` / search tokens (not annotations alone).

### registry {list|add|remove}

Global router file: `~/.lime/projects.json` (small JSON map only; indexes stay in each repo’s `.lime/`).

No `.lime` init is required to **register** a path: `registry add` only records the root so `--external` can route reads (and safe `annotate add`) there.

```bash
lime --json registry list
lime --json registry add                         # register current working directory
lime --json registry add ../some-repo            # id defaults to folder basename
lime --json registry add --id tokio C:/dev/tokio
lime --json registry remove tokio
```

**JSON:** `{ok, command:"registry", action, registry_path, ...}` (list includes `projects: [...]` entries)

### Cross-repository `--external <projectID>`

Route supported commands to another registered repository without copying indexes. Register roots first (`lime registry add` from that tree, or `lime registry add <path>`).

```bash
lime --json show --external tokio fn-abc123
lime --json search --external tokio reactor
lime --json links show --external tokio runtime
lime --json annotate add --external tokio fn-abc123 -m "used by app X"
```

Responses for routed commands include:

```json
"target": { "mode": "external", "project_id": "tokio", "root": "..." }
```

Safety policy (foreign repository target):

| Command area | `--external` |
|---|---|
| `show`, `deps`, `search`, `list`, `sum`, `links show/list`, `annotate show/list` | Allowed (read-only) |
| `annotate add` | Allowed (**annotation markdown only**) |
| `annotate remove` | Blocked |
| `sync`, `add`, `remove`, `links add/remove/compact`, `config`, `registry` | Blocked |

### config … [--global]

**All** `lime config <subcommand>` actions support **`--global`** on the `config` command (same flags, writes `~/.config/lime/lime.json` / Windows `%APPDATA%\lime\lime.json`). Omit `--global` for **project** `.lime/lime.json`.

You can set values entirely from the terminal—no editor required. Subcommands that take optional flags typically **show** current values when flags are omitted and **write** when you pass a new value.

| Config key | CLI (project) | CLI (global template) |
|------------|---------------|------------------------|
| *(full dump)* | `lime config show` | `lime config --global show` |
| `diagnostics.*` | `lime config diagnostics --enabled true --timeout 60` | add `--global` |
| `death_seeds.*` | `lime config death-seeds --seed-file …` | add `--global` |
| `index_pretty` | `lime config index --pretty false` | add `--global` |
| `git_partial_sync.empty_sync_uses_git` | `lime config git-partial-sync --use-git-for-empty-sync true` | add `--global` |
| | Alias: `--git-empty-sync true` | |
| `default_dependency_depth` | `lime config dependency-depth --depth 3` | add `--global` |
| `ignore_patterns` | `lime config ignores --add dist/ --remove tmp/` | add `--global` |
| `index_storage` | `lime config index-storage --path .lime/index.json` | add `--global` |

```bash
lime --json config show
lime --json config diagnostics --enabled true --timeout 60
lime --json config death-seeds --seed-file "src/main.rs" --seed-name "main" --seed-type "fn"
lime --json config death-seeds --clear-seed-files   # clear before adding
lime --json config index --pretty false              # compact JSON writes
lime --json config git-partial-sync                  # show empty_sync_uses_git
lime --json config git-partial-sync --git-empty-sync true
lime --json config dependency-depth --depth 4
lime --json config ignores --add "vendor/"
lime --json config index-storage --path .lime/index.json

# Global template (new projects seed from this when .lime/lime.json is created):
lime --json config --global show
lime --json config --global diagnostics --enabled true
lime --json config --global git-partial-sync --git-empty-sync true
```

death-seeds flags: `--seed-file <pattern>`, `--seed-name <name>`, `--seed-type <type>`, `--clear-seed-files`, `--clear-seed-names`, `--clear-seed-types`.

**JSON:** `"global": bool`, `"config_path": "<abs path>"`, and `"updated": bool` where applicable (false when no file was written).

## Global Config (`~/.config/lime/lime.json`)

Lime supports a **global config** that lives outside any repository:

| Platform | Path |
|----------|------|
| Linux / macOS | `~/.config/lime/lime.json` |
| Windows | `%APPDATA%\lime\lime.json` |

**Purpose:** When `lime sync` (or any command that triggers `load_or_create`) initialises a **new** project config (`.lime/lime.json` not yet present), the global config is used as the starting template instead of hard-coded defaults. Preferences set once globally carry over to every new repository automatically.

`index_storage` is always reset to `.lime/index.json` regardless of what the global config says, so one project's custom index path never bleeds into another.

Use `lime config --global <subcommand>` to manage global preferences. The file is created automatically on first write.

## Config (`.lime/lime.json`)

| Key | Default | Description |
|-----|---------|-------------|
| `default_dependency_depth` | 2 | Max dep traversal depth |
| `ignore_patterns` | [] | Extra glob patterns to exclude |
| `index_storage` | `.lime/index.json` | Index file path |
| `index_pretty` | true | Pretty-print index JSON |
| `diagnostics.enabled` | false | Run analyzers on sync |
| `diagnostics.timeout_secs` | 120 | Per-analyzer timeout |
| `death_seeds.seed_files` | [] | File patterns (always-alive) |
| `death_seeds.seed_names` | [] | Component name patterns (always-alive) |
| `death_seeds.seed_types` | [] | Component types (always-alive) |
| `git_partial_sync.empty_sync_uses_git` | false | If true, bare `lime sync` uses git dirty paths (partial) instead of full rebuild |

Default ignores: `node_modules`, `target`, `.git`, `.lime`, `.lemon`

## Storage

- Index: `.lime/index.json` (persistent; `lime sync` after code changes)
- Annotations: `.lime/annotations/<type>/<id>.md`
- **Link membership:** `.lime/component_links.json` (source of truth; unioned with annotation `links` on read)
- **Optional:** `.lime/link_catalog.json` (titles / `sort_key` overrides per path)
- Index is line/regex parsed (not AST); treat as approximate for macros/generated code

### Agent workflow (links)

1. `lime --json sum` → scan `links_top` for hot paths.
2. `lime --json link <path>` → pull grouped components for that topic.
3. Prefer `lime --json links add <id> <path>` for membership without touching prose; use `annotate add -l` when you also need a written note (**dual-write** keeps legacy annotation `links` in sync).
4. After migrating to the store, `lime --json links compact` removes redundant `links` lines from annotation frontmatter.
5. Re-run `lime sync` if IDs drift; fix **orphan_memberships** (stale IDs in the link store) or re-link components.

## Languages & Types

| Language | Extensions | Types |
|----------|-----------|-------|
| rust | .rs | fn, struct, enum, trait, impl, mod, use |
| javascript | .js, .jsx, .mjs | function, class, const, let, var, export |
| typescript | .ts, .tsx | function, class, interface, type, const, let, var, export |
| python | .py | def, async def, class, import, from |
| go | .go | func, type, struct, interface, const, var |
| zig | .zig | fn, struct, enum, const |
| c | .c, .h | function, struct, enum, typedef |
| cpp | .cpp, .hpp, .cc, .cxx | function, class, struct, enum, typedef |

## Errors

All errors: `{"ok":false, "command":"<cmd>", "error":"<message>"}`

Usage Scenarios:
'lime sync' for codebase indexation
'lime list -a --json' to retrieve all components
'lime show {componentID}' to show the component content, attached annotations for context
'lime links show {path-or-prefix}' — components via merged link store + annotations; 
'lime links list|add|remove|compact' for paths and '.lime/component_links.json'


