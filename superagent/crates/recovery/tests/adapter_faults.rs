//! P2-7 / #84 real-adapter fault integration (model-free).
//!
//! Bridges the recovery core to the REAL #79 adapter driven by the model-free
//! fake_engine, proving the runtime acceptance over genuine transport faults:
//!   - engine crash mid-turn -> fresh-connection thread/read reconcile ->
//!     unproven => Wait (non-idempotent never blind-replayed);
//!   - crash then proven-completed reconcile -> BindSucceeded (no replay of an
//!     already-confirmed success);
//!   - pending approval is never auto-answered; the old connection's approval
//!     request ID is not replayed on a fresh connection (resume re-confirms wait);
//!   - SessionEnd (engine exit) alone never marks a run succeeded;
//!   - an oversize frame surfaces FrameViolation (terminal never silently lost).
//!
//! No model, no network. fake_engine is deliberately NOT named "codex".

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use superagent_adapter::{
    reconcile_thread, Adapter, AdapterConfig, AdapterEvent, ApprovalHandler, Decision, Disposition,
    ServerRequest,
};
use superagent_contracts::{Node, NodeKind, Workflow, SCHEMA_VERSION};
use superagent_orchestrator::NodeState;
use superagent_recovery::{
    reconcile_unconfirmed, resume_plan, ExternalConfirmation, PriorNode, ReconcileAction,
    ResumeContext, ResumeDecision,
};

fn fake_engine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../adapter/tests/fake_engine.py")
        .canonicalize()
        .expect("fake_engine.py present")
}

fn cfg(
    dir: &std::path::Path,
    scenario: &str,
    turn_status: &str,
    max_frame: usize,
) -> AdapterConfig {
    AdapterConfig {
        argv: vec![
            "python3".into(),
            fake_engine().to_string_lossy().into_owned(),
            "--scenario".into(),
            scenario.into(),
            "--state-file".into(),
            dir.join("state.json").to_string_lossy().into_owned(),
            "--turn-status".into(),
            turn_status.into(),
        ],
        codex_home: None,
        inherit_env: true,
        cwd: Some(dir.to_path_buf()),
        run_log_path: Some(dir.join("engine.jsonl")),
        run_deadline: Duration::from_secs(30),
        max_frame_bytes: max_frame,
        ..AdapterConfig::default()
    }
}

fn spawn(dir: &std::path::Path, scenario: &str, turn_status: &str, max_frame: usize) -> Adapter {
    let a = Adapter::spawn_default(cfg(dir, scenario, turn_status, max_frame)).expect("spawn");
    a.handshake().expect("handshake");
    a
}

/// Human-gated handler: defers command-execution approvals so they stay pending
/// (the adapter never invents a grant); recovery waits for an explicit decision.
struct DeferHandler;
impl ApprovalHandler for DeferHandler {
    fn handle(&self, r: &ServerRequest) -> Decision {
        if r.method == "item/commandExecution/requestApproval" {
            Decision::Defer
        } else {
            Decision::Error {
                code: -32000,
                message: "declined".into(),
            }
        }
    }
}

fn start_thread(a: &Adapter, dir: &std::path::Path) -> String {
    let r = a
        .request(
            "thread/start",
            json!({"cwd": dir}),
            Some(Duration::from_secs(10)),
        )
        .expect("thread/start");
    r.get("thread")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .expect("thread id")
        .to_string()
}

fn input() -> Value {
    json!([{"type": "text", "text": "probe"}])
}

/// Drain events until a terminal fault/terminal turn is observed (no silent drop).
fn wait_terminal(a: &Adapter, secs: u64) -> AdapterEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            panic!("no terminal event observed within {secs}s");
        }
        match a.wait_event(remaining) {
            Ok(ev) if ev.is_terminal() => return ev,
            Ok(_) => continue,
            Err(e) => panic!("wait_event error before terminal: {e}"),
        }
    }
}

fn one_node_wf() -> Workflow {
    Workflow {
        schema_version: SCHEMA_VERSION,
        id: "rec".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![Node {
            id: "n".into(),
            depends_on: vec![],
            kind: NodeKind::Capability {
                command_ref: "c".into(),
                inputs: vec![],
                outputs: vec![],
            },
            model: None,
            budget_tokens: None,
        }],
        gates: BTreeMap::new(),
        artifacts: BTreeMap::new(),
    }
}

#[test]
fn crash_mid_turn_reconciles_unproven_and_never_blind_replays() {
    let dir = tempfile::tempdir().unwrap();
    let a = spawn(dir.path(), "crash", "inProgress", 16 * 1024 * 1024);
    let tid = start_thread(&a, dir.path());
    // Engine exits(9) before answering turn/start; the fault must surface.
    let _ = a.request(
        "turn/start",
        json!({"threadId": tid, "input": input()}),
        Some(Duration::from_secs(5)),
    );
    let ev = wait_terminal(&a, 10);
    assert!(
        ev.is_terminal(),
        "crash must surface a terminal fault: {ev:?}"
    );
    a.close();

    // Fresh-connection recovery: thread/read shows the turn still inProgress
    // (non-terminal) => outcome unproven => Wait, never blind replay.
    let b = spawn(dir.path(), "reconcile", "inProgress", 16 * 1024 * 1024);
    let report = reconcile_thread(&b, &tid, false).expect("reconcile");
    assert!(matches!(report.disposition, Disposition::Uncertain { .. }));
    assert_eq!(report.last_turn_status.as_deref(), Some("inProgress"));
    b.close();

    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::Unproven),
        ReconcileAction::Wait,
        "non-idempotent unproven must wait, never blind retry"
    );
    assert_eq!(
        reconcile_unconfirmed(false, ExternalConfirmation::Unproven),
        ReconcileAction::Wait
    );
}

#[test]
fn crash_then_proven_completed_binds_success_without_replay() {
    let dir = tempfile::tempdir().unwrap();
    let a = spawn(dir.path(), "crash", "inProgress", 16 * 1024 * 1024);
    let tid = start_thread(&a, dir.path());
    let _ = a.request(
        "turn/start",
        json!({"threadId": tid, "input": input()}),
        Some(Duration::from_secs(5)),
    );
    let _ = wait_terminal(&a, 10);
    a.close();

    // Fresh connection proves the turn actually completed => bind, do not replay.
    let b = spawn(dir.path(), "reconcile", "completed", 16 * 1024 * 1024);
    let report = reconcile_thread(&b, &tid, false).expect("reconcile");
    assert_eq!(report.disposition, Disposition::ReconciledTerminal);
    assert_eq!(report.last_turn_status.as_deref(), Some("completed"));
    b.close();

    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::ConfirmedExecuted),
        ReconcileAction::BindSucceeded,
        "confirmed success is bound, never re-executed"
    );
}

#[test]
fn pending_approval_never_auto_answered_and_old_id_not_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let a = Adapter::spawn(
        cfg(dir.path(), "approval_hold", "inProgress", 16 * 1024 * 1024),
        Arc::new(DeferHandler),
    )
    .unwrap();
    a.handshake().unwrap();
    let tid = start_thread(&a, dir.path());
    // holdApproval: engine raises a server request and holds the turn open.
    let _ = a
        .send_request(
            "turn/start",
            json!({"threadId": tid, "input": input(), "holdApproval": true}),
        )
        .unwrap();
    let pending = a
        .wait_pending_server_request(Duration::from_secs(10))
        .expect("approval must surface as pending (never auto-answered)");
    assert_eq!(pending.method, "item/commandExecution/requestApproval");
    // The adapter never invents an approval decision; recovery waits.
    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::Unproven),
        ReconcileAction::Wait
    );
    a.close();

    // A fresh connection has NO pending server request: the old connection's
    // approval request ID is not carried over / replayed. Resume must re-confirm
    // the waiting state rather than answer a stale ID.
    let b = spawn(dir.path(), "echo", "completed", 16 * 1024 * 1024);
    let stale = b.wait_pending_server_request(Duration::from_secs(2));
    assert!(
        stale.is_err(),
        "fresh connection must have no replayed approval"
    );
    b.close();
}

#[test]
fn session_end_alone_never_marks_run_succeeded() {
    let dir = tempfile::tempdir().unwrap();
    let a = spawn(dir.path(), "ignore_eof", "completed", 16 * 1024 * 1024);
    let tid = start_thread(&a, dir.path());
    let _ = a.request(
        "turn/start",
        json!({"threadId": tid, "input": input()}),
        Some(Duration::from_secs(10)),
    );
    // Close stdin; ignore_eof engine refuses to exit -> adapter kills it. The
    // resulting SessionEnd (EngineExited) is a fault signal, NOT success.
    a.close();
    let ev = wait_terminal(&a, 10);
    assert!(
        ev.is_terminal(),
        "session end must surface a terminal event: {ev:?}"
    );

    // An interrupted run whose node terminal was never bound stays Uncertain;
    // resume yields Wait, never Reuse/Succeeded merely because the session ended.
    let wf = one_node_wf();
    let mut prior = BTreeMap::new();
    prior.insert(
        "n".to_string(),
        PriorNode {
            state: NodeState::Uncertain,
            artifact_digest: None,
        },
    );
    let ctx = ResumeContext {
        prior,
        current_artifacts: BTreeMap::new(),
        non_idempotent: BTreeSet::new(),
    };
    let plan = resume_plan(&wf, &ctx).expect("plan");
    assert_eq!(plan["n"], ResumeDecision::Wait);
}

#[test]
fn oversize_frame_surfaces_violation_terminal_not_silently_lost() {
    let dir = tempfile::tempdir().unwrap();
    let a = spawn(dir.path(), "echo", "completed", 1024);
    let tid = start_thread(&a, dir.path());
    let _ = a.request(
        "test/triggerOversize",
        json!({"bytes": 200000}),
        Some(Duration::from_secs(5)),
    );
    let ev = wait_terminal(&a, 10);
    assert!(
        matches!(ev, AdapterEvent::FrameViolation { .. }),
        "oversize frame must surface FrameViolation, not be dropped: {ev:?}"
    );
    a.close();
    // Transport fault leaves the outcome unproven => recovery waits.
    assert_eq!(
        reconcile_unconfirmed(true, ExternalConfirmation::Unproven),
        ReconcileAction::Wait
    );
    let _ = tid;
}
