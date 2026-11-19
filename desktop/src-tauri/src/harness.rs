//! U05 Harness business-task panel backend (HARNESS_CLAUDECODEUI_MIGRATION.md).
//!
//! Reuses the SuperAgent deterministic orchestrator (P2-4) + contracts (P2-1) +
//! storage (P2-3) as the authoritative, persisted, gate-evidence-only state
//! machine. This is NOT TaskMaster: no TaskMaster CLI/data is involved, and a
//! node only succeeds when its gates are proven by concrete evidence (a real
//! exit code or a durable artifact reference) — never by a natural-language
//! "done". The WebView reads backend-resolved task/artifact identifiers; it never
//! supplies arbitrary host paths or spawns processes.
//!
//! For the desktop MVP the node executor is a deterministic team-delivery fixture
//! (T18): capability nodes emit artifact evidence, verification nodes run their
//! declared gate argv as real processes and capture the actual exit code. Wiring
//! the live AgentExecutor/VerificationExecutor (P2-5/P2-6) to the engine adapter is
//! the documented next step; the scheduler/state contract is already the real one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use serde_json::{json, Value};
use superagent_contracts::{
    ArtifactSpec, DryRunContext, Gate, GateKind, Node, NodeKind, Workflow, SCHEMA_VERSION,
};
use superagent_orchestrator::{
    ExecKind, ExecOutcome, GateOutcome, NodeExecutor, NodeState, Orchestrator, Policy,
};
use superagent_storage::{RunSpec, Store};

/// Which deterministic fixture path to run. The product default is `Success`;
/// the failure/waiting variants exist so the state machine's failed/blocked and
/// waiting_input branches are exercised by tests (and reachable for diagnosis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureMode {
    Success,
    VerifyFails,
    NeedsInput,
}

impl FixtureMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "verify_fails" => FixtureMode::VerifyFails,
            "needs_input" => FixtureMode::NeedsInput,
            _ => FixtureMode::Success,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GateView {
    pub id: String,
    pub passed: bool,
    pub proven: bool,
    pub exit_code: Option<i32>,
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeView {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub attempts: u32,
    pub reason: String,
    pub depends_on: Vec<String>,
    pub gates: Vec<GateView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactView {
    pub id: String,
    pub node_id: Option<String>,
    pub name: String,
    pub media_type: String,
    pub size_bytes: usize,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventView {
    pub seq: i64,
    pub kind: String,
    pub summary: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub workflow_id: String,
    pub state: String,
    pub created_at: i64,
    pub node_count: usize,
    pub succeeded_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskDetail {
    pub id: String,
    pub title: String,
    pub workflow_id: String,
    pub state: String,
    pub created_at: i64,
    pub nodes: Vec<NodeView>,
    pub artifacts: Vec<ArtifactView>,
    pub report: String,
    pub tokens_used: u64,
    pub wall_secs: u64,
    pub order: Vec<String>,
    pub events: Vec<EventView>,
}

struct Task {
    detail: TaskDetail,
    contents: BTreeMap<String, String>,
}

/// Authoritative harness state: a persisted Store (run identity, events,
/// artifacts) plus an in-session cache of detailed node/gate snapshots. The
/// scheduler decides every terminal state; the cache is only for rendering.
pub struct Harness {
    store: Store,
    tasks: BTreeMap<String, Task>,
    counter: u64,
}

/// Isolated harness database. Never the user's data; override for tests/diagnosis.
pub fn resolve_harness_db() -> PathBuf {
    if let Ok(p) = std::env::var("YOURAGENT_HARNESS_DB") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".youragent").join("harness.db")
}

impl Harness {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let store = Store::open(path).map_err(|e| e.to_string())?;
        Ok(Harness {
            store,
            tasks: BTreeMap::new(),
            counter: 0,
        })
    }

    pub fn in_memory() -> Self {
        Harness {
            store: Store::open_in_memory().expect("in-memory store"),
            tasks: BTreeMap::new(),
            counter: 0,
        }
    }

    pub fn list_tasks(&self) -> Vec<TaskSummary> {
        self.tasks
            .values()
            .map(|t| {
                let d = &t.detail;
                TaskSummary {
                    id: d.id.clone(),
                    title: d.title.clone(),
                    workflow_id: d.workflow_id.clone(),
                    state: d.state.clone(),
                    created_at: d.created_at,
                    node_count: d.nodes.len(),
                    succeeded_count: d.nodes.iter().filter(|n| n.state == "succeeded").count(),
                }
            })
            .collect()
    }

    pub fn get_task(&self, id: &str) -> Option<TaskDetail> {
        let t = self.tasks.get(id)?;
        let mut d = t.detail.clone();
        d.events = self.events(id);
        Some(d)
    }

    pub fn read_artifact(&self, task_id: &str, artifact_id: &str) -> Option<(String, String)> {
        let t = self.tasks.get(task_id)?;
        let content = t.contents.get(artifact_id)?;
        let media = t
            .detail
            .artifacts
            .iter()
            .find(|a| a.id == artifact_id)
            .map(|a| a.media_type.clone())
            .unwrap_or_else(|| "text/plain".into());
        Some((media, content.clone()))
    }

    pub fn export_report(&self, task_id: &str) -> Option<String> {
        self.tasks.get(task_id).map(|t| t.detail.report.clone())
    }

    fn events(&self, run_id: &str) -> Vec<EventView> {
        self.store
            .events_after(run_id, 0)
            .unwrap_or_default()
            .into_iter()
            .map(|e| EventView {
                seq: e.seq,
                kind: e.kind,
                summary: e.summary,
                created_at: e.created_at,
            })
            .collect()
    }

    /// Create and run a team-delivery business task through the real orchestrator.
    pub fn create_task(&mut self, title: &str, mode: FixtureMode) -> Result<TaskDetail, String> {
        self.counter += 1;
        let task_id = format!("task-{}", self.counter);
        let now = chrono::Utc::now().timestamp();

        let wf = fixture_workflow(mode);
        let ctx = fixture_context();
        let orch = Orchestrator::new(wf.clone(), ctx, Policy::default());
        let mut executor = FixtureExecutor::new(mode, wf.gates.clone());

        let spec = RunSpec {
            id: task_id.clone(),
            workflow_digest: format!("{}@{}", wf.id, wf.version),
            bundle_digest: "team-delivery-fixture".into(),
            input_summary: title.to_string(),
            repo_identity: "desktop-fixture".into(),
            start_head: "fixture".into(),
            config_summary: format!("{mode:?}"),
        };

        let report = orch
            .run(&mut executor, &self.store, &task_id, &spec, now)
            .map_err(|e| e.0)?;

        let nodes = build_nodes(&wf, &report, &executor);
        let (artifacts, contents) = self.record_artifacts(&task_id, &wf, &executor);
        let report_md = render_report(title, &task_id, &report, &nodes);
        let state = state_str(report.state);

        let detail = TaskDetail {
            id: task_id.clone(),
            title: title.to_string(),
            workflow_id: wf.id.clone(),
            state,
            created_at: now,
            nodes,
            artifacts,
            report: report_md,
            tokens_used: report.tokens_used,
            wall_secs: report.wall_secs,
            order: report.order.clone(),
            events: self.events(&task_id),
        };
        self.tasks.insert(
            task_id,
            Task {
                detail: detail.clone(),
                contents,
            },
        );
        Ok(detail)
    }

    fn record_artifacts(
        &self,
        task_id: &str,
        wf: &Workflow,
        executor: &FixtureExecutor,
    ) -> (Vec<ArtifactView>, BTreeMap<String, String>) {
        let mut views = Vec::new();
        let mut contents = BTreeMap::new();
        for (node_id, arts) in &executor.artifacts {
            for (key, content) in arts {
                let spec = wf.artifacts.get(key);
                let media = spec
                    .map(|s| s.media_type.clone())
                    .unwrap_or_else(|| "text/plain".into());
                let path = spec.map(|s| s.path.clone()).unwrap_or_else(|| key.clone());
                let id = format!("{task_id}/{key}");
                let _ = self.store.record_artifact(
                    task_id,
                    Some(node_id),
                    &path,
                    &media,
                    None,
                    Some(content.len() as i64),
                    None,
                    Some(key),
                );
                views.push(ArtifactView {
                    id: id.clone(),
                    node_id: Some(node_id.clone()),
                    name: key.clone(),
                    media_type: media,
                    size_bytes: content.len(),
                    path,
                });
                contents.insert(id, content.clone());
            }
        }
        (views, contents)
    }
}

fn state_str(s: NodeState) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "failed".into())
}

fn kind_str(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Capability { .. } => "capability",
        NodeKind::Verification { .. } => "verification",
        NodeKind::Loop { .. } => "loop",
    }
}

fn build_nodes(
    wf: &Workflow,
    report: &superagent_orchestrator::RunReport,
    executor: &FixtureExecutor,
) -> Vec<NodeView> {
    report
        .order
        .iter()
        .map(|id| {
            let nr = &report.nodes[id];
            let node = wf.nodes.iter().find(|n| &n.id == id);
            let gates = executor
                .gates_by_node
                .get(id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|g| GateView {
                    proven: g.is_proven(),
                    id: g.gate_id,
                    passed: g.passed,
                    exit_code: g.exit_code,
                    evidence_ref: g.evidence_ref,
                })
                .collect();
            NodeView {
                id: id.clone(),
                kind: node
                    .map(|n| kind_str(&n.kind))
                    .unwrap_or("capability")
                    .into(),
                state: state_str(nr.state),
                attempts: nr.attempts,
                reason: nr.reason.clone(),
                depends_on: node.map(|n| n.depends_on.clone()).unwrap_or_default(),
                gates,
            }
        })
        .collect()
}

fn render_report(
    title: &str,
    task_id: &str,
    report: &superagent_orchestrator::RunReport,
    nodes: &[NodeView],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Harness report — {title}\n\n"));
    s.push_str(&format!("- Task: `{task_id}`\n"));
    s.push_str(&format!("- Run state: **{}**\n", state_str(report.state)));
    s.push_str(&format!("- Tokens used: {}\n", report.tokens_used));
    s.push_str(&format!("- Wall seconds: {}\n\n", report.wall_secs));
    s.push_str("## Nodes\n\n");
    for n in nodes {
        s.push_str(&format!("### {} ({}) — {}\n", n.id, n.kind, n.state));
        s.push_str(&format!("- attempts: {}\n", n.attempts));
        if !n.reason.is_empty() {
            s.push_str(&format!("- reason: {}\n", n.reason));
        }
        if n.gates.is_empty() {
            s.push_str("- gates: (none executed)\n");
        } else {
            for g in &n.gates {
                let exit = g
                    .exit_code
                    .map(|c| format!(", exit {c}"))
                    .unwrap_or_default();
                let ev = g
                    .evidence_ref
                    .clone()
                    .unwrap_or_else(|| "no evidence".into());
                s.push_str(&format!(
                    "- gate `{}`: {}{exit} — {ev}\n",
                    g.id,
                    if g.proven { "proven" } else { "unproven" }
                ));
            }
        }
        s.push('\n');
    }
    s.push_str("## Limitations\n\n");
    s.push_str("Success is decided only by gate evidence (real exit codes / durable artifacts), never by model text. ");
    s.push_str("This run used the deterministic team-delivery fixture executor; live engine-driven execution is a follow-up.\n");
    s
}

/// Run a gate's declared argv as a real process (no shell) and capture its actual
/// exit code plus a durable evidence reference.
fn run_argv(argv: &[String]) -> (Option<i32>, Option<String>) {
    if argv.is_empty() {
        return (None, None);
    }
    match Command::new(&argv[0]).args(&argv[1..]).output() {
        Ok(out) => {
            let code = out.status.code();
            let ev = format!("cmd:{} -> exit {:?}", argv.join(" "), code);
            (code, Some(ev))
        }
        Err(e) => (
            None,
            Some(format!("cmd:{} -> spawn error {e}", argv.join(" "))),
        ),
    }
}

fn fixture_artifact_content(command_ref: &str, node_id: &str, key: &str) -> String {
    match key {
        "report_doc" => format!(
            "# Team delivery report\n\nNode `{node_id}` ({command_ref}) compiled the run summary.\n"
        ),
        "review_doc" => format!("Review notes from `{node_id}` ({command_ref}): changes reviewed.\n"),
        "changes_doc" => format!("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ fixture change by {node_id} @@\n"),
        "plan_doc" => json!({ "node": node_id, "command": command_ref, "steps": ["implement", "verify", "review"] }).to_string(),
        _ => json!({ "node": node_id, "command": command_ref, "artifact": key }).to_string(),
    }
}

struct FixtureExecutor {
    mode: FixtureMode,
    gates: BTreeMap<String, Gate>,
    gates_by_node: BTreeMap<String, Vec<GateOutcome>>,
    artifacts: BTreeMap<String, Vec<(String, String)>>,
}

impl FixtureExecutor {
    fn new(mode: FixtureMode, gates: BTreeMap<String, Gate>) -> Self {
        FixtureExecutor {
            mode,
            gates,
            gates_by_node: BTreeMap::new(),
            artifacts: BTreeMap::new(),
        }
    }

    fn run_gate(&self, gid: &str) -> GateOutcome {
        match self.gates.get(gid).map(|g| &g.kind) {
            Some(GateKind::Command {
                argv,
                expect_exit_code,
            }) => {
                let (exit, ev) = run_argv(argv);
                GateOutcome {
                    gate_id: gid.to_string(),
                    passed: exit == Some(*expect_exit_code),
                    evidence_ref: ev,
                    exit_code: exit,
                }
            }
            _ => GateOutcome {
                gate_id: gid.to_string(),
                passed: false,
                evidence_ref: None,
                exit_code: None,
            },
        }
    }
}

impl NodeExecutor for FixtureExecutor {
    fn execute(&mut self, node: &Node, _attempt: u32) -> ExecOutcome {
        match &node.kind {
            NodeKind::Capability {
                command_ref,
                outputs,
                ..
            } => {
                if self.mode == FixtureMode::NeedsInput && node.id == "implement" {
                    return ExecOutcome {
                        kind: ExecKind::NeedsInput,
                        tokens_used: 10,
                        elapsed_secs: 1,
                        gates: vec![],
                        artifacts: vec![],
                        detail: "acceptance command missing; awaiting user input".into(),
                    };
                }
                let mut arts = Vec::new();
                for out in outputs {
                    let content = fixture_artifact_content(command_ref, &node.id, out);
                    self.artifacts
                        .entry(node.id.clone())
                        .or_default()
                        .push((out.clone(), content));
                    arts.push(out.clone());
                }
                let gate = GateOutcome {
                    gate_id: format!("{}:evidence", node.id),
                    passed: true,
                    evidence_ref: arts.first().cloned(),
                    exit_code: None,
                };
                self.gates_by_node
                    .insert(node.id.clone(), vec![gate.clone()]);
                ExecOutcome {
                    kind: ExecKind::Completed,
                    tokens_used: 100,
                    elapsed_secs: 2,
                    gates: vec![gate],
                    artifacts: arts,
                    detail: format!("{command_ref} produced artifact evidence"),
                }
            }
            NodeKind::Verification { gates, .. } => {
                let outcomes: Vec<GateOutcome> = gates.iter().map(|g| self.run_gate(g)).collect();
                self.gates_by_node.insert(node.id.clone(), outcomes.clone());
                ExecOutcome {
                    kind: ExecKind::Completed,
                    tokens_used: 50,
                    elapsed_secs: 3,
                    gates: outcomes,
                    artifacts: vec![],
                    detail: "verification gates executed with real exit codes".into(),
                }
            }
            NodeKind::Loop { gate, .. } => {
                let outcome = self.run_gate(gate);
                self.gates_by_node
                    .insert(node.id.clone(), vec![outcome.clone()]);
                ExecOutcome {
                    kind: ExecKind::Completed,
                    tokens_used: 50,
                    elapsed_secs: 3,
                    gates: vec![outcome],
                    artifacts: vec![],
                    detail: "loop gate executed".into(),
                }
            }
        }
    }
}

fn cap_node(
    id: &str,
    command_ref: &str,
    depends_on: &[&str],
    inputs: &[&str],
    outputs: &[&str],
) -> Node {
    Node {
        id: id.into(),
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        kind: NodeKind::Capability {
            command_ref: command_ref.into(),
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
        },
        model: None,
        budget_tokens: Some(1000),
    }
}

fn artifact(path: &str, media: &str) -> ArtifactSpec {
    ArtifactSpec {
        path: path.into(),
        media_type: media.into(),
        schema_ref: None,
    }
}

/// The team-delivery fixture workflow: intake → plan → implement → verify →
/// review → report. `verify` runs a real gate command; VerifyFails swaps it for a
/// program that exits non-zero so the scheduler must fail/block (never succeed).
pub fn fixture_workflow(mode: FixtureMode) -> Workflow {
    let verify_prog = if mode == FixtureMode::VerifyFails {
        "false"
    } else {
        "true"
    };
    let mut gates = BTreeMap::new();
    gates.insert(
        "tests_pass".into(),
        Gate {
            id: "tests_pass".into(),
            kind: GateKind::Command {
                argv: vec![verify_prog.into()],
                expect_exit_code: 0,
            },
        },
    );
    let mut artifacts = BTreeMap::new();
    artifacts.insert(
        "intake_doc".into(),
        artifact("intake.json", "application/json"),
    );
    artifacts.insert("plan_doc".into(), artifact("plan.json", "application/json"));
    artifacts.insert(
        "changes_doc".into(),
        artifact("changes.patch", "text/plain"),
    );
    artifacts.insert("review_doc".into(), artifact("review.md", "text/markdown"));
    artifacts.insert("report_doc".into(), artifact("report.md", "text/markdown"));

    Workflow {
        schema_version: SCHEMA_VERSION,
        id: "team-delivery".into(),
        version: "1.0.0".into(),
        variables: BTreeMap::new(),
        nodes: vec![
            cap_node("intake", "team/intake", &[], &[], &["intake_doc"]),
            cap_node(
                "plan",
                "team/plan",
                &["intake"],
                &["intake_doc"],
                &["plan_doc"],
            ),
            cap_node(
                "implement",
                "team/implement",
                &["plan"],
                &["plan_doc"],
                &["changes_doc"],
            ),
            Node {
                id: "verify".into(),
                depends_on: vec!["implement".into()],
                kind: NodeKind::Verification {
                    argv: vec![verify_prog.into()],
                    gates: vec!["tests_pass".into()],
                },
                model: None,
                budget_tokens: Some(500),
            },
            cap_node(
                "review",
                "team/review",
                &["verify"],
                &["changes_doc"],
                &["review_doc"],
            ),
            cap_node(
                "report",
                "team/report",
                &["review"],
                &["review_doc"],
                &["report_doc"],
            ),
        ],
        gates,
        artifacts,
    }
}

pub fn fixture_context() -> DryRunContext {
    DryRunContext {
        variables: BTreeMap::new(),
        command_catalog: vec![
            "team/intake".into(),
            "team/plan".into(),
            "team/implement".into(),
            "team/review".into(),
            "team/report".into(),
        ],
        models: vec![],
        programs: vec!["true".into(), "false".into()],
        schema_registry: vec![],
    }
}

/// JSON shape returned by the list command.
pub fn list_json(h: &Harness) -> Value {
    json!({ "tasks": h.list_tasks() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_run_proves_every_gate() {
        let mut h = Harness::in_memory();
        let d = h.create_task("demo success", FixtureMode::Success).unwrap();
        assert_eq!(d.state, "succeeded");
        assert_eq!(d.nodes.len(), 6);
        for n in &d.nodes {
            assert_eq!(n.state, "succeeded", "node {} reason {}", n.id, n.reason);
        }
        // The verification gate carries a REAL exit code 0 and evidence.
        let verify = d.nodes.iter().find(|n| n.id == "verify").unwrap();
        let g = &verify.gates[0];
        assert_eq!(g.exit_code, Some(0));
        assert!(g.proven);
        assert!(g.evidence_ref.as_deref().unwrap().contains("exit"));
        // Capability nodes succeed via artifact evidence, not bare text.
        let intake = d.nodes.iter().find(|n| n.id == "intake").unwrap();
        assert!(intake.gates[0].proven);
        assert!(!d.artifacts.is_empty());
        assert!(d.report.contains("Run state: **succeeded**"));
    }

    #[test]
    fn failing_verify_blocks_downstream_and_fails_run() {
        let mut h = Harness::in_memory();
        let d = h
            .create_task("demo fail", FixtureMode::VerifyFails)
            .unwrap();
        assert_eq!(d.state, "failed");
        let verify = d.nodes.iter().find(|n| n.id == "verify").unwrap();
        assert_eq!(verify.state, "failed");
        assert_eq!(verify.gates[0].exit_code, Some(1));
        assert!(!verify.gates[0].proven);
        // A genuine failure dominates; dependents are blocked, never succeeded.
        for id in ["review", "report"] {
            let n = d.nodes.iter().find(|n| n.id == id).unwrap();
            assert_eq!(n.state, "blocked", "{id} must be blocked");
        }
        // Finite retry: verify attempted up to the policy max, not infinite.
        assert!(verify.attempts >= 1 && verify.attempts <= 3);
    }

    #[test]
    fn missing_acceptance_waits_for_input() {
        let mut h = Harness::in_memory();
        let d = h.create_task("demo wait", FixtureMode::NeedsInput).unwrap();
        assert_eq!(d.state, "waiting_input");
        let implement = d.nodes.iter().find(|n| n.id == "implement").unwrap();
        assert_eq!(implement.state, "waiting_input");
        let verify = d.nodes.iter().find(|n| n.id == "verify").unwrap();
        assert_eq!(verify.state, "blocked");
    }

    #[test]
    fn artifact_read_by_backend_id_and_report_export() {
        let mut h = Harness::in_memory();
        let d = h
            .create_task("demo artifacts", FixtureMode::Success)
            .unwrap();
        let art = d.artifacts.iter().find(|a| a.name == "report_doc").unwrap();
        let (media, content) = h.read_artifact(&d.id, &art.id).unwrap();
        assert_eq!(media, "text/markdown");
        assert!(content.contains("Team delivery report"));
        // Unknown artifact id is rejected (no arbitrary host path access).
        assert!(h.read_artifact(&d.id, "task-1/../../etc/passwd").is_none());
        assert!(h.read_artifact("nope", &art.id).is_none());
        let report = h.export_report(&d.id).unwrap();
        assert!(report.contains("Harness report"));
        assert!(h.export_report("nope").is_none());
    }

    #[test]
    fn list_and_get_refresh_events() {
        let mut h = Harness::in_memory();
        assert!(h.list_tasks().is_empty());
        let d = h.create_task("demo list", FixtureMode::Success).unwrap();
        let list = h.list_tasks();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].succeeded_count, 6);
        let got = h.get_task(&d.id).unwrap();
        assert!(!got.events.is_empty(), "persisted events must be readable");
        assert!(got.events.iter().any(|e| e.kind == "run_final"));
        assert!(h.get_task("missing").is_none());
    }

    #[test]
    fn fixture_workflow_plans_without_validation_errors() {
        let wf = fixture_workflow(FixtureMode::Success);
        let orch = Orchestrator::new(wf, fixture_context(), Policy::default());
        let order = orch.plan().expect("valid fixture workflow");
        assert_eq!(
            order,
            vec!["intake", "plan", "implement", "verify", "review", "report"]
        );
    }
}
