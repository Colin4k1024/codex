//! P2-8 / #85 MVP exit: the product team-delivery workflow runs end-to-end
//! through the deterministic orchestrator + real executor over genuine
//! transport (fake_engine subprocess, real stdio JSON-RPC, real `command/exec`
//! child processes with real exit codes). Model-free: no model, no network.
//!
//! Acceptance bar (NOT "it completed"): every node terminal state has evidence;
//! a real fixture defect is genuinely fixed and its unit test genuinely passes;
//! a failing test earns at most two repairs then Failed; a node lacking an
//! acceptance command goes to WaitingInput; an authorization need is Blocked;
//! the run records the initial HEAD and never auto git add/commit/reset; the
//! end-to-end trace + report are archived.
//!
//! Model-driven autonomous role delegation is proven separately by #77 (real
//! TUI team delivery) and #82 (real exit-code gates). This suite proves the
//! product orchestrator's team-delivery DAG machinery with real gate evidence.

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};
use superagent_cli::plan::ctx_from_workflow;
use superagent_cli::run::{execute_run, RunOptions};
use superagent_cli::status::status_json;
use superagent_cli::{exit, CancelToken, CliError};
use superagent_contracts::{Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION};
use superagent_executor::EngineSpec;
use superagent_orchestrator::{NodeState, Policy};
use superagent_storage::{RunSpec, Store};

fn fake_engine() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../adapter/tests/fake_engine.py")
        .canonicalize()
        .expect("fake_engine.py present")
}

fn engine_spec(dir: &Path) -> EngineSpec {
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
        run_deadline: Duration::from_secs(120),
        max_artifact_bytes: 1 << 20,
    }
}

fn spec(id: &str, start_head: &str) -> RunSpec {
    RunSpec {
        id: id.into(),
        workflow_digest: "wd".into(),
        bundle_digest: "bd".into(),
        input_summary: "is".into(),
        repo_identity: "ri".into(),
        start_head: start_head.into(),
        config_summary: "cs".into(),
    }
}

fn exec_node(id: &str, deps: Vec<String>, argv: Vec<String>, gates: Vec<String>) -> Node {
    Node {
        id: id.into(),
        depends_on: deps,
        kind: NodeKind::Verification { argv, gates },
        model: None,
        budget_tokens: None,
    }
}

fn cmd_gate(id: &str, argv: Vec<String>) -> Gate {
    Gate {
        id: id.into(),
        kind: GateKind::Command {
            argv,
            expect_exit_code: 0,
        },
    }
}

fn py(code: String) -> Vec<String> {
    vec!["python3".into(), "-c".into(), code]
}

fn workflow(nodes: Vec<Node>, gates: Vec<Gate>) -> Workflow {
    Workflow {
        schema_version: SCHEMA_VERSION,
        id: "team-delivery-mvp".into(),
        version: "1".into(),
        variables: BTreeMap::new(),
        nodes,
        gates: gates.into_iter().map(|g| (g.id.clone(), g)).collect(),
        artifacts: BTreeMap::new(),
    }
}

/// Seed the genuine p0-2 fixture: a buggy calc.py (subtraction) + unit test.
fn seed_fixture(dir: &Path) {
    std::fs::write(dir.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
    std::fs::write(
        dir.join("test_calc.py"),
        "import unittest\nfrom calc import add\n\nclass TestAdd(unittest.TestCase):\n    \
         def test_positive(self):\n        self.assertEqual(add(2, 3), 5)\n    \
         def test_negative(self):\n        self.assertEqual(add(-2, -3), -5)\n",
    )
    .unwrap();
}

fn patch_argv(dir: &Path) -> Vec<String> {
    py(format!(
        "import pathlib,shutil; p=pathlib.Path(r'{d}'); \
         (p/'calc.py').write_text('def add(a, b):\\n    return a + b\\n'); \
         shutil.rmtree(p/'__pycache__', ignore_errors=True)",
        d = dir.display()
    ))
}

fn patch_applied_gate(dir: &Path) -> Gate {
    cmd_gate(
        "patch-applied",
        py(format!(
            "import pathlib,sys; sys.exit(0 if 'a + b' in pathlib.Path(r'{d}/calc.py').read_text() else 1)",
            d = dir.display()
        )),
    )
}

fn unittest_argv(dir: &Path) -> Vec<String> {
    vec![
        "python3".into(),
        "-B".into(),
        "-c".into(),
        format!(
            "import os,sys,unittest; sys.dont_write_bytecode=True; os.chdir(r'{d}'); \
             sys.path.insert(0,r'{d}'); import test_calc; \
             r=unittest.TextTestRunner(verbosity=0).run(unittest.defaultTestLoader.loadTestsFromModule(test_calc)); \
             sys.exit(0 if r.wasSuccessful() else 1)",
            d = dir.display()
        ),
    ]
}

fn opts(dir: &Path, wf: Workflow, run_id: &str, start_head: &str, policy: Policy) -> RunOptions {
    let ctx = ctx_from_workflow(&wf);
    RunOptions {
        workflow: wf,
        ctx,
        policy,
        engine: engine_spec(dir),
        workspace: dir.to_path_buf(),
        evidence_dir: dir.join("ev"),
        run_id: run_id.into(),
        spec: spec(run_id, start_head),
        now: 0,
        schema_registry: BTreeMap::new(),
        gate_timeout: Duration::from_secs(60),
        output_cap_bytes: 1 << 20,
        task_override: Some("Fix the calc.py defect so the unit test passes.".into()),
        cancel: CancelToken::new(),
    }
}

fn run_unittest(dir: &Path) -> bool {
    std::process::Command::new("python3")
        .args(["-B", "-m", "unittest", "test_calc", "-v"])
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Wrap a python `-B -c` program with a `p = Path(dir)` prelude. `code` is a
/// plain literal (its braces are python, not format placeholders).
fn node_argv(dir: &Path, code: &str) -> Vec<String> {
    vec![
        "python3".into(),
        "-B".into(),
        "-c".into(),
        format!(
            "import json,sys,pathlib,shutil; p=pathlib.Path(r'{d}'); {code}",
            d = dir.display(),
            code = code
        ),
    ]
}

/// A command gate that exits 0 iff the python boolean `expr` holds (p = dir).
fn check_gate(id: &str, dir: &Path, expr: &str) -> Gate {
    cmd_gate(
        id,
        vec![
            "python3".into(),
            "-B".into(),
            "-c".into(),
            format!(
                "import json,sys,pathlib; p=pathlib.Path(r'{d}'); sys.exit(0 if ({e}) else 1)",
                d = dir.display(),
                e = expr
            ),
        ],
    )
}

// 1. Full team-delivery DAG producing the named artifact chain:
//    intake(scope.json) -> plan(plan.json, schema-valid) -> implement(calc fix +
//    change-summary.json) -> verify(real unittest exit code) -> review(review.json,
//    no unresolved blocking item) -> report(report.md + report.json with
//    platforms/limits). A genuine defect is fixed and its test genuinely passes;
//    every node reaches a terminal state with gate evidence; trace+report archived.
#[test]
fn mvp_team_delivery_real_defect_fix_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    seed_fixture(dir.path());
    assert!(!run_unittest(dir.path()), "baseline fixture must FAIL");

    let store = Rc::new(Store::open_in_memory().unwrap());
    let wf = workflow(
        vec![
            exec_node(
                "intake",
                vec![],
                node_argv(
                    dir.path(),
                    "json.dump({'goal':'fix calc defect','scope':['calc.py'],\
                     'acceptance':'unittest passes','budget':{'tokens':1000}}, \
                     open(p/'scope.json','w'))",
                ),
                vec!["scope-valid".into()],
            ),
            exec_node(
                "plan",
                vec!["intake".into()],
                node_argv(
                    dir.path(),
                    "json.dump({'steps':[{'id':'implement','acceptanceCommand':'python3 -m unittest test_calc'},\
                     {'id':'verify','acceptanceCommand':'python3 -m unittest test_calc'}]}, \
                     open(p/'plan.json','w'))",
                ),
                vec!["plan-schema".into()],
            ),
            exec_node(
                "implement",
                vec!["plan".into()],
                node_argv(
                    dir.path(),
                    "(p/'calc.py').write_text('def add(a, b):\\n    return a + b\\n'); \
                     shutil.rmtree(p/'__pycache__', ignore_errors=True); \
                     json.dump({'files':['calc.py'],'before':'return a - b','after':'return a + b'}, \
                     open(p/'change-summary.json','w'))",
                ),
                vec!["patch-applied".into(), "change-summary-valid".into()],
            ),
            exec_node(
                "verify",
                vec!["implement".into()],
                unittest_argv(dir.path()),
                vec!["tests-pass".into()],
            ),
            exec_node(
                "review",
                vec!["verify".into()],
                node_argv(
                    dir.path(),
                    "json.dump({'findings':[],'blocking':0,'verdict':'approve'}, open(p/'review.json','w'))",
                ),
                vec!["review-clear".into()],
            ),
            exec_node(
                "report",
                vec!["review".into()],
                node_argv(
                    dir.path(),
                    "(p/'report.md').write_text('# MVP report\\nVerified: calc fix + unittest pass.\\n\
                     Platforms: macOS arm64. Limits: model-free fixture; serial single workspace.\\n'); \
                     json.dump({'verified':['calc-fix','unittest-pass'],'unverified':[],\
                     'platforms':['macos-arm64'],'limits':['model-free fixture','serial single workspace']}, \
                     open(p/'report.json','w'))",
                ),
                vec!["report-archive".into()],
            ),
        ],
        vec![
            check_gate(
                "scope-valid",
                dir.path(),
                "all(k in json.load(open(p/'scope.json')) for k in ('goal','scope','acceptance','budget'))",
            ),
            check_gate(
                "plan-schema",
                dir.path(),
                "isinstance((s:=json.load(open(p/'plan.json')).get('steps',[])),list) and len(s)>0 \
                 and all('id' in x and 'acceptanceCommand' in x for x in s)",
            ),
            patch_applied_gate(dir.path()),
            check_gate(
                "change-summary-valid",
                dir.path(),
                "(d:=json.load(open(p/'change-summary.json'))).get('files')==['calc.py'] \
                 and 'before' in d and 'after' in d",
            ),
            cmd_gate("tests-pass", unittest_argv(dir.path())),
            check_gate(
                "review-clear",
                dir.path(),
                "(d:=json.load(open(p/'review.json'))).get('blocking',1)==0 \
                 and all(x.get('resolved',True) for x in d.get('findings',[]) if x.get('severity')=='blocking')",
            ),
            check_gate(
                "report-archive",
                dir.path(),
                "(p/'report.md').exists() and all(k in json.load(open(p/'report.json')) \
                 for k in ('verified','unverified','platforms','limits'))",
            ),
        ],
    );
    let report = execute_run(
        store.clone(),
        opts(dir.path(), wf, "r-mvp", "initial-head", Policy::default()),
    )
    .unwrap();
    assert_eq!(
        report.state,
        NodeState::Succeeded,
        "nodes: {:?}",
        report.nodes
    );
    for id in ["intake", "plan", "implement", "verify", "review", "report"] {
        assert_eq!(report.nodes[id].state, NodeState::Succeeded, "node {id}");
    }
    assert!(report.nodes["verify"]
        .proven_gates
        .contains(&"tests-pass".to_string()));
    assert!(report.nodes["implement"]
        .proven_gates
        .contains(&"patch-applied".to_string()));
    assert!(report.nodes["plan"]
        .proven_gates
        .contains(&"plan-schema".to_string()));
    assert!(report.nodes["review"]
        .proven_gates
        .contains(&"review-clear".to_string()));
    assert!(report.nodes["report"]
        .proven_gates
        .contains(&"report-archive".to_string()));

    // The named artifact chain genuinely exists on disk with the required shape.
    for art in [
        "scope.json",
        "plan.json",
        "change-summary.json",
        "review.json",
        "report.json",
        "report.md",
    ] {
        assert!(dir.path().join(art).exists(), "artifact {art} archived");
    }
    let rj: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("report.json")).unwrap())
            .unwrap();
    assert!(!rj["platforms"].as_array().unwrap().is_empty());
    assert!(!rj["limits"].as_array().unwrap().is_empty());
    assert!(!rj["verified"].as_array().unwrap().is_empty());

    // The fixture was GENUINELY fixed and the test GENUINELY passes (independent).
    let calc = std::fs::read_to_string(dir.path().join("calc.py")).unwrap();
    assert!(
        calc.contains("a + b") && !calc.contains("a - b"),
        "calc: {calc}"
    );
    assert!(
        run_unittest(dir.path()),
        "unit test must pass after the run"
    );

    // Trace + report archived: durable run_report event + gate evidence on disk.
    let v: Value = status_json(&store, "r-mvp").unwrap();
    assert_eq!(v["state"], json!("succeeded"));
    let events = store.events_after("r-mvp", 0).unwrap();
    assert!(
        events.iter().any(|e| e.kind == "run_report"),
        "run_report event archived"
    );
    assert!(dir.path().join("ev").exists(), "evidence dir archived");
    let ev_files: Vec<_> = std::fs::read_dir(dir.path().join("ev"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!ev_files.is_empty(), "gate evidence files archived");
}

// 2. A failing test earns at most two repairs (max_attempts=3) then Failed,
//    with the gate's real exit code recorded — never a silent success.
#[test]
fn mvp_failing_test_bounded_retry_then_failed() {
    let dir = tempfile::tempdir().unwrap();
    seed_fixture(dir.path()); // buggy; implement does NOT fix it
    let store = Rc::new(Store::open_in_memory().unwrap());
    let wf = workflow(
        vec![exec_node(
            "verify",
            vec![],
            py("pass".into()),
            vec!["tests-pass".into()],
        )],
        vec![cmd_gate("tests-pass", unittest_argv(dir.path()))],
    );
    let report = execute_run(
        store.clone(),
        opts(dir.path(), wf, "r-fail", "h", Policy::default()),
    )
    .unwrap();
    assert_eq!(report.state, NodeState::Failed, "nodes: {:?}", report.nodes);
    let nr = &report.nodes["verify"];
    assert_eq!(nr.state, NodeState::Failed);
    assert!(
        nr.attempts >= 2 && nr.attempts <= 3,
        "bounded repairs, attempts={}",
        nr.attempts
    );
    // The fixture is still buggy; the test still fails (no fabricated fix).
    assert!(!run_unittest(dir.path()));
    let v: Value = status_json(&store, "r-fail").unwrap();
    assert_eq!(v["state"], json!("failed"));
}

// 3. A node lacking an acceptance command (approval-only) goes to WaitingInput;
//    non-interactive run refuses with a stable blocked exit, never succeeds.
#[test]
fn mvp_missing_acceptance_command_waiting_input() {
    let dir = tempfile::tempdir().unwrap();
    let store = Rc::new(Store::open_in_memory().unwrap());
    let gate = Gate {
        id: "needs-acceptance".into(),
        kind: GateKind::Approval {
            prompt: "provide the acceptance command for this node".into(),
        },
    };
    let wf = workflow(
        vec![exec_node(
            "verify",
            vec![],
            py("pass".into()),
            vec!["needs-acceptance".into()],
        )],
        vec![gate],
    );
    let err = execute_run(
        store.clone(),
        opts(dir.path(), wf, "r-wait", "h", Policy::default()),
    )
    .unwrap_err();
    assert!(matches!(err, CliError::Blocked(_)), "got: {err:?}");
    assert_eq!(err.exit_code(), exit::BLOCKED);
    let v: Value = status_json(&store, "r-wait").unwrap();
    let nodes = v["nodes"].as_array().unwrap();
    let verify = nodes.iter().find(|n| n["id"] == json!("verify")).unwrap();
    assert_eq!(verify["state"], json!("WaitingInput"), "nodes: {nodes:?}");
}

// 4. The run records the initial HEAD and never auto git add/commit/reset:
//    after a genuine fix the working tree change is left UNSTAGED and HEAD is
//    unchanged (serial single workspace, no automatic git mutation).
#[test]
fn mvp_records_initial_head_and_no_auto_git_ops() {
    let dir = tempfile::tempdir().unwrap();
    seed_fixture(dir.path());
    let git = |args: &[&str]| -> String {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["add", "calc.py", "test_calc.py"]);
    git(&["commit", "-qm", "initial fixture"]);
    let head_before = git(&["rev-parse", "HEAD"]);

    let store = Rc::new(Store::open_in_memory().unwrap());
    let wf = workflow(
        vec![
            exec_node(
                "implement",
                vec![],
                patch_argv(dir.path()),
                vec!["patch-applied".into()],
            ),
            exec_node(
                "verify",
                vec!["implement".into()],
                unittest_argv(dir.path()),
                vec!["tests-pass".into()],
            ),
        ],
        vec![
            patch_applied_gate(dir.path()),
            cmd_gate("tests-pass", unittest_argv(dir.path())),
        ],
    );
    let report = execute_run(
        store.clone(),
        opts(dir.path(), wf, "r-git", &head_before, Policy::default()),
    )
    .unwrap();
    assert_eq!(report.state, NodeState::Succeeded);

    // HEAD unchanged, no new commit, and the fix is left unstaged (no auto add/commit).
    let head_after = git(&["rev-parse", "HEAD"]);
    assert_eq!(head_before, head_after, "run must not move HEAD");
    let status = git(&["status", "--porcelain"]);
    assert!(
        status.contains("calc.py"),
        "fix left in working tree: {status}"
    );
    assert!(
        !status.contains("M  calc.py"),
        "calc.py must NOT be auto-staged: {status}"
    );
    // Initial HEAD is recorded in the durable run identity.
    let v: Value = status_json(&store, "r-git").unwrap();
    assert_eq!(v["identity"]["startHead"], json!(head_before));
}
