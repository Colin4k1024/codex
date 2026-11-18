//! P2-6 / #83 CLI integration tests (model-free).
//!
//! Proves the binding acceptance over genuine transport (fake_engine subprocess,
//! real stdio JSON-RPC) and pure store/recovery logic — no model, no network:
//!   - dry-run starts no model/command and rejects a cycle before execution;
//!   - cancel terminates the active exec, drains terminals, closes the engine
//!     and marks the run cancelled with durable cleanup evidence;
//!   - the cancel guard schedules no new node after cancellation;
//!   - report separates verified/unverified and baseline/this-run changes and
//!     records version/usage/evidence;
//!   - status reads durable state/identity/nodes/events;
//!   - resume enforces exclusive identity (mismatch refused) + reuse/wait plan;
//!   - a non-interactive run needing an approval returns blocked + stable exit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};
use superagent_adapter::{Adapter, AdapterConfig};
use superagent_cli::cancel::{cancel_exec_session, CancelGuard};
use superagent_cli::plan::{ctx_from_workflow, dry_run_plan, to_json};
use superagent_cli::report::{build_report, Change, NodeOutcome, ReportInput};
use superagent_cli::resume::{resume_run, ResumeLease};
use superagent_cli::run::{execute_run, RunOptions};
use superagent_cli::status::status_json;
use superagent_cli::{exit, CancelToken, CliError};
use superagent_contracts::{Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION};
use superagent_executor::EngineSpec;
use superagent_orchestrator::{ExecOutcome, NodeExecutor, NodeState, Policy};
use superagent_recovery::{PriorNode, ResumeContext, RunIdentity};
use superagent_storage::{RunSpec, Store};

fn fake_engine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../adapter/tests/fake_engine.py")
        .canonicalize()
        .expect("fake_engine.py present")
}

fn engine_spec(dir: &std::path::Path) -> EngineSpec {
    EngineSpec {
        argv: vec![
            "python3".into(),
            fake_engine().to_string_lossy().into_owned(),
            "--scenario".into(),
            "echo".into(),
            "--state-file".into(),
            dir.join("state.json").to_string_lossy().into_owned(),
            "--turn-status".into(),
            "completed".into(),
        ],
        codex_home: None,
        cwd: Some(dir.to_path_buf()),
        run_deadline: Duration::from_secs(30),
        max_artifact_bytes: 1 << 20,
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

fn cap_node(id: &str) -> Node {
    Node {
        id: id.into(),
        depends_on: vec![],
        kind: NodeKind::Capability {
            command_ref: "edit".into(),
            inputs: vec![],
            outputs: vec![],
        },
        model: None,
        budget_tokens: None,
    }
}

fn verify_node(id: &str, deps: Vec<String>, gate: &str) -> Node {
    Node {
        id: id.into(),
        depends_on: deps,
        kind: NodeKind::Verification {
            argv: vec!["python3".into(), "-c".into(), "pass".into()],
            gates: vec![gate.into()],
        },
        model: None,
        budget_tokens: None,
    }
}

fn cmd_gate(id: &str, code: &str) -> Gate {
    Gate {
        id: id.into(),
        kind: GateKind::Command {
            argv: vec!["python3".into(), "-c".into(), code.into()],
            expect_exit_code: 0,
        },
    }
}

fn workflow(nodes: Vec<Node>, gates: Vec<Gate>) -> Workflow {
    Workflow {
        schema_version: SCHEMA_VERSION,
        id: "cli-test".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes,
        gates: gates.into_iter().map(|g| (g.id.clone(), g)).collect(),
        artifacts: BTreeMap::new(),
    }
}

// 1. dry-run is pure: validates + schedules, attests no model/command started.
#[test]
fn dry_run_starts_no_model_or_command() {
    let wf = workflow(
        vec![
            cap_node("modify"),
            verify_node("verify", vec!["modify".into()], "g1"),
        ],
        vec![cmd_gate("g1", "import sys; sys.exit(0)")],
    );
    let ctx = ctx_from_workflow(&wf);
    let plan = dry_run_plan(&wf, &ctx).expect("valid plan");
    assert_eq!(plan.order, vec!["modify".to_string(), "verify".to_string()]);
    let j = to_json(&plan);
    assert_eq!(j["dryRun"], json!(true));
    assert_eq!(j["startedModel"], json!(false));
    assert_eq!(j["startedCommand"], json!(false));
    assert_eq!(j["validated"], json!(true));
}

// 2. a DAG cycle is rejected before any execution could occur.
#[test]
fn dry_run_rejects_cycle_before_execution() {
    let mut a = cap_node("a");
    let mut b = cap_node("b");
    a.depends_on = vec!["b".into()];
    b.depends_on = vec!["a".into()];
    let wf = workflow(vec![a, b], vec![]);
    let ctx = ctx_from_workflow(&wf);
    let err = dry_run_plan(&wf, &ctx).expect_err("cycle must fail");
    assert!(err.0.contains("cycle"), "got: {}", err.0);
}

// 3. report separates verified/unverified + baseline/this-run changes; complete.
#[test]
fn report_separates_verified_unverified_and_changes() {
    let inp = ReportInput {
        run_id: "r1".into(),
        version: "0.1.0".into(),
        run_state: "Failed".into(),
        nodes: vec![
            NodeOutcome {
                id: "ok".into(),
                state: "Succeeded".into(),
                proven_gates: vec!["g1".into()],
                reason: "all required gates proven".into(),
            },
            NodeOutcome {
                id: "bad".into(),
                state: "Failed".into(),
                proven_gates: vec![],
                reason: "unproven gates".into(),
            },
        ],
        tokens_used: 1234,
        wall_secs: 56,
        baseline_changes: vec![Change {
            path: "pre.txt".into(),
            digest: Some("aaa".into()),
        }],
        run_changes: vec![Change {
            path: "new.txt".into(),
            digest: Some("bbb".into()),
        }],
        evidence: vec!["event:1:node_final".into()],
    };
    let r = build_report(&inp);
    assert_eq!(r["verified"].as_array().unwrap().len(), 1);
    assert_eq!(r["verified"][0]["id"], json!("ok"));
    assert_eq!(r["unverified"].as_array().unwrap().len(), 1);
    assert_eq!(r["unverified"][0]["id"], json!("bad"));
    assert_eq!(r["changes"]["baseline"][0]["path"], json!("pre.txt"));
    assert_eq!(r["changes"]["thisRun"][0]["path"], json!("new.txt"));
    assert_eq!(r["version"], json!("0.1.0"));
    assert_eq!(r["usage"]["tokensUsed"], json!(1234));
    assert_eq!(r["usage"]["wallSecs"], json!(56));
    assert!(!r["evidence"].as_array().unwrap().is_empty());
}

// 4. status reads durable run state/identity/nodes/events from the store.
#[test]
fn status_json_reads_durable_state() {
    let store = Store::open_in_memory().unwrap();
    store.create_run(&spec("r1"), 0).unwrap();
    store.set_run_state("r1", "succeeded").unwrap();
    store
        .append_event("r1", "node_final", "modify: Succeeded", 1)
        .unwrap();
    store
        .append_event("r1", "run_final", "run: Succeeded", 2)
        .unwrap();
    let v = status_json(&store, "r1").unwrap();
    assert_eq!(v["state"], json!("succeeded"));
    assert_eq!(v["identity"]["workflowDigest"], json!("wd"));
    assert_eq!(v["nodes"][0]["id"], json!("modify"));
    assert_eq!(v["nodes"][0]["state"], json!("Succeeded"));
    assert!(v["events"].as_array().unwrap().len() >= 2);
}

// 5. resume enforces exclusive identity; mismatch refused; reuse/wait plan.
#[test]
fn resume_identity_match_reuse_and_mismatch_refused() {
    let store = Store::open_in_memory().unwrap();
    store.create_run(&spec("r1"), 0).unwrap();
    let wf = workflow(vec![cap_node("n")], vec![]);
    let mut prior = BTreeMap::new();
    prior.insert(
        "n".to_string(),
        PriorNode {
            state: NodeState::Succeeded,
            artifact_digest: Some("d1".into()),
        },
    );
    let mut arts = BTreeMap::new();
    arts.insert("n".to_string(), "d1".to_string());
    let ctx = ResumeContext {
        prior,
        current_artifacts: arts,
        non_idempotent: BTreeSet::new(),
    };
    let expected = RunIdentity::from_spec(&spec("r1"));
    let v = resume_run(
        &store,
        "r1",
        &ResumeLease {
            owner: "owner",
            now: 0,
            ttl_secs: 3600,
        },
        &expected,
        &wf,
        &ctx,
    )
    .unwrap();
    assert_eq!(v["identityMatched"], json!(true));
    assert_eq!(v["decisions"]["n"], json!("Reuse"));
    store.release_lease("r1", "owner").unwrap();

    // Mismatched identity is refused (and the lease released by the core).
    let mut bad = spec("r1");
    bad.bundle_digest = "different".into();
    let bad_id = RunIdentity::from_spec(&bad);
    let err = resume_run(
        &store,
        "r1",
        &ResumeLease {
            owner: "owner2",
            now: 0,
            ttl_secs: 3600,
        },
        &bad_id,
        &wf,
        &ctx,
    )
    .unwrap_err();
    assert!(matches!(err, CliError::Recovery(_)), "got: {err:?}");
}

// 6. once cancelled, the guard schedules NO new node (inner never invoked).
struct PanicExecutor;
impl NodeExecutor for PanicExecutor {
    fn execute(&mut self, _: &Node, _: u32) -> ExecOutcome {
        panic!("cancelled guard must not schedule new work");
    }
}

#[test]
fn cancel_guard_stops_new_nodes() {
    let token = CancelToken::new();
    token.cancel();
    let mut guard = CancelGuard {
        inner: PanicExecutor,
        token,
    };
    let out = guard.execute(&cap_node("n"), 1);
    assert_eq!(out.kind, superagent_orchestrator::ExecKind::Blocked);
    assert!(out.detail.contains("cancelled"));
}

// 7. cancel terminates the active exec, closes the engine, marks cancelled.
#[test]
fn cancel_exec_session_cleanup_evidence_and_cancelled_mark() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = AdapterConfig {
        argv: vec![
            "python3".into(),
            fake_engine().to_string_lossy().into_owned(),
            "--scenario".into(),
            "echo".into(),
            "--state-file".into(),
            dir.path().join("state.json").to_string_lossy().into_owned(),
            "--turn-status".into(),
            "completed".into(),
        ],
        cwd: Some(dir.path().to_path_buf()),
        run_deadline: Duration::from_secs(30),
        ..Default::default()
    };
    let adapter = Adapter::spawn_default(cfg).unwrap();
    adapter.handshake().unwrap();
    let store = Store::open_in_memory().unwrap();
    store.create_run(&spec("r1"), 0).unwrap();
    // Start a long-running exec with a caller-known processId (non-blocking).
    // pidFile is written by the engine right after it registers the child, so
    // polling for it gives a deterministic "exec is live" sync point.
    let pid_file = dir.path().join("exec.pid");
    let _rid = adapter
        .send_request(
            "command/exec",
            json!({
                "command": ["python3", "-c", "import time; time.sleep(30)"],
                "processId": "p-cancel",
                "pidFile": pid_file,
                "cwd": dir.path(),
            }),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !pid_file.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "exec never registered (pidFile absent)"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let ev = cancel_exec_session(&adapter, "p-cancel", &store, "r1", 1).unwrap();
    assert!(ev.terminated, "exec must be terminated");
    assert!(ev.engine_closed, "engine must be closed");
    let (_, state) = store.get_run("r1").unwrap();
    assert_eq!(state, "cancelled");
    let events = store.events_after("r1", 0).unwrap();
    assert!(
        events.iter().any(|e| e.kind == "run_cancelled"),
        "cleanup evidence must be durable"
    );
}

// 8. non-interactive run needing an approval -> blocked + stable exit code.
#[test]
fn non_interactive_approval_returns_blocked_stable_exit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Rc::new(Store::open_in_memory().unwrap());
    let gate = Gate {
        id: "ga".into(),
        kind: GateKind::Approval {
            prompt: "operator authorization required".into(),
        },
    };
    let wf = workflow(vec![verify_node("approve", vec![], "ga")], vec![gate]);
    let ctx = ctx_from_workflow(&wf);
    let opts = RunOptions {
        workflow: wf,
        ctx,
        policy: Policy::default(),
        engine: engine_spec(dir.path()),
        workspace: dir.path().to_path_buf(),
        evidence_dir: dir.path().join("ev"),
        run_id: "r-blocked".into(),
        spec: spec("r-blocked"),
        now: 0,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(10),
        output_cap_bytes: 4096,
        task_override: None,
        cancel: CancelToken::new(),
    };
    let err = execute_run(store, opts).unwrap_err();
    assert!(matches!(err, CliError::Blocked(_)), "got: {err:?}");
    assert_eq!(err.exit_code(), exit::BLOCKED);
}

// 9. a real (model-free) run succeeds only via gate evidence end-to-end.
#[test]
fn run_succeeds_via_real_gate_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let store = Rc::new(Store::open_in_memory().unwrap());
    let wf = workflow(
        vec![
            cap_node("modify"),
            verify_node("verify", vec!["modify".into()], "g1"),
        ],
        vec![cmd_gate("g1", "import sys; sys.exit(0)")],
    );
    let ctx = ctx_from_workflow(&wf);
    let opts = RunOptions {
        workflow: wf,
        ctx,
        policy: Policy::default(),
        engine: engine_spec(dir.path()),
        workspace: dir.path().to_path_buf(),
        evidence_dir: dir.path().join("ev"),
        run_id: "r-ok".into(),
        spec: spec("r-ok"),
        now: 0,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(10),
        output_cap_bytes: 4096,
        task_override: Some("Reply with the word done.".into()),
        cancel: CancelToken::new(),
    };
    let report = execute_run(store.clone(), opts).unwrap();
    assert_eq!(
        report.state,
        NodeState::Succeeded,
        "nodes: {:?}",
        report.nodes
    );
    assert!(report.nodes["verify"]
        .proven_gates
        .contains(&"g1".to_string()));
    let v: Value = status_json(&store, "r-ok").unwrap();
    assert_eq!(v["state"], json!("succeeded"));
}
