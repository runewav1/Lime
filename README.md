# Lime

**Lime** is a CLI that indexes your repo into **components** (functions, types, classes, modules, …), tracks **dependencies**, and supports **annotations** and **hierarchical links**—with first-class **`--json`** output for tools and agents.

[Quick start](#quick-start) · [Commands](#commands) · [Config & files](#config--files) · [Docs](#documentation)

## Features

- **Index** — full sync, optional git-partial sync on dirty paths (after the index has components), or per-file `add` / `remove`
- **Discover** — `search` (substring or `--fuzzy`), `list` with `--dead` / `--fault` filters
- **Inspect** — `show` (source + line refs + optional diagnostics), `deps` (uses / used-by graph; heuristic edges + optional `dep_edges` kinds in the index)
- **Notes & navigation** — `annotate`, `links` (merged store + annotation paths), `sum` overview
- **Multiple repos** — `registry` writes `~/.lime/projects.json`; **`--external <id>`** routes reads and scoped workflows (`links`, `annotate add`, …) to another registered root; **scoped link paths** `@registered_id/topic` tie components to other checkouts without a separate graph file (see [SKILL.md](SKILL.md))
- **Settings** — project `.lime/lime.json` and optional **global** template via `lime config --global …`

**Language aliases** (also for `list`, `annotate list`, …): `rs`→rust, `py`→python, `js`→javascript, `ts`→typescript.

## Installation

From a clone of this repository:

```bash
cargo build --release
# target/release/lime   (lime.exe on Windows)
```

Or install the package from the repo root:

```bash
cargo install --path .
```

Pre-built release binaries are not published yet; building from source is the supported path today.

## Quick start

```bash
lime sync
lime list
lime search run
lime show fn-<id>          # use an ID from list/search
lime deps fn-<id>
```

Agents and scripts should pass **`--json`** on every invocation for stable, parseable output.

## Commands

| Command | What it does |
|--------|----------------|
| `sync` | Rebuild or refresh the index (`--git` / `--no-git`, optional `--diagnostics`, paths for partial) |
| `add` / `remove` | Index or drop a single file |
| `search` | Find components by name, path, or ID; `--fuzzy` for token + annotation matching |
| `list` | Languages, per-type counts, or all components (`-a` / `--all`) |
| `show` | Component body with line numbers |
| `deps` | Dependency matrix; `--depth` |
| `annotate` | `add` / `show` / `list` / `remove` markdown notes |
| `links` | `show` / `list` / `add` / `remove` / `compact` for topic paths |
| `sum` | Bounded workspace summary (e.g. link hotspots) |
| `registry` | `list` / `add` / `remove` registered project roots |
| `config` | View or set options; **`lime config --global <subcmd>`** for the user-wide template (place `--global` immediately after `lime config`) |

`--external <projectID>` is rejected for `sync`, per-file `add`/`remove`, `config`, and `registry`. It is allowed for read-only inspection and for **`links` add/remove/compact** and **`annotate add`** against the foreign repo’s `.lime/` (see `lime --help` and [SKILL.md](SKILL.md)).

Run **`lime --help`** and **`lime <cmd> --help`** for flags and examples.

## Config & files

| Location | Role |
|----------|------|
| `.lime/lime.json` | Project settings (ignores, dependency depth, diagnostics, death seeds, git partial behavior, …) |
| `.lime/index.json` | Default index (path overridable via config) |
| `.lime/annotations/` | One markdown file per annotated component |
| `.lime/component_links.json` | Link path membership (merged with annotation `links` on read) |
| `~/.lime/projects.json` | Registered roots for `--external` |
| `~/.config/lime/lime.json` (Unix) or `%APPDATA%\lime\lime.json` (Windows) | Global config template for **new** projects |

Indexing uses **regex-oriented** parsing, not a full semantic analyzer—results are fast and good for navigation, but macros and generated code can be approximate.

## Supported languages

Rust, Python, JavaScript/TypeScript, Go, Zig, C, C++, and Swift (see **`SKILL.md`** for extensions and component kinds).

## Documentation

- **[SKILL.md](SKILL.md)** — detailed command reference, JSON field notes, agent-oriented workflows, and config tables.

## License

MIT
