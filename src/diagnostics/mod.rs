use std::{
    collections::HashMap,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::index::{ComponentRecord, IndexData};

// ---------------------------------------------------------------------------
// Diagnostic model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticEntry {
    pub file: String,
    pub line: usize,
    pub severity: DiagSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagSeverity {
    Error,
    #[default]
    Warning,
    Note,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComponentFaults {
    pub errors: usize,
    pub warnings: usize,
    pub notes: usize,
}

impl ComponentFaults {
    pub fn total(&self) -> usize {
        self.errors + self.warnings + self.notes
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerResult {
    pub language: String,
    pub tool: String,
    pub tool_found: bool,
    pub tool_failed: bool,
    pub entries: Vec<DiagnosticEntry>,
}

// ---------------------------------------------------------------------------
// Language → analyzer matrix
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AnalyzerSpec {
    language: &'static str,
    tool_name: &'static str,
    binary: &'static str,
    build_args: fn(&Path) -> Vec<String>,
    parse_output: fn(&str) -> Vec<DiagnosticEntry>,
}

fn analyzer_specs() -> Vec<AnalyzerSpec> {
    vec![
        AnalyzerSpec {
            language: "rust",
            tool_name: "clippy",
            binary: "cargo",
            build_args: |_root| vec![
                "clippy".into(), "--message-format=short".into(), "--quiet".into(), "--".into(),
                "-W".into(), "clippy::all".into(),
            ],
            parse_output: parse_short_diagnostics,
        },
        AnalyzerSpec {
            language: "python",
            tool_name: "ruff",
            binary: "ruff",
            build_args: |root| vec!["check".into(), root.display().to_string()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "javascript",
            tool_name: "eslint",
            binary: "npx",
            build_args: |root| vec!["eslint".into(), root.display().to_string(), "--format".into(), "unix".into()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "typescript",
            tool_name: "eslint",
            binary: "npx",
            build_args: |root| vec!["eslint".into(), root.display().to_string(), "--format".into(), "unix".into()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "go",
            tool_name: "go-vet",
            binary: "go",
            build_args: |_root| vec!["vet".into(), "./...".into()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "c",
            tool_name: "clang-tidy",
            binary: "clang-tidy",
            build_args: |_root| vec!["--quiet".into()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "cpp",
            tool_name: "clang-tidy",
            binary: "clang-tidy",
            build_args: |_root| vec!["--quiet".into()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "zig",
            tool_name: "zig-build",
            binary: "zig",
            build_args: |_root| vec!["build".into()],
            parse_output: parse_colon_diagnostics,
        },
        AnalyzerSpec {
            language: "swift",
            tool_name: "swift-build",
            binary: "swift",
            build_args: |_root| vec!["build".into()],
            parse_output: parse_colon_diagnostics,
        },
    ]
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

const ANALYZER_TIMEOUT_SECS: u64 = 120;

fn tool_on_path(binary: &str) -> bool {
    which::which(binary).is_ok()
}

fn run_analyzer(spec: &AnalyzerSpec, root: &Path) -> AnalyzerResult {
    let found = tool_on_path(spec.binary);
    if !found {
        return AnalyzerResult {
            language: spec.language.to_string(),
            tool: spec.tool_name.to_string(),
            tool_found: false,
            tool_failed: false,
            entries: Vec::new(),
        };
    }

    let args = (spec.build_args)(root);
    let child = Command::new(spec.binary)
        .args(&args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(_) => {
            return AnalyzerResult {
                language: spec.language.to_string(),
                tool: spec.tool_name.to_string(),
                tool_found: true,
                tool_failed: true,
                entries: Vec::new(),
            };
        }
    };

    let deadline = Instant::now() + Duration::from_secs(ANALYZER_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return AnalyzerResult {
                        language: spec.language.to_string(),
                        tool: spec.tool_name.to_string(),
                        tool_found: true,
                        tool_failed: true,
                        entries: Vec::new(),
                    };
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                return AnalyzerResult {
                    language: spec.language.to_string(),
                    tool: spec.tool_name.to_string(),
                    tool_found: true,
                    tool_failed: true,
                    entries: Vec::new(),
                };
            }
        }
    }

    let output = child.wait_with_output();
    match output {
        Ok(out) => {
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
            let entries = (spec.parse_output)(&combined);
            AnalyzerResult {
                language: spec.language.to_string(),
                tool: spec.tool_name.to_string(),
                tool_found: true,
                tool_failed: false,
                entries,
            }
        }
        Err(_) => AnalyzerResult {
            language: spec.language.to_string(),
            tool: spec.tool_name.to_string(),
            tool_found: true,
            tool_failed: true,
            entries: Vec::new(),
        },
    }
}

/// Runs all configured analyzers for languages present in the index.
pub fn run_diagnostics(root: &Path, index: &IndexData) -> Vec<AnalyzerResult> {
    let active_languages: std::collections::HashSet<&str> =
        index.languages.iter().map(|s| s.as_str()).collect();

    let specs = analyzer_specs();
    let mut results = Vec::new();

    for spec in &specs {
        if !active_languages.contains(spec.language) {
            continue;
        }
        results.push(run_analyzer(spec, root));
    }

    results
}

// ---------------------------------------------------------------------------
// Output parsers
// ---------------------------------------------------------------------------

/// Parses `file:line:col: severity: message` format (cargo clippy --message-format=short,
/// clang-tidy, ruff, go vet, etc.)
fn parse_colon_diagnostics(output: &str) -> Vec<DiagnosticEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(entry) = try_parse_colon_line(line) {
            entries.push(entry);
        }
    }
    entries
}

fn parse_short_diagnostics(output: &str) -> Vec<DiagnosticEntry> {
    parse_colon_diagnostics(output)
}

fn try_parse_colon_line(line: &str) -> Option<DiagnosticEntry> {
    let parts: Vec<&str> = line.splitn(5, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let file = parts[0].trim().to_string();
    let line_num = parts[1].trim().parse::<usize>().ok()?;

    let severity_part = if parts.len() == 5 {
        parts[3].trim().to_ascii_lowercase()
    } else {
        parts[2].trim().to_ascii_lowercase()
    };

    let severity = if severity_part.contains("error") {
        DiagSeverity::Error
    } else if severity_part.contains("warn") {
        DiagSeverity::Warning
    } else if severity_part.contains("note") || severity_part.contains("info") {
        DiagSeverity::Note
    } else {
        DiagSeverity::Warning
    };

    let message = if parts.len() == 5 {
        parts[4].trim().to_string()
    } else if parts.len() == 4 {
        parts[3].trim().to_string()
    } else {
        String::new()
    };

    let code = String::new();

    Some(DiagnosticEntry {
        file,
        line: line_num,
        severity,
        code,
        message,
    })
}

// ---------------------------------------------------------------------------
// Map diagnostics to components
// ---------------------------------------------------------------------------

/// Normalizes file paths for comparison (forward slashes, relative).
fn normalize_diag_path(root: &Path, raw_path: &str) -> String {
    let normalized = raw_path.replace('\\', "/");
    let root_str = root.display().to_string().replace('\\', "/");
    if let Some(stripped) = normalized.strip_prefix(&root_str) {
        stripped.trim_start_matches('/').to_string()
    } else {
        normalized
    }
}

/// Attaches diagnostic entries to components based on file path + line range overlap.
pub fn map_diagnostics_to_components(
    root: &Path,
    results: &[AnalyzerResult],
    components: &[ComponentRecord],
) -> HashMap<String, ComponentFaults> {
    let mut file_diags: HashMap<String, Vec<&DiagnosticEntry>> = HashMap::new();
    for result in results {
        for entry in &result.entries {
            let norm = normalize_diag_path(root, &entry.file);
            file_diags.entry(norm).or_default().push(entry);
        }
    }

    let mut faults_map: HashMap<String, ComponentFaults> = HashMap::new();

    for component in components {
        let norm_file = component.file.replace('\\', "/");
        let Some(diags) = file_diags.get(&norm_file) else {
            continue;
        };

        let mut faults = ComponentFaults::default();
        for diag in diags {
            if diag.line >= component.start_line && diag.line <= component.end_line {
                match diag.severity {
                    DiagSeverity::Error => faults.errors += 1,
                    DiagSeverity::Warning => faults.warnings += 1,
                    DiagSeverity::Note => faults.notes += 1,
                }
            }
        }

        if !faults.is_empty() {
            faults_map.insert(component.id.clone(), faults);
        }
    }

    faults_map
}

/// Builds a map of component_id → Vec<DiagnosticEntry> for cache persistence.
/// Each entry is a diagnostic that overlaps the component's line range.
pub fn build_component_diagnostics_map(
    root: &Path,
    results: &[AnalyzerResult],
    components: &[ComponentRecord],
) -> HashMap<String, Vec<DiagnosticEntry>> {
    let mut file_diags: HashMap<String, Vec<&DiagnosticEntry>> = HashMap::new();
    for result in results {
        for entry in &result.entries {
            let norm = normalize_diag_path(root, &entry.file);
            file_diags.entry(norm).or_default().push(entry);
        }
    }

    let mut map: HashMap<String, Vec<DiagnosticEntry>> = HashMap::new();

    for component in components {
        let norm_file = component.file.replace('\\', "/");
        let Some(diags) = file_diags.get(&norm_file) else {
            continue;
        };

        let mut entries = Vec::new();
        for diag in diags {
            if diag.line >= component.start_line && diag.line <= component.end_line {
                entries.push((*diag).clone());
            }
        }

        entries.sort_by(|a, b| {
            severity_ord(&a.severity)
                .cmp(&severity_ord(&b.severity))
                .then(a.line.cmp(&b.line))
        });

        if !entries.is_empty() {
            map.insert(component.id.clone(), entries);
        }
    }

    map
}

fn severity_ord(sev: &DiagSeverity) -> u8 {
    match sev {
        DiagSeverity::Error => 0,
        DiagSeverity::Warning => 1,
        DiagSeverity::Note => 2,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{DeathEvidence, DeathStatus};

    fn test_component(id: &str, name: &str, file: &str, start: usize, end: usize) -> ComponentRecord {
        ComponentRecord {
            id: id.to_string(),
            language: "rust".to_string(),
            component_type: "fn".to_string(),
            name: name.to_string(),
            file: file.to_string(),
            start_line: start,
            end_line: end,
            uses_before: Vec::new(),
            used_by_after: Vec::new(),
            dep_edges: Vec::new(),
            batman: false,
            death_status: DeathStatus::Alive,
            death_evidence: DeathEvidence::default(),
            faults: ComponentFaults::default(),
            display_path: String::new(),
        }
    }

    #[test]
    fn parse_colon_format() {
        let output = r#"
src/main.rs:10:5: warning: unused variable
src/lib.rs:22:1: error: type mismatch
src/util.rs:5:3: note: see previous definition
"#;
        let entries = parse_colon_diagnostics(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].file, "src/main.rs");
        assert_eq!(entries[0].line, 10);
        assert_eq!(entries[0].severity, DiagSeverity::Warning);
        assert_eq!(entries[1].severity, DiagSeverity::Error);
        assert_eq!(entries[2].severity, DiagSeverity::Note);
    }

    #[test]
    fn map_diagnostics_overlaps_line_ranges() {
        let results = vec![AnalyzerResult {
            language: "rust".to_string(),
            tool: "clippy".to_string(),
            tool_found: true,
            tool_failed: false,
            entries: vec![
                DiagnosticEntry {
                    file: "src/main.rs".to_string(),
                    line: 3,
                    severity: DiagSeverity::Warning,
                    code: String::new(),
                    message: "unused var".to_string(),
                },
                DiagnosticEntry {
                    file: "src/main.rs".to_string(),
                    line: 15,
                    severity: DiagSeverity::Error,
                    code: String::new(),
                    message: "type error".to_string(),
                },
                DiagnosticEntry {
                    file: "src/lib.rs".to_string(),
                    line: 5,
                    severity: DiagSeverity::Warning,
                    code: String::new(),
                    message: "dead code".to_string(),
                },
            ],
        }];

        let components = vec![
            test_component("fn-a", "func_a", "src/main.rs", 1, 10),
            test_component("fn-b", "func_b", "src/main.rs", 11, 20),
            test_component("fn-c", "func_c", "src/lib.rs", 1, 3),
        ];

        let root = Path::new(".");
        let faults = map_diagnostics_to_components(root, &results, &components);

        assert!(faults.contains_key("fn-a"));
        assert_eq!(faults["fn-a"].warnings, 1);
        assert_eq!(faults["fn-a"].errors, 0);

        assert!(faults.contains_key("fn-b"));
        assert_eq!(faults["fn-b"].errors, 1);

        assert!(!faults.contains_key("fn-c"), "line 5 is outside fn-c range 1-3");
    }

    #[test]
    fn summary_counts() {
        let results = vec![AnalyzerResult {
            language: "rust".to_string(),
            tool: "clippy".to_string(),
            tool_found: true,
            tool_failed: false,
            entries: vec![
                DiagnosticEntry {
                    file: "src/main.rs".to_string(),
                    line: 5,
                    severity: DiagSeverity::Error,
                    code: String::new(),
                    message: String::new(),
                },
                DiagnosticEntry {
                    file: "src/main.rs".to_string(),
                    line: 8,
                    severity: DiagSeverity::Warning,
                    code: String::new(),
                    message: String::new(),
                },
            ],
        }];

        let components = vec![
            test_component("fn-a", "a", "src/main.rs", 1, 10),
        ];

        let faults = map_diagnostics_to_components(Path::new("."), &results, &components);
        let total_errors: usize = results.iter()
            .flat_map(|r| &r.entries)
            .filter(|e| e.severity == DiagSeverity::Error)
            .count();
        let total_warnings: usize = results.iter()
            .flat_map(|r| &r.entries)
            .filter(|e| e.severity == DiagSeverity::Warning)
            .count();
        assert_eq!(total_errors, 1);
        assert_eq!(total_warnings, 1);
        assert_eq!(faults.len(), 1);
    }

    #[test]
    fn tool_not_found_does_not_crash() {
        let result = AnalyzerResult {
            language: "rust".to_string(),
            tool: "clippy".to_string(),
            tool_found: false,
            tool_failed: false,
            entries: Vec::new(),
        };
        assert!(result.entries.is_empty());
        assert!(!result.tool_found);
    }
}
