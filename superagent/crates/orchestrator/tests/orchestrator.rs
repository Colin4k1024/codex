//! P2-4 / #81 acceptance tests: DAG cycle / failure propagation, failed gate
//! blocks succeeded, budget exhaustion blocked/failed, finite retry,
//! non-idempotent no-blind-retry, evidence-only success, invalidation.
//! All model-free via an injected mock executor.

use std::collections::BTreeMap;

use superagent_contracts::{
    DryRunContext, Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION,
};
use superagent_orchestrator::{
    downstream, topo_order, ExecKind, ExecOutcome, GateOutcome, NodeExecutor, NodeState,
    Orchestrator, Policy,
};
use superagent_storage::{RunSpec, Store};

struct FnExec<F: FnMut(&Node, u32) -> ExecOutcome>(F);
impl<F: FnMut(&Node, u32) -> ExecOutcome> NodeExecutor for FnExec<F> {
    fn execute(&mut self, n: &Node, a: u32) -> ExecOutcome {
        (self.0)(n, a)
    }
}

fn ctx() -> DryRunContext {
    DryRunContext {
        variables: BTreeMap::new(),
        command_catalog: vec!["cmd".to_string()],
        models: vec!["m".to_string()],
        programs: vec!["true".to_string(), "echo".to_string()],
        schema_registry: vec!["s".to_string()],
    }
}

fn gate_g() -> Gate {
    Gate {
        id: "g".to_string(),
        kind: GateKind::Command {
            argv: vec!["true".to_string()],
            expect_exit_code: 0,
        },
    }
}

fn vnode(id: &str, deps: Vec<&str>) -> Node {
    Node {
        id: id.to_string(),
        depends_on: deps.into_iter().map(String::from).collect(),
        kind: NodeKind::Verification {
            argv: vec!["true".to_string()],
            gates: vec!["g".to_string()],
        },
        model: None,
        budget_tokens: None,
    }
}

fn wf(nodes: Vec<Node>) -> Workflow {
    let mut gates = BTreeMap::new();
    gates.insert("g".to_string(), gate_g());
    Workflow {
        schema_version: SCHEMA_VERSION,
        id: "wf".to_string(),
        version: "1".to_string(),
        variables: BTreeMap::new(),
        nodes,
        gates,
        artifacts: BTreeMap::new(),
    }
}

fn spec() -> RunSpec {
    RunSpec {
        id: "run".to_string(),
        workflow_digest: "wfd".to_string(),
        bundle_digest: "bd".to_string(),
        input_summary: "in".to_string(),
        repo_identity: "Colin4k1024/codex".to_string(),
        start_head: "abc".to_string(),
        config_summary: "cfg".to_string(),
    }
}

fn proven(g: &str) -> GateOutcome {
    GateOutcome {
        gate_id: g.to_string(),
        passed: true,
        evidence_ref: Some(format!("ev:{g}")),
        exit_code: Some(0),
    }
}

fn run(
    wf: Workflow,
    policy: Policy,
    exec: &mut impl NodeExecutor,
) -> superagent_orchestrator::RunReport {
    let store = Store::open_in_memory().unwrap();
    let orch = Orchestrator::new(wf, ctx(), policy);
    orch.run(exec, &store, "run", &spec(), 1000).unwrap()
}

#[test]
fn topo_order_serial_and_deterministic() {
    let w = wf(vec![
        vnode("b", vec!["a"]),
        vnode("a", vec![]),
        vnode("c", vec!["a"]),
    ]);
    let order = topo_order(&w).unwrap();
    assert_eq!(order[0], "a");
    assert!(order.iter().position(|x| x == "b").unwrap() > 0);
    assert!(order.iter().position(|x| x == "c").unwrap() > 0);
}

#[test]
fn dag_cycle_rejected_at_plan() {
    let w = wf(vec![vnode("a", vec!["b"]), vnode("b", vec!["a"])]);
    let orch = Orchestrator::new(w, ctx(), Policy::default());
    let err = orch.plan().unwrap_err();
    assert!(err.0.to_lowercase().contains("cycle"));
}

#[test]
fn unknown_dependency_rejected() {
    let w = wf(vec![vnode("a", vec!["ghost"])]);
    let err = topo_order(&w).unwrap_err();
    assert!(err.0.contains("unknown"));
}

#[test]
fn all_gates_proven_succeeds() {
    let w = wf(vec![vnode("a", vec![])]);
    let mut e = FnExec(|_n, _a| ExecOutcome::completed(vec![proven("g")], 10, 1));
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.state, NodeState::Succeeded);
    assert_eq!(rep.nodes["a"].state, NodeState::Succeeded);
    assert_eq!(rep.nodes["a"].proven_gates, vec!["g".to_string()]);
}

#[test]
fn failed_gate_blocks_succeeded() {
    let w = wf(vec![vnode("a", vec![])]);
    let bad = GateOutcome {
        gate_id: "g".into(),
        passed: false,
        evidence_ref: Some("ev".into()),
        exit_code: Some(1),
    };
    let mut e = FnExec(move |_n, _a| ExecOutcome::completed(vec![bad.clone()], 10, 1));
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Failed);
    assert_eq!(rep.state, NodeState::Failed);
}

#[test]
fn gate_pass_without_evidence_is_not_success() {
    let w = wf(vec![vnode("a", vec![])]);
    // passed=true but NO evidence_ref -> not proven (no NL success).
    let bare = GateOutcome {
        gate_id: "g".into(),
        passed: true,
        evidence_ref: None,
        exit_code: Some(0),
    };
    let mut e = FnExec(move |_n, _a| ExecOutcome::completed(vec![bare.clone()], 10, 1));
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Failed);
    assert_eq!(rep.state, NodeState::Failed);
}

#[test]
fn failure_propagates_to_downstream() {
    let w = wf(vec![vnode("a", vec![]), vnode("b", vec!["a"])]);
    let mut e = FnExec(|n, _a| {
        if n.id == "a" {
            ExecOutcome {
                kind: ExecKind::FatalFailure,
                tokens_used: 1,
                elapsed_secs: 1,
                gates: vec![],
                artifacts: vec![],
                detail: "boom".into(),
            }
        } else {
            ExecOutcome::completed(vec![proven("g")], 1, 1)
        }
    });
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Failed);
    assert_eq!(rep.nodes["b"].state, NodeState::Blocked);
    assert_eq!(rep.state, NodeState::Failed);
}

#[test]
fn verification_failure_repaired_then_succeeds() {
    let w = wf(vec![vnode("a", vec![])]);
    let mut e = FnExec(|_n, a| {
        if a == 1 {
            let bad = GateOutcome {
                gate_id: "g".into(),
                passed: false,
                evidence_ref: Some("ev".into()),
                exit_code: Some(1),
            };
            ExecOutcome::completed(vec![bad], 10, 1)
        } else {
            ExecOutcome::completed(vec![proven("g")], 10, 1)
        }
    });
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Succeeded);
    assert_eq!(rep.nodes["a"].attempts, 2);
}

#[test]
fn max_attempts_exhausted_is_failed() {
    let w = wf(vec![vnode("a", vec![])]);
    let mut e = FnExec(|_n, _a| ExecOutcome {
        kind: ExecKind::RetryableFailure,
        tokens_used: 1,
        elapsed_secs: 1,
        gates: vec![],
        artifacts: vec![],
        detail: "flaky".into(),
    });
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Failed);
    assert_eq!(rep.nodes["a"].attempts, 3); // 1 + 2 repairs
}

#[test]
fn non_idempotent_node_never_blindly_retried() {
    let w = wf(vec![vnode("publish", vec![])]);
    let mut policy = Policy::default();
    policy.non_idempotent.insert("publish".to_string());
    let mut e = FnExec(|_n, _a| ExecOutcome {
        kind: ExecKind::RetryableFailure,
        tokens_used: 1,
        elapsed_secs: 1,
        gates: vec![],
        artifacts: vec![],
        detail: "side-effect".into(),
    });
    let rep = run(w, policy, &mut e);
    assert_eq!(rep.nodes["publish"].state, NodeState::Uncertain);
    assert_eq!(rep.nodes["publish"].attempts, 1);
}

#[test]
fn fatal_failure_not_retried() {
    let w = wf(vec![vnode("a", vec![])]);
    let mut e = FnExec(|_n, _a| ExecOutcome {
        kind: ExecKind::FatalFailure,
        tokens_used: 1,
        elapsed_secs: 1,
        gates: vec![],
        artifacts: vec![],
        detail: "fatal".into(),
    });
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Failed);
    assert_eq!(rep.nodes["a"].attempts, 1);
}

#[test]
fn token_budget_exhaustion_blocks() {
    let mut w = wf(vec![vnode("a", vec![])]);
    w.nodes[0].budget_tokens = Some(100);
    let policy = Policy {
        max_tokens: 50,
        ..Default::default()
    };
    let mut e = FnExec(|_n, _a| ExecOutcome::completed(vec![proven("g")], 10, 1));
    let rep = run(w, policy, &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Blocked);
    assert_eq!(rep.state, NodeState::Blocked);
    assert_eq!(rep.nodes["a"].attempts, 0);
}

#[test]
fn wall_budget_exhaustion_cancels_remaining() {
    let w = wf(vec![vnode("a", vec![]), vnode("b", vec!["a"])]);
    let policy = Policy {
        max_wall_secs: 5,
        ..Default::default()
    };
    let mut e = FnExec(|_n, _a| ExecOutcome::completed(vec![proven("g")], 1, 10));
    let rep = run(w, policy, &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Succeeded);
    assert_eq!(rep.nodes["b"].state, NodeState::Cancelling);
    assert_eq!(rep.state, NodeState::Cancelling);
}

#[test]
fn needs_input_yields_waiting_input() {
    let w = wf(vec![vnode("a", vec![])]);
    let mut e = FnExec(|_n, _a| ExecOutcome {
        kind: ExecKind::NeedsInput,
        tokens_used: 1,
        elapsed_secs: 1,
        gates: vec![],
        artifacts: vec![],
        detail: "approve?".into(),
    });
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::WaitingInput);
    assert_eq!(rep.state, NodeState::WaitingInput);
}

#[test]
fn uncertain_kind_yields_uncertain() {
    let w = wf(vec![vnode("a", vec![])]);
    let mut e = FnExec(|_n, _a| ExecOutcome {
        kind: ExecKind::Uncertain,
        tokens_used: 1,
        elapsed_secs: 1,
        gates: vec![],
        artifacts: vec![],
        detail: "lost ack".into(),
    });
    let rep = run(w, Policy::default(), &mut e);
    assert_eq!(rep.nodes["a"].state, NodeState::Uncertain);
    assert_eq!(rep.state, NodeState::Uncertain);
}

#[test]
fn capability_requires_evidence_not_nl() {
    let w = wf(vec![Node {
        id: "cap".into(),
        depends_on: vec![],
        kind: NodeKind::Capability {
            command_ref: "cmd".into(),
            inputs: vec![],
            outputs: vec![],
        },
        model: None,
        budget_tokens: None,
    }]);
    // No gates, no artifacts -> cannot claim success.
    let mut e = FnExec(|_n, _a| ExecOutcome::completed(vec![], 1, 1));
    let rep = run(w.clone(), Policy::default(), &mut e);
    assert_eq!(rep.nodes["cap"].state, NodeState::Failed);

    // With an artifact evidence reference -> success.
    let mut e2 = FnExec(|_n, _a| {
        let mut o = ExecOutcome::completed(vec![], 1, 1);
        o.artifacts = vec!["out.json".into()];
        o
    });
    let rep2 = run(w, Policy::default(), &mut e2);
    assert_eq!(rep2.nodes["cap"].state, NodeState::Succeeded);
}

#[test]
fn downstream_invalidation_is_transitive() {
    let w = wf(vec![
        vnode("a", vec![]),
        vnode("b", vec!["a"]),
        vnode("c", vec!["b"]),
        vnode("x", vec![]),
    ]);
    let ds = downstream(&w, "a");
    assert!(ds.contains("b"));
    assert!(ds.contains("c"));
    assert!(!ds.contains("x"));
    assert!(!ds.contains("a"));
}

#[test]
fn run_persists_events_and_intents_to_store() {
    let w = wf(vec![vnode("a", vec![])]);
    let store = Store::open_in_memory().unwrap();
    let orch = Orchestrator::new(w, ctx(), Policy::default());
    let mut e = FnExec(|_n, _a| ExecOutcome::completed(vec![proven("g")], 10, 1));
    let rep = orch.run(&mut e, &store, "run", &spec(), 1000).unwrap();
    assert_eq!(rep.state, NodeState::Succeeded);
    let events = store.events_after("run", 0).unwrap();
    assert!(events.iter().any(|ev| ev.kind == "run_final"));
    assert!(events.iter().any(|ev| ev.kind == "attempt_finished"));
    // Intent was persisted and bound (no unbound leftovers).
    assert!(store.unbound_intents("run").unwrap().is_empty());
    let (got, state) = store.get_run("run").unwrap();
    assert_eq!(got.workflow_digest, "wfd");
    assert_eq!(state, "succeeded");
}
