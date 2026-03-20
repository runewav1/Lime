---
name: lime
description: codebase indexation for most languages; high-level component retrieval, annotation, linking, and more.
---

Language-aware codebase index for AI agents. Indexes functions, structs, classes, and more. All commands support `--json` (global) for raw JSON output — always use it.

## Quick Start

```
lime --json sync                          # build/rebuild full index
lime --json search run                    # find components named "run"
lime --json list rust --all               # list all rust components with IDs
lime --json show fn-61bcc6dabec3f308      # show component source
lime --json deps fn-61bcc6dabec3f308      # dependency matrix
```

## Commands

### sync [files...] [--diagnostics] [-v]
Rebuild index. No args = full scan. With files = re-index only those.

```bash
lime --json sync
lime --json sync src/main.rs src/lib.rs
lime --json sync --diagnostics            # attach static-analyzer faults
lime --json sync -v                       # verbose breakdown
```

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

```bash
lime --json search run                    # all languages
lime --json search rust run               # filter language
lime --json search rust fn run            # filter language + type
lime --json search --fuzzy auth           # fuzzy: token + annotation search
```

**JSON:** `{ok, command:"search", query, fuzzy, result_count, results:[{id, name, type, file, start_line, language, death_status, match_type?, annotation_preview?}], elapsed_secs}`

`--fuzzy` adds `match_type`: `exact`, `prefix`, `substring`, `annotation`. Annotation matches include `annotation_preview` (first 80 chars).

### list [language] [type] [-a|--all] [--dead] [--fault]
```bash
lime --json list                          # list indexed languages
lime --json list rust                     # component counts by type
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
lime --json annotate list [language] [type]
lime --json annotate remove <id>
```

Flags: `-m/--message` (inline body), `--file` (body from path; exclusive with `-m`), `-t/--tag` (repeatable), `-l/--link` (repeatable link paths; **dual-written** to `.lime/component_links.json` and the annotation file).

**JSON:** `{ok, command:"annotate", action, elapsed_secs, annotation:{hash_id, content, tags, links, created_at, updated_at}}` (list → `results:[{component, annotation:{...preview}}]`, remove → `{removed}`)

### Link paths (naming)

- **Delimiter:** `/` only. Segments are non-empty; no leading/trailing `/`, no `//`.
- **Hierarchy:** prefix = ancestry (`auth` is parent of `auth/login`).
- **Order (default):** Unicode **lexicographic** sort on the full path. Encode timelines with zero-padded segments, e.g. `auth/01-discovery`, `auth/02-design`, or date buckets `auth/2026-03/01-login`.
- **Matching:** `lime link <query>` matches a path **equal** to `query` or any path **under** `query/` (case-insensitive).
- **Limits:** path length and count per component are capped (see code: `MAX_LINK_PATH_LEN`, `MAX_PATHS_PER_COMPONENT`).

Optional **`.lime/link_catalog.json`**: map path → `{ title, description, parent_path, sort_key }`. When present, `sort_key` orders paths before lexicographic path (used by `lime links list` and `lime link` grouping).

### links {add|remove|list|compact}

Manage membership in **`.lime/component_links.json`** without editing annotations.

```bash
lime --json links add fn-abc123 auth/login
lime --json links remove fn-abc123 auth/login
lime --json links list                  # all distinct paths (merged store + annotations)
lime --json links list auth --tree      # indent by / depth
lime --json links compact               # strip duplicate link lines from annotations when store already has path
```

**JSON:** `{ok, command:"links", action, elapsed_secs, ...}` — `list` adds `paths`, `path_count`, `tree`, `prefix`; `remove` adds `removed`; `compact` adds `annotations_updated`.

### link <path-or-prefix> [--notes]

List indexed components whose **merged** link paths match the query (link store ∪ annotation `links`). Terminal output groups by path (sorted) with `/`-depth indentation.

```bash
lime --json link auth
lime --json link auth/login
lime --json link auth --notes             # include full annotation markdown when present
```

**JSON:** `{ok, command:"link", link, notes, result_count, results, path_groups:[{path, depth, components}], orphan_count, orphans:[{kind:"annotation",...}], orphan_memberships:[{kind:"link_store", component_id, paths}], ...}`

### sum [--top-links N]

Bounded overview; **`links_top`** counts components per path using the **same merged** membership as `lime link` / search tokens (not annotations alone).

### config {show|diagnostics|death-seeds|index}

```bash
lime --json config show
lime --json config diagnostics --enabled true --timeout 60
lime --json config death-seeds --seed-file "src/main.rs" --seed-name "main" --seed-type "fn"
lime --json config death-seeds --clear-seed-files   # clear before adding
lime --json config index --pretty false              # compact JSON writes
```

death-seeds flags: `--seed-file <pattern>`, `--seed-name <name>`, `--seed-type <type>`, `--clear-seed-files`, `--clear-seed-names`, `--clear-seed-types`.

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
'lime link {path-or-prefix}' — components via merged link store + annotations; `lime links add|list` for CRUD on `.lime/component_links.json`


