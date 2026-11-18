//! P2-5 / #82 runtime acceptance probe (REAL model, not model-free).
//!
//! Exercises the actual executor crate against the live engine:
//!   1. AgentExecutor runs ONE real model turn that MODIFIES a fixture file
//!      (hello.txt -> exactly HELLO-P25) under an engine-enforced workspaceWrite
//!      sandbox. Turn completion is execution end; success needs evidence.
//!   2. VerificationExecutor runs a Command gate as a REAL process via the
//!      engine `command/exec` RPC; the gate's ACTUAL exit code (not a model
//!      self-report) decides pass/fail.
//!
//! Run: SUPERAGENT_ENGINE=<engine> cargo run --example p2_5_runtime
//! Writes superagent/evidence/p2-5/p2-5-runtime-conclusion.json. Honest: allPass
//! only if the model really modified the fixture AND the gate captured exit 0.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::json;
use superagent_contracts::{Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION};
use superagent_executor::{
    AgentExecutor, EngineSpec, EvidenceRecorder, PermissionsConfig, VerificationExecutor,
};
use superagent_orchestrator::{ExecKind, NodeExecutor};
use superagent_storage::{RunSpec, Store};

fn main() {
    let engine = std::env::var("SUPERAGENT_ENGINE")
        .expect("SUPERAGENT_ENGINE must point at the engine binary");
    let home = std::env::var("SUPERAGENT_CODEX_HOME").unwrap_or_else(|_| {
        let h = std::env::var("HOME").unwrap();
        format!("{h}/Library/Application Support/SuperAgent/codex-home")
    });
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let ev = root.join("evidence/p2-5");
    std::fs::create_dir_all(&ev).unwrap();

    let work = tempfile::tempdir().unwrap();
    let fixture = work.path().join("hello.txt");
    std::fs::write(&fixture, "INITIAL\n").unwrap();

    let store = Rc::new(Store::open_in_memory().unwrap());
    let run_id = "p2-5-runtime";
    store
        .create_run(
            &RunSpec {
                id: run_id.into(),
                workflow_digest: "wd".into(),
                bundle_digest: "bd".into(),
                input_summary: "runtime probe".into(),
                repo_identity: "codex".into(),
                start_head: "head".into(),
                config_summary: "cfg".into(),
            },
            0,
        )
        .unwrap();
    let rec = EvidenceRecorder::new(store.clone(), run_id, ev.join("evidence"));

    let spec = EngineSpec {
        argv: vec![
            engine.clone(),
            "app-server".into(),
            "--listen".into(),
            "stdio://".into(),
        ],
        codex_home: Some(PathBuf::from(&home)),
        cwd: Some(work.path().to_path_buf()),
        run_deadline: Duration::from_secs(240),
        max_artifact_bytes: 20 * 1024 * 1024,
    };
    let perms = PermissionsConfig::workspace_write(vec![work.path().to_path_buf()], false);

    // 1. Real model turn modifies the fixture.
    let cap = Node {
        id: "modify".into(),
        depends_on: vec![],
        kind: NodeKind::Capability {
            command_ref: "team.modify".into(),
            inputs: vec![],
            outputs: vec![],
        },
        model: None,
        budget_tokens: None,
    };
    let mut agent = AgentExecutor {
        engine: spec.clone(),
        permissions: perms.clone(),
        workspace: work.path().to_path_buf(),
        evidence: rec.clone(),
        task_override: Some(
            "Edit the file hello.txt in the current workspace so its entire content is \
             exactly HELLO-P25 followed by a single newline. Use your tools to write the \
             file. Then reply with the final content."
                .into(),
        ),
    };
    let agent_out = agent.execute(&cap, 1);
    let fixture_after = std::fs::read_to_string(&fixture).unwrap_or_default();

    // 2. Real-exit-code gate verifies the fixture via engine command/exec.
    let check = "import sys; sys.exit(0 if open('hello.txt').read().strip()=='HELLO-P25' else 1)";
    let mut gates = BTreeMap::new();
    gates.insert(
        "g1".to_string(),
        Gate {
            id: "g1".into(),
            kind: GateKind::Command {
                argv: vec!["python3".into(), "-c".into(), check.into()],
                expect_exit_code: 0,
            },
        },
    );
    let vnode = Node {
        id: "verify".into(),
        depends_on: vec![],
        kind: NodeKind::Verification {
            argv: vec!["python3".into(), "-c".into(), "pass".into()],
            gates: vec!["g1".into()],
        },
        model: None,
        budget_tokens: None,
    };
    let wf = Workflow {
        schema_version: SCHEMA_VERSION,
        id: "p2-5".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes: vec![vnode.clone()],
        gates,
        artifacts: BTreeMap::new(),
    };
    let mut ver = VerificationExecutor {
        wf,
        engine: spec,
        permissions: perms,
        workspace: work.path().to_path_buf(),
        evidence: rec,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(60),
        output_cap_bytes: 4096,
    };
    let ver_out = ver.execute(&vnode, 1);
    let g1 = ver_out.gates.iter().find(|g| g.gate_id == "g1");
    let gate_exit = g1.and_then(|g| g.exit_code);
    let gate_proven = g1.map(|g| g.is_proven()).unwrap_or(false);

    let fixture_ok = fixture_after.trim() == "HELLO-P25";
    let agent_completed = agent_out.kind == ExecKind::Completed;
    let all_pass = agent_completed && fixture_ok && gate_proven && gate_exit == Some(0);

    let conclusion = json!({
        "issue": 82,
        "criterion": "real model node modifies fixture; gate captures real exit code",
        "engine": engine,
        "codexHome": home,
        "workspace": work.path(),
        "agent": {
            "kind": format!("{:?}", agent_out.kind),
            "detail": agent_out.detail,
            "tokensUsed": agent_out.tokens_used,
            "artifacts": agent_out.artifacts,
            "turnCompleted": agent_completed,
        },
        "fixtureBefore": "INITIAL",
        "fixtureAfter": fixture_after,
        "fixtureModifiedByModel": fixture_ok,
        "gate": {
            "gateId": "g1",
            "path": "command/exec (engine-enforced sandbox)",
            "realExitCode": gate_exit,
            "proven": gate_proven,
            "evidenceRef": g1.and_then(|g| g.evidence_ref.clone()),
            "usesModelSelfReportedExitCode": false,
        },
        "allPass": all_pass,
        "honestyNote": "allPass only if a real model turn modified hello.txt to HELLO-P25 AND \
                        the verification gate captured the REAL process exit code 0 via \
                        command/exec (never a model self-report). Otherwise false with facts.",
    });
    let out_path = ev.join("p2-5-runtime-conclusion.json");
    std::fs::write(&out_path, serde_json::to_vec_pretty(&conclusion).unwrap()).unwrap();
    println!(
        "{}",
        serde_json::to_string(&json!({
            "allPass": all_pass, "agentCompleted": agent_completed,
            "fixtureModifiedByModel": fixture_ok, "gateRealExitCode": gate_exit,
            "gateProven": gate_proven, "conclusion": out_path,
        }))
        .unwrap()
    );
    std::process::exit(if all_pass { 0 } else { 1 });
}
