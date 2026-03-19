//! Git-assisted detection of whether the on-disk codebase may have diverged from the saved index.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    process::Command,
};

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
    match git_output(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(s) if s.trim() == "true" => {}
        _ => return None,
    }
    git_output(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
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

    let inside = match git_output(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(s) => s.trim() == "true",
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

    let head_current = match git_output(root, &["rev-parse", "HEAD"]) {
        Ok(s) => Some(s.trim().to_string()),
        Err(e) => {
            report.git_error = Some(e);
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

/// Paths that differ from `HEAD` or are untracked (respecting exclude rules), repo-relative, `/`-separated.
fn git_dirty_paths(root: &Path) -> Result<HashSet<String>, String> {
    let mut paths = HashSet::new();

    let diff = git_output(root, &["diff", "--name-only", "HEAD"])?;
    for line in diff.lines() {
        let p = line.trim();
        if !p.is_empty() {
            paths.insert(normalize_path_str(p));
        }
    }

    let untracked = git_output(root, &["ls-files", "--others", "--exclude-standard"])?;
    for line in untracked.lines() {
        let p = line.trim();
        if !p.is_empty() {
            paths.insert(normalize_path_str(p));
        }
    }

    Ok(paths)
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
}
