//! U04: workspace file + read-only Git adapter.
//!
//! Security boundary (HARNESS_CLAUDECODEUI_MIGRATION.md §6):
//! - Every file path is resolved against a backend-owned workspace root, never an
//!   arbitrary host path supplied by the WebView. Absolute paths and `..`
//!   traversal are rejected; symlinks are canonicalized and must stay inside the
//!   root. Reads/saves are size-limited.
//! - Git uses FIXED read-only subcommands with structured arguments (no shell).
//!   External diff/textconv helpers and the pager are disabled so a read-only
//!   request cannot trigger an executable helper from repo config.
//! - Writes to Git (commit/checkout/push/...) are NOT implemented here; the
//!   frontend disables them in the desktop build (read-only U04).

use serde_json::{json, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

/// Default workspace root for the built-in fixture project. Overridden by the
/// `YOURAGENT_WORKSPACE_ROOT` environment variable for any project.
pub const FIXTURE_WORKSPACE_ROOT: &str = "/Users/fanjia/Desktop/code/codex";

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024; // 2 MiB
const MAX_TREE_ENTRIES: usize = 4000;
const MAX_TREE_DEPTH: usize = 12;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".next",
    "__pycache__",
    ".venv",
    ".turbo",
];

/// Resolve a backend-owned workspace root from a project name. The WebView never
/// supplies the host path directly.
pub fn resolve_workspace_root(project_name: &str) -> Result<PathBuf, String> {
    let raw = if let Ok(root) = std::env::var("YOURAGENT_WORKSPACE_ROOT") {
        root
    } else if project_name == "superagent" {
        FIXTURE_WORKSPACE_ROOT.to_string()
    } else {
        return Err(format!("unknown project: {project_name}"));
    };
    let canon = fs::canonicalize(&raw).map_err(|e| format!("workspace root unavailable: {e}"))?;
    if !canon.is_dir() {
        return Err("workspace root is not a directory".to_string());
    }
    Ok(canon)
}

/// Confine a project-relative path to the workspace root. Rejects absolute paths
/// and `..` traversal, canonicalizes symlinks, and verifies the result stays
/// inside the root. For not-yet-existing files (save), the parent is canonicalized.
fn confine(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() {
        return Err("empty path".to_string());
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path traversal is not allowed".to_string());
    }
    let joined = root.join(p);
    if joined.exists() {
        let canon = fs::canonicalize(&joined).map_err(|e| format!("path error: {e}"))?;
        if !canon.starts_with(root) {
            return Err("path escapes workspace".to_string());
        }
        Ok(canon)
    } else {
        let parent = joined.parent().ok_or_else(|| "invalid path".to_string())?;
        let canon_parent = fs::canonicalize(parent).map_err(|e| format!("path error: {e}"))?;
        if !canon_parent.starts_with(root) {
            return Err("path escapes workspace".to_string());
        }
        let name = joined
            .file_name()
            .ok_or_else(|| "invalid path".to_string())?;
        Ok(canon_parent.join(name))
    }
}

fn mtime_ms(path: &Path) -> Option<i64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn iso_from_ms(ms: Option<i64>) -> String {
    match ms.and_then(chrono::DateTime::from_timestamp_millis) {
        Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        None => "1970-01-01T00:00:00Z".to_string(),
    }
}

#[cfg(unix)]
fn permissions_rwx(meta: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    let bit = |shift: u32, ch: char| {
        if mode & (1 << shift) != 0 {
            ch
        } else {
            '-'
        }
    };
    format!(
        "{}{}{}{}{}{}{}{}{}",
        bit(8, 'r'),
        bit(7, 'w'),
        bit(6, 'x'),
        bit(5, 'r'),
        bit(4, 'w'),
        bit(3, 'x'),
        bit(2, 'r'),
        bit(1, 'w'),
        bit(0, 'x'),
    )
}

#[cfg(not(unix))]
fn permissions_rwx(_meta: &fs::Metadata) -> String {
    String::new()
}

fn rel_posix(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn walk_dir(root: &Path, dir: &Path, depth: usize, count: &mut usize) -> Vec<Value> {
    let mut entries: Vec<(bool, String, PathBuf)> = Vec::new();
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    for entry in read.flatten() {
        let path = entry.path();
        // Use symlink_metadata so we never follow symlinks (escape/loop safety).
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        entries.push((is_dir, name, path));
    }
    // Directories first, then alphabetical.
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });

    let mut nodes = Vec::new();
    for (is_dir, name, path) in entries {
        if *count >= MAX_TREE_ENTRIES {
            break;
        }
        *count += 1;
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let rel = rel_posix(root, &path);
        if is_dir {
            let children = if depth + 1 < MAX_TREE_DEPTH {
                walk_dir(root, &path, depth + 1, count)
            } else {
                Vec::new()
            };
            nodes.push(json!({
                "name": name,
                "type": "directory",
                "path": rel,
                "modified": iso_from_ms(mtime_ms(&path)),
                "permissionsRwx": permissions_rwx(&meta),
                "children": children,
            }));
        } else {
            nodes.push(json!({
                "name": name,
                "type": "file",
                "path": rel,
                "size": meta.len(),
                "modified": iso_from_ms(mtime_ms(&path)),
                "permissionsRwx": permissions_rwx(&meta),
            }));
        }
    }
    nodes
}

/// File tree as a bare array of FileTreeNode (the shape useFileTreeData expects).
/// On a workspace error returns an empty array so the tree degrades gracefully.
pub fn list_files(project_name: &str) -> Value {
    let root = match resolve_workspace_root(project_name) {
        Ok(r) => r,
        Err(_) => return json!([]),
    };
    let mut count = 0usize;
    json!(walk_dir(&root, &root, 0, &mut count))
}

pub fn read_file(project_name: &str, file_path: &str) -> Value {
    let root = match resolve_workspace_root(project_name) {
        Ok(r) => r,
        Err(e) => return json!({ "error": e }),
    };
    let path = match confine(&root, file_path) {
        Ok(p) => p,
        Err(e) => return json!({ "error": e }),
    };
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return json!({ "error": format!("cannot stat file: {e}") }),
    };
    if !meta.is_file() {
        return json!({ "error": "not a regular file" });
    }
    if meta.len() as usize > MAX_FILE_BYTES {
        return json!({ "error": "file too large to open in the editor" });
    }
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => return json!({ "error": format!("cannot read file: {e}") }),
    };
    json!({
        "content": String::from_utf8_lossy(&bytes).into_owned(),
        "mtimeMs": mtime_ms(&path),
    })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "invalid path".to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| "invalid path".to_string())?
        .to_string_lossy();
    let tmp = parent.join(format!(".youragent-tmp-{}-{}", std::process::id(), name));
    fs::write(&tmp, bytes).map_err(|e| format!("cannot write file: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot replace file: {e}")
    })
}

/// Save with optimistic concurrency: when the editor supplies the mtime it loaded,
/// a differing on-disk mtime is reported as a conflict instead of silently
/// overwriting (e.g. an agent edited the file meanwhile).
pub fn save_file(
    project_name: &str,
    file_path: &str,
    content: &str,
    expected_mtime_ms: Option<i64>,
) -> Value {
    let root = match resolve_workspace_root(project_name) {
        Ok(r) => r,
        Err(e) => return json!({ "error": e }),
    };
    let path = match confine(&root, file_path) {
        Ok(p) => p,
        Err(e) => return json!({ "error": e }),
    };
    if content.len() > MAX_FILE_BYTES {
        return json!({ "error": "content too large to save" });
    }
    if let Some(expected) = expected_mtime_ms {
        if path.exists() {
            if let Some(current) = mtime_ms(&path) {
                if current != expected {
                    return json!({
                        "error": "File changed on disk since it was opened. Reload to avoid overwriting newer changes.",
                        "conflict": true,
                        "currentMtimeMs": current,
                    });
                }
            }
        }
    }
    if let Err(e) = atomic_write(&path, content.as_bytes()) {
        return json!({ "error": e });
    }
    json!({ "success": true, "mtimeMs": mtime_ms(&path) })
}

// ─── Read-only Git ───────────────────────────────────────────────────────────

/// Run a fixed git subcommand. External diff/textconv helpers and the pager are
/// disabled via config env so read-only requests cannot spawn repo-configured
/// executables. No shell is involved; arguments are structured.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("GIT_EXTERNAL_DIFF", "")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "diff.external")
        .env("GIT_CONFIG_VALUE_0", "")
        .output()
        .map_err(|e| format!("git unavailable: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git command failed".to_string()
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_root(project_name: &str) -> Result<PathBuf, Value> {
    let root = resolve_workspace_root(project_name).map_err(|e| json!({ "error": e }))?;
    git(&root, &["rev-parse", "--is-inside-work-tree"])
        .map_err(|e| json!({ "error": "not a git repository", "details": e }))?;
    Ok(root)
}

/// Parse `git status --porcelain=v1 --branch` into the flat-array GitStatusResponse
/// the frontend expects ({branch, hasCommits, modified[], added[], deleted[], untracked[]}).
pub fn parse_porcelain_status(text: &str) -> Value {
    let mut branch = String::new();
    let mut has_commits = true;
    let (mut modified, mut added, mut deleted, mut untracked) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(name) = rest.strip_prefix("No commits yet on ") {
                branch = name.to_string();
                has_commits = false;
            } else {
                branch = rest
                    .split("...")
                    .next()
                    .unwrap_or(rest)
                    .split(' ')
                    .next()
                    .unwrap_or(rest)
                    .to_string();
            }
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        let xy = &line[..2];
        let mut path = line[3..].to_string();
        if let Some((_old, new)) = path.split_once(" -> ") {
            path = new.to_string();
        }
        let path = path.trim_matches('"').to_string();
        if xy == "??" {
            untracked.push(path);
        } else if xy.contains('A') {
            added.push(path);
        } else if xy.contains('D') {
            deleted.push(path);
        } else if xy.contains('M') || xy.contains('R') || xy.contains('C') {
            modified.push(path);
        }
    }

    json!({
        "branch": branch,
        "hasCommits": has_commits,
        "modified": modified,
        "added": added,
        "deleted": deleted,
        "untracked": untracked,
    })
}

pub fn git_status(project_name: &str) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match git(&root, &["status", "--porcelain=v1", "--branch"]) {
        Ok(text) => parse_porcelain_status(&text),
        Err(e) => json!({ "error": e }),
    }
}

pub fn git_diff(project_name: &str, file_path: &str) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if let Err(e) = confine(&root, file_path) {
        return json!({ "error": e });
    }
    match git(
        &root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
            file_path,
        ],
    ) {
        Ok(diff) => json!({ "diff": diff }),
        Err(e) => json!({ "error": e }),
    }
}

pub fn git_branches(project_name: &str) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let local = git(&root, &["branch", "--format=%(refname:short)"])
        .map(|s| s.lines().map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    let remote = git(&root, &["branch", "-r", "--format=%(refname:short)"])
        .map(|s| {
            s.lines()
                .filter(|l| !l.contains("HEAD"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({ "branches": local, "localBranches": local, "remoteBranches": remote })
}

pub fn git_remote_status(project_name: &str) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let has_remote = git(&root, &["remote"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let upstream = git(
        &root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());

    let (has_upstream, remote_name, remote_branch, ahead, behind) = match upstream {
        Some(u) => {
            let (rn, rb) = u.split_once('/').unwrap_or((u.as_str(), ""));
            let (a, b) = git(
                &root,
                &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
            )
            .ok()
            .and_then(|s| {
                let mut it = s.split_whitespace();
                let a = it.next()?.parse::<i64>().ok()?;
                let b = it.next()?.parse::<i64>().ok()?;
                Some((a, b))
            })
            .unwrap_or((0, 0));
            (true, rn.to_string(), rb.to_string(), a, b)
        }
        None => (false, String::new(), String::new(), 0, 0),
    };

    json!({
        "hasRemote": has_remote,
        "hasUpstream": has_upstream,
        "branch": branch,
        "remoteName": remote_name,
        "remoteBranch": remote_branch,
        "ahead": ahead,
        "behind": behind,
        "isUpToDate": has_upstream && ahead == 0 && behind == 0,
    })
}

pub fn git_commits(project_name: &str, limit: i64) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let n = limit.clamp(1, 200).to_string();
    let out = git(
        &root,
        &[
            "log",
            "-n",
            &n,
            "--no-ext-diff",
            "--no-textconv",
            "--pretty=format:%H%x1f%an%x1f%ae%x1f%aI%x1f%s",
        ],
    );
    let text = match out {
        Ok(t) => t,
        Err(_) => return json!({ "commits": [] }),
    };
    let commits: Vec<Value> = text
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\u{1f}');
            let hash = f.next()?;
            if hash.is_empty() {
                return None;
            }
            Some(json!({
                "hash": hash,
                "author": f.next().unwrap_or(""),
                "email": f.next().unwrap_or(""),
                "date": f.next().unwrap_or(""),
                "message": f.next().unwrap_or(""),
            }))
        })
        .collect();
    json!({ "commits": commits })
}

/// Only accept a plain hex object id so a commit argument can never inject flags
/// or revision syntax into `git show`.
pub fn is_valid_commit_hash(s: &str) -> bool {
    (4..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn git_commit_diff(project_name: &str, commit: &str) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if !is_valid_commit_hash(commit) {
        return json!({ "error": "invalid commit hash" });
    }
    match git(
        &root,
        &[
            "show",
            "--no-ext-diff",
            "--no-textconv",
            "--format=medium",
            commit,
        ],
    ) {
        Ok(diff) => json!({ "diff": diff }),
        Err(e) => json!({ "error": e }),
    }
}

pub fn git_file_with_diff(project_name: &str, file_path: &str) -> Value {
    let root = match git_root(project_name) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let path = match confine(&root, file_path) {
        Ok(p) => p,
        Err(e) => return json!({ "error": e }),
    };
    let rel = rel_posix(&root, &path);
    let old_content = git(&root, &["show", &format!("HEAD:{rel}")]).unwrap_or_default();
    let exists = path.exists();
    let current_content = if exists {
        fs::read(&path)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let is_untracked = exists && git(&root, &["ls-files", "--error-unmatch", "--", &rel]).is_err();
    json!({
        "oldContent": old_content,
        "currentContent": current_content,
        "isDeleted": !exists && !old_content.is_empty(),
        "isUntracked": is_untracked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests that mutate YOURAGENT_WORKSPACE_ROOT must not run concurrently.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("youragent-ws-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn confine_rejects_absolute_and_traversal() {
        let root = temp_workspace("confine");
        assert!(confine(&root, "/etc/passwd").is_err());
        assert!(confine(&root, "../escape").is_err());
        assert!(confine(&root, "a/../../escape").is_err());
        assert!(confine(&root, "").is_err());
        assert!(confine(&root, "ok.txt").is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn confine_rejects_symlink_escape() {
        let root = temp_workspace("symlink");
        let outside =
            std::env::temp_dir().join(format!("youragent-outside-{}", std::process::id()));
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        #[cfg(unix)]
        assert!(confine(&root, "link/secret.txt").is_err());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn read_save_roundtrip_and_mtime_conflict() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = temp_workspace("save");
        // Use the env override so resolve_workspace_root points at our temp dir.
        std::env::set_var("YOURAGENT_WORKSPACE_ROOT", &root);
        fs::write(root.join("a.txt"), "hello").unwrap();

        let read = read_file("superagent", "a.txt");
        assert_eq!(read["content"], "hello");
        let mtime = read["mtimeMs"].as_i64().unwrap();

        // Ensure the on-disk mtime advances past the loaded value so the stale
        // save below is detectably stale (filesystem timestamp granularity).
        std::thread::sleep(std::time::Duration::from_millis(15));

        // Save with the correct expected mtime succeeds.
        let ok = save_file("superagent", "a.txt", "world", Some(mtime));
        assert_eq!(ok["success"], true);
        assert_eq!(read_file("superagent", "a.txt")["content"], "world");

        // Save with a stale mtime is a conflict, not an overwrite.
        let conflict = save_file("superagent", "a.txt", "clobber", Some(mtime));
        assert_eq!(conflict["conflict"], true);
        assert_eq!(read_file("superagent", "a.txt")["content"], "world");

        // Traversal save is rejected.
        assert!(save_file("superagent", "../evil.txt", "x", None)["error"].is_string());

        std::env::remove_var("YOURAGENT_WORKSPACE_ROOT");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_rejects_large_and_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = temp_workspace("read");
        std::env::set_var("YOURAGENT_WORKSPACE_ROOT", &root);
        assert!(read_file("superagent", "missing.txt")["error"].is_string());
        let big = vec![b'a'; MAX_FILE_BYTES + 1];
        fs::write(root.join("big.bin"), &big).unwrap();
        assert!(read_file("superagent", "big.bin")["error"].is_string());
        std::env::remove_var("YOURAGENT_WORKSPACE_ROOT");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn list_files_skips_noise_and_marks_types() {
        let _guard = ENV_LOCK.lock().unwrap();
        let root = temp_workspace("tree");
        std::env::set_var("YOURAGENT_WORKSPACE_ROOT", &root);
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules/x.js"), "x").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main(){}").unwrap();
        fs::write(root.join("README.md"), "hi").unwrap();
        let tree = list_files("superagent");
        let arr = tree.as_array().unwrap();
        let names: Vec<&str> = arr.iter().map(|n| n["name"].as_str().unwrap()).collect();
        assert!(!names.contains(&"node_modules"));
        assert!(names.contains(&"src"));
        assert!(names.contains(&"README.md"));
        let src = arr.iter().find(|n| n["name"] == "src").unwrap();
        assert_eq!(src["type"], "directory");
        assert_eq!(src["children"][0]["name"], "main.rs");
        assert_eq!(src["children"][0]["path"], "src/main.rs");
        std::env::remove_var("YOURAGENT_WORKSPACE_ROOT");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_status_categorizes_and_branch() {
        let text = "## main...origin/main [ahead 1]\n M src/a.rs\nA  src/b.rs\n D old.rs\n?? new.txt\nR  old_name.rs -> new_name.rs\n";
        let s = parse_porcelain_status(text);
        assert_eq!(s["branch"], "main");
        assert_eq!(s["hasCommits"], true);
        assert_eq!(s["modified"], json!(["src/a.rs", "new_name.rs"]));
        assert_eq!(s["added"], json!(["src/b.rs"]));
        assert_eq!(s["deleted"], json!(["old.rs"]));
        assert_eq!(s["untracked"], json!(["new.txt"]));
    }

    #[test]
    fn parse_status_no_commits_yet() {
        let s = parse_porcelain_status("## No commits yet on main\n?? file.txt\n");
        assert_eq!(s["branch"], "main");
        assert_eq!(s["hasCommits"], false);
        assert_eq!(s["untracked"], json!(["file.txt"]));
    }

    #[test]
    fn commit_hash_validation_blocks_injection() {
        assert!(is_valid_commit_hash("abc1234"));
        assert!(is_valid_commit_hash(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_valid_commit_hash("--help"));
        assert!(!is_valid_commit_hash("HEAD"));
        assert!(!is_valid_commit_hash("abc; rm -rf /"));
        assert!(!is_valid_commit_hash("ab"));
        assert!(!is_valid_commit_hash(""));
    }

    // Run with: YOURAGENT_WORKSPACE_ROOT=/path/to/repo cargo test -- --ignored real_workspace
    // Skips when the env var is unset so it never depends on a machine-specific path.
    #[test]
    #[ignore]
    fn real_workspace_view_consistency() {
        let _guard = ENV_LOCK.lock().unwrap();
        if std::env::var("YOURAGENT_WORKSPACE_ROOT").is_err() {
            eprintln!("YOURAGENT_WORKSPACE_ROOT unset; skipping real-workspace smoke");
            return;
        }

        let tree = list_files("superagent");
        let arr = tree.as_array().expect("file tree is a bare array");
        assert!(!arr.is_empty(), "real repo tree should not be empty");
        for node in arr {
            assert!(node["name"].is_string());
            assert!(node["path"].is_string());
            let ty = node["type"].as_str().unwrap();
            assert!(ty == "file" || ty == "directory");
        }

        let status = git_status("superagent");
        assert!(status["error"].is_null(), "git status: {:?}", status);
        for key in ["modified", "added", "deleted", "untracked"] {
            assert!(status[key].is_array(), "{key} must be a flat array");
        }

        let branches = git_branches("superagent");
        assert!(branches["branches"].is_array());
        assert!(branches["localBranches"].is_array());
        assert!(branches["remoteBranches"].is_array());

        let remote = git_remote_status("superagent");
        assert!(remote["hasRemote"].is_boolean());
        assert!(remote["hasUpstream"].is_boolean());

        let commits = git_commits("superagent", 5);
        let list = commits["commits"].as_array().expect("commits array");
        assert!(!list.is_empty(), "real repo should have commits");
        for c in list {
            assert!(c["hash"].is_string());
            assert!(c["author"].is_string());
            assert!(c["message"].is_string());
        }
        let head = list[0]["hash"].as_str().unwrap();
        let diff = git_commit_diff("superagent", head);
        assert!(diff["diff"].is_string(), "commit diff for HEAD");

        // Path boundary holds on the real root too.
        assert!(read_file("superagent", "../../../etc/passwd")["error"].is_string());
        assert!(git_diff("superagent", "../../evil")["error"].is_string());
        assert!(git_commit_diff("superagent", "--help")["error"].is_string());
    }
}
