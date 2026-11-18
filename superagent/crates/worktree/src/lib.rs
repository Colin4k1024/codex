//! SuperAgent controlled parallel isolation (P2-9 / #86).
//!
//! Each write task runs in its own real `git worktree`, so concurrent tasks
//! never share a working tree. Concurrency is bounded by the pool. Dependency
//! artifacts propagate ONLY through an explicit merge; a merge conflict aborts
//! the merge and blocks downstream (the target branch is left unchanged). After
//! a merge the caller re-verifies the relevant gate via [`run_gate`] (real exit
//! code).
//!
//! Safety invariants (enforced by construction, proved by tests):
//! - the pool NEVER runs `git add -A`, `git commit`, or `git reset` on its own;
//!   [`commit_paths`] is an explicit, caller-driven snapshot scoped to the given
//!   paths only;
//! - worktree creation / merge / release operate on isolated worktrees and never
//!   touch the primary working tree, so the user's uncommitted content is never
//!   swallowed or staged.
//!
//! Model-free: the only subprocess is `git` (and the caller's gate command).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug)]
pub enum WorktreeError {
    Git(String),
    ConcurrencyLimit(usize),
    Io(std::io::Error),
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorktreeError::Git(s) => write!(f, "git: {s}"),
            WorktreeError::ConcurrencyLimit(n) => {
                write!(f, "concurrency limit reached ({n})")
            }
            WorktreeError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for WorktreeError {}

impl From<std::io::Error> for WorktreeError {
    fn from(e: std::io::Error) -> Self {
        WorktreeError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, WorktreeError>;

/// Outcome of an explicit merge of a task branch into a target worktree.
#[derive(Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// Merge committed into the target branch.
    Merged,
    /// Conflict: merge aborted, target branch unchanged, downstream blocked.
    Conflict { files: Vec<String> },
}

/// A real gate re-verification result (real process exit code).
#[derive(Debug, PartialEq, Eq)]
pub struct GateResult {
    pub exit_code: i32,
    pub passed: bool,
}

fn git_raw(dir: &Path, args: &[&str]) -> Result<Output> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "superagent")
        .env("GIT_AUTHOR_EMAIL", "superagent@local")
        .env("GIT_COMMITTER_NAME", "superagent")
        .env("GIT_COMMITTER_EMAIL", "superagent@local")
        .output()?;
    Ok(out)
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git_raw(dir, args)?;
    if !out.status.success() {
        return Err(WorktreeError::Git(format!(
            "{}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// An isolated per-task worktree handle.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub task_id: String,
    pub dir: PathBuf,
    pub branch: String,
}

impl Worktree {
    pub fn path(&self) -> &Path {
        &self.dir
    }
    pub fn branch(&self) -> &str {
        &self.branch
    }
}

/// A bounded pool of per-task git worktrees over a single repository.
pub struct WorktreePool {
    repo: PathBuf,
    base: PathBuf,
    max_concurrent: usize,
    active: RefCell<BTreeSet<String>>,
}

impl WorktreePool {
    /// Create a pool. `repo` must be a git repository; `base` is the directory
    /// under which isolated worktrees are created; `max_concurrent` bounds the
    /// number of simultaneously active task worktrees (>= 1).
    pub fn new(repo: &Path, base: &Path, max_concurrent: usize) -> Result<Self> {
        let max_concurrent = max_concurrent.max(1);
        std::fs::create_dir_all(base)?;
        git(repo, &["rev-parse", "--git-dir"])?;
        Ok(Self {
            repo: repo.to_path_buf(),
            base: base.to_path_buf(),
            max_concurrent,
            active: RefCell::new(BTreeSet::new()),
        })
    }

    pub fn active_count(&self) -> usize {
        self.active.borrow().len()
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Acquire an isolated worktree for `task_id` branched from `base_ref`
    /// (e.g. "HEAD" or an integration branch). Refuses beyond the concurrency
    /// cap. Creates a real `git worktree` — the primary working tree is untouched.
    pub fn acquire_from(&self, task_id: &str, base_ref: &str) -> Result<Worktree> {
        if self.active.borrow().len() >= self.max_concurrent {
            return Err(WorktreeError::ConcurrencyLimit(self.max_concurrent));
        }
        let dir = self.base.join(task_id);
        let branch = format!("sa/{task_id}");
        let dir_s = dir.to_string_lossy().into_owned();
        git(
            &self.repo,
            &["worktree", "add", "-b", &branch, &dir_s, base_ref],
        )?;
        self.active.borrow_mut().insert(task_id.to_string());
        Ok(Worktree {
            task_id: task_id.to_string(),
            dir,
            branch,
        })
    }

    /// Acquire from HEAD.
    pub fn acquire(&self, task_id: &str) -> Result<Worktree> {
        self.acquire_from(task_id, "HEAD")
    }

    /// Release a worktree: remove the isolated working tree (force, since a task
    /// may leave uncommitted work) and free its concurrency slot. The task branch
    /// is preserved (no destructive branch deletion); the primary tree is untouched.
    pub fn release(&self, wt: &Worktree) -> Result<()> {
        let dir_s = wt.dir.to_string_lossy().into_owned();
        git(&self.repo, &["worktree", "remove", "--force", &dir_s])?;
        self.active.borrow_mut().remove(&wt.task_id);
        Ok(())
    }
}

/// Explicit, caller-driven snapshot of a task's work, scoped to `paths` only.
/// NEVER `git add -A`: only the listed paths are staged, then committed in the
/// task's own worktree. This is the sole commit path and is never invoked
/// automatically by the pool.
pub fn commit_paths(wt: &Worktree, paths: &[&str], message: &str) -> Result<String> {
    if paths.is_empty() {
        return Err(WorktreeError::Git(
            "commit_paths requires explicit paths".into(),
        ));
    }
    let mut add_args: Vec<&str> = vec!["add", "--"];
    add_args.extend_from_slice(paths);
    git(&wt.dir, &add_args)?;
    git(&wt.dir, &["commit", "-m", message])?;
    git(&wt.dir, &["rev-parse", "HEAD"])
}

/// Explicitly merge `source`'s branch into the branch checked out in `target`.
/// On conflict the merge is aborted (target branch unchanged) and
/// [`MergeOutcome::Conflict`] is returned so downstream is blocked. No auto
/// commit/reset beyond the merge itself; the primary working tree is untouched.
pub fn merge(source: &Worktree, target: &Worktree) -> Result<MergeOutcome> {
    let out = git_raw(&target.dir, &["merge", "--no-ff", &source.branch])?;
    if out.status.success() {
        return Ok(MergeOutcome::Merged);
    }
    let conflicts = git_raw(&target.dir, &["diff", "--name-only", "--diff-filter=U"])?;
    let files: Vec<String> = String::from_utf8_lossy(&conflicts.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    // Abort so the target branch is left exactly as it was (blocks downstream).
    let _ = git_raw(&target.dir, &["merge", "--abort"]);
    Ok(MergeOutcome::Conflict { files })
}

/// Re-verify a gate after merge by running `argv` in `dir` and reporting the
/// real process exit code. Success is the real exit code 0 — never a bare claim.
pub fn run_gate(dir: &Path, argv: &[String]) -> Result<GateResult> {
    let Some((prog, rest)) = argv.split_first() else {
        return Err(WorktreeError::Git("empty gate argv".into()));
    };
    let out = Command::new(prog).args(rest).current_dir(dir).output()?;
    let exit_code = out.status.code().unwrap_or(-1);
    Ok(GateResult {
        exit_code,
        passed: out.status.success(),
    })
}
