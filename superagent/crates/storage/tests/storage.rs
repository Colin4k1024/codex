//! P2-3 / #80 acceptance tests: lease contention rejected, transaction
//! consistency, injectable send/persist breakpoints (for P2-7 / #84), event
//! replay sequence, bounded summaries, migrations. All model-free.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use superagent_storage::{
    AttemptState, Breakpoint, RunSpec, Store, StoreError, MAX_SUMMARY_BYTES, SCHEMA_VERSION,
};
use tempfile::TempDir;

fn spec(id: &str) -> RunSpec {
    RunSpec {
        id: id.to_string(),
        workflow_digest: "wf-digest-abc".to_string(),
        bundle_digest: "bundle-digest-def".to_string(),
        input_summary: "goal: close issues".to_string(),
        repo_identity: "Colin4k1024/codex".to_string(),
        start_head: "697de6498".to_string(),
        config_summary: "model=gpt-6-astra".to_string(),
    }
}

fn file_store(dir: &TempDir) -> Store {
    Store::open(dir.path().join("state.db")).unwrap()
}

#[test]
fn migration_is_idempotent_and_versions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.db");
    {
        let s = Store::open(&path).unwrap();
        s.create_run(&spec("r1"), 1000).unwrap();
    }
    // Reopen: migration must not wipe data or fail.
    let s = Store::open(&path).unwrap();
    let (got, state) = s.get_run("r1").unwrap();
    assert_eq!(got.workflow_digest, "wf-digest-abc");
    assert_eq!(state, "pending");
    let v: u32 = s.schema_version().unwrap();
    assert_eq!(v, SCHEMA_VERSION);
}

#[test]
fn run_fixed_identity_round_trips() {
    let s = Store::open_in_memory().unwrap();
    let sp = spec("run-identity");
    s.create_run(&sp, 42).unwrap();
    let (got, state) = s.get_run("run-identity").unwrap();
    assert_eq!(got.id, sp.id);
    assert_eq!(got.workflow_digest, sp.workflow_digest);
    assert_eq!(got.bundle_digest, sp.bundle_digest);
    assert_eq!(got.input_summary, sp.input_summary);
    assert_eq!(got.repo_identity, sp.repo_identity);
    assert_eq!(got.start_head, sp.start_head);
    assert_eq!(got.config_summary, sp.config_summary);
    assert_eq!(state, "pending");
}

#[test]
fn duplicate_run_id_rejected() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("dup"), 1).unwrap();
    let err = s.create_run(&spec("dup"), 2).unwrap_err();
    assert!(matches!(err, StoreError::Sqlite(_)));
}

#[test]
fn exclusive_lease_contention_rejected() {
    let dir = TempDir::new().unwrap();
    let a = file_store(&dir);
    let b = file_store(&dir);
    a.create_run(&spec("leased"), 1000).unwrap();

    // A acquires; B must be rejected while A's lease is unexpired.
    a.acquire_lease("leased", "owner-A", 1000, 60).unwrap();
    let err = b.acquire_lease("leased", "owner-B", 1001, 60).unwrap_err();
    match err {
        StoreError::LeaseHeld { run_id, owner } => {
            assert_eq!(run_id, "leased");
            assert_eq!(owner, "owner-A");
        }
        other => panic!("expected LeaseHeld, got {other:?}"),
    }

    // Same owner may renew.
    a.acquire_lease("leased", "owner-A", 1050, 60).unwrap();

    // After A releases, B can acquire.
    a.release_lease("leased", "owner-A").unwrap();
    b.acquire_lease("leased", "owner-B", 1100, 60).unwrap();

    // A cannot steal B's live lease.
    let err = a.acquire_lease("leased", "owner-A", 1101, 60).unwrap_err();
    assert!(matches!(err, StoreError::LeaseHeld { .. }));
}

#[test]
fn expired_lease_can_be_taken_over() {
    let dir = TempDir::new().unwrap();
    let a = file_store(&dir);
    let b = file_store(&dir);
    a.create_run(&spec("exp"), 1000).unwrap();
    a.acquire_lease("exp", "owner-A", 1000, 10).unwrap(); // expires 1010
    // B arrives after expiry -> takeover allowed.
    b.acquire_lease("exp", "owner-B", 1011, 10).unwrap();
    let err = a.acquire_lease("exp", "owner-A", 1012, 10).unwrap_err();
    assert!(matches!(err, StoreError::LeaseHeld { .. }));
}

#[test]
fn lease_on_missing_run_is_not_found() {
    let s = Store::open_in_memory().unwrap();
    let err = s.acquire_lease("nope", "o", 1, 10).unwrap_err();
    assert!(matches!(err, StoreError::NotFound(_)));
}

#[test]
fn intent_persists_then_binds_in_separate_transactions() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    let id = s.persist_intent("r", "n1", 1, "{\"argv\":[\"echo\"]}", 10).unwrap();
    let a = s.get_attempt(id).unwrap();
    assert_eq!(a.state, AttemptState::IntentPersisted);
    assert!(a.thread_id.is_none());
    assert!(a.turn_id.is_none());
    assert!(a.process_id.is_none());

    s.bind_external_ids(id, Some("thread-1"), Some("turn-1"), Some(4242)).unwrap();
    let a = s.get_attempt(id).unwrap();
    assert_eq!(a.state, AttemptState::Bound);
    assert_eq!(a.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(a.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(a.process_id, Some(4242));
}

#[test]
fn fault_between_persist_and_bind_leaves_recoverable_intent() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    // Abort right before the external bind commits.
    s.set_fault_hook(|bp| {
        if bp == Breakpoint::BeforeExternalBind {
            Err(StoreError::FaultInjected(bp))
        } else {
            Ok(())
        }
    });
    let err = s
        .send_intent("r", "n1", 1, "{}", 10, Some("t"), Some("u"), Some(1))
        .unwrap_err();
    assert!(matches!(err, StoreError::FaultInjected(Breakpoint::BeforeExternalBind)));

    // Intent is durable and unbound -> recoverable.
    let unbound = s.unbound_intents("r").unwrap();
    assert_eq!(unbound.len(), 1);
    assert_eq!(unbound[0].state, AttemptState::IntentPersisted);
    assert!(unbound[0].thread_id.is_none());

    // Recovery: clear fault, re-bind the same intent (no duplicate attempt).
    s.clear_fault_hook();
    let id = unbound[0].id;
    s.bind_external_ids(id, Some("t"), Some("u"), Some(1)).unwrap();
    assert_eq!(s.get_attempt(id).unwrap().state, AttemptState::Bound);
    assert!(s.unbound_intents("r").unwrap().is_empty());
}

#[test]
fn fault_after_persist_breakpoint_is_injectable() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    s.set_fault_hook(move |bp| {
        if bp == Breakpoint::AfterIntentPersist {
            h.fetch_add(1, Ordering::SeqCst);
            Err(StoreError::FaultInjected(bp))
        } else {
            Ok(())
        }
    });
    let err = s.persist_intent("r", "n1", 1, "{}", 10).unwrap_err();
    assert!(matches!(err, StoreError::FaultInjected(Breakpoint::AfterIntentPersist)));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // The intent committed before the breakpoint still exists.
    assert_eq!(s.unbound_intents("r").unwrap().len(), 1);
}

#[test]
fn duplicate_attempt_no_rejected() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    s.persist_intent("r", "n1", 1, "{}", 10).unwrap();
    let err = s.persist_intent("r", "n1", 1, "{}", 11).unwrap_err();
    assert!(matches!(err, StoreError::Sqlite(_)));
    // A different attempt_no is fine.
    s.persist_intent("r", "n1", 2, "{}", 12).unwrap();
}

#[test]
fn event_sequence_is_monotonic_and_replayable() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    let s1 = s.append_event("r", "node_started", "n1 started", 10).unwrap();
    let s2 = s.append_event("r", "node_finished", "n1 done", 11).unwrap();
    let s3 = s.append_event("r", "gate_passed", "gate ok", 12).unwrap();
    assert!(s1 < s2 && s2 < s3);

    let replay = s.events_after("r", s1).unwrap();
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].seq, s2);
    assert_eq!(replay[1].seq, s3);
    assert_eq!(replay[0].kind, "node_finished");

    // Full replay from 0.
    assert_eq!(s.events_after("r", 0).unwrap().len(), 3);
}

#[test]
fn event_append_breakpoint_injectable() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    s.set_fault_hook(|bp| {
        if bp == Breakpoint::BeforeEventAppend {
            Err(StoreError::FaultInjected(bp))
        } else {
            Ok(())
        }
    });
    let err = s.append_event("r", "k", "summary", 1).unwrap_err();
    assert!(matches!(err, StoreError::FaultInjected(Breakpoint::BeforeEventAppend)));
    assert!(s.events_after("r", 0).unwrap().is_empty());
}

#[test]
fn oversize_summaries_rejected_explicitly() {
    let s = Store::open_in_memory().unwrap();
    let big = "x".repeat(MAX_SUMMARY_BYTES + 1);

    let mut sp = spec("big");
    sp.input_summary = big.clone();
    let err = s.create_run(&sp, 1).unwrap_err();
    assert!(matches!(err, StoreError::SummaryTooLarge { .. }));

    s.create_run(&spec("ok"), 1).unwrap();
    let err = s.append_event("ok", "k", &big, 1).unwrap_err();
    assert!(matches!(err, StoreError::SummaryTooLarge { .. }));

    let err = s
        .record_artifact("ok", None, "/tmp/a", "text/plain", None, None, None, Some(&big))
        .unwrap_err();
    assert!(matches!(err, StoreError::SummaryTooLarge { .. }));

    // Exactly at the cap is accepted.
    let at_cap = "y".repeat(MAX_SUMMARY_BYTES);
    s.append_event("ok", "k", &at_cap, 2).unwrap();
}

#[test]
fn artifact_reference_stored_without_blob() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    let id = s
        .record_artifact("r", Some("n1"), "/tmp/out.json", "application/json",
                         Some("sha256:abc"), Some(123), Some("gate-result"), Some("bounded"))
        .unwrap();
    assert!(id > 0);
}

#[test]
fn seed_nodes_from_contracts_workflow() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    let wf = superagent_contracts::Workflow {
        schema_version: superagent_contracts::SCHEMA_VERSION,
        id: "wf".to_string(),
        version: "1".to_string(),
        variables: Default::default(),
        nodes: vec![
            superagent_contracts::Node {
                id: "a".to_string(),
                depends_on: vec![],
                kind: superagent_contracts::NodeKind::Verification {
                    argv: vec!["true".to_string()],
                    gates: vec!["g".to_string()],
                },
                model: None,
                budget_tokens: None,
            },
            superagent_contracts::Node {
                id: "b".to_string(),
                depends_on: vec!["a".to_string()],
                kind: superagent_contracts::NodeKind::Verification {
                    argv: vec!["true".to_string()],
                    gates: vec!["g".to_string()],
                },
                model: None,
                budget_tokens: None,
            },
        ],
        gates: Default::default(),
        artifacts: Default::default(),
    };
    s.seed_nodes("r", &wf).unwrap();
    // Idempotent re-seed.
    s.seed_nodes("r", &wf).unwrap();
}

#[test]
fn attempt_state_transitions_explicit() {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec("r"), 1).unwrap();
    let id = s.send_intent("r", "n1", 1, "{}", 10, Some("t"), None, None).unwrap();
    s.set_attempt_state(id, AttemptState::Running).unwrap();
    assert_eq!(s.get_attempt(id).unwrap().state, AttemptState::Running);
    s.set_attempt_state(id, AttemptState::Succeeded).unwrap();
    assert_eq!(s.get_attempt(id).unwrap().state, AttemptState::Succeeded);
    let err = s.set_attempt_state(9999, AttemptState::Failed).unwrap_err();
    assert!(matches!(err, StoreError::NotFound(_)));
}