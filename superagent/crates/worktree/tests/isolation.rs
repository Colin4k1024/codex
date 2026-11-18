//! P2-9 / #86 integration tests over REAL git worktrees (model-free).
//!
//! Proves: per-task isolation (primary tree untouched), bounded concurrency,
//! dependency artifacts propagate only by explicit merge, a merge conflict
//! aborts and blocks downstream (target unchanged), post-merge gate re-verify
//! uses real exit codes, and the pool never auto add -A/commit/reset nor swallows
//! the user's uncommitted content.

use std::path::Path;
use std::process::Command;

use superagent_worktree::{
    commit_paths, merge, run_gate, MergeOutcome, WorktreeError, WorktreePool,
};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn init_repo(dir: &Path) -> String {
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    git(dir, &["add", "README.md"]);
    git(dir, &["commit", "-qm", "initial"]);
    git(dir, &["rev-parse", "HEAD"])
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

// 1. Each task is isolated in its own worktree; the primary working tree never
//    sees task writes; releasing frees slots and leaves primary HEAD unchanged.
#[test]
fn isolation_separate_worktrees_primary_untouched() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let head = init_repo(&repo);
    let base = root.path().join("wt");
    let pool = WorktreePool::new(&repo, &base, 4).unwrap();

    let a = pool.acquire("task-a").unwrap();
    let b = pool.acquire("task-b").unwrap();
    assert_ne!(a.path(), b.path());
    write(a.path(), "a.txt", "from A");
    write(b.path(), "b.txt", "from B");

    // Each worktree sees only its own write.
    assert!(a.path().join("a.txt").exists());
    assert!(!a.path().join("b.txt").exists());
    assert!(b.path().join("b.txt").exists());
    assert!(!b.path().join("a.txt").exists());
    // Primary working tree sees NEITHER (isolation).
    assert!(!repo.join("a.txt").exists());
    assert!(!repo.join("b.txt").exists());
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");

    commit_paths(&a, &["a.txt"], "A work").unwrap();
    commit_paths(&b, &["b.txt"], "B work").unwrap();
    assert_eq!(pool.active_count(), 2);
    pool.release(&a).unwrap();
    pool.release(&b).unwrap();
    assert_eq!(pool.active_count(), 0);
    // Primary HEAD unchanged by isolated task commits.
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
}

// 2. The concurrency cap is enforced; releasing a slot admits a waiting task.
#[test]
fn concurrency_cap_enforced() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let pool = WorktreePool::new(&repo, &root.path().join("wt"), 2).unwrap();

    let a = pool.acquire("a").unwrap();
    let _b = pool.acquire("b").unwrap();
    assert_eq!(pool.active_count(), 2);
    let err = pool.acquire("c").unwrap_err();
    assert!(
        matches!(err, WorktreeError::ConcurrencyLimit(2)),
        "got: {err:?}"
    );
    pool.release(&a).unwrap();
    assert_eq!(pool.active_count(), 1);
    let _c = pool.acquire("c").unwrap();
    assert_eq!(pool.active_count(), 2);
}

// 3. A dependency artifact reaches a downstream task ONLY via explicit merge.
#[test]
fn dependency_artifact_propagates_via_explicit_merge() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let pool = WorktreePool::new(&repo, &root.path().join("wt"), 8).unwrap();

    let integ = pool.acquire("integration").unwrap();
    let a = pool.acquire("task-a").unwrap();
    write(a.path(), "artifact.txt", "dependency payload");
    commit_paths(&a, &["artifact.txt"], "produce artifact").unwrap();

    // Before merge the integration branch does NOT have the artifact.
    assert!(!integ.path().join("artifact.txt").exists());
    assert_eq!(merge(&a, &integ).unwrap(), MergeOutcome::Merged);
    assert!(integ.path().join("artifact.txt").exists());

    // A downstream task branched from integration inherits the artifact.
    let b = pool.acquire_from("task-b", integ.branch()).unwrap();
    assert_eq!(read(b.path(), "artifact.txt"), "dependency payload");
}

// 4. A merge conflict aborts (target unchanged) and blocks downstream.
#[test]
fn merge_conflict_blocks_downstream_target_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    write(&repo, "shared.txt", "base\n");
    git(&repo, &["add", "shared.txt"]);
    git(&repo, &["commit", "-qm", "shared base"]);
    let pool = WorktreePool::new(&repo, &root.path().join("wt"), 8).unwrap();

    let integ = pool.acquire("integration").unwrap();
    let a = pool.acquire("task-a").unwrap();
    let b = pool.acquire("task-b").unwrap();
    write(a.path(), "shared.txt", "A version\n");
    write(b.path(), "shared.txt", "B version\n");
    commit_paths(&a, &["shared.txt"], "A edit").unwrap();
    commit_paths(&b, &["shared.txt"], "B edit").unwrap();

    assert_eq!(merge(&a, &integ).unwrap(), MergeOutcome::Merged);
    let conflict = merge(&b, &integ).unwrap();
    match &conflict {
        MergeOutcome::Conflict { files } => {
            assert!(files.iter().any(|f| f == "shared.txt"), "files: {files:?}");
        }
        other => panic!("expected conflict, got {other:?}"),
    }
    // Target branch unchanged by the aborted merge; no merge left in progress.
    assert_eq!(read(integ.path(), "shared.txt"), "A version\n");
    assert_eq!(git(integ.path(), &["status", "--porcelain"]), "");
    // Downstream sees A (the conflict blocked B), never a silent swallow.
    let down = pool.acquire_from("downstream", integ.branch()).unwrap();
    assert_eq!(read(down.path(), "shared.txt"), "A version\n");
}

// 5. After merge the relevant gate is re-verified with a REAL exit code.
#[test]
fn post_merge_gate_reverify_real_exit_code() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let pool = WorktreePool::new(&repo, &root.path().join("wt"), 8).unwrap();
    let integ = pool.acquire("integration").unwrap();

    let gate = vec![
        "python3".to_string(),
        "-B".to_string(),
        "-c".to_string(),
        "import pathlib,sys; sys.exit(0 if pathlib.Path('calc.py').read_text().strip()=='ok' else 1)"
            .to_string(),
    ];

    // Good artifact -> merged gate passes (exit 0).
    let good = pool.acquire("good").unwrap();
    write(good.path(), "calc.py", "ok\n");
    commit_paths(&good, &["calc.py"], "good").unwrap();
    assert_eq!(merge(&good, &integ).unwrap(), MergeOutcome::Merged);
    let pass = run_gate(integ.path(), &gate).unwrap();
    assert_eq!(pass.exit_code, 0);
    assert!(pass.passed);

    // Bad artifact merged on top -> gate re-verify fails (exit 1), not a claim.
    let bad = pool.acquire_from("bad", integ.branch()).unwrap();
    write(bad.path(), "calc.py", "bad\n");
    commit_paths(&bad, &["calc.py"], "bad").unwrap();
    assert_eq!(merge(&bad, &integ).unwrap(), MergeOutcome::Merged);
    let fail = run_gate(integ.path(), &gate).unwrap();
    assert_eq!(fail.exit_code, 1);
    assert!(!fail.passed);
}

// 6. The pool never auto add -A/commit/reset and never swallows the user's
//    uncommitted content in the primary working tree.
#[test]
fn no_auto_git_ops_protects_user_uncommitted_content() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let head = init_repo(&repo);
    // User's in-progress uncommitted work in the PRIMARY tree.
    write(&repo, "README.md", "base\nuser local edit\n");
    write(&repo, "user-notes.txt", "precious uncommitted\n");
    let status_before = git(&repo, &["status", "--porcelain"]);
    assert!(status_before.contains("README.md"));
    assert!(status_before.contains("user-notes.txt"));

    let pool = WorktreePool::new(&repo, &root.path().join("wt"), 8).unwrap();
    let integ = pool.acquire("integration").unwrap();
    let a = pool.acquire("task-a").unwrap();
    write(a.path(), "task.txt", "task work\n");
    commit_paths(&a, &["task.txt"], "task").unwrap();
    assert_eq!(merge(&a, &integ).unwrap(), MergeOutcome::Merged);
    pool.release(&a).unwrap();

    // Primary HEAD unchanged; user's edit still unstaged; notes still untracked.
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    let status_after = git(&repo, &["status", "--porcelain"]);
    assert!(
        status_after.contains(" M README.md"),
        "user edit must remain unstaged: {status_after}"
    );
    assert!(
        status_after.contains("?? user-notes.txt"),
        "user notes must remain untracked: {status_after}"
    );
    assert!(
        !status_after.contains("M  README.md"),
        "pool must not auto-stage: {status_after}"
    );
    assert_eq!(read(&repo, "user-notes.txt"), "precious uncommitted\n");
    // The task artifact never leaked into the primary tree.
    assert!(!repo.join("task.txt").exists());
}
