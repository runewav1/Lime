use std::fmt::Write;
use std::io::IsTerminal;

use serde_json::Value;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;

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

    fn gray(&self, t: &str) -> String {
        // "Bright black" (often renders as a readable gray).
        self.wrap(t, "90")
    }
}

/// Right-pad a styled string using its plain-text width for measurement.
fn pad_styled(styled: &str, plain_len: usize, width: usize) -> String {
    let pad = width.saturating_sub(plain_len);
    format!("{styled}{}", " ".repeat(pad))
}

// ── index staleness (git) ───────────────────────────────────────

fn staleness_from_payload(v: &Value) -> Option<&Value> {
    v.get("index_staleness")
        .or_else(|| v.get("index").and_then(|i| i.get("index_staleness")))
}

fn write_staleness_banner(out: &mut String, v: &Value, s: &Style) {
    let Some(st) = staleness_from_payload(v) else {
        return;
    };
    if !st.get("is_stale").and_then(Value::as_bool).unwrap_or(false) {
        return;
    }
    let reason = st
        .get("reason_short")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
        .unwrap_or("Index may be out of date; run `lime sync`.");
    let _ = writeln!(out, "{} {}", s.bold("warning:"), s.yellow(reason));
    let _ = writeln!(out);
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
        "link" => render_link(payload, &s),
        "list" => render_list(payload, &s),
        "deps" => render_deps(payload, &s),
        "annotate" => render_annotate(payload, &s),
        "show" => render_show(payload, &s),
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
    write_staleness_banner(&mut out, v, s);
    let scope = str_val(v, "scope");
    let verbose = v.get("verbose").and_then(Value::as_bool).unwrap_or(false);

    if scope == "full" {
        if let Some(idx) = v.get("index") {
            let files = num_val(idx, "file_count");
            let components = num_val(idx, "component_count");
            let batman_count = num_val(idx, "batman_count");
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
            if batman_count > 0 {
                let _ = write!(out, " {}", s.bold_red(&format!("[{batman_count} dead]")));
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
        write_index_summary(&mut out, v, s, true);
        if verbose {
            if let Some(idx) = v.get("index") {
                write_component_breakdown(&mut out, idx, s);
            }
        }
    }

    out
}

fn write_component_breakdown(out: &mut String, idx: &Value, s: &Style) {
    let batman_count = num_val(idx, "batman_count");
    if batman_count > 0 {
        let _ = writeln!(
            out,
            "  {} {}",
            s.bold_red("[dead]"),
            s.dim(&format!("{batman_count} flagged"))
        );
    }

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
    write_staleness_banner(&mut out, v, s);

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

    write_index_summary(&mut out, v, s, false);
    out
}

// ── remove ──────────────────────────────────────────────────────

fn render_remove(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    write_staleness_banner(&mut out, v, s);
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

    write_index_summary(&mut out, v, s, false);
    out
}

// ── search ──────────────────────────────────────────────────────

fn render_search(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    write_staleness_banner(&mut out, v, s);
    let count = num_val(v, "result_count");
    let fuzzy_mode = v.get("fuzzy").and_then(Value::as_bool).unwrap_or(false);

    if count == 0 {
        let _ = writeln!(out, "No results");
        if fuzzy_mode {
            let _ = writeln!(out, "{}", s.dim("(try without --fuzzy for exact matching)"));
        }
        return out;
    }

    if let Some(results) = v.get("results").and_then(Value::as_array) {
        let mut plain_labels = Vec::with_capacity(results.len());
        let mut styled_labels = Vec::with_capacity(results.len());
        let mut locations = Vec::with_capacity(results.len());
        let mut ids = Vec::with_capacity(results.len());
        let mut batman_flags = Vec::with_capacity(results.len());
        let mut match_types: Vec<String> = Vec::with_capacity(results.len());
        let mut annotation_previews: Vec<Option<String>> = Vec::with_capacity(results.len());

        for c in results {
            let name = display_name(&str_val(c, "name"));
            let ctype = str_val(c, "type");
            let file = str_val(c, "file");
            let start = num_val(c, "start_line");

            plain_labels.push(format!("{name} ({ctype})"));
            styled_labels.push(format!("{} ({})", s.bold(&name), s.cyan(&ctype)));
            locations.push(format!("{file}:{start}"));
            ids.push(str_val(c, "id"));
            batman_flags.push(bool_val(c, "batman"));
            match_types.push(str_val(c, "match_type"));
            annotation_previews.push(
                c.get("annotation_preview")
                    .and_then(Value::as_str)
                    .map(String::from),
            );
        }

        let max_label = plain_labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);

        for i in 0..results.len() {
            let padded_label = pad_styled(&styled_labels[i], plain_labels[i].len(), max_label);
            let padded_loc = pad_styled(&locations[i], locations[i].len(), max_loc);
            let _ = write!(out, "  {padded_label}  {padded_loc}  {}", s.dim(&ids[i]));

            if batman_flags[i] {
                let _ = write!(out, " {}", s.bold_red("[dead]"));
            }

            if fuzzy_mode && !match_types[i].is_empty() && match_types[i] != "exact" {
                let _ = write!(out, " {}", s.dim(&format!("({})", match_types[i])));
            }
            let _ = writeln!(out);

            if let Some(preview) = &annotation_previews[i] {
                if !preview.is_empty() {
                    let _ = writeln!(out, "    {}", s.dim(preview));
                }
            }
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

// ── link (annotation link labels) ───────────────────────────────

fn render_link(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    write_staleness_banner(&mut out, v, s);
    let count = num_val(v, "result_count");
    let notes = v.get("notes").and_then(Value::as_bool).unwrap_or(false);
    let link_label = str_val(v, "link");

    if count == 0 {
        let _ = writeln!(
            out,
            "No components with link {}",
            s.dim(&link_label)
        );
        return out;
    }

    let _ = writeln!(
        out,
        "{} {}",
        s.bold("link:"),
        s.cyan(&link_label)
    );
    if notes {
        let _ = writeln!(out, "{}", s.dim("(with full annotation notes)"));
    }
    let _ = writeln!(out);

    if let Some(results) = v.get("results").and_then(Value::as_array) {
        for entry in results {
            let comp = entry.get("component").unwrap_or(entry);
            let name = display_name(&str_val(comp, "name"));
            let ctype = str_val(comp, "type");
            let file = str_val(comp, "file");
            let start = num_val(comp, "start_line");
            let id = str_val(comp, "id");
            let _ = writeln!(
                out,
                "  {} ({})  {file}:{start}  {}",
                s.bold(&name),
                s.cyan(&ctype),
                s.dim(&id)
            );

            let links_json = entry.get("links").and_then(Value::as_array);
            let tags_json = entry.get("tags").and_then(Value::as_array);
            if let Some(arr) = links_json {
                let joined: Vec<String> = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|x| x.to_string())
                    .collect();
                if !joined.is_empty() {
                    let _ = writeln!(out, "    {} {}", s.dim("links:"), joined.join(", "));
                }
            }
            if let Some(arr) = tags_json {
                let joined: Vec<String> = arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|x| x.to_string())
                    .collect();
                if !joined.is_empty() {
                    let _ = writeln!(out, "    {} {}", s.dim("tags:"), joined.join(", "));
                }
            }

            if notes {
                if let Some(ann) = entry.get("annotation") {
                    let content = str_val(ann, "content");
                    for line in content.lines() {
                        let _ = writeln!(out, "    {line}");
                    }
                    if content.is_empty() {
                        let _ = writeln!(out, "    {}", s.dim("(empty)"));
                    }
                }
            } else {
                let preview = str_val(entry, "annotation_preview");
                if !preview.is_empty() {
                    let _ = writeln!(out, "    {}", s.dim(&preview));
                }
            }
            let _ = writeln!(out);
        }
    }

    let timing = v
        .get("elapsed_secs")
        .and_then(Value::as_f64)
        .map(|e| format!(" in {}", s.dim(&format_duration(e))))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{}{}",
        s.bold(&format!("{count} component{}", plural(count))),
        timing
    );
    out
}

// ── list ────────────────────────────────────────────────────────

fn render_list(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    write_staleness_banner(&mut out, v, s);
    let body = match str_val(v, "mode").as_str() {
        "languages" => render_list_languages(v, s),
        "language_summary" => render_list_summary(v, s),
        "language_all" => render_list_components(v, "all", s),
        "language_and_type" => render_list_components(v, &str_val(v, "type"), s),
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    out + &body
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

    let dead = num_val(v, "dead");
    let faulty = num_val(v, "faulty");
    if dead > 0 {
        let _ = writeln!(out, "  {}", s.bold_red(&format!("{dead} dead")));
    }
    if faulty > 0 {
        let _ = writeln!(out, "  {}", s.bold_red(&format!("{faulty} faulty")));
    }

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
        if !components.is_empty() {
            let term_w = terminal_width();
            let (max_id_len, _max_line, max_marker) = list_precompute_layout(components);
            let colon_col = list_colon_column(term_w, max_id_len, max_marker);

            let mut idx = 0usize;
            let mut first_file = true;
            while idx < components.len() {
                let file = str_val(&components[idx], "file");
                let mut end = idx + 1;
                while end < components.len() && str_val(&components[end], "file") == file {
                    end += 1;
                }
                let chunk = &components[idx..end];

                if !first_file {
                    let _ = writeln!(out);
                }
                first_file = false;
                let _ = writeln!(out, "  {}", s.bold(&file));

                for component in chunk {
                    let dp = str_val(component, "display_path");
                    let depth = list_nesting_depth_from_display_path(&dp, &language);
                    let base_indent = 4usize;
                    let extra = depth.saturating_mul(2);
                    let indent = base_indent + extra;

                    let label_slot = list_label_slot_width(indent, colon_col);
                    let label_core_raw = if depth == 0 {
                        str_val(component, "name")
                    } else {
                        dp
                    };

                    let (plain_label, styled_label) = if show_type {
                        let ctype = str_val(component, "type");
                        let prefix_plain = format!("({ctype}) ");
                        let prefix_w = prefix_plain.chars().count();
                        let name_budget = label_slot.saturating_sub(prefix_w).max(1);
                        let name_part = truncate_chars_ellipsis(&label_core_raw, name_budget);
                        let plain = format!("{prefix_plain}{name_part}");
                        let styled = format!("({}) {}", s.cyan(&ctype), name_part);
                        (plain, styled)
                    } else {
                        let plain = truncate_chars_ellipsis(&label_core_raw, label_slot);
                        (plain.clone(), plain)
                    };

                    // Pad using plain char count (styled adds ANSI; same visible length as plain).
                    let pad = label_slot.saturating_sub(plain_label.chars().count());
                    let padded_styled = format!("{styled_label}{}", " ".repeat(pad));

                    let start = num_val(component, "start_line");
                    let id = str_val(component, "id");
                    let batman = bool_val(component, "batman");
                    let batman_marker = if batman {
                        format!(" {}", s.bold_red("[dead]"))
                    } else {
                        String::new()
                    };
                    let fault_total = component
                        .get("faults")
                        .and_then(|f| {
                            let e = f.get("errors").and_then(Value::as_u64).unwrap_or(0);
                            let w = f.get("warnings").and_then(Value::as_u64).unwrap_or(0);
                            let n = f.get("notes").and_then(Value::as_u64).unwrap_or(0);
                            let t = e + w + n;
                            if t > 0 { Some(t) } else { None }
                        });
                    let fault_marker = fault_total
                        .map(|n| format!(" {}", s.bold_red(&format!("[{n} fault{}]", plural(n)))))
                        .unwrap_or_default();

                    let id_padded = pad_plain_right(&id, max_id_len);
                    let id_dim = s.dim(&id_padded);
                    let line_part = format!(
                        ":{:>lw$}",
                        start,
                        lw = LIST_LINE_NUMBER_COL_WIDTH
                    );
                    let _ = writeln!(
                        out,
                        "{}{}  {}  {}{}{}",
                        " ".repeat(indent),
                        padded_styled,
                        line_part,
                        id_dim,
                        batman_marker,
                        fault_marker
                    );
                }

                idx = end;
            }
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

// ── show ────────────────────────────────────────────────────────

fn render_show(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    out.push('\n');
    write_staleness_banner(&mut out, v, s);

    if let Some(component) = v.get("component") {
        let name = component_label_for_display(component);
        let ctype = str_val(component, "type");
        let file = str_val(component, "file");
        let start = num_val(component, "start_line");
        let end = num_val(component, "end_line");
        let id = str_val(component, "id");
        let death = str_val(component, "death_status");
        let dead = !death.is_empty() && death != "alive";
        let dead_marker = if dead {
            format!(" {}", s.bold_red("[dead]"))
        } else {
            String::new()
        };
        let fault_total = component
            .get("faults")
            .and_then(|f| {
                let e = f.get("errors").and_then(Value::as_u64).unwrap_or(0);
                let w = f.get("warnings").and_then(Value::as_u64).unwrap_or(0);
                let n = f.get("notes").and_then(Value::as_u64).unwrap_or(0);
                let t = e + w + n;
                if t > 0 { Some(t) } else { None }
            });
        let fault_marker = fault_total
            .map(|n| format!(" {}", s.bold_red(&format!("[{n} fault{}]", plural(n)))))
            .unwrap_or_default();

        let _ = writeln!(
            out,
            "{} ({})  {file}:{start}-{end}  {}{}{}",
            s.bold(&name),
            s.cyan(&ctype),
            s.dim(&id),
            dead_marker,
            fault_marker,
        );
    }

    if bool_val(v, "file_changed") {
        let _ = writeln!(out, "{}", s.yellow("(file changed since last sync)"));
    }

    let _ = writeln!(out);

    if let Some(source_lines) = v.get("source_lines").and_then(Value::as_array) {
        let lang = v
            .get("component")
            .and_then(|c| c.get("language"))
            .and_then(Value::as_str)
            .unwrap_or("");

        let ss = SyntaxSet::load_defaults_newlines();
        let ts = ThemeSet::load_defaults();
        let theme = &ts.themes["base16-ocean.dark"];
        let syntax_ext = match lang {
            "rust" => "rs",
            "python" => "py",
            "javascript" => "js",
            "typescript" => "ts",
            "go" => "go",
            "c" => "c",
            "cpp" => "cpp",
            "zig" => "zig",
            "swift" => "swift",
            _ => "txt",
        };
        let syntax = ss
            .find_syntax_by_extension(syntax_ext)
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(syntax, theme);

        let max_line_num = source_lines
            .last()
            .and_then(|l| l.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let line_width = max_line_num.to_string().len().max(1);

        for entry in source_lines {
            let line_num = entry.get("line").and_then(Value::as_u64).unwrap_or(0);
            let code = entry.get("code").and_then(Value::as_str).unwrap_or("");
            let diags = entry.get("diagnostics").and_then(Value::as_array);

            let highlighted = if s.enabled {
                let line_with_nl = format!("{code}\n");
                let ranges = highlighter
                    .highlight_line(&line_with_nl, &ss)
                    .unwrap_or_default();
                let escaped = as_24_bit_terminal_escaped(&ranges, false);
                escaped.trim_end().to_string()
            } else {
                code.to_string()
            };

            let _ = writeln!(
                out,
                " {:>lw$} {} {}",
                line_num,
                s.dim("|"),
                highlighted,
                lw = line_width,
            );

            if let Some(arr) = diags {
                for d in arr {
                    let sev = str_val(d, "severity");
                    let msg = str_val(d, "message");
                    let sev_label = match sev.as_str() {
                        "error" => s.bold_red("error"),
                        "warning" => s.yellow("warning"),
                        "note" => s.cyan("note"),
                        _ => s.dim(&sev),
                    };
                    let _ = writeln!(
                        out,
                        " {:>lw$} {} {} {}",
                        "",
                        s.dim("|"),
                        sev_label,
                        s.gray(&msg),
                        lw = line_width,
                    );
                }
            }
        }
    }

    if let Some(ann) = v.get("annotation") {
        if !ann.is_null() {
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", s.bold("Annotation:"));
            let content = str_val(ann, "content");
            if content.is_empty() {
                let _ = writeln!(out, "  {}", s.dim("(empty)"));
            } else {
                for line in content.lines() {
                    let _ = writeln!(out, "  {line}");
                }
            }
        }
    }

    out.push('\n');
    out
}

// ── deps ────────────────────────────────────────────────────────

fn render_deps(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    write_staleness_banner(&mut out, v, s);

    if let Some(component) = v.get("component") {
        let ctype = str_val(component, "type");
        let name = display_name(&str_val(component, "name"));
        let file = str_val(component, "file");
        let start = num_val(component, "start_line");
        let id = str_val(component, "id");
        let batman = bool_val(component, "batman");
        let batman_marker = if batman {
            format!(" {}", s.bold_red("[dead]"))
        } else {
            String::new()
        };

        let _ = writeln!(
            out,
            "{} ({})  {file}:{start}  {}{}",
            s.bold(&name),
            s.cyan(&ctype),
            s.dim(&id),
            batman_marker
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
    let mut batman_flags = Vec::with_capacity(nodes.len());

    for n in nodes {
        let ctype = str_val(n, "type");
        let name = display_name(&str_val(n, "name"));
        let file = str_val(n, "file");
        let start = num_val(n, "start_line");

        plain_labels.push(format!("{name} ({ctype})"));
        styled_labels.push(format!("{name} ({})", s.cyan(&ctype)));
        locations.push(format!("{file}:{start}"));
        ids.push(str_val(n, "id"));
        batman_flags.push(bool_val(n, "batman"));
    }

    let max_label = plain_labels.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);
    let max_id = ids.iter().map(|i| i.len()).max().unwrap_or(0);

    for (i, node) in nodes.iter().enumerate() {
        let depth = num_val(node, "depth");
        let padded_label = pad_styled(&styled_labels[i], plain_labels[i].len(), max_label);
        let padded_loc = pad_styled(&locations[i], locations[i].len(), max_loc);
        let batman_marker = if batman_flags[i] {
            format!(" {}", s.bold_red("[dead]"))
        } else {
            String::new()
        };

        if max_id > 0 {
            let dim_id = s.dim(&ids[i]);
            let padded_id = pad_styled(&dim_id, ids[i].len(), max_id);
            let _ = writeln!(
                out,
                "    {padded_label}  {padded_loc}  {padded_id}  {}{}",
                s.dim(&format!("depth:{depth}")),
                batman_marker
            );
        } else {
            let _ = writeln!(
                out,
                "    {padded_label}  {padded_loc}  {}{}",
                s.dim(&format!("depth:{depth}")),
                batman_marker
            );
        }
    }
}

// ── annotate ────────────────────────────────────────────────────

fn render_annotate(v: &Value, s: &Style) -> String {
    match str_val(v, "action").as_str() {
        "add" => render_annotate_add(v, s),
        "show" => render_annotate_show(v, s),
        "list" => render_annotate_list(v, s),
        "remove" => render_annotate_remove(v, s),
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn render_annotate_add(v: &Value, s: &Style) -> String {
    let mut out = String::new();

    if let Some(component) = v.get("component") {
        let name = display_name(&str_val(component, "name"));
        let ctype = str_val(component, "type");
        let id = str_val(component, "id");
        let _ = writeln!(
            out,
            "  {} {} ({})  {}",
            s.green("+"),
            s.bold(&name),
            s.cyan(&ctype),
            s.dim(&id)
        );
    }

    if let Some(ann) = v.get("annotation") {
        let content = str_val(ann, "content");
        let preview = if content.len() > 80 {
            format!("{}...", &content[..77])
        } else {
            content
        };
        let _ = writeln!(out, "    {}", s.dim(&preview));
        if let Some(arr) = ann.get("links").and_then(Value::as_array) {
            let joined: Vec<String> = arr
                .iter()
                .filter_map(Value::as_str)
                .map(|x| x.to_string())
                .collect();
            if !joined.is_empty() {
                let _ = writeln!(out, "    {} {}", s.dim("links:"), joined.join(", "));
            }
        }
    }

    if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
        let _ = writeln!(
            out,
            "\n{}",
            s.dim(&format!("Saved in {}", format_duration(elapsed)))
        );
    }

    out
}

fn render_annotate_show(v: &Value, s: &Style) -> String {
    let mut out = String::new();

    if let Some(component) = v.get("component") {
        let name = display_name(&str_val(component, "name"));
        let ctype = str_val(component, "type");
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

    if let Some(ann) = v.get("annotation") {
        let created = str_val(ann, "created_at");
        let updated = str_val(ann, "updated_at");
        let _ = writeln!(
            out,
            "  {} {}  {} {}",
            s.dim("created:"),
            s.dim(&created),
            s.dim("updated:"),
            s.dim(&updated)
        );
        if let Some(arr) = ann.get("tags").and_then(Value::as_array) {
            let joined: Vec<String> = arr
                .iter()
                .filter_map(Value::as_str)
                .map(|x| x.to_string())
                .collect();
            if !joined.is_empty() {
                let _ = writeln!(out, "  {} {}", s.dim("tags:"), joined.join(", "));
            }
        }
        if let Some(arr) = ann.get("links").and_then(Value::as_array) {
            let joined: Vec<String> = arr
                .iter()
                .filter_map(Value::as_str)
                .map(|x| x.to_string())
                .collect();
            if !joined.is_empty() {
                let _ = writeln!(out, "  {} {}", s.dim("links:"), joined.join(", "));
            }
        }
        let _ = writeln!(out);
        let content = str_val(ann, "content");
        for line in content.lines() {
            let _ = writeln!(out, "  {line}");
        }
        if content.is_empty() {
            let _ = writeln!(out, "  {}", s.dim("(empty)"));
        }
    }

    if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
        let _ = writeln!(
            out,
            "\n{}",
            s.dim(&format!("Loaded in {}", format_duration(elapsed)))
        );
    }

    out
}

fn render_annotate_list(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let count = num_val(v, "count");

    if count == 0 {
        let _ = writeln!(out, "No annotations");
        return out;
    }

    if let Some(results) = v.get("results").and_then(Value::as_array) {
        let mut plain_labels = Vec::with_capacity(results.len());
        let mut styled_labels = Vec::with_capacity(results.len());
        let mut locations = Vec::with_capacity(results.len());
        let mut previews = Vec::with_capacity(results.len());

        for entry in results {
            let comp = entry.get("component").unwrap_or(entry);
            let ann = entry.get("annotation").unwrap_or(entry);

            let name = display_name(&str_val(comp, "name"));
            let ctype = str_val(comp, "type");
            let file = str_val(comp, "file");
            let start = num_val(comp, "start_line");

            plain_labels.push(format!("{name} ({ctype})"));
            styled_labels.push(format!("{} ({})", s.bold(&name), s.cyan(&ctype)));
            locations.push(format!("{file}:{start}"));
            previews.push(str_val(ann, "preview"));
        }

        let max_label = plain_labels.iter().map(|l| l.len()).max().unwrap_or(0);
        let max_loc = locations.iter().map(|l| l.len()).max().unwrap_or(0);

        for i in 0..results.len() {
            let padded_label = pad_styled(&styled_labels[i], plain_labels[i].len(), max_label);
            let padded_loc = pad_styled(&locations[i], locations[i].len(), max_loc);
            let _ = writeln!(out, "  {padded_label}  {padded_loc}");
            if !previews[i].is_empty() {
                let _ = writeln!(out, "    {}", s.dim(&previews[i]));
            }
            if let Some(entry) = results.get(i) {
                if let Some(arr) = entry
                    .get("annotation")
                    .and_then(|a| a.get("links"))
                    .and_then(Value::as_array)
                {
                    let joined: Vec<String> = arr
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|x| x.to_string())
                        .collect();
                    if !joined.is_empty() {
                        let _ = writeln!(out, "    {} {}", s.dim("links:"), joined.join(", "));
                    }
                }
            }
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
        s.bold(&format!("{count} annotation{}", plural(count))),
        timing
    );
    out
}

fn render_annotate_remove(v: &Value, s: &Style) -> String {
    let mut out = String::new();
    let component_id = str_val(v, "component_id");
    let removed = v.get("removed").and_then(Value::as_bool).unwrap_or(false);

    if removed {
        let _ = writeln!(out, "  {} {component_id}", s.red("-"));
    } else {
        let _ = writeln!(out, "  {} {component_id}", s.yellow("not found:"));
    }

    if let Some(elapsed) = v.get("elapsed_secs").and_then(Value::as_f64) {
        let _ = writeln!(out, "\n{}", s.dim(&format_duration(elapsed)));
    }

    out
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

fn write_index_summary(out: &mut String, v: &Value, s: &Style, include_batman: bool) {
    if let Some(idx) = v.get("index") {
        let files = num_val(idx, "file_count");
        let components = num_val(idx, "component_count");
        let batman_count = num_val(idx, "batman_count");
        let batman_suffix = if include_batman && batman_count > 0 {
            format!(" {}", s.bold_red(&format!("[{batman_count} dead]")))
        } else {
            String::new()
        };
        let timing = v
            .get("elapsed_secs")
            .and_then(Value::as_f64)
            .map(|e| format!(" in {}", s.dim(&format_duration(e))))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{} {files} file{}, {components} component{}{}{}",
            s.bold("Index:"),
            plural(files),
            plural(components),
            batman_suffix,
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

fn bool_val(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
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

/// Fixed width for the `start_line` field after `:` (right-aligned digits).
const LIST_LINE_NUMBER_COL_WIDTH: usize = 6;

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(48, 256)
}

/// Plain-text length of optional `[dead]` / `[N faults]` suffix for a row.
fn list_marker_plain_len(c: &Value) -> usize {
    let mut n = 0usize;
    if bool_val(c, "batman") {
        n += 6; // " [dead]"
    }
    let fault_total = c.get("faults").and_then(|f| {
        let e = f.get("errors").and_then(Value::as_u64).unwrap_or(0);
        let w = f.get("warnings").and_then(Value::as_u64).unwrap_or(0);
        let note = f.get("notes").and_then(Value::as_u64).unwrap_or(0);
        let t = e + w + note;
        if t > 0 { Some(t) } else { None }
    });
    if let Some(t) = fault_total {
        n += 2 + format!("[{t} fault{}]", plural(t)).len();
    }
    n
}

fn list_precompute_layout(components: &[Value]) -> (usize, u64, usize) {
    let mut max_id_len = 0usize;
    let mut max_line = 0u64;
    let mut max_marker = 0usize;
    for c in components {
        max_id_len = max_id_len.max(str_val(c, "id").chars().count());
        max_line = max_line.max(num_val(c, "start_line"));
        max_marker = max_marker.max(list_marker_plain_len(c));
    }
    (max_id_len, max_line, max_marker)
}

/// Width of the fixed region **starting at `:`** through end-of-line (`:` + line + gap + id + markers).
fn list_width_after_colon(max_id_len: usize, max_marker: usize) -> usize {
    // `:` + line_digits + `  ` + id + markers
    1 + LIST_LINE_NUMBER_COL_WIDTH + 2 + max_id_len + max_marker
}

/// 0-based column index of the `:` that follows the label and two spaces (`  `).
fn list_colon_column(term_w: usize, max_id_len: usize, max_marker: usize) -> usize {
    term_w.saturating_sub(list_width_after_colon(max_id_len, max_marker))
}

/// Visible width available for the label (after indent, before `  :`).
fn list_label_slot_width(indent_cols: usize, colon_col: usize) -> usize {
    // indent + label + "  " + ":" ... => label ends at colon_col - 2
    colon_col.saturating_sub(indent_cols).saturating_sub(2).max(1)
}

fn collapse_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate to at most `max_chars` Unicode scalars; if truncated, append `...`.
fn truncate_chars_ellipsis(raw: &str, max_chars: usize) -> String {
    let collapsed = collapse_whitespace(raw);
    let count = collapsed.chars().count();
    if count <= max_chars {
        return collapsed;
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }
    let keep = max_chars.saturating_sub(3);
    let truncated: String = collapsed.chars().take(keep).collect();
    format!("{truncated}...")
}

fn pad_plain_right(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - len))
}

/// Prefer `display_path` (nested qualified name) when present; otherwise bare `name`.
fn component_label_for_display(c: &Value) -> String {
    let dp = str_val(c, "display_path");
    if !dp.is_empty() {
        display_name(&dp)
    } else {
        display_name(&str_val(c, "name"))
    }
}

/// How many levels deep a component is nested in its file (0 = top-level). Matches index `display_path` rules.
fn list_nesting_depth_from_display_path(display_path: &str, language: &str) -> usize {
    if display_path.is_empty() {
        return 0;
    }
    let sep = match language {
        "python" | "javascript" | "typescript" | "go" | "swift" => ".",
        _ => "::",
    };
    display_path.matches(sep).count()
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
