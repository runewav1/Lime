use std::fmt::Write;

use serde_json::Value;

pub fn render(payload: &Value) -> String {
    let command = payload.get("command").and_then(Value::as_str).unwrap_or("");
    match command {
        "sync" => render_sync(payload),
        "add" => render_add(payload),
        "remove" => render_remove(payload),
        "search" => render_search(payload),
        "list" => render_list(payload),
        "deps" => render_deps(payload),
        _ => serde_json::to_string_pretty(payload).unwrap_or_default(),
    }
}

pub fn render_error(message: &str) -> String {
    format!("error: {message}\n")
}

// ── sync ────────────────────────────────────────────────────────

fn render_sync(v: &Value) -> String {
    let mut out = String::new();
    let scope = str_val(v, "scope");

    if scope == "full" {
        if let Some(idx) = v.get("index") {
            let files = num_val(idx, "file_count");
            let components = num_val(idx, "component_count");
            let languages = str_array(idx, "languages");
            let _ = write!(
                out,
                "Indexed {files} file{}, {components} component{}",
                plural(files),
                plural(components)
            );
            if !languages.is_empty() {
                let _ = write!(out, " ({})", languages.join(", "));
            }
            if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
                let _ = write!(out, " in {}", format_duration(elapsed));
            }
            let _ = writeln!(out);
        }
    } else if scope == "partial" {
        if let Some(result) = v.get("result") {
            write_file_result(&mut out, result);
        }
        write_index_summary(&mut out, v);
    }

    out
}

// ── add ─────────────────────────────────────────────────────────

fn render_add(v: &Value) -> String {
    let mut out = String::new();

    if let Some(result) = v.get("result") {
        write_file_result(&mut out, result);
    } else {
        let filename = v
            .get("request")
            .and_then(|r| r.get("filename"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let _ = writeln!(out, "  + {filename}");
    }

    write_index_summary(&mut out, v);
    out
}

// ── remove ──────────────────────────────────────────────────────

fn render_remove(v: &Value) -> String {
    let mut out = String::new();
    let filename = v
        .get("request")
        .and_then(|r| r.get("filename"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    let removed = v.get("removed").and_then(Value::as_bool).unwrap_or(false);

    if removed {
        let _ = writeln!(out, "  - {filename}");
    } else {
        let _ = writeln!(out, "  not indexed: {filename}");
    }

    write_index_summary(&mut out, v);
    out
}

// ── search ──────────────────────────────────────────────────────

fn render_search(v: &Value) -> String {
    let mut out = String::new();
    let count = num_val(v, "result_count");

    if count == 0 {
        let _ = writeln!(out, "No results");
        return out;
    }

    if let Some(results) = v.get("results").and_then(Value::as_array) {
        let labels: Vec<String> = results
            .iter()
            .map(|c| {
                let name = display_name(&str_val(c, "name"));
                let ctype = str_val(c, "type");
                format!("{name} ({ctype})")
            })
            .collect();

        let locations: Vec<String> = results
            .iter()
            .map(|c| {
                let file = str_val(c, "file");
                let start = num_val(c, "start_line");
                format!("{file}:{start}")
            })
            .collect();

        let max_label = labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);

        for (i, component) in results.iter().enumerate() {
            let id = str_val(component, "id");
            let _ = writeln!(
                out,
                "  {:<lw$}  {:<fw$}  {id}",
                labels[i],
                locations[i],
                lw = max_label,
                fw = max_loc
            );
        }
    }

    let _ = writeln!(out, "\n{count} result{}", plural(count));
    out
}

// ── list ────────────────────────────────────────────────────────

fn render_list(v: &Value) -> String {
    match str_val(v, "mode").as_str() {
        "languages" => render_list_languages(v),
        "language_summary" => render_list_summary(v),
        "language_all" => render_list_components(v, "all"),
        "language_and_type" => render_list_components(v, &str_val(v, "type")),
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn render_list_languages(v: &Value) -> String {
    let mut out = String::new();
    let languages = str_array(v, "languages");

    if languages.is_empty() {
        let _ = writeln!(out, "No indexed languages");
        return out;
    }

    for lang in &languages {
        let _ = writeln!(out, "{lang}");
    }

    out
}

fn render_list_summary(v: &Value) -> String {
    let mut out = String::new();
    let language = str_val(v, "language");

    let Some(counts) = v.get("component_counts").and_then(Value::as_object) else {
        let _ = writeln!(out, "{language}: no components");
        return out;
    };

    if counts.is_empty() {
        let _ = writeln!(out, "{language}: no components");
        return out;
    }

    let max_key = counts.keys().map(|k| k.len()).max().unwrap_or(0);
    let total: u64 = counts.values().filter_map(Value::as_u64).sum();
    let count_width = total.to_string().len().max(1);

    let _ = writeln!(out, "{language}:");
    for (ctype, count) in counts {
        let n = count.as_u64().unwrap_or(0);
        let _ = writeln!(
            out,
            "  {ctype:<w$}  {n:>cw$}",
            w = max_key,
            cw = count_width
        );
    }

    let _ = writeln!(out, "  {}", "-".repeat(max_key + 2 + count_width));
    let _ = writeln!(
        out,
        "  {:<w$}  {total:>cw$}",
        "total",
        w = max_key,
        cw = count_width
    );

    out
}

fn render_list_components(v: &Value, label: &str) -> String {
    let mut out = String::new();
    let language = str_val(v, "language");
    let count = num_val(v, "count");
    let show_type = label == "all";

    let _ = writeln!(out, "{language} {label}:");

    if count == 0 {
        let _ = writeln!(out, "  (none)");
        return out;
    }

    if let Some(components) = v.get("components").and_then(Value::as_array) {
        let (max_label, max_name) = if show_type {
            let ml = components
                .iter()
                .map(|c| {
                    let name = display_name(&str_val(c, "name"));
                    let ctype = str_val(c, "type");
                    format!("{name} ({ctype})").len()
                })
                .max()
                .unwrap_or(0);
            (ml, 0usize)
        } else {
            let mn = components
                .iter()
                .map(|c| display_name(&str_val(c, "name")).len())
                .max()
                .unwrap_or(0)
                .min(40);
            (0usize, mn)
        };

        let mut current_file = String::new();

        for component in components {
            let file = str_val(component, "file");

            if file != current_file {
                if !current_file.is_empty() {
                    let _ = writeln!(out);
                }
                let _ = writeln!(out, "  {file}");
                current_file = file;
            }

            let name = display_name(&str_val(component, "name"));
            let start = num_val(component, "start_line");
            let id = str_val(component, "id");

            if show_type {
                let ctype = str_val(component, "type");
                let label = format!("{name} ({ctype})");
                let _ = writeln!(out, "    {label:<lw$}  :{start}  {id}", lw = max_label);
            } else {
                let _ = writeln!(out, "    {name:<nw$}  :{start}  {id}", nw = max_name);
            }
        }
    }

    let _ = writeln!(out, "\n{count} component{}", plural(count));
    out
}

// ── deps ────────────────────────────────────────────────────────

fn render_deps(v: &Value) -> String {
    let mut out = String::new();

    if let Some(component) = v.get("component") {
        let ctype = str_val(component, "type");
        let name = display_name(&str_val(component, "name"));
        let file = str_val(component, "file");
        let start = num_val(component, "start_line");
        let id = str_val(component, "id");

        let _ = writeln!(out, "{name} ({ctype})  {file}:{start}  {id}");
    }

    if let Some(matrix) = v.get("dependency_matrix") {
        let before = matrix.get("before").and_then(Value::as_array);
        let after = matrix.get("after").and_then(Value::as_array);

        let before_empty = before.map(|a| a.is_empty()).unwrap_or(true);
        let after_empty = after.map(|a| a.is_empty()).unwrap_or(true);

        if let Some(nodes) = before {
            if !nodes.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "  uses:");
                write_dep_nodes(&mut out, nodes);
            }
        }

        if let Some(nodes) = after {
            if !nodes.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "  used by:");
                write_dep_nodes(&mut out, nodes);
            }
        }

        if before_empty && after_empty {
            let _ = writeln!(out, "\n  no dependencies");
        }
    }

    out
}

fn write_dep_nodes(out: &mut String, nodes: &[Value]) {
    let labels: Vec<String> = nodes
        .iter()
        .map(|n| {
            let ctype = str_val(n, "type");
            let name = display_name(&str_val(n, "name"));
            format!("{name} ({ctype})")
        })
        .collect();

    let locations: Vec<String> = nodes
        .iter()
        .map(|n| {
            let file = str_val(n, "file");
            let start = num_val(n, "start_line");
            format!("{file}:{start}")
        })
        .collect();

    let ids: Vec<String> = nodes.iter().map(|n| str_val(n, "id")).collect();

    let max_label = labels.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_id = ids.iter().map(|i| i.len()).max().unwrap_or(0);

    for (i, node) in nodes.iter().enumerate() {
        let depth = num_val(node, "depth");
        if max_id > 0 {
            let _ = writeln!(
                out,
                "    {:<lw$}  {:<fw$}  {:<iw$}  depth:{depth}",
                labels[i],
                locations[i],
                ids[i],
                lw = max_label,
                fw = max_loc,
                iw = max_id
            );
        } else {
            let _ = writeln!(
                out,
                "    {:<lw$}  {:<fw$}  depth:{depth}",
                labels[i],
                locations[i],
                lw = max_label,
                fw = max_loc
            );
        }
    }
}

// ── shared formatting helpers ───────────────────────────────────

fn write_file_result(out: &mut String, result: &Value) {
    let indexed = str_array(result, "indexed");
    for path in &indexed {
        let _ = writeln!(out, "  + {path}");
    }

    let removed = str_array(result, "removed");
    for path in &removed {
        let _ = writeln!(out, "  - {path}");
    }

    if let Some(skipped) = result.get("skipped").and_then(Value::as_array) {
        for entry in skipped {
            let path = str_val(entry, "path");
            let reason = str_val(entry, "reason");
            let _ = writeln!(out, "  ~ {path} -- {reason}");
        }
    }
}

fn write_index_summary(out: &mut String, v: &Value) {
    if let Some(idx) = v.get("index") {
        let files = num_val(idx, "file_count");
        let components = num_val(idx, "component_count");
        let timing = v
            .get("elapsed_secs")
            .and_then(Value::as_f64)
            .map(|s| format!(" in {}", format_duration(s)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "Index: {files} file{}, {components} component{}{}",
            plural(files),
            plural(components),
            timing
        );
    }
}

fn str_val(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn num_val(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn str_array(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn display_name(raw: &str) -> String {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > 40 {
        format!("{}...", &collapsed[..37])
    } else {
        collapsed
    }
}

fn plural(count: u64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn format_duration(elapsed_secs: f64) -> String {
    if elapsed_secs >= 60.0 {
        let mins = (elapsed_secs / 60.0).floor() as u64;
        let secs_remainder = elapsed_secs - (mins as f64 * 60.0);
        format!("{}m {:.2}s", mins, secs_remainder)
    } else {
        format!("{:.2}s", elapsed_secs)
    }
}
