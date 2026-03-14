use std::fmt::Write;
use std::io::IsTerminal;

use serde_json::Value;

// ── ANSI styling ────────────────────────────────────────────────

struct Style {
    enabled: bool,
}

impl Style {
    fn for_stdout() -> Self {
        Self {
            enabled: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn for_stderr() -> Self {
        Self {
            enabled: std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn wrap(&self, text: &str, code: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn bold(&self, t: &str) -> String {
        self.wrap(t, "1")
    }
    fn dim(&self, t: &str) -> String {
        self.wrap(t, "2")
    }
    fn red(&self, t: &str) -> String {
        self.wrap(t, "31")
    }
    fn green(&self, t: &str) -> String {
        self.wrap(t, "32")
    }
    fn yellow(&self, t: &str) -> String {
        self.wrap(t, "33")
    }
    fn cyan(&self, t: &str) -> String {
        self.wrap(t, "36")
    }
    fn bold_red(&self, t: &str) -> String {
        self.wrap(t, "1;31")
    }
    fn bold_green(&self, t: &str) -> String {
        self.wrap(t, "1;32")
    }
}

/// Right-pad a styled string using its plain-text width for measurement.
fn pad_styled(styled: &str, plain_len: usize, width: usize) -> String {
    let pad = width.saturating_sub(plain_len);
    format!("{styled}{}", " ".repeat(pad))
}

// ── public API ──────────────────────────────────────────────────

pub fn render(payload: &Value) -> String {
    let s = Style::for_stdout();
    let command = payload.get("command").and_then(Value::as_str).unwrap_or("");
    match command {
        "sync" => render_sync(payload, &s),
        "add" => render_add(payload, &s),
        "remove" => render_remove(payload, &s),
        "search" => render_search(payload, &s),
        "list" => render_list(payload, &s),
        "deps" => render_deps(payload, &s),
        _ => serde_json::to_string_pretty(payload).unwrap_or_default(),
    }
}

pub fn render_error(message: &str) -> String {
    let s = Style::for_stderr();
    format!("{} {message}\n", s.bold_red("error:"))
}

// ── sync ────────────────────────────────────────────────────────

fn render_sync(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let scope = str_val(v, "scope");
    let verbose = v.get("verbose").and_then(Value::as_bool).unwrap_or(false);

    if scope == "full" {
        if let Some(idx) = v.get("index") {
            let files = num_val(idx, "file_count");
            let components = num_val(idx, "component_count");
            let languages = str_array(idx, "languages");
            let _ = write!(
                out,
                "{} {} file{}, {} component{}",
                s.bold("Indexed"),
                s.bold_green(&files.to_string()),
                plural(files),
                s.bold_green(&components.to_string()),
                plural(components)
            );
            if !languages.is_empty() {
                let colored: Vec<String> = languages.iter().map(|l| s.cyan(l)).collect();
                let _ = write!(out, " ({})", colored.join(", "));
            }
            if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
                let _ = write!(out, " in {}", s.dim(&format_duration(elapsed)));
            }
            let _ = writeln!(out);

            if verbose {
                write_component_breakdown(&mut out, idx, s);
            }
        }
    } else if scope == "partial" {
        if let Some(result) = v.get("result") {
            write_file_result(&mut out, result, s);
        }
        write_index_summary(&mut out, v, s);
        if verbose {
            if let Some(idx) = v.get("index") {
                write_component_breakdown(&mut out, idx, s);
            }
        }
    }

    out
}

fn write_component_breakdown(out: &mut String, idx: &Value, s: &Style) {
    let Some(breakdown) = idx.get("component_breakdown").and_then(Value::as_object) else {
        return;
    };
    for (lang, types) in breakdown {
        if let Some(types_obj) = types.as_object() {
            let _ = writeln!(out, "  {}:", s.bold(lang));
            let max_key = types_obj.keys().map(|k| k.len()).max().unwrap_or(0);
            for (ctype, count) in types_obj {
                let n = count.as_u64().unwrap_or(0);
                let padding = max_key.saturating_sub(ctype.len());
                let _ = writeln!(out, "    {}{}  {n}", s.cyan(ctype), " ".repeat(padding));
            }
        }
    }
}

// ── add ─────────────────────────────────────────────────────────

fn render_add(v: &Value, s: &Style) -> String {
    let mut out = String::new();

    if let Some(result) = v.get("result") {
        write_file_result(&mut out, result, s);
    } else {
        let filename = v
            .get("request")
            .and_then(|r| r.get("filename"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let _ = writeln!(out, "  {} {filename}", s.green("+"));
    }

    write_index_summary(&mut out, v, s);
    out
}

// ── remove ──────────────────────────────────────────────────────

fn render_remove(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let filename = v
        .get("request")
        .and_then(|r| r.get("filename"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    let removed = v.get("removed").and_then(Value::as_bool).unwrap_or(false);

    if removed {
        let _ = writeln!(out, "  {} {filename}", s.red("-"));
    } else {
        let _ = writeln!(out, "  {} {filename}", s.yellow("not indexed:"));
    }

    write_index_summary(&mut out, v, s);
    out
}

// ── search ──────────────────────────────────────────────────────

fn render_search(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let count = num_val(v, "result_count");

    if count == 0 {
        let _ = writeln!(out, "No results");
        return out;
    }

    if let Some(results) = v.get("results").and_then(Value::as_array) {
        let mut plain_labels = Vec::with_capacity(results.len());
        let mut styled_labels = Vec::with_capacity(results.len());
        let mut locations = Vec::with_capacity(results.len());
        let mut ids = Vec::with_capacity(results.len());

        for c in results {
            let name = display_name(&str_val(c, "name"));
            let ctype = str_val(c, "type");
            let file = str_val(c, "file");
            let start = num_val(c, "start_line");

            plain_labels.push(format!("{name} ({ctype})"));
            styled_labels.push(format!("{} ({})", s.bold(&name), s.cyan(&ctype)));
            locations.push(format!("{file}:{start}"));
            ids.push(str_val(c, "id"));
        }

        let max_label = plain_labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);

        for i in 0..results.len() {
            let padded_label = pad_styled(&styled_labels[i], plain_labels[i].len(), max_label);
            let padded_loc = pad_styled(&locations[i], locations[i].len(), max_loc);
            let _ = writeln!(out, "  {padded_label}  {padded_loc}  {}", s.dim(&ids[i]));
        }
    }

    let timing = v
        .get("elapsed_secs")
        .and_then(Value::as_f64)
        .map(|e| format!(" in {}", s.dim(&format_duration(e))))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "\n{}{}",
        s.bold(&format!("{count} result{}", plural(count))),
        timing
    );
    out
}

// ── list ────────────────────────────────────────────────────────

fn render_list(v: &Value, s: &Style) -> String {
    match str_val(v, "mode").as_str() {
        "languages" => render_list_languages(v, s),
        "language_summary" => render_list_summary(v, s),
        "language_all" => render_list_components(v, "all", s),
        "language_and_type" => render_list_components(v, &str_val(v, "type"), s),
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn render_list_languages(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let languages = str_array(v, "languages");

    if languages.is_empty() {
        let _ = writeln!(out, "No indexed languages");
        return out;
    }

    for lang in &languages {
        let _ = writeln!(out, "{}", s.bold(lang));
    }

    if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
        let _ = writeln!(
            out,
            "\n{} in {}",
            s.bold(&format!(
                "{} language{}",
                languages.len(),
                plural(languages.len() as u64)
            )),
            s.dim(&format_duration(elapsed))
        );
    }

    out
}

fn render_list_summary(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let language = str_val(v, "language");

    let Some(counts) = v.get("component_counts").and_then(Value::as_object) else {
        let _ = writeln!(out, "{}: no components", s.bold(&language));
        return out;
    };

    if counts.is_empty() {
        let _ = writeln!(out, "{}: no components", s.bold(&language));
        return out;
    }

    let max_key = counts.keys().map(|k| k.len()).max().unwrap_or(0);
    let total: u64 = counts.values().filter_map(Value::as_u64).sum();
    let count_width = total.to_string().len().max(1);

    let _ = writeln!(out, "{}:", s.bold(&language));
    for (ctype, count) in counts {
        let n = count.as_u64().unwrap_or(0);
        let padding = max_key.saturating_sub(ctype.len());
        let _ = writeln!(
            out,
            "  {}{}  {:>cw$}",
            s.cyan(ctype),
            " ".repeat(padding),
            n,
            cw = count_width
        );
    }

    let sep = "-".repeat(max_key + 2 + count_width);
    let _ = writeln!(out, "  {}", s.dim(&sep));
    let timing = v
        .get("elapsed_secs")
        .and_then(Value::as_f64)
        .map(|e| format!(" in {}", s.dim(&format_duration(e))))
        .unwrap_or_default();
    let total_pad = max_key.saturating_sub(5);
    let _ = writeln!(
        out,
        "  {}{}  {:>cw$}{}",
        s.bold("total"),
        " ".repeat(total_pad),
        total,
        timing,
        cw = count_width
    );

    out
}

fn render_list_components(v: &Value, label: &str, s: &Style) -> String {
    let mut out = String::new();
    let language = str_val(v, "language");
    let count = num_val(v, "count");
    let show_type = label == "all";

    let header_label = if show_type {
        label.to_string()
    } else {
        s.cyan(label)
    };
    let _ = writeln!(out, "{} {header_label}:", s.bold(&language));

    if count == 0 {
        let _ = writeln!(out, "  (none)");
        return out;
    }

    if let Some(components) = v.get("components").and_then(Value::as_array) {
        let mut plain_labels: Vec<String> = Vec::with_capacity(components.len());
        let mut styled_labels: Vec<String> = Vec::with_capacity(components.len());

        for c in components {
            let name = display_name(&str_val(c, "name"));
            if show_type {
                let ctype = str_val(c, "type");
                plain_labels.push(format!("{name} ({ctype})"));
                styled_labels.push(format!("{name} ({})", s.cyan(&ctype)));
            } else {
                plain_labels.push(name.clone());
                styled_labels.push(name);
            }
        }

        let max_label = plain_labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let mut current_file = String::new();

        for (i, component) in components.iter().enumerate() {
            let file = str_val(component, "file");

            if file != current_file {
                if !current_file.is_empty() {
                    let _ = writeln!(out);
                }
                let _ = writeln!(out, "  {}", s.bold(&file));
                current_file = file;
            }

            let start = num_val(component, "start_line");
            let id = str_val(component, "id");
            let padded = pad_styled(&styled_labels[i], plain_labels[i].len(), max_label);
            let _ = writeln!(out, "    {padded}  :{start}  {}", s.dim(&id));
        }
    }

    let timing = v
        .get("elapsed_secs")
        .and_then(Value::as_f64)
        .map(|e| format!(" in {}", s.dim(&format_duration(e))))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "\n{}{}",
        s.bold(&format!("{count} component{}", plural(count))),
        timing
    );
    out
}

// ── deps ────────────────────────────────────────────────────────

fn render_deps(v: &Value, s: &Style) -> String {
    let mut out = String::new();

    if let Some(component) = v.get("component") {
        let ctype = str_val(component, "type");
        let name = display_name(&str_val(component, "name"));
        let file = str_val(component, "file");
        let start = num_val(component, "start_line");
        let id = str_val(component, "id");

        let _ = writeln!(
            out,
            "{} ({})  {file}:{start}  {}",
            s.bold(&name),
            s.cyan(&ctype),
            s.dim(&id)
        );
    }

    if let Some(matrix) = v.get("dependency_matrix") {
        let before = matrix.get("before").and_then(Value::as_array);
        let after = matrix.get("after").and_then(Value::as_array);

        let before_empty = before.map(|a| a.is_empty()).unwrap_or(true);
        let after_empty = after.map(|a| a.is_empty()).unwrap_or(true);

        if let Some(nodes) = before {
            if !nodes.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "  {}:", s.bold("uses"));
                write_dep_nodes(&mut out, nodes, s);
            }
        }

        if let Some(nodes) = after {
            if !nodes.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "  {}:", s.bold("used by"));
                write_dep_nodes(&mut out, nodes, s);
            }
        }

        if before_empty && after_empty {
            let _ = writeln!(out, "\n  no dependencies");
        }
    }

    if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
        let _ = writeln!(
            out,
            "\n{}",
            s.dim(&format!("Resolved in {}", format_duration(elapsed)))
        );
    }

    out
}

fn write_dep_nodes(out: &mut String, nodes: &[Value], s: &Style) {
    let mut plain_labels = Vec::with_capacity(nodes.len());
    let mut styled_labels = Vec::with_capacity(nodes.len());
    let mut locations = Vec::with_capacity(nodes.len());
    let mut ids = Vec::with_capacity(nodes.len());

    for n in nodes {
        let ctype = str_val(n, "type");
        let name = display_name(&str_val(n, "name"));
        let file = str_val(n, "file");
        let start = num_val(n, "start_line");

        plain_labels.push(format!("{name} ({ctype})"));
        styled_labels.push(format!("{name} ({})", s.cyan(&ctype)));
        locations.push(format!("{file}:{start}"));
        ids.push(str_val(n, "id"));
    }

    let max_label = plain_labels.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_id = ids.iter().map(|i| i.len()).max().unwrap_or(0);

    for (i, node) in nodes.iter().enumerate() {
        let depth = num_val(node, "depth");
        let padded_label = pad_styled(&styled_labels[i], plain_labels[i].len(), max_label);
        let padded_loc = pad_styled(&locations[i], locations[i].len(), max_loc);

        if max_id > 0 {
            let dim_id = s.dim(&ids[i]);
            let padded_id = pad_styled(&dim_id, ids[i].len(), max_id);
            let _ = writeln!(
                out,
                "    {padded_label}  {padded_loc}  {padded_id}  {}",
                s.dim(&format!("depth:{depth}"))
            );
        } else {
            let _ = writeln!(
                out,
                "    {padded_label}  {padded_loc}  {}",
                s.dim(&format!("depth:{depth}"))
            );
        }
    }
}

// ── shared formatting helpers ───────────────────────────────────

fn write_file_result(out: &mut String, result: &Value, s: &Style) {
    let indexed = str_array(result, "indexed");
    for path in &indexed {
        let _ = writeln!(out, "  {} {path}", s.green("+"));
    }

    let removed = str_array(result, "removed");
    for path in &removed {
        let _ = writeln!(out, "  {} {path}", s.red("-"));
    }

    if let Some(skipped) = result.get("skipped").and_then(Value::as_array) {
        for entry in skipped {
            let path = str_val(entry, "path");
            let reason = str_val(entry, "reason");
            let _ = writeln!(out, "  {} {path} {} {reason}", s.yellow("~"), s.dim("--"));
        }
    }
}

fn write_index_summary(out: &mut String, v: &Value, s: &Style) {
    if let Some(idx) = v.get("index") {
        let files = num_val(idx, "file_count");
        let components = num_val(idx, "component_count");
        let timing = v
            .get("elapsed_secs")
            .and_then(Value::as_f64)
            .map(|e| format!(" in {}", s.dim(&format_duration(e))))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{} {files} file{}, {components} component{}{}",
            s.bold("Index:"),
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
