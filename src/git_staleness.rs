//! Git-assisted detection of whether the on-disk codebase may have diverged from the saved index.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    process::Command,
};

/// True when `root` is inside a git work tree (`git rev-parse --is-inside-work-tree`).
pub fn is_inside_git_work_tree(root: &Path) -> bool {
    match git_output(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(s) => s.trim() == "true",
        Err(_) => false,
    }
}

/// Repo-relative paths (`/` separators) with working-tree changes vs `HEAD` (porcelain), including untracked.
pub fn worktree_changed_paths(root: &Path) -> Result<HashSet<String>, String> {
    git_dirty_paths(root)
}

use serde::Serialize;

use crate::index::IndexData;

/// Report describing how the current git work tree relates to the last saved index.
#[derive(Debug, Clone, Serialize)]
pub struct GitStalenessReport {
    /// `git` executed successfully for basic queries.
    pub git_available: bool,
    /// Project root is inside a git work tree.
    pub git_repo: bool,
    /// Last error from git, if any (stdout/stderr combined for failures).
    pub git_error: Option<String>,
    /// `git rev-parse HEAD` value stored on the last successful `save_index` (if any).
    pub head_at_last_sync: Option<String>,
    /// Current `git rev-parse HEAD`.
    pub head_current: Option<String>,
    /// True when `head_at_last_sync` is known and differs from `head_current`.
    pub head_changed_since_last_sync: bool,
    /// Number of indexed paths that appeared in git's dirty set and were hash-checked.
    pub indexed_paths_checked_for_drift: usize,
    /// Indexed files whose bytes no longer match the stored Blake3 hash (or are missing).
    pub indexed_files_hash_mismatch: Vec<String>,
    /// Short human-readable explanation for terminal output when `is_stale` is true.
    pub reason_short: String,
    /// True when HEAD moved since the last sync save, or when drift was detected for indexed files.
    pub is_stale: bool,
}

/// Records the current `HEAD` into the index before persisting (no-op if not a git repo).
pub fn refresh_git_head_at_sync(index: &mut IndexData, root: &Path) {
    index.source_git_head = read_head_if_git_repo(root);
}

fn read_head_if_git_repo(root: &Path) -> Option<String> {
    let out = git_output(root, &["rev-parse", "--is-inside-work-tree", "HEAD"]).ok()?;
    parse_inside_and_head(out).1
}

/// Parses `git rev-parse is-inside-work-tree HEAD` stdout: line1 `true`/`false`, line2 SHA (if any).
fn parse_inside_and_head(stdout: String) -> (bool, Option<String>) {
    let mut lines = stdout.lines();
    let Some(first) = lines.next() else {
        return (false, None);
    };
    let inside = first.trim() == "true";
    if !inside {
        return (false, None);
    }
    let head = lines.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    (true, head)
}

/// Computes staleness for commands that load an index from disk (list, search, show, deps, etc.).
pub fn evaluate(root: &Path, index: &IndexData) -> GitStalenessReport {
    let indexed: HashMap<String, String> = index
        .files
        .iter()
        .map(|f| (normalize_path_str(&f.path), f.file_hash.clone()))
        .collect();

    let mut report = GitStalenessReport {
        git_available: true,
        git_repo: false,
        git_error: None,
        head_at_last_sync: index.source_git_head.clone(),
        head_current: None,
        head_changed_since_last_sync: false,
        indexed_paths_checked_for_drift: 0,
        indexed_files_hash_mismatch: Vec::new(),
        reason_short: String::new(),
        is_stale: false,
    };

    let (inside, head_current) = match git_output(root, &["rev-parse", "--is-inside-work-tree", "HEAD"]) {
        Ok(s) => parse_inside_and_head(s),
        Err(e) => {
            report.git_error = Some(e);
            report.git_available = false;
            return report;
        }
    };

    if !inside {
        return report;
    }

    report.git_repo = true;

    let head_current = match head_current {
        Some(h) => Some(h),
        None => {
            report.git_error = Some("git rev-parse HEAD produced no output".to_string());
            return report;
        }
    };
    report.head_current = head_current.clone();

    report.head_changed_since_last_sync = match (&index.source_git_head, &head_current) {
        (Some(saved), Some(cur)) => saved != cur,
        _ => false,
    };

    let dirty_paths = match git_dirty_paths(root) {
        Ok(p) => p,
        Err(e) => {
            report.git_error = Some(e);
            report.is_stale = report.head_changed_since_last_sync;
            finalize_reason(&mut report);
            return report;
        }
    };

    let mut candidates: Vec<String> = dirty_paths
        .into_iter()
        .filter(|p| indexed.contains_key(p))
        .collect();
    candidates.sort();

    report.indexed_paths_checked_for_drift = candidates.len();

    for path in candidates {
        let full = root.join(&path);
        let expected = indexed.get(&path).map(String::as_str).unwrap_or("");

        let actual_hash = match std::fs::read(&full) {
            Ok(bytes) => hash_bytes(&bytes),
            Err(_) => {
                report.indexed_files_hash_mismatch.push(path);
                continue;
            }
        };

        if actual_hash != expected {
            report.indexed_files_hash_mismatch.push(path);
        }
    }

    report.is_stale = report.head_changed_since_last_sync
        || !report.indexed_files_hash_mismatch.is_empty();

    finalize_reason(&mut report);
    report
}

fn finalize_reason(report: &mut GitStalenessReport) {
    if !report.is_stale {
        report.reason_short.clear();
        return;
    }

    let mut parts: Vec<String> = Vec::new();
    if report.head_changed_since_last_sync {
        parts.push("Git HEAD changed since the last index save".to_string());
    }
    let n = report.indexed_files_hash_mismatch.len();
    if n > 0 {
        parts.push(format!(
            "{n} indexed file{} changed on disk since the last index update",
            if n == 1 { "" } else { "s" }
        ));
    }

    report.reason_short = if parts.is_empty() {
        "Index may be out of date; run `lime sync`.".to_string()
    } else {
        format!("{} — run `lime sync`.", parts.join("; "))
    };
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("git {} failed: {stderr}{stdout}", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Paths with working-tree changes vs `HEAD` or untracked (non-ignored), repo-relative, `/`-separated.
/// Uses a single `git status --porcelain` to avoid multiple subprocess round-trips.
fn git_dirty_paths(root: &Path) -> Result<HashSet<String>, String> {
    let out = git_output(root, &["status", "--porcelain=v1"])?;
    Ok(parse_status_porcelain_paths(&out))
}

fn parse_status_porcelain_paths(porcelain: &str) -> HashSet<String> {
    let mut paths = HashSet::new();
    for line in porcelain.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // `?? path` (untracked)
        if line.starts_with("?? ") {
            if let Some(p) = line.get(3..) {
                paths.insert(normalize_path_str(p.trim()));
            }
            continue;
        }
        // `XY path` — first two columns are status; path starts after a space.
        if line.len() < 4 {
            continue;
        }
        let rest = line.get(3..).unwrap_or("").trim();
        if rest.is_empty() {
            continue;
        }
        // Rename: `R  old -> new` or `R  old -> new` with spaces
        if rest.contains(" -> ") {
            if let Some((a, b)) = rest.split_once(" -> ") {
                paths.insert(normalize_path_str(a.trim()));
                paths.insert(normalize_path_str(b.trim()));
            }
        } else {
            paths.insert(normalize_path_str(rest));
        }
    }
    paths
}

fn normalize_path_str(s: &str) -> String {
    s.replace('\\', "/")
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_str_flips_backslashes() {
        assert_eq!(normalize_path_str(r"a\b\c"), "a/b/c");
    }

    #[test]
    fn parse_inside_and_head_two_lines() {
        let (inside, h) = parse_inside_and_head("true\nabc123def\n".to_string());
        assert!(inside);
        assert_eq!(h.as_deref(), Some("abc123def"));
    }

    #[test]
    fn parse_status_porcelain_collects_paths() {
        let sample = " M src/a.rs\n?? b.txt\nR  old.rs -> new.rs\n";
        let p = parse_status_porcelain_paths(sample);
        assert!(p.contains("src/a.rs"));
        assert!(p.contains("b.txt"));
        assert!(p.contains("old.rs"));
        assert!(p.contains("new.rs"));
    }
}
