# Lime
*THE* CLI tool for codebase indexation, component-dependency matrix retrieval, annotation, and more.

## Overview
Lime has several uses. For one, you can focus and latch onto one of the components in your codebase by running "lime show {type}-{hashID}", which outputs, with syntactial highlights,
the component you requested; with the line numbers, of course.

It also allows you to view the dependencies (at various "depths" or chain-related dependent components) of a specific component to see what's used by, or uses, the component you've requested; run "lime deps {type}-{hashID}"
to see the "dependency matrix". 

Lime can also reference components from other indexed repositories on your machine: add them to the global router (`lime registry add` for the current folder, or `lime registry add <path>`) and query with `--external <projectID>`, for example
`lime show --external tokio fn-...`. This routes reads to the target repo's existing `.lime` data (no duplicated global index).

Lime additionally has a component death detection algorithm, which requires a component's dependencies and/or line-by-line validation to satisfy specific "inverse" requirements, which ensure components aren't marked as "dead"
while they're still alive. You can see "[dead]" flagged components in "lime list {lang} -a", or filter by "dead" using the "--dead" flag. 

Similarly, Lime also integrates (if available on the user's device) language specific linter tools, language analyzers to mark "faulty" components, which are the subject of error in your codebase (at least based on syntax). 
You can see these components in the component list marked with "[Fault(s): n]". You can also filter the components with the flag "--fault". 

For speed, binary size optimization, Lime utilizes regex-based component parsing, instead of AST analyzers (still deciding on picking one or the other, though regex is the current implementation). 

## How does it work?
Lime works by identifying "components" of your codebase, instead of indexing the entire codebase, line by line. For Rust, it retrieves, for example, fn, structs, enums, etc. 
Each component is assigned a hashID via Blake3 hash, prefixed with the component type; i.e. "fn-{hashID}". 



## This README.md is a work in progress, as is the project itself (though it is functional). 
I'll probably release pre-built binaries once I really get this stable, but for now you can build from source by cloning the repository, and running "cargo build --release". It's a single binary, and currently only works
on (been tested on; I don't actually know if it works elsewhere, the main problem is likely only path configurations) Windows. For the time being, all commits will only be made after thorough testing of each commit's
contributions, so you should be ok using the tool from any source version (any commit).
