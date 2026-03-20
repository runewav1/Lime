# Skill: Lime

Lime is a fast, language-aware codebase index for AI coding agents. It builds and maintains a persistent index of code components (functions, structs, classes, etc.) enabling agents to efficiently search, list, annotate, and traverse dependencies.

## Global Flag

| Flag | Description |
|------|-------------|
| `--json` | Output raw JSON for scripts and agents (use this for all agent interactions) |

## Commands

### lime sync

Build/rebuild the index. Without arguments, scans entire codebase. With file arguments, indexes only those files.

```bash
lime --json sync
lime --json sync src/main.rs
lime --json sync src/a.rs src/b.rs
lime --json sync --diagnostics
lime --json sync -v
```

| Flag | Description |
|------|-------------|
| `--diagnostics` | Run static analyzers and attach fault data to components |
| `-v, --verbose` | Show detailed output |

**JSON Output:**
```json
{
  "ok": true,
  "command": "sync",
  "elapsed_secs": 0.03,
  "index": {
    "component_count": 138,
    "file_count": 8,
    "languages": ["rust"],
    "batman_count": 3
  },
  "index_staleness": {
    "stale": false,
    "last_indexed_commit": "abc123",
    "current_commit": "abc123"
  }
}
```

### lime add

Add or refresh a single file in the index.

```bash
lime --json add src/auth.rs
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "add",
  "file": "src/auth.rs",
  "elapsed_secs": 0.01,
  "index": { "component_count": 142, "file_count": 9, "languages": ["rust"] }
}
```

### lime remove

Remove a file and all its components from the index. Disk untouched.

```bash
lime --json remove src/old.rs
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "remove",
  "file": "src/old.rs",
  "removed_components": 5,
  "elapsed_secs": 0.01
}
```

### lime search

Search indexed components by name, file path, or ID. Case-insensitive substring match.

```bash
lime --json search run
lime --json search rust run
lime --json search rust fn run
lime --json search --fuzzy auth
lime --json search python class Auth
```

| Flag | Description |
|------|-------------|
| `--fuzzy` | Enable fuzzy matching with token-based and annotation search |

**Supported Languages:** rust, javascript, typescript, python, go, zig, c, cpp, swift

**Supported Types:** fn, struct, enum, trait, impl, mod, use, class, function, const, let, var, interface, type, export, def, async def, async, import, from, func

**JSON Output:**
```json
{
  "ok": true,
  "command": "search",
  "query": "run",
  "results": [
    {
      "id": "fn-61bcc6dabec3f308",
      "name": "run",
      "type": "fn",
      "file": "src/main.rs",
      "line": 16,
      "language": "rust",
      "batman": false,
      "match_type": "exact",
      "annotation_preview": null
    }
  ],
  "elapsed_secs": 0.01
}
```

When `--fuzzy` is used, `match_type` can be: `exact`, `prefix`, `substring`, `annotation`. If match came from annotation content, `annotation_preview` contains first 80 chars.

### lime list

List indexed languages or components.

```bash
lime --json list
lime --json list rust
lime --json list rust -a
lime --json list rust fn
lime --json list rust --dead
lime --json list rust --fault
lime --json list rust --dead --fault
```

| Flag | Description |
|------|-------------|
| `-a, --all` | List all components for the language with IDs |
| `--dead` | Only components marked as dead (batman) |
| `--fault` | Only components with analyzer faults |

**JSON Output (languages):**
```json
{
  "ok": true,
  "command": "list",
  "languages": ["rust", "typescript"]
}
```

**JSON Output (language summary):**
```json
{
  "ok": true,
  "command": "list",
  "language": "rust",
  "counts": {
    "fn": 84,
    "struct": 12,
    "enum": 8,
    "impl": 20,
    "total": 138
  }
}
```

**JSON Output (all components):**
```json
{
  "ok": true,
  "command": "list",
  "language": "rust",
  "components": [
    {
      "id": "fn-61bcc6dabec3f308",
      "name": "run",
      "type": "fn",
      "file": "src/main.rs",
      "line": 16,
      "batman": false
    }
  ]
}
```

### lime show

Show component source code with line numbers and inline diagnostics.

```bash
lime --json show fn-61bcc6dabec3f308
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "show",
  "component": {
    "id": "fn-61bcc6dabec3f308",
    "name": "run",
    "type": "fn",
    "file": "src/main.rs",
    "line": 16
  },
  "source_lines": [
    { "line": 16, "content": "pub fn run() -> Result<()> {" },
    { "line": 17, "content": "    let config = Config::load()?;" }
  ],
  "diagnostics": [],
  "annotation": null,
  "file_changed": false
}
```

### lime deps

Show dependency matrix for a component (uses/used-by relationships).

```bash
lime --json deps fn-61bcc6dabec3f308
lime --json deps fn-61bcc6dabec3f308 --depth 3
lime --json deps fn-61bcc6dabec3f308 --depth 0
```

| Flag | Description |
|------|-------------|
| `--depth <n>` | Traversal depth (default from config, usually 2). Use 0 for component only |

**JSON Output:**
```json
{
  "ok": true,
  "command": "deps",
  "component": {
    "id": "fn-61bcc6dabec3f308",
    "name": "run",
    "type": "fn",
    "batman": false
  },
  "depth": 2,
  "uses": [
    { "id": "struct-abc123", "name": "Config", "type": "struct", "depth": 1, "batman": false }
  ],
  "used_by": [
    { "id": "fn-def456", "name": "main", "type": "fn", "depth": 1, "batman": false }
  ]
}
```

### lime annotate

Attach semantic annotations to components.

#### lime annotate add

```bash
lime --json annotate add fn-61bcc6dabec3f308 -m "Entry point for CLI execution"
lime --json annotate add fn-abc123 -m "Auth handler" -t security -t critical
```

| Flag | Description |
|------|-------------|
| `-m <message>` | Annotation content (required) |
| `-t <tag>` | Add tag (repeatable) |

**JSON Output:**
```json
{
  "ok": true,
  "command": "annotate add",
  "component_id": "fn-61bcc6dabec3f308",
  "annotation": {
    "hash_id": "fn-61bcc6dabec3f308",
    "component_name": "run",
    "component_type": "fn",
    "content": "Entry point for CLI execution",
    "tags": [],
    "created_at": "2026-03-20T05:00:00Z",
    "updated_at": "2026-03-20T05:00:00Z"
  }
}
```

#### lime annotate show

```bash
lime --json annotate show fn-61bcc6dabec3f308
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "annotate show",
  "component_id": "fn-61bcc6dabec3f308",
  "annotation": {
    "hash_id": "fn-61bcc6dabec3f308",
    "component_name": "run",
    "component_type": "fn",
    "content": "Entry point for CLI execution",
    "tags": [],
    "created_at": "2026-03-20T05:00:00Z",
    "updated_at": "2026-03-20T05:00:00Z"
  }
}
```

#### lime annotate list

```bash
lime --json annotate list
lime --json annotate list rust
lime --json annotate list rust fn
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "annotate list",
  "annotations": [
    {
      "hash_id": "fn-61bcc6dabec3f308",
      "component_name": "run",
      "component_type": "fn",
      "content": "Entry point for CLI execution",
      "tags": [],
      "created_at": "2026-03-20T05:00:00Z"
    }
  ]
}
```

#### lime annotate remove

```bash
lime --json annotate remove fn-61bcc6dabec3f308
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "annotate remove",
  "component_id": "fn-61bcc6dabec3f308",
  "removed": true
}
```

### lime config

View and modify Lime configuration.

#### lime config show

```bash
lime --json config show
```

**JSON Output:**
```json
{
  "ok": true,
  "command": "config show",
  "config": {
    "default_dependency_depth": 2,
    "ignore_patterns": [],
    "index_path": null,
    "diagnostics": {
      "enabled": false,
      "timeout_secs": 30
    },
    "death_seeds": {
      "seed_files": [],
      "seed_names": [],
      "seed_types": []
    }
  }
}
```

#### lime config diagnostics

```bash
lime --json config diagnostics --enabled true
lime --json config diagnostics --timeout 60
lime --json config diagnostics --enabled true --timeout 45
```

| Flag | Description |
|------|-------------|
| `--enabled <true\|false>` | Enable/disable diagnostics during sync |
| `--timeout <secs>` | Per-analyzer timeout in seconds |

**JSON Output:**
```json
{
  "ok": true,
  "command": "config diagnostics",
  "diagnostics": {
    "enabled": true,
    "timeout_secs": 60
  }
}
```

#### lime config death-seeds

Configure seed patterns for dead-code detection (components matching seeds are always considered alive).

```bash
lime --json config death-seeds --seed-file "src/main.rs"
lime --json config death-seeds --seed-name "main"
lime --json config death-seeds --seed-type "fn"
lime --json config death-seeds --clear-seed-files
lime --json config death-seeds --clear-seed-names
lime --json config death-seeds --clear-seed-types
```

| Flag | Description |
|------|-------------|
| `--seed-file <pattern>` | Add file path pattern as alive root |
| `--seed-name <name>` | Add component name pattern as alive root |
| `--seed-type <type>` | Add component type as alive root |
| `--clear-seed-files` | Clear all seed file patterns |
| `--clear-seed-names` | Clear all seed name patterns |
| `--clear-seed-types` | Clear all seed type patterns |

**JSON Output:**
```json
{
  "ok": true,
  "command": "config death-seeds",
  "death_seeds": {
    "seed_files": ["src/main.rs"],
    "seed_names": ["main", "run"],
    "seed_types": ["fn"]
  }
}
```

## Configuration

Lime reads `.lime/lime.json` for settings:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_dependency_depth` | int | 2 | Max depth for dependency traversal |
| `ignore_patterns` | array | [] | Additional patterns to exclude from indexing |
| `index_path` | string | null | Custom location for index file |
| `diagnostics.enabled` | bool | false | Run static analyzers during sync |
| `diagnostics.timeout_secs` | int | 30 | Per-analyzer timeout |
| `death_seeds.seed_files` | array | [] | File path patterns (always-alive roots) |
| `death_seeds.seed_names` | array | [] | Component name patterns (always-alive roots) |
| `death_seeds.seed_types` | array | [] | Component types (always-alive roots) |

**Default Ignore Patterns:** node_modules, target, .git, .lime, .lemon

## Index Storage

- Index stored at `.lime/index.json`
- Annotations stored at `.lime/annotations/<type>/<id>.md`
- Index is persistent; use `lime sync` after code changes
- Use `lime sync --diagnostics` to attach static analysis data

## Supported Languages

| Language | File Extensions |
|----------|-----------------|
| rust | .rs |
| javascript | .js, .jsx, .mjs |
| typescript | .ts, .tsx |
| python | .py |
| go | .go |
| zig | .zig |
| c | .c, .h |
| cpp | .cpp, .hpp, .cc, .cxx |
| swift | .swift |

## Component Types by Language

| Language | Types |
|----------|-------|
| rust | fn, struct, enum, trait, impl, mod, use |
| javascript | function, class, const, let, var, export |
| typescript | function, class, interface, type, const, let, var, export |
| python | def, async def, class, import, from |
| go | func, type, struct, interface, const, var |
| zig | fn, struct, enum, const |
| c | function, struct, enum, typedef |
| cpp | function, class, struct, enum, typedef |
| swift | struct, class, enum, protocol, actor, extension, func, init, deinit, import, typealias |

## Error Handling

All commands return `"ok": false` on error with an `"error"` field:

```json
{
  "ok": false,
  "command": "show",
  "error": "Component not found: fn-invalid123"
}
```
