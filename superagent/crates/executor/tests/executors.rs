//! Model-free integration tests for the #82 executors against the #79 fake
//! stdio engine. The fake engine runs `command/exec` argv as REAL child
//! processes (real exit codes) and completes turns deterministically — no
//! model, no network. A live-model runtime probe lives in
//! `superagent/tests/p2_5.py`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};
use superagent_contracts::{
    ArtifactSpec, DryRunContext, Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION,
};
use superagent_executor::{
    AgentExecutor, EngineSpec, EnvironmentConfig, EvidenceRecorder, PermissionsConfig,
    SandboxRequest, SwarmExecutor, VerificationExecutor,
};
use superagent_orchestrator::{ExecKind, NodeExecutor, NodeState, Orchestrator, Policy};
use superagent_storage::{RunSpec, Store};

fn fake_engine(scenario: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../adapter/tests/fake_engine.py")
        .canonicalize()
        .unwrap();
    vec![
        "python3".into(),
        path.to_string_lossy().into_owned(),
        "--scenario".into(),
        scenario.into(),
    ]
}

fn engine_spec(scenario: &str) -> EngineSpec {
    EngineSpec {
        argv: fake_engine(scenario),
        codex_home: None,
        cwd: None,
        environment: None,
        run_deadline: Duration::from_secs(30),
        max_artifact_bytes: 20 * 1024 * 1024,
    }
}

/// Like [`engine_spec`] but with a state file the fake engine writes its
/// observed `environment/add`, `environment/info` and `thread/start` calls to.
fn engine_spec_state(
    scenario: &str,
    state_file: &std::path::Path,
    environment: Option<EnvironmentConfig>,
) -> EngineSpec {
    let mut spec = engine_spec(scenario);
    spec.argv.push("--state-file".into());
    spec.argv.push(state_file.to_string_lossy().into_owned());
    spec.environment = environment;
    spec
}

fn run_spec(id: &str) -> RunSpec {
    RunSpec {
        id: id.into(),
        workflow_digest: "wd".into(),
        bundle_digest: "bd".into(),
        input_summary: "in".into(),
        repo_identity: "repo".into(),
        start_head: "head".into(),
        config_summary: "cfg".into(),
    }
}

fn recorder(tmp: &tempfile::TempDir, run_id: &str) -> (Rc<Store>, EvidenceRecorder) {
    let store = Rc::new(Store::open_in_memory().unwrap());
    store.create_run(&run_spec(run_id), 0).unwrap();
    let rec = EvidenceRecorder::new(store.clone(), run_id, tmp.path().join("evidence"));
    (store, rec)
}

/// Like [`recorder`] but without pre-creating the run row: the orchestrator's
/// `run` creates it (avoids a duplicate runs.id).
fn recorder_raw(tmp: &tempfile::TempDir, run_id: &str) -> (Rc<Store>, EvidenceRecorder) {
    let store = Rc::new(Store::open_in_memory().unwrap());
    let rec = EvidenceRecorder::new(store.clone(), run_id, tmp.path().join("evidence"));
    (store, rec)
}

fn capability_node(id: &str) -> Node {
    Node {
        id: id.into(),
        depends_on: vec![],
        kind: NodeKind::Capability {
            command_ref: "team.do".into(),
            inputs: vec![],
            outputs: vec![],
        },
        model: None,
        budget_tokens: None,
    }
}

fn verification_node(id: &str, gates: Vec<String>) -> Node {
    Node {
        id: id.into(),
        depends_on: vec![],
        kind: NodeKind::Verification {
            argv: vec!["python3".into(), "-c".into(), "pass".into()],
            gates,
        },
        model: None,
        budget_tokens: None,
    }
}

fn command_gate(id: &str, script: &str, expect: i32) -> Gate {
    Gate {
        id: id.into(),
        kind: GateKind::Command {
            argv: vec!["python3".into(), "-c".into(), script.into()],
            expect_exit_code: expect,
        },
    }
}

fn workspace_write(tmp: &tempfile::TempDir) -> PermissionsConfig {
    PermissionsConfig::workspace_write(vec![tmp.path().to_path_buf()], false)
}

// --- AgentExecutor ---------------------------------------------------------

#[test]
fn agent_turn_completion_yields_durable_artifact_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-agent");
    let mut ex = AgentExecutor {
        engine: engine_spec("echo"),
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: Some("produce the fixture".into()),
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::Completed, "detail={}", out.detail);
    assert!(
        !out.artifacts.is_empty(),
        "completed turn must carry evidence"
    );
    assert!(out.gates.iter().any(|g| g.is_proven()));
    assert!(out.detail.contains("thread=thr-"));
    assert!(out.detail.contains("turn=turn-1"));
    let art = PathBuf::from(&out.artifacts[0]);
    assert!(art.exists());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&art).unwrap()).unwrap();
    assert_eq!(v["status"], "completed");
    assert_eq!(v["threadId"], "thr-1");
    assert_eq!(v["turnId"], "turn-1");
}

#[test]
fn agent_crash_mid_turn_is_uncertain_not_success() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-crash");
    let mut ex = AgentExecutor {
        engine: engine_spec("crash"),
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: None,
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::Uncertain);
    assert!(out.artifacts.is_empty());
}

#[test]
fn agent_inexpressible_sandbox_is_blocked_no_engine_spawn() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-blocked");
    let mut ex = AgentExecutor {
        engine: engine_spec("echo"),
        permissions: PermissionsConfig {
            sandbox: SandboxRequest::Inexpressible {
                reason: "externalSandbox requested".into(),
            },
        },
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: None,
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::Blocked);
    assert!(out.detail.contains("inexpressible"));
    // No engine run log was created (the node was refused before spawning).
    assert!(!tmp.path().join("evidence/n1-a1-engine.jsonl").exists());
}

#[test]
fn agent_local_process_intent_is_blocked_for_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-lp");
    let mut ex = AgentExecutor {
        engine: engine_spec("echo"),
        permissions: PermissionsConfig::local_process(),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: None,
    };
    assert_eq!(
        ex.execute(&capability_node("n1"), 1).kind,
        ExecKind::Blocked
    );
}

// --- P5-1 (#95): environment wiring ---------------------------------------

#[test]
fn agent_with_environment_registers_before_thread_and_records_capabilities() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-env");
    let state_file = tmp.path().join("state.json");
    let remote_cwd = PathBuf::from("/remote/work");
    let remote_root = PathBuf::from("/remote/extra");
    let spec = engine_spec_state(
        "echo",
        &state_file,
        Some(EnvironmentConfig {
            id: "env-p51".into(),
            exec_server_url: "ws://127.0.0.1:1".into(),
            auth_bearer_token: Some("executor-secret-token".into()),
            cwd: Some(remote_cwd.clone()),
            workspace_roots: vec![remote_root.clone()],
        }),
    );
    let mut ex = AgentExecutor {
        engine: spec,
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: Some("run against the remote environment".into()),
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::Completed, "detail={}", out.detail);

    // Capability discovery lands in the durable turn-result evidence.
    let art = PathBuf::from(&out.artifacts[0]);
    let v: Value = serde_json::from_slice(&std::fs::read(&art).unwrap()).unwrap();
    assert_eq!(v["environment"]["id"], "env-p51");
    assert_eq!(v["environment"]["info"]["shell"]["name"], "zsh");
    assert_eq!(v["environment"]["info"]["shell"]["path"], "/bin/zsh");

    // The engine observed registration BEFORE thread/start and the
    // environments entry on thread/start.
    let st: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(st["environmentAdds"].as_array().unwrap().len(), 1);
    assert_eq!(st["environmentAdds"][0]["environmentId"], "env-p51");
    assert_eq!(st["environmentAdds"][0]["hasBearerToken"], true);
    assert_eq!(st["environmentInfos"].as_array().unwrap().len(), 1);
    assert_eq!(st["initializeCapabilities"]["experimentalApi"], true);
    // Strict wire order: initialize -> register -> discover -> thread -> turn.
    let calls: Vec<&str> = st["callLog"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        &calls[..4],
        &[
            "initialize",
            "environment/add",
            "environment/info",
            "thread/start"
        ]
    );
    let add = calls.iter().position(|c| *c == "environment/add").unwrap();
    let info = calls.iter().position(|c| *c == "environment/info").unwrap();
    let thread = calls.iter().position(|c| *c == "thread/start").unwrap();
    let turn = calls.iter().position(|c| *c == "turn/start").unwrap();
    assert!(add < info && info < thread && thread < turn, "{calls:?}");
    let thread_env = &st["threadStarts"][0]["environments"][0];
    assert_eq!(thread_env["environmentId"], "env-p51");
    assert_eq!(thread_env["cwd"], remote_cwd.to_string_lossy().as_ref());
    assert_eq!(
        thread_env["runtimeWorkspaceRoots"],
        json!([remote_root.to_string_lossy()])
    );

    // The bearer token never reaches the per-attempt transcript log.
    let log = std::fs::read_to_string(tmp.path().join("evidence/n1-a1-engine.jsonl")).unwrap();
    assert!(
        !log.contains("executor-secret-token"),
        "executor engine log leaked the bearer token"
    );
    assert!(log.contains("<redacted>"));
}

#[test]
fn agent_environment_registration_failure_is_fatal_no_local_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-env-fail");
    let state_file = tmp.path().join("state.json");
    let spec = engine_spec_state(
        "environment_fail",
        &state_file,
        Some(EnvironmentConfig {
            id: "env-down".into(),
            exec_server_url: "ws://127.0.0.1:1".into(),
            auth_bearer_token: None,
            cwd: None,
            workspace_roots: vec![],
        }),
    );
    let mut ex = AgentExecutor {
        engine: spec,
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        // A local fallback would complete on this fake engine; it must not run.
        task_override: Some("must not execute locally".into()),
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::FatalFailure, "detail={}", out.detail);
    assert!(
        out.detail.contains("environment/add"),
        "failure must name the registration call: {}",
        out.detail
    );
    // No thread/turn started and no turn-result evidence written.
    let st: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(st["callLog"], json!(["initialize", "environment/add"]));
    assert!(st["threads"].as_object().is_none_or(|m| m.is_empty()));
    assert!(st["threadStarts"].as_array().unwrap().is_empty());
    assert!(!tmp.path().join("evidence/n1-turn-result.json").exists());
}

#[test]
fn agent_environment_info_failure_is_fatal_before_thread_start() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-env-info-fail");
    let state_file = tmp.path().join("state.json");
    let spec = engine_spec_state(
        "environment_info_fail",
        &state_file,
        Some(EnvironmentConfig {
            id: "env-no-info".into(),
            exec_server_url: "ws://127.0.0.1:1".into(),
            auth_bearer_token: None,
            cwd: None,
            workspace_roots: vec![],
        }),
    );
    let mut ex = AgentExecutor {
        engine: spec,
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: Some("must not execute locally".into()),
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::FatalFailure, "detail={}", out.detail);
    assert!(
        out.detail.contains("environment/info"),
        "failure must name capability discovery: {}",
        out.detail
    );
    let st: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(st["environmentAdds"].as_array().unwrap().len(), 1);
    assert_eq!(
        st["callLog"],
        json!(["initialize", "environment/add", "environment/info"])
    );
    assert!(st["threads"].as_object().is_none_or(|m| m.is_empty()));
    assert!(st["threadStarts"].as_array().unwrap().is_empty());
}

#[test]
fn agent_without_environment_sends_no_environment_rpcs() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-no-env");
    let state_file = tmp.path().join("state.json");
    let spec = engine_spec_state("echo", &state_file, None);
    let mut ex = AgentExecutor {
        engine: spec,
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        task_override: Some("local turn".into()),
    };
    let out = ex.execute(&capability_node("n1"), 1);
    assert_eq!(out.kind, ExecKind::Completed, "detail={}", out.detail);
    let st: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert!(st["environmentAdds"].as_array().unwrap().is_empty());
    assert!(st["environmentInfos"].as_array().unwrap().is_empty());
    assert_eq!(st["initializeCapabilities"], json!({}));
    // Existing thread/start params are unchanged (no environments key).
    assert!(st["threadStarts"][0].get("environments").is_none());
    // No environment block in the turn-result evidence.
    let v: Value = serde_json::from_slice(&std::fs::read(&out.artifacts[0]).unwrap()).unwrap();
    assert!(v.get("environment").is_none());
}

// --- VerificationExecutor: command gates (real exit codes) -----------------

#[test]
fn command_gate_via_engine_exec_captures_real_exit_code_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-gate-pass");
    let mut gates = BTreeMap::new();
    gates.insert(
        "g1".to_string(),
        command_gate("g1", "import sys; sys.exit(0)", 0),
    );
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["g1".into()])],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["g1".into()]), 1);
    assert_eq!(out.kind, ExecKind::Completed);
    let g1 = out.gates.iter().find(|g| g.gate_id == "g1").unwrap();
    assert!(g1.is_proven());
    assert_eq!(g1.exit_code, Some(0));
}

#[test]
fn command_gate_real_nonzero_exit_is_unproven() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-gate-fail");
    let mut gates = BTreeMap::new();
    gates.insert(
        "g1".to_string(),
        command_gate("g1", "import sys; sys.exit(3)", 0),
    );
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["g1".into()])],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["g1".into()]), 1);
    let g1 = out.gates.iter().find(|g| g.gate_id == "g1").unwrap();
    assert!(!g1.is_proven());
    assert_eq!(g1.exit_code, Some(3), "must capture the REAL exit code 3");
    assert!(
        g1.evidence_ref.is_some(),
        "failure still has durable evidence"
    );
}

#[test]
fn local_process_gate_runs_direct_child_real_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-direct");
    let mut gates = BTreeMap::new();
    gates.insert(
        "g1".to_string(),
        command_gate("g1", "import sys; sys.exit(0)", 0),
    );
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["g1".into()])],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: PermissionsConfig::local_process(),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["g1".into()]), 1);
    let g1 = out.gates.iter().find(|g| g.gate_id == "g1").unwrap();
    assert!(g1.is_proven());
    assert_eq!(g1.exit_code, Some(0));
    // Direct path writes a .runlog, not an engine jsonl.
    assert!(g1.evidence_ref.as_ref().unwrap().ends_with(".runlog"));
}

#[test]
fn kernel_sandbox_intent_blocks_direct_gate_but_engine_path_runs() {
    // workspaceWrite is expressible via the engine command/exec path, so the
    // gate runs there (engine enforces sandbox); it is NOT a direct child.
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-engine-path");
    let mut gates = BTreeMap::new();
    gates.insert("g1".to_string(), command_gate("g1", "pass", 0));
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["g1".into()])],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: workspace_write(&tmp),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["g1".into()]), 1);
    let g1 = out.gates.iter().find(|g| g.gate_id == "g1").unwrap();
    assert!(g1.evidence_ref.as_ref().unwrap().ends_with("-gate.json"));
}

#[test]
fn inexpressible_sandbox_blocks_gate_no_host_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-gate-blocked");
    let mut gates = BTreeMap::new();
    gates.insert("g1".to_string(), command_gate("g1", "pass", 0));
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["g1".into()])],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: PermissionsConfig {
            sandbox: SandboxRequest::Inexpressible {
                reason: "externalSandbox".into(),
            },
        },
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["g1".into()]), 1);
    assert_eq!(out.kind, ExecKind::Blocked);
}

// --- ArtifactSchema + Approval gates ---------------------------------------

#[test]
fn artifact_schema_gate_validates_real_json() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-schema");
    std::fs::write(tmp.path().join("out.json"), br#"{"name":"ok","count":3}"#).unwrap();
    let mut artifacts = BTreeMap::new();
    artifacts.insert(
        "out".to_string(),
        ArtifactSpec {
            path: "out.json".into(),
            media_type: "application/json".into(),
            schema_ref: Some("schema://out".into()),
        },
    );
    let mut gates = BTreeMap::new();
    gates.insert(
        "gs".to_string(),
        Gate {
            id: "gs".into(),
            kind: GateKind::ArtifactSchema {
                artifact: "out".into(),
                schema_ref: "schema://out".into(),
            },
        },
    );
    let mut schema_registry = BTreeMap::new();
    schema_registry.insert(
        "schema://out".to_string(),
        json!({
            "type":"object",
            "required":["name","count"],
            "properties":{"name":{"type":"string"},"count":{"type":"integer"}}
        }),
    );
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["gs".into()])],
        gates,
        artifacts,
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: PermissionsConfig::local_process(),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry,
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["gs".into()]), 1);
    let gs = out.gates.iter().find(|g| g.gate_id == "gs").unwrap();
    assert!(gs.is_proven(), "valid artifact must pass schema gate");

    // Now corrupt the artifact: the same gate must fail with durable evidence.
    std::fs::write(tmp.path().join("out.json"), br#"{"name":123}"#).unwrap();
    let out2 = ex.execute(&verification_node("v1", vec!["gs".into()]), 2);
    let gs2 = out2.gates.iter().find(|g| g.gate_id == "gs").unwrap();
    assert!(!gs2.is_proven());
    assert!(gs2.evidence_ref.is_some());
}

#[test]
fn approval_gate_returns_needs_input_never_auto_approves() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, rec) = recorder(&tmp, "run-approval");
    let mut gates = BTreeMap::new();
    gates.insert(
        "ga".to_string(),
        Gate {
            id: "ga".into(),
            kind: GateKind::Approval {
                prompt: "publish to production?".into(),
            },
        },
    );
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![verification_node("v1", vec!["ga".into()])],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ex = VerificationExecutor {
        wf,
        engine: engine_spec("echo"),
        permissions: PermissionsConfig::local_process(),
        workspace: tmp.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(20),
        output_cap_bytes: 4096,
    };
    let out = ex.execute(&verification_node("v1", vec!["ga".into()]), 1);
    assert_eq!(out.kind, ExecKind::NeedsInput);
    assert!(out.detail.contains("publish to production?"));
}

// --- Orchestrator integration (success only via evidence) ------------------

fn ctx() -> DryRunContext {
    DryRunContext {
        variables: BTreeMap::new(),
        command_catalog: vec!["team.do".into()],
        models: vec![],
        programs: vec!["python3".into()],
        schema_registry: vec![],
    }
}

#[test]
fn orchestrator_capability_succeeds_only_with_artifact_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, rec) = recorder_raw(&tmp, "run-orch-cap");
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![capability_node("c1")],
        gates: BTreeMap::new(),
        artifacts: BTreeMap::new(),
    };
    let orch = Orchestrator::new(wf, ctx(), Policy::default());
    let mut ex = SwarmExecutor {
        agent: AgentExecutor {
            engine: engine_spec("echo"),
            permissions: workspace_write(&tmp),
            workspace: tmp.path().to_path_buf(),
            evidence: rec.clone(),
            task_override: Some("do it".into()),
        },
        verification: VerificationExecutor {
            wf: Workflow {
                schema_version: SCHEMA_VERSION,
                id: "wf".into(),
                version: "1".into(),
                variables: BTreeMap::new(),
                nodes: vec![],
                gates: BTreeMap::new(),
                artifacts: BTreeMap::new(),
            },
            engine: engine_spec("echo"),
            permissions: workspace_write(&tmp),
            workspace: tmp.path().to_path_buf(),
            evidence: rec,
            schema_registry: BTreeMap::new(),
            gate_timeout: Duration::from_secs(20),
            output_cap_bytes: 4096,
        },
    };
    let report = orch
        .run(
            &mut ex,
            &store,
            "run-orch-cap",
            &run_spec("run-orch-cap"),
            0,
        )
        .unwrap();
    assert_eq!(report.nodes["c1"].state, NodeState::Succeeded);
}

#[test]
fn orchestrator_blocked_executor_yields_blocked_node() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, rec) = recorder_raw(&tmp, "run-orch-blocked");
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![capability_node("c1")],
        gates: BTreeMap::new(),
        artifacts: BTreeMap::new(),
    };
    let orch = Orchestrator::new(wf, ctx(), Policy::default());
    let mut ex = SwarmExecutor {
        agent: AgentExecutor {
            engine: engine_spec("echo"),
            permissions: PermissionsConfig {
                sandbox: SandboxRequest::Inexpressible {
                    reason: "externalSandbox".into(),
                },
            },
            workspace: tmp.path().to_path_buf(),
            evidence: rec.clone(),
            task_override: None,
        },
        verification: VerificationExecutor {
            wf: Workflow {
                schema_version: SCHEMA_VERSION,
                id: "wf".into(),
                version: "1".into(),
                variables: BTreeMap::new(),
                nodes: vec![],
                gates: BTreeMap::new(),
                artifacts: BTreeMap::new(),
            },
            engine: engine_spec("echo"),
            permissions: PermissionsConfig::local_process(),
            workspace: tmp.path().to_path_buf(),
            evidence: rec,
            schema_registry: BTreeMap::new(),
            gate_timeout: Duration::from_secs(20),
            output_cap_bytes: 4096,
        },
    };
    let report = orch
        .run(
            &mut ex,
            &store,
            "run-orch-blocked",
            &run_spec("run-orch-blocked"),
            0,
        )
        .unwrap();
    assert_eq!(report.nodes["c1"].state, NodeState::Blocked);
    assert_eq!(report.nodes["c1"].attempts, 1);
}
