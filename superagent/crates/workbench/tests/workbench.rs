use std::collections::BTreeMap;

use superagent_contracts::{
    ArtifactSpec, DryRunContext, Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION,
};
use superagent_orchestrator::{
    ExecOutcome, GateOutcome, NodeExecutor, NodeState, Orchestrator, Policy,
};
use superagent_storage::{RunSpec, Store};
use superagent_workbench::{AppSpace, Canvas, CanvasNode, Position};

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

fn vnode(id: &str, deps: Vec<&str>) -> CanvasNode {
    CanvasNode {
        id: id.to_string(),
        depends_on: deps.into_iter().map(String::from).collect(),
        kind: NodeKind::Verification {
            argv: vec!["true".to_string()],
            gates: vec!["g".to_string()],
        },
        model: None,
        budget_tokens: None,
        position: Position { x: 0.0, y: 0.0 },
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

fn canvas3() -> Canvas {
    let mut c = Canvas::new("wf", "1");
    c.add_gate(gate_g());
    c.add_node(vnode("a", vec![]));
    c.add_node(vnode("b", vec!["a"]));
    c.add_node(vnode("c", vec!["a"]));
    c
}

#[test]
fn canvas_generates_valid_workflow() {
    let c = canvas3();
    let wf = c.to_workflow();
    assert_eq!(wf.schema_version, SCHEMA_VERSION);
    assert_eq!(wf.id, "wf");
    assert_eq!(wf.nodes.len(), 3);
    assert!(c.validate(&ctx()).is_empty());

    let json = c.to_workflow_json();
    // editor-only layout is stripped from the export.
    assert!(!serde_json::to_string(&json).unwrap().contains("position"));
    // the export round-trips back into a contracts Workflow.
    let back: Workflow = serde_json::from_value(json).unwrap();
    assert_eq!(back.nodes.len(), 3);
    assert_eq!(back.gates.len(), 1);
}

#[test]
fn canvas_workflow_executes_via_orchestrator() {
    let c = canvas3();
    assert!(c.validate(&ctx()).is_empty());
    let store = Store::open_in_memory().unwrap();
    let orch = Orchestrator::new(c.to_workflow(), ctx(), Policy::default());
    let order = orch.plan().unwrap();
    assert_eq!(order[0], "a");

    let mut e = FnExec(|_n, _a| ExecOutcome::completed(vec![proven("g")], 10, 1));
    let rep = orch.run(&mut e, &store, "run", &spec(), 1000).unwrap();
    assert_eq!(rep.state, NodeState::Succeeded);
    for id in ["a", "b", "c"] {
        assert_eq!(rep.nodes[id].state, NodeState::Succeeded);
        assert!(rep.nodes[id].proven_gates.contains(&"g".to_string()));
    }
}

#[test]
fn invalid_canvas_rejected() {
    // DAG cycle.
    let mut c = Canvas::new("wf", "1");
    c.add_gate(gate_g());
    c.add_node(vnode("a", vec!["b"]));
    c.add_node(vnode("b", vec!["a"]));
    assert!(c
        .validate(&ctx())
        .iter()
        .any(|e| e.message.to_lowercase().contains("cycle")));
    let orch = Orchestrator::new(c.to_workflow(), ctx(), Policy::default());
    assert!(orch.plan().is_err());

    // Unknown dependency.
    let mut c2 = Canvas::new("wf", "1");
    c2.add_gate(gate_g());
    c2.add_node(vnode("a", vec!["ghost"]));
    assert!(c2
        .validate(&ctx())
        .iter()
        .any(|e| e.message.contains("unknown")));

    // Unknown gate reference.
    let mut c3 = Canvas::new("wf", "1");
    c3.add_node(CanvasNode {
        id: "a".into(),
        depends_on: vec![],
        kind: NodeKind::Verification {
            argv: vec!["true".into()],
            gates: vec!["nope".into()],
        },
        model: None,
        budget_tokens: None,
        position: Position { x: 0.0, y: 0.0 },
    });
    assert!(c3
        .validate(&ctx())
        .iter()
        .any(|e| e.message.contains("unknown gate")));
}

#[test]
fn canvas_json_schema_roundtrip() {
    let c = canvas3();
    let json = c.to_workflow_json();
    let back: Workflow = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.nodes[0].id, "a");

    // unknown node type fails deserialization (never a success stub).
    let mut bad = json.clone();
    bad["nodes"][0]["type"] = serde_json::json!("mystery");
    assert!(serde_json::from_value::<Workflow>(bad).is_err());

    // unknown top-level field rejected (deny_unknown_fields).
    let mut bad2 = json.clone();
    bad2["surprise"] = serde_json::json!(1);
    assert!(serde_json::from_value::<Workflow>(bad2).is_err());
}

#[test]
fn app_space_tenant_resource_view() {
    let mut space = AppSpace::new();
    space.register_workflow("t1", "wf-a", &["art1".to_string()]);
    space.register_workflow("t1", "wf-b", &["art2".to_string()]);
    space.record_run("t1", "run-1");
    space.register_workflow("t2", "wf-c", &["art3".to_string()]);

    let v1 = space.view("t1").unwrap();
    assert_eq!(v1.workflows, vec!["wf-a", "wf-b"]);
    assert_eq!(v1.artifacts, vec!["art1", "art2"]);
    assert_eq!(v1.runs, vec!["run-1"]);

    let v2 = space.view("t2").unwrap();
    assert_eq!(v2.workflows, vec!["wf-c"]);
    // tenant isolation: t2 cannot see t1 resources.
    assert!(!v2.artifacts.contains(&"art1".to_string()));
    assert!(!v2.workflows.contains(&"wf-a".to_string()));
    assert!(space.view("t3").is_none());
    assert_eq!(space.tenants(), vec!["t1", "t2"]);

    // registration is idempotent.
    space.register_workflow("t1", "wf-a", &[]);
    assert_eq!(space.view("t1").unwrap().workflows.len(), 2);
}

#[test]
fn canvas_visualization_metadata() {
    let mut c = Canvas::new("wf", "1");
    c.add_gate(gate_g());
    c.add_node(vnode("a", vec![]));
    c.add_node(vnode("b", vec!["a"]));
    c.add_artifact(
        "out",
        ArtifactSpec {
            path: "out.json".into(),
            media_type: "application/json".into(),
            schema_ref: Some("s".into()),
        },
    );
    // dependency edges for the canvas graph view.
    assert_eq!(c.edges(), vec![("a".to_string(), "b".to_string())]);
    assert!(c.gates.contains_key("g"));
    assert!(c.artifacts.contains_key("out"));
    // layout preserved in the canvas model...
    assert_eq!(c.nodes[0].position, Position { x: 0.0, y: 0.0 });
    // ...but stripped from the workflow export.
    let json = c.to_workflow_json();
    assert!(!serde_json::to_string(&json).unwrap().contains("position"));
    assert_eq!(json["artifacts"]["out"]["path"], "out.json");
}
