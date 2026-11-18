//! #87 runtime integration: the source adapter over REAL captured engine
//! rollouts (tests/fixtures/rollouts/*.jsonl, genuine codex-tui sessions from
//! the P1-5 fixture demo) plus memory/board fixtures. Proves the closure gate:
//! new sessions/posts auto-index and are retrievable; changed content stops
//! hitting the old query; deleted files are removed; Chinese trigram queries
//! hit; an unchanged pass does no redundant work; search latency is bounded.
//! Model-free and process-free: only files on disk are read.

use std::path::PathBuf;
use std::time::Instant;

use superagent_memory_service::source_adapter::{SourceAdapter, SourceRoots};
use superagent_memory_service::{MemoryService, SourceKind};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn seed_roots(tmp: &std::path::Path) -> SourceRoots {
    let rollouts = tmp.join("rollouts");
    let memories = tmp.join("memories");
    let board = tmp.join("board");
    std::fs::create_dir_all(&rollouts).unwrap();
    std::fs::create_dir_all(&memories).unwrap();
    std::fs::create_dir_all(&board).unwrap();
    for entry in std::fs::read_dir(fixtures().join("rollouts")).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            std::fs::copy(&p, rollouts.join(p.file_name().unwrap())).unwrap();
        }
    }
    std::fs::write(
        memories.join("note-1.md"),
        "# 团队记忆\n元智能体协调者负责编排，执行者负责落地。\n",
    )
    .unwrap();
    std::fs::write(
        board.join("post-1.json"),
        r#"{"posts":[{"id":"p1","text":"BOARDMARKERXYZ unique board post about calc"}]}"#,
    )
    .unwrap();
    SourceRoots {
        rollouts: Some(rollouts),
        memories: Some(memories),
        board: Some(board),
    }
}

#[test]
fn real_rollouts_auto_index_and_retrieve() {
    let tmp = tempfile::tempdir().unwrap();
    let roots = seed_roots(tmp.path());
    let svc = MemoryService::open_in_memory().unwrap();

    let report = SourceAdapter::sync(&svc, &roots).unwrap();
    // 3 real rollouts + 1 memory + 1 board, all new.
    assert_eq!(report.inserted, 5, "report={report:?}");
    assert_eq!(report.reindexed, 0);
    assert_eq!(report.removed, 0);
    assert_eq!(svc.count(SourceKind::Rollout).unwrap(), 3);
    assert_eq!(svc.count(SourceKind::Memory).unwrap(), 1);
    assert_eq!(svc.count(SourceKind::Board).unwrap(), 1);

    // A term genuinely present in the captured rollouts retrieves a rollout hit
    // citing its source id.
    let hits = svc.search("calc", 20).unwrap();
    assert!(
        hits.iter().any(|h| h.kind == "rollout"),
        "expected a rollout hit for 'calc', got {hits:?}"
    );
    assert!(hits
        .iter()
        .all(|h| !h.source_id.is_empty() && !h.snippet.is_empty()));
}

#[test]
fn changed_content_invalidates_old_query() {
    let tmp = tempfile::tempdir().unwrap();
    let roots = seed_roots(tmp.path());
    let svc = MemoryService::open_in_memory().unwrap();
    SourceAdapter::sync(&svc, &roots).unwrap();

    assert!(svc
        .search("BOARDMARKERXYZ", 10)
        .unwrap()
        .iter()
        .any(|h| h.kind == "board"));

    // Rewrite the board post so the old marker is gone.
    let board_file = roots.board.clone().unwrap().join("post-1.json");
    std::fs::write(
        &board_file,
        r#"{"posts":[{"id":"p1","text":"revised board post, marker removed"}]}"#,
    )
    .unwrap();
    let report = SourceAdapter::sync(&svc, &roots).unwrap();
    assert!(report.reindexed >= 1, "report={report:?}");
    assert!(
        svc.search("BOARDMARKERXYZ", 10).unwrap().is_empty(),
        "old content must no longer hit after change"
    );
    assert!(svc
        .search("revised", 10)
        .unwrap()
        .iter()
        .any(|h| h.kind == "board"));
}

#[test]
fn deleted_file_is_removed_from_index() {
    let tmp = tempfile::tempdir().unwrap();
    let roots = seed_roots(tmp.path());
    let svc = MemoryService::open_in_memory().unwrap();
    SourceAdapter::sync(&svc, &roots).unwrap();
    assert_eq!(svc.count(SourceKind::Rollout).unwrap(), 3);

    std::fs::remove_file(roots.rollouts.clone().unwrap().join("rollout-2.jsonl")).unwrap();
    let report = SourceAdapter::sync(&svc, &roots).unwrap();
    assert!(report.removed >= 1, "report={report:?}");
    assert_eq!(svc.count(SourceKind::Rollout).unwrap(), 2);
    assert!(!svc
        .source_ids(SourceKind::Rollout)
        .unwrap()
        .contains(&"rollout-2.jsonl".to_string()));
}

#[test]
fn chinese_trigram_query_hits_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let roots = seed_roots(tmp.path());
    let svc = MemoryService::open_in_memory().unwrap();
    SourceAdapter::sync(&svc, &roots).unwrap();

    let hits = svc.search("元智能体", 10).unwrap();
    assert!(
        hits.iter().any(|h| h.kind == "memory"),
        "Chinese substring query must hit the memory note, got {hits:?}"
    );
}

#[test]
fn unchanged_pass_is_idempotent_and_search_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let roots = seed_roots(tmp.path());
    let svc = MemoryService::open_in_memory().unwrap();
    SourceAdapter::sync(&svc, &roots).unwrap();

    // Second pass with no file changes does no redundant indexing work.
    let report = SourceAdapter::sync(&svc, &roots).unwrap();
    assert_eq!(report.inserted, 0);
    assert_eq!(report.reindexed, 0);
    assert_eq!(report.removed, 0);
    assert_eq!(report.unchanged, 5, "report={report:?}");

    // Latency baseline: a lexical search over the real rollouts is fast and
    // never blocks on a model/vector backend (none is invoked).
    let start = Instant::now();
    let hits = svc.search("calc", 50).unwrap();
    let elapsed = start.elapsed();
    assert!(!hits.is_empty());
    assert!(
        elapsed.as_millis() < 500,
        "search latency baseline exceeded: {elapsed:?}"
    );
    eprintln!(
        "search latency baseline: {elapsed:?} for {} hits",
        hits.len()
    );
}
