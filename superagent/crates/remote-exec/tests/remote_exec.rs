//! P5-1 / #95 acceptance tests: a task completes tool execution in a remote
//! environment via the orchestrator; capability discovery feeds dispatch;
//! permission boundary (capability + sandbox) refuses with Blocked (no retry,
//! no host-shell fallback); authenticated connection rejects bad/expired tokens;
//! environment bootstrap; capability-node artifact evidence. Model-free over a
//! fixture exec-server backend + the real #81 orchestrator.

use std::collections::{BTreeMap, BTreeSet};

use superagent_contracts::{
    DryRunContext, Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION,
};
use superagent_orchestrator::{NodeState, Orchestrator, Policy};
use superagent_remote_exec::{
    CapabilityKind, Connection, RemoteError, RemoteExecExecutor, RemoteExecServer, SandboxMode,
    ToolRequest,
};
use superagent_storage::{RunSpec, Store};

const SECRET: &str = "remote-exec-signing-key";
const NOW: i64 = 1_700_000_000;

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

fn capnode(id: &str) -> Node {
    Node {
        id: id.to_string(),
        depends_on: vec![],
        kind: NodeKind::Capability {
            command_ref: "cmd".into(),
            inputs: vec![],
            outputs: vec![],
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

fn caps_all() -> BTreeSet<CapabilityKind> {
    [
        CapabilityKind::FileSystem,
        CapabilityKind::Process,
        CapabilityKind::Sandbox,
    ]
    .into_iter()
    .collect()
}

fn proc_tool(tool_id: &str, min_sandbox: SandboxMode) -> ToolRequest {
    ToolRequest {
        tool_id: tool_id.into(),
        required_capability: CapabilityKind::Process,
        argv: vec!["true".into()],
        min_sandbox,
    }
}

/// Build a server with one bootstrapped environment + an authenticated connection.
fn server_with_env(
    env_id: &str,
    capabilities: BTreeSet<CapabilityKind>,
    sandbox: SandboxMode,
) -> (RemoteExecServer, Connection) {
    let mut s = RemoteExecServer::new(SECRET);
    s.register_environment(env_id, capabilities, sandbox);
    let token = s.issue_token(env_id, NOW, 3600).unwrap();
    let conn = s.connect(&token, NOW).unwrap();
    (s, conn)
}

fn run(wf: Workflow, exec: &mut RemoteExecExecutor) -> superagent_orchestrator::RunReport {
    let store = Store::open_in_memory().unwrap();
    let orch = Orchestrator::new(wf, ctx(), Policy::default());
    orch.run(exec, &store, "run", &spec(), 1000).unwrap()
}

#[test]
fn task_completes_tool_execution_in_remote_environment() {
    let (s, conn) = server_with_env("env-1", caps_all(), SandboxMode::FullAccess);
    let mut exec = RemoteExecExecutor::new(s, conn);
    exec.bind_tool("a", proc_tool("build", SandboxMode::WorkspaceWrite));

    let rep = run(wf(vec![vnode("a", vec![])]), &mut exec);
    assert_eq!(rep.state, NodeState::Succeeded);
    assert_eq!(rep.nodes["a"].state, NodeState::Succeeded);
    assert_eq!(rep.nodes["a"].proven_gates, vec!["g".to_string()]);

    // The execution really went through the remote environment + audit.
    let audit = exec.server().audit();
    assert!(audit.iter().any(|e| e["event"] == "environment_bootstrap"));
    assert!(audit
        .iter()
        .any(|e| e["event"] == "remote_exec" && e["env"] == "env-1" && e["tool"] == "build"));
}

#[test]
fn capability_discovery_feeds_dispatch_and_denies_missing() {
    // Environment only has FileSystem; a Process tool must be refused.
    let fs_only: BTreeSet<CapabilityKind> = [CapabilityKind::FileSystem].into_iter().collect();
    let (s, conn) = server_with_env("env-fs", fs_only.clone(), SandboxMode::FullAccess);

    // Discovery returns exactly the environment inventory.
    let discovered = s.capability_discovery(&conn).unwrap();
    assert_eq!(discovered, fs_only);

    let mut exec = RemoteExecExecutor::new(s, conn);
    exec.bind_tool("a", proc_tool("build", SandboxMode::WorkspaceWrite));
    let rep = run(wf(vec![vnode("a", vec![])]), &mut exec);
    assert_eq!(rep.nodes["a"].state, NodeState::Blocked);
    assert_eq!(rep.state, NodeState::Blocked);
    assert_eq!(rep.nodes["a"].attempts, 1); // refused, never retried
}

#[test]
fn permission_boundary_sandbox_denied_no_fallback() {
    // ReadOnly environment cannot satisfy a WorkspaceWrite tool: hard refusal.
    let (s, conn) = server_with_env("env-ro", caps_all(), SandboxMode::ReadOnly);
    let mut exec = RemoteExecExecutor::new(s, conn);
    exec.bind_tool("a", proc_tool("write", SandboxMode::WorkspaceWrite));

    let rep = run(wf(vec![vnode("a", vec![])]), &mut exec);
    assert_eq!(rep.nodes["a"].state, NodeState::Blocked);
    assert_eq!(rep.state, NodeState::Blocked);
    assert_eq!(rep.nodes["a"].attempts, 1); // no retry, no host-shell fallback
    assert!(rep.nodes["a"].reason.contains("sandbox"));

    // Direct exec_tool surfaces the SandboxDenied error.
    let err = exec
        .server_mut()
        .exec_tool(
            &Connection {
                env_id: "env-ro".into(),
            },
            &proc_tool("write", SandboxMode::WorkspaceWrite),
        )
        .unwrap_err();
    assert!(matches!(err, RemoteError::SandboxDenied(_)));
}

#[test]
fn authenticated_connection_rejects_bad_and_expired_tokens() {
    let (s, _conn) = server_with_env("env-1", caps_all(), SandboxMode::FullAccess);

    // Malformed / tampered tokens rejected.
    assert!(matches!(
        s.connect("garbage", NOW).unwrap_err(),
        RemoteError::Auth
    ));
    let good = s.issue_token("env-1", NOW, 3600).unwrap();
    let tampered = format!("{}deadbeef", &good[..good.len() - 8]);
    assert!(matches!(
        s.connect(&tampered, NOW).unwrap_err(),
        RemoteError::Auth
    ));

    // Expired token rejected.
    let exp_token = s.issue_token("env-1", NOW - 7200, 3600).unwrap();
    assert!(matches!(
        s.connect(&exp_token, NOW).unwrap_err(),
        RemoteError::Expired
    ));

    // Token for an unknown environment rejected at issue time.
    assert!(matches!(
        s.issue_token("ghost", NOW, 3600).unwrap_err(),
        RemoteError::UnknownEnvironment(_)
    ));
}

#[test]
fn environment_bootstrap_and_unknown_environment() {
    let (mut s, _conn) = server_with_env("env-1", caps_all(), SandboxMode::FullAccess);
    // Bootstrap registered the environment with its sandbox + capabilities.
    let env = s.environment("env-1").unwrap();
    assert!(env.bootstrapped);
    assert_eq!(env.sandbox, SandboxMode::FullAccess);
    assert!(s
        .audit()
        .iter()
        .any(|e| e["event"] == "environment_bootstrap"));

    // A connection pointing at an unknown environment is refused.
    let ghost = Connection {
        env_id: "ghost".into(),
    };
    assert!(matches!(
        s.capability_discovery(&ghost).unwrap_err(),
        RemoteError::UnknownEnvironment(_)
    ));
    assert!(matches!(
        s.exec_tool(&ghost, &proc_tool("x", SandboxMode::ReadOnly))
            .unwrap_err(),
        RemoteError::UnknownEnvironment(_)
    ));
}

#[test]
fn capability_node_remote_artifact_evidence() {
    let (s, conn) = server_with_env("env-1", caps_all(), SandboxMode::FullAccess);
    let mut exec = RemoteExecExecutor::new(s, conn);
    exec.bind_tool("cap", proc_tool("fetch", SandboxMode::ReadOnly));

    // Capability node succeeds only with artifact evidence from the remote exec.
    let rep = run(wf(vec![capnode("cap")]), &mut exec);
    assert_eq!(rep.nodes["cap"].state, NodeState::Succeeded);
    assert_eq!(rep.state, NodeState::Succeeded);

    // An unbound node is a fatal failure (no silent success).
    let (s2, conn2) = server_with_env("env-1", caps_all(), SandboxMode::FullAccess);
    let mut exec2 = RemoteExecExecutor::new(s2, conn2);
    let rep2 = run(wf(vec![vnode("a", vec![])]), &mut exec2);
    assert_eq!(rep2.nodes["a"].state, NodeState::Failed);
}
