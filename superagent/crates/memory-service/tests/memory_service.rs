use superagent_memory_service::{
    Hit, MemoryError, MemoryService, ReservedVectorIndex, SourceKind, SyncOutcome, VectorIndex,
    MAX_RESULTS, SCHEMA_VERSION,
};

fn svc() -> MemoryService {
    MemoryService::open_in_memory().unwrap()
}

#[test]
fn migration_is_idempotent_and_versions() {
    let s = svc();
    assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    s.migrate().unwrap();
    s.migrate().unwrap();
    assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
}

#[test]
fn index_rollout_and_search() {
    let s = svc();
    let o = s
        .index(
            SourceKind::Rollout,
            "thread-1",
            "the orchestrator retries a failed gate",
            &serde_json::json!({}),
        )
        .unwrap();
    assert_eq!(o, SyncOutcome::Inserted);
    let hits = s.search("orchestrator", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, "rollout");
    assert_eq!(hits[0].source_id, "thread-1");
}

#[test]
fn index_memory_and_search() {
    let s = svc();
    s.index(
        SourceKind::Memory,
        "mem-1",
        "proxy must be exported for cargo and git",
        &serde_json::json!({"tag":"env"}),
    )
    .unwrap();
    let hits = s.search("proxy", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, "memory");
}

#[test]
fn index_board_post_and_search() {
    let s = svc();
    s.index(
        SourceKind::Board,
        "post-1",
        "leader delegates verification to the verifier role",
        &serde_json::json!({}),
    )
    .unwrap();
    let hits = s.search("verifier", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, "board");
}

#[test]
fn chinese_trigram_search() {
    let s = svc();
    s.index(
        SourceKind::Memory,
        "zh-1",
        "元智能体通过混合检索复用跨会话经验",
        &serde_json::json!({}),
    )
    .unwrap();
    s.index(
        SourceKind::Memory,
        "zh-2",
        "今天天气很好适合散步",
        &serde_json::json!({}),
    )
    .unwrap();
    let hits = s.search("混合检索", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source_id, "zh-1");
    let hits2 = s.search("元智能体", 10).unwrap();
    assert_eq!(hits2.len(), 1);
    assert_eq!(hits2[0].source_id, "zh-1");
}

#[test]
fn incremental_sync_new_source_auto_indexed() {
    let s = svc();
    assert_eq!(s.count(SourceKind::Board).unwrap(), 0);
    s.index(
        SourceKind::Board,
        "p1",
        "first post about leases",
        &serde_json::json!({}),
    )
    .unwrap();
    s.index(
        SourceKind::Board,
        "p2",
        "second post about events",
        &serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(s.count(SourceKind::Board).unwrap(), 2);
    assert_eq!(s.search("leases", 10).unwrap().len(), 1);
    assert_eq!(s.search("events", 10).unwrap().len(), 1);
}

#[test]
fn incremental_sync_unchanged_skipped() {
    let s = svc();
    s.index(
        SourceKind::Memory,
        "m1",
        "stable content",
        &serde_json::json!({}),
    )
    .unwrap();
    let o = s
        .index(
            SourceKind::Memory,
            "m1",
            "stable content",
            &serde_json::json!({}),
        )
        .unwrap();
    assert_eq!(o, SyncOutcome::Unchanged);
    assert_eq!(s.count(SourceKind::Memory).unwrap(), 1);
}

#[test]
fn incremental_sync_changed_reindexed() {
    let s = svc();
    s.index(
        SourceKind::Memory,
        "m1",
        "old content about apples",
        &serde_json::json!({}),
    )
    .unwrap();
    let o = s
        .index(
            SourceKind::Memory,
            "m1",
            "new content about bananas",
            &serde_json::json!({}),
        )
        .unwrap();
    assert_eq!(o, SyncOutcome::Reindexed);
    assert_eq!(s.count(SourceKind::Memory).unwrap(), 1);
    assert_eq!(s.search("apples", 10).unwrap().len(), 0);
    assert_eq!(s.search("bananas", 10).unwrap().len(), 1);
}

#[test]
fn bounded_results_limit() {
    let s = svc();
    for i in 0..(MAX_RESULTS + 20) {
        s.index(
            SourceKind::Rollout,
            &format!("t{i}"),
            "common token everywhere",
            &serde_json::json!({}),
        )
        .unwrap();
    }
    let hits = s.search("common", 5).unwrap();
    assert_eq!(hits.len(), 5);
    let capped = s.search("common", MAX_RESULTS + 50).unwrap();
    assert!(capped.len() <= MAX_RESULTS);
}

#[test]
fn search_cites_source_and_snippet() {
    let s = svc();
    s.index(
        SourceKind::Board,
        "post-42",
        "the verifier gate must pass with real exit code",
        &serde_json::json!({}),
    )
    .unwrap();
    let hits: Vec<Hit> = s.search("verifier", 10).unwrap();
    assert_eq!(hits[0].source_id, "post-42");
    assert_eq!(hits[0].kind, "board");
    assert!(hits[0].snippet.contains("verifier"));
}

#[test]
fn hybrid_ranking_bm25_order() {
    let s = svc();
    s.index(
        SourceKind::Memory,
        "low",
        "gate gate gate gate gate unrelated filler text here",
        &serde_json::json!({}),
    )
    .unwrap();
    s.index(SourceKind::Memory, "high", "gate", &serde_json::json!({}))
        .unwrap();
    let hits = s.search("gate", 10).unwrap();
    assert_eq!(hits.len(), 2);
    // bm25 lower is better; results must be returned in non-decreasing score order.
    assert!(hits[0].score <= hits[1].score);
    let ids: Vec<&str> = hits.iter().map(|h| h.source_id.as_str()).collect();
    assert!(ids.contains(&"low") && ids.contains(&"high"));
}

#[test]
fn embeddings_interface_reserved_no_model() {
    let mut v = ReservedVectorIndex;
    let e = v
        .upsert_embedding(SourceKind::Memory, "m1", &[0.1, 0.2, 0.3])
        .unwrap_err();
    assert!(matches!(e, MemoryError::EmbeddingsUnsupported));
    let e2 = v.search_vector(&[0.1, 0.2, 0.3], 5).unwrap_err();
    assert!(matches!(e2, MemoryError::EmbeddingsUnsupported));
}

#[test]
fn remove_source_removes_from_index() {
    let s = svc();
    s.index(
        SourceKind::Rollout,
        "t1",
        "removable content token",
        &serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(s.search("removable", 10).unwrap().len(), 1);
    assert!(s.remove(SourceKind::Rollout, "t1").unwrap());
    assert_eq!(s.search("removable", 10).unwrap().len(), 0);
    assert!(!s.remove(SourceKind::Rollout, "t1").unwrap());
}

#[test]
fn empty_query_rejected() {
    let s = svc();
    assert!(matches!(
        s.search("   ", 10).unwrap_err(),
        MemoryError::EmptyQuery
    ));
}

#[test]
fn cross_kind_search_isolation_by_filter() {
    let s = svc();
    s.index(
        SourceKind::Rollout,
        "r1",
        "shared keyword alpha",
        &serde_json::json!({}),
    )
    .unwrap();
    s.index(
        SourceKind::Board,
        "b1",
        "shared keyword alpha",
        &serde_json::json!({}),
    )
    .unwrap();
    let hits = s.search("alpha", 10).unwrap();
    assert_eq!(hits.len(), 2);
    let kinds: Vec<&str> = hits.iter().map(|h| h.kind.as_str()).collect();
    assert!(kinds.contains(&"rollout"));
    assert!(kinds.contains(&"board"));
}
