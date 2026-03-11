# Skill: Lime

Lime is a fast, language-aware codebase index for AI coding agents. It builds and maintains a persistent index of code components (functions, structs, classes, etc.) enabling agents to efficiently search, list, and traverse dependencies.

## Commands

### lime sync [files...]
Rebuild the index. Without arguments, scans the entire codebase. With file arguments, indexes only those files.

```
"lime sync"                  rebuild entire index
"lime sync src/main.rs"      index specific file
"lime sync src/a.rs src/b.rs" index multiple files
```

### lime add <filepath>
Add or refresh a single file in the index.

```
"lime add src/auth.rs"
```

### lime remove <filepath>
Remove a file and all its components from the index. File on disk is untouched.

```
"lime remove src/old.rs"
```

### lime search [language] [type] <query>
Search indexed components by name, file, or ID. Case-insensitive substring match.

```
"lime search run"                 search all languages
"lime search rust run"           filter by language
"lime search rust fn run"        filter by language and type
"lime search python class Auth"  filter by language and type
```

Supported languages: rust, javascript, typescript, python, go
Supported types: fn, struct, enum, class, def, func, trait, impl, and more

### lime list [language] [-all] [type]
List languages or components in the index.

```
"lime list"               list all indexed languages
"lime list rust"         show rust component counts
"lime list rust -all"    list all rust components
"lime list rust fn"       list only rust functions
```

### lime deps <component_id> [--depth <n>]
Show dependency matrix for a component. Displays what it uses ("uses") and what uses it ("used by"), traversed to specified depth.

```
"lime deps fn-abc123"            default depth (2)
"lime deps fn-abc123 --depth 3"  traverse 3 levels
"lime deps fn-abc123 --depth 0"  component only, no deps
```

Obtain component IDs from `lime search` or `lime list -all`.

## Global Options

| Flag | Description |
|------|-------------|
| `--json` | Output raw JSON (for scripts/agents) |
| `-V, --version` | Print version |
| `-h, --help` | Print help |

## Output

### Human-Readable (default)
```
Indexed 8 files, 138 components (rust) in 0.03s
```

```
rust:
  struct   12
  fn       84
  ...
  total   138
```

```
run (fn)  src/main.rs:16  fn-61bcc6dabec3f308
```

### JSON (`--json`)
```json
{"command":"sync","index":{"component_count":138,"file_count":8,"languages":["rust"]},"ok":true}
```

## Configuration

Lime reads `.lime/lime.json` in the project's `.lime/` directory for:
- `default_dependency_depth`: Max depth for dependency traversal (default: 2)
- `ignore_patterns`: Additional patterns to exclude from indexing
- `index_path`: Custom location for the index file

Default ignore patterns include: node_modules, target, .git, .lime, .lemon

## Index Storage

- Index stored at `.lime/index.json` (by default)
- Index is persistent — only rebuild when code changes
- Use `lime sync` after adding, removing, or modifying files
