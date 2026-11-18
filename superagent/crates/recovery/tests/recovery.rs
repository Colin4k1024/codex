use std::collections::{BTreeMap, BTreeSet};

use superagent_contracts::{Node, NodeKind, Workflow};
use superagent_orchestrator::NodeState;
use superagent_recovery::{
    drive_gate_commit_with_fault, drive_send_with_fault, exclusive_resume, reconcile_unconfirmed,
    resume_plan, ExternalConfirmation, PriorNode, ReconcileAction, RecoveryBreakpoint,
    RecoveryError, ResumeContext, ResumeDecision, RunIdentity, SendFaultOutcome,
};
use superagent_storage::{AttemptState, Breakpoint, RunSpec, Store, StoreError};

fn node(id: &str, deps: &[&str]) -> Node {
    Node {
        id: id.into(),
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        kind: NodeKind::Capability {
            command_ref: "team/cmd".into(),
            inputs: vec![],
            outputs: vec![],
        },
        model: None,
        budget_tokens: None,
    }
}

fn wf() -> Workflow {
    Workflow {
        schema_version: 1,
        id: "w".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![node("a", &[]), node("b", &["a"]), node("c", &["b"])],
        gates: BTreeMap::new(),
        artifacts: BTreeMap::new(),
    }
}

fn spec(id: &str) -> RunSpec {
    RunSpec {
        id: id.into(),
        workflow_digest: "wd".into(),
        bundle_digest: "bd".into(),
        input_summary: "is".into(),
        repo_identity: "ri".into(),
        start_head: "sh".into(),
        config_summary: "cs".into(),
    }
}

fn store_with_run(id: &str) -> Store {
    let s = Store::open_in_memory().unwrap();
    s.create_run(&spec(id), 100).unwrap();
    s
}

fn prior(state: NodeState, digest: Option<&str>) -> PriorNode {
    PriorNode {
        state,
        artifact_digest: digest.map(|s| s.into()),
    }
}

// ---- exclusive resume ----

#[test]
fn exclusive_resume_matching_identity_succeeds() {
    let s = store_with_run("r1");
    let id = RunIdentity::from_spec(&spec("r1"));
    exclusive_resume(&s, "r1", "owner-a", 100, 60, &id).unwrap();
}

#[test]
fn exclusive_resume_mismatch_refused_and_lease_released() {
    let s = store_with_run("r1");
    let mut bad = RunIdentity::from_spec(&spec("r1"));
    bad.workflow_digest = "different".into();
    let e = exclusive_resume(&s, "r1", "owner-a", 100, 60, &bad).unwrap_err();
    assert!(matches!(e, RecoveryError::IdentityMismatch { field } if field == "workflow_digest"));
    // Lease was released on refusal: another owner can now acquire it.
    s.acquire_lease("r1", "owner-b", 100, 60).unwrap();
}

#[test]
fn exclusive_resume_lease_contention_rejected() {
    let s = store_with_run("r1");
    s.acquire_lease("r1", "owner-a", 100, 600).unwrap();
    let id = RunIdentity::from_spec(&spec("r1"));
    let e = exclusive_resume(&s, "r1", "owner-b", 100, 60, &id).unwrap_err();
    assert!(matches!(
        e,
        RecoveryError::Store(StoreError::LeaseHeld { .. })
    ));
}

#[test]
fn exclusive_resume_missing_run_not_found() {
    let s = Store::open_in_memory().unwrap();
    let id = RunIdentity::from_spec(&spec("nope"));
    let e = exclusive_resume(&s, "nope", "owner-a", 100, 60, &id).unwrap_err();
    assert!(matches!(e, RecoveryError::Store(StoreError::NotFound(..))));
}

// ---- digest-gated reuse / transitive invalidation ----

fn ctx_all_succeeded() -> ResumeContext {
    let mut pmap = BTreeMap::new();
    let mut cur = BTreeMap::new();
    for id in ["a", "b", "c"] {
        pmap.insert(
            id.into(),
            prior(NodeState::Succeeded, Some(&format!("dig-{id}"))),
        );
        cur.insert(id.into(), format!("dig-{id}"));
    }
    ResumeContext {
        prior: pmap,
        current_artifacts: cur,
        non_idempotent: BTreeSet::new(),
    }
}

#[test]
fn resume_reuses_succeeded_with_matching_digest() {
    let d = resume_plan(&wf(), &ctx_all_succeeded()).unwrap();
    assert_eq!(d["a"], ResumeDecision::Reuse);
    assert_eq!(d["b"], ResumeDecision::Reuse);
    assert_eq!(d["c"], ResumeDecision::Reuse);
}

#[test]
fn resume_reruns_when_artifact_digest_changed() {
    let mut ctx = ctx_all_succeeded();
    ctx.current_artifacts.insert("b".into(), "changed".into());
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Reuse);
    assert_eq!(d["b"], ResumeDecision::Rerun);
}

#[test]
fn resume_invalidates_downstream_transitively() {
    let mut ctx = ctx_all_succeeded();
    // Root artifact changed -> a Rerun -> b and c must Rerun even though their
    // own digests still match (dependency changed).
    ctx.current_artifacts.insert("a".into(), "changed".into());
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Rerun);
    assert_eq!(d["b"], ResumeDecision::Rerun);
    assert_eq!(d["c"], ResumeDecision::Rerun);
}

#[test]
fn resume_uncertain_yields_wait() {
    let mut ctx = ctx_all_succeeded();
    ctx.prior
        .insert("b".into(), prior(NodeState::Uncertain, Some("dig-b")));
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Reuse);
    assert_eq!(d["b"], ResumeDecision::Wait);
    assert_eq!(d["c"], ResumeDecision::Rerun);
}

#[test]
fn resume_missing_prior_yields_rerun() {
    let mut ctx = ctx_all_succeeded();
    ctx.prior.remove("a");
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Rerun);
    assert_eq!(d["b"], ResumeDecision::Rerun);
}

#[test]
fn resume_reruns_when_dependency_failed() {
    let mut ctx = ctx_all_succeeded();
    ctx.prior
        .insert("a".into(), prior(NodeState::Failed, Some("dig-a")));
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Rerun);
    assert_eq!(d["b"], ResumeDecision::Rerun);
    assert_eq!(d["c"], ResumeDecision::Rerun);
}

#[test]
fn resume_succeeded_without_artifact_digest_not_reused() {
    let mut ctx = ctx_all_succeeded();
    ctx.prior
        .insert("a".into(), prior(NodeState::Succeeded, None));
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Rerun);
}

// ---- uncertain reconciliation ----

#[test]
fn reconcile_confirmed_executed_binds_no_replay() {
    assert_eq!(
        reconcile_unconfirmed(false, ExternalConfirmation::ConfirmedExecuted),
        ReconcileAction::BindSucceeded
    );
    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::ConfirmedExecuted),
        ReconcileAction::BindSucceeded
    );
}

#[test]
fn reconcile_confirmed_not_executed_idempotent_retry() {
    assert_eq!(
        reconcile_unconfirmed(false, ExternalConfirmation::ConfirmedNotExecuted),
        ReconcileAction::Retry
    );
}

#[test]
fn reconcile_non_idempotent_never_blind_retry() {
    // Non-idempotent: even confirmed-not-executed waits for human handling;
    // unproven always waits. No blind replay path exists.
    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::ConfirmedNotExecuted),
        ReconcileAction::Wait
    );
    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::Unproven),
        ReconcileAction::Wait
    );
}

#[test]
fn reconcile_unproven_waits() {
    assert_eq!(
        reconcile_unconfirmed(false, ExternalConfirmation::Unproven),
        ReconcileAction::Wait
    );
}

// ---- four-breakpoint fault injection / crash recovery ----

#[test]
fn recovery_breakpoints_map_to_storage_hooks() {
    assert_eq!(RecoveryBreakpoint::BeforeSend.storage_hook(), None);
    assert_eq!(
        RecoveryBreakpoint::AfterSendBeforeResponse.storage_hook(),
        Some(Breakpoint::AfterIntentPersist)
    );
    assert_eq!(
        RecoveryBreakpoint::AfterResponseBeforeBind.storage_hook(),
        Some(Breakpoint::BeforeExternalBind)
    );
    assert_eq!(
        RecoveryBreakpoint::AfterGateBeforeCommit.storage_hook(),
        Some(Breakpoint::BeforeEventAppend)
    );
}

#[test]
fn fault_after_send_before_response_leaves_recoverable_intent() {
    let s = store_with_run("r1");
    s.set_fault_hook(|bp| {
        if bp == Breakpoint::AfterIntentPersist {
            Err(StoreError::FaultInjected(bp))
        } else {
            Ok(())
        }
    });
    let e = s
        .send_intent("r1", "a", 1, "{}", 100, Some("t"), Some("turn"), None)
        .unwrap_err();
    assert!(matches!(
        e,
        StoreError::FaultInjected(Breakpoint::AfterIntentPersist)
    ));
    s.clear_fault_hook();
    let unbound = s.unbound_intents("r1").unwrap();
    assert_eq!(unbound.len(), 1);
    assert_eq!(unbound[0].state, AttemptState::IntentPersisted);
    assert_eq!(unbound[0].node_id, "a");
}

#[test]
fn fault_after_response_before_bind_recoverable() {
    let s = store_with_run("r1");
    s.set_fault_hook(|bp| {
        if bp == Breakpoint::BeforeExternalBind {
            Err(StoreError::FaultInjected(bp))
        } else {
            Ok(())
        }
    });
    let e = s
        .send_intent("r1", "a", 1, "{}", 100, Some("t"), Some("turn"), None)
        .unwrap_err();
    assert!(matches!(
        e,
        StoreError::FaultInjected(Breakpoint::BeforeExternalBind)
    ));
    s.clear_fault_hook();
    let unbound = s.unbound_intents("r1").unwrap();
    assert_eq!(unbound.len(), 1);
    assert_eq!(unbound[0].state, AttemptState::IntentPersisted);
}

#[test]
fn crash_recovery_does_not_repeat_confirmed_success() {
    // A crash mid-run leaves an unbound intent for node 'a', but nodes already
    // confirmed Succeeded with matching digests are reused, never re-executed.
    let s = store_with_run("r1");
    s.set_fault_hook(|bp| {
        if bp == Breakpoint::AfterIntentPersist {
            Err(StoreError::FaultInjected(bp))
        } else {
            Ok(())
        }
    });
    let _ = s.send_intent("r1", "a", 1, "{}", 100, None, None, None);
    s.clear_fault_hook();
    assert_eq!(s.unbound_intents("r1").unwrap().len(), 1);

    let mut ctx = ctx_all_succeeded();
    // 'b' and 'c' were confirmed succeeded before the crash; 'a' is uncertain.
    ctx.prior
        .insert("a".into(), prior(NodeState::Uncertain, Some("dig-a")));
    let d = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d["a"], ResumeDecision::Wait);
    assert_eq!(d["b"], ResumeDecision::Rerun); // dependency 'a' not reused
    assert_eq!(d["c"], ResumeDecision::Rerun);

    // Once 'a' is reconciled as confirmed-executed and reused, downstream reuses.
    ctx.prior
        .insert("a".into(), prior(NodeState::Succeeded, Some("dig-a")));
    let d2 = resume_plan(&wf(), &ctx).unwrap();
    assert_eq!(d2["a"], ResumeDecision::Reuse);
    assert_eq!(d2["b"], ResumeDecision::Reuse);
    assert_eq!(d2["c"], ResumeDecision::Reuse);
}

#[test]
fn before_send_fault_aborts_clean_no_durable_intent() {
    let s = store_with_run("r1");
    let out = drive_send_with_fault(&s, "r1", "a", 1, "{}", 100, RecoveryBreakpoint::BeforeSend);
    assert!(matches!(out, SendFaultOutcome::AbortedBeforeSend));
    // BeforeSend aborts before persist: nothing durable, clean recoverable state.
    assert_eq!(s.unbound_intents("r1").unwrap().len(), 0);
}

#[test]
fn after_gate_before_commit_fault_recoverable_not_lost() {
    let s = store_with_run("r1");
    let e = drive_gate_commit_with_fault(&s, "r1", "node_succeeded", "a done", 100).unwrap_err();
    assert!(matches!(
        e,
        StoreError::FaultInjected(Breakpoint::BeforeEventAppend)
    ));
    // Terminal event was not appended (not silently lost); recovery re-appends.
    assert_eq!(s.events_after("r1", 0).unwrap().len(), 0);
    let seq = s
        .append_event("r1", "node_succeeded", "a done", 100)
        .unwrap();
    assert!(seq > 0);
    assert_eq!(s.events_after("r1", 0).unwrap().len(), 1);
}

#[test]
fn all_four_breakpoints_have_fault_coverage() {
    // BeforeSend -> clean abort; the two send boundaries -> recoverable intent;
    // AfterGateBeforeCommit -> recoverable terminal event. Every breakpoint is
    // exercised and leaves a non-lost state.
    let s = store_with_run("r1");
    assert!(matches!(
        drive_send_with_fault(&s, "r1", "a", 1, "{}", 100, RecoveryBreakpoint::BeforeSend),
        SendFaultOutcome::AbortedBeforeSend
    ));
    assert!(matches!(
        drive_send_with_fault(
            &s,
            "r1",
            "a",
            2,
            "{}",
            100,
            RecoveryBreakpoint::AfterSendBeforeResponse
        ),
        SendFaultOutcome::FaultedRecoverable(_)
    ));
    assert!(matches!(
        drive_send_with_fault(
            &s,
            "r1",
            "a",
            3,
            "{}",
            100,
            RecoveryBreakpoint::AfterResponseBeforeBind
        ),
        SendFaultOutcome::FaultedRecoverable(_)
    ));
    assert!(drive_gate_commit_with_fault(&s, "r1", "node_succeeded", "a", 100).is_err());
    // Two recoverable unbound intents from the send-boundary faults (a2, a3).
    assert_eq!(s.unbound_intents("r1").unwrap().len(), 2);
}
