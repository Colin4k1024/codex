//! SuperAgent deterministic outer orchestrator (P2-4 / #81).
//!
//! Success is decided ONLY by gate evidence — never by a natural-language
//! "done". The core is a serial DAG scheduler over a contracts [`Workflow`]:
//! an explicit node state machine, finite retry (a verification failure earns
//! at most two repairs, each re-running the relevant gates), token/wall
//! budgets, and dependency invalidation propagation.
//!
//! Node execution is injected via [`NodeExecutor`]; this crate itself never
//! spawns a process or calls a model, so the whole scheduler is unit-testable
//! with a mock executor (model-free). State is persisted through the #80
//! [`Store`] (intents, events, attempt states).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use superagent_contracts::{dry_run, DryRunContext, Node, NodeKind, Workflow};
use superagent_storage::{AttemptState, RunSpec, Store};

/// Explicit node lifecycle states (design §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Pending,
    Ready,
    Running,
    Verifying,
    Succeeded,
    WaitingInput,
    Blocked,
    Failed,
    Uncertain,
    Cancelling,
}

/// Outcome of a single gate. A gate counts as passed ONLY when `passed` is true
/// AND concrete `evidence_ref` is present — a bare assertion is not proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateOutcome {
    pub gate_id: String,
    pub passed: bool,
    pub evidence_ref: Option<String>,
    pub exit_code: Option<i32>,
}

impl GateOutcome {
    /// Evidence-backed pass: explicit success plus a durable evidence reference.
    pub fn is_proven(&self) -> bool {
        self.passed && self.evidence_ref.as_ref().is_some_and(|s| !s.is_empty())
    }
}

/// The kind of result an executor returns for one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecKind {
    Completed,
    NeedsInput,
    RetryableFailure,
    FatalFailure,
    Uncertain,
}

/// Result of one node attempt, carrying resource usage and gate evidence.
#[derive(Debug, Clone)]
pub struct ExecOutcome {
    pub kind: ExecKind,
    pub tokens_used: u64,
    pub elapsed_secs: u64,
    pub gates: Vec<GateOutcome>,
    pub artifacts: Vec<String>,
    pub detail: String,
}

impl ExecOutcome {
    pub fn completed(gates: Vec<GateOutcome>, tokens: u64, secs: u64) -> Self {
        ExecOutcome {
            kind: ExecKind::Completed,
            tokens_used: tokens,
            elapsed_secs: secs,
            gates,
            artifacts: vec![],
            detail: String::new(),
        }
    }
}

/// Injected node executor. The real implementation (#82) wires the adapter;
/// tests use a deterministic mock. `attempt` is 1-based.
pub trait NodeExecutor {
    fn execute(&mut self, node: &Node, attempt: u32) -> ExecOutcome;
}

/// Retry / budget policy.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Total attempts allowed per node (1 initial + up to 2 repairs = 3).
    pub max_attempts: u32,
    pub max_tokens: u64,
    pub max_wall_secs: u64,
    /// Nodes that must never be blindly retried (publish/migrate side effects).
    pub non_idempotent: BTreeSet<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            max_attempts: 3,
            max_tokens: 1_000_000,
            max_wall_secs: 3600,
            non_idempotent: BTreeSet::new(),
        }
    }
}

/// Per-node result in the final report.
#[derive(Debug, Clone)]
pub struct NodeReport {
    pub state: NodeState,
    pub attempts: u32,
    pub reason: String,
    pub proven_gates: Vec<String>,
}

/// Whole-run result.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub state: NodeState,
    pub nodes: BTreeMap<String, NodeReport>,
    pub tokens_used: u64,
    pub wall_secs: u64,
    pub order: Vec<String>,
}

/// Planning errors (e.g. DAG cycle) detected before any execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError(pub String);

/// Compute a serial topological order, or fail on a cycle / unknown dependency.
pub fn topo_order(wf: &Workflow) -> Result<Vec<String>, PlanError> {
    let ids: BTreeSet<&str> = wf.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut indeg: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &wf.nodes {
        for dep in &node.depends_on {
            if !ids.contains(dep.as_str()) {
                return Err(PlanError(format!(
                    "node '{}' depends on unknown '{}'",
                    node.id, dep
                )));
            }
            if dep == &node.id {
                return Err(PlanError(format!("node '{}' depends on itself", node.id)));
            }
            adj.entry(dep.as_str()).or_default().push(node.id.as_str());
            *indeg.get_mut(node.id.as_str()).unwrap() += 1;
        }
    }
    // Deterministic tie-break by node id for reproducible serial scheduling.
    let mut queue: VecDeque<&str> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut order = Vec::new();
    while let Some(id) = queue.pop_front() {
        order.push(id.to_string());
        if let Some(next) = adj.get(id) {
            let mut ready: Vec<&str> = Vec::new();
            for n in next {
                let d = indeg.get_mut(n).unwrap();
                *d -= 1;
                if *d == 0 {
                    ready.push(n);
                }
            }
            ready.sort();
            for n in ready {
                queue.push_back(n);
            }
        }
    }
    if order.len() != ids.len() {
        let cyclic: Vec<&str> = indeg
            .iter()
            .filter(|(_, d)| **d > 0)
            .map(|(id, _)| *id)
            .collect();
        return Err(PlanError(format!("DAG cycle among nodes: {cyclic:?}")));
    }
    Ok(order)
}

/// Transitive downstream dependents of `changed` (invalidation propagation).
pub fn downstream(wf: &Workflow, changed: &str) -> BTreeSet<String> {
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &wf.nodes {
        for dep in &node.depends_on {
            adj.entry(dep.as_str()).or_default().push(node.id.as_str());
        }
    }
    let mut out = BTreeSet::new();
    let mut stack = vec![changed];
    while let Some(cur) = stack.pop() {
        if let Some(next) = adj.get(cur) {
            for n in next {
                if out.insert(n.to_string()) {
                    stack.push(n);
                }
            }
        }
    }
    out
}

/// The deterministic orchestrator.
pub struct Orchestrator {
    wf: Workflow,
    ctx: DryRunContext,
    policy: Policy,
}

impl Orchestrator {
    pub fn new(wf: Workflow, ctx: DryRunContext, policy: Policy) -> Self {
        Orchestrator { wf, ctx, policy }
    }

    /// Validate the workflow (contracts dry-run) and compute the serial order.
    pub fn plan(&self) -> Result<Vec<String>, PlanError> {
        let errs = dry_run(&self.wf, &self.ctx);
        if !errs.is_empty() {
            let msg = errs
                .iter()
                .map(|e| format!("{}: {}", e.location, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(PlanError(msg));
        }
        topo_order(&self.wf)
    }

    fn node(&self, id: &str) -> &Node {
        self.wf
            .nodes
            .iter()
            .find(|n| n.id == id)
            .expect("node in topo order")
    }

    /// Gates a node must prove to succeed. Verification nodes list their gates;
    /// other nodes require at least one proven gate too (no silent success).
    fn required_gates(&self, node: &Node) -> Vec<String> {
        match &node.kind {
            NodeKind::Verification { gates, .. } => gates.clone(),
            NodeKind::Loop { gate, .. } => vec![gate.clone()],
            NodeKind::Capability { .. } => vec![],
        }
    }

    /// Run the workflow serially against an injected executor, persisting state
    /// to the #80 store. `now` is the logical start timestamp (deterministic).
    pub fn run<E: NodeExecutor>(
        &self,
        executor: &mut E,
        store: &Store,
        run_id: &str,
        spec: &RunSpec,
        now: i64,
    ) -> Result<RunReport, PlanError> {
        let order = self.plan()?;
        store
            .create_run(spec, now)
            .map_err(|e| PlanError(e.to_string()))?;
        store
            .seed_nodes(run_id, &self.wf)
            .map_err(|e| PlanError(e.to_string()))?;

        let mut states: BTreeMap<String, NodeState> = order
            .iter()
            .map(|id| (id.clone(), NodeState::Pending))
            .collect();
        let mut reports: BTreeMap<String, NodeReport> = BTreeMap::new();
        let mut tokens_used = 0u64;
        let mut wall_secs = 0u64;
        let mut clock = now;

        for id in &order {
            let node = self.node(id);

            // Failure / invalidation propagation: a dependency that did not
            // succeed blocks this node (never silently skipped to success).
            let blocked_dep = node
                .depends_on
                .iter()
                .find(|d| states.get(*d) != Some(&NodeState::Succeeded));
            if let Some(dep) = blocked_dep {
                let st = states.get(dep).copied().unwrap_or(NodeState::Pending);
                let reason = format!("dependency '{dep}' not succeeded ({st:?})");
                states.insert(id.clone(), NodeState::Blocked);
                reports.insert(
                    id.clone(),
                    NodeReport {
                        state: NodeState::Blocked,
                        attempts: 0,
                        reason,
                        proven_gates: vec![],
                    },
                );
                let _ = store.append_event(
                    run_id,
                    "node_blocked",
                    &format!("{id}: dependency {dep}"),
                    clock,
                );
                continue;
            }

            // Wall budget exhausted -> cancel remaining work explicitly.
            if wall_secs >= self.policy.max_wall_secs {
                states.insert(id.clone(), NodeState::Cancelling);
                reports.insert(
                    id.clone(),
                    NodeReport {
                        state: NodeState::Cancelling,
                        attempts: 0,
                        reason: "wall budget exhausted".into(),
                        proven_gates: vec![],
                    },
                );
                let _ = store.append_event(
                    run_id,
                    "node_cancelling",
                    &format!("{id}: wall budget"),
                    clock,
                );
                continue;
            }

            states.insert(id.clone(), NodeState::Ready);
            let required = self.required_gates(node);
            let idempotent = !self.policy.non_idempotent.contains(id);
            let mut attempt = 0u32;
            let mut final_state = NodeState::Failed;
            let mut reason = String::new();
            let mut proven: Vec<String> = vec![];

            while attempt < self.policy.max_attempts {
                // Token budget check before committing an attempt (a blocked
                // node has executed zero attempts).
                let node_budget = node.budget_tokens.unwrap_or(0);
                if tokens_used + node_budget > self.policy.max_tokens {
                    final_state = NodeState::Blocked;
                    reason = "token budget exhausted".into();
                    let _ = store.append_event(
                        run_id,
                        "node_blocked",
                        &format!("{id}: token budget"),
                        clock,
                    );
                    break;
                }

                attempt += 1;
                states.insert(id.clone(), NodeState::Running);
                // Persist intent first, then bind a logical external id (#80).
                let intent = serde_json::json!({ "node": id, "attempt": attempt }).to_string();
                match store.persist_intent(run_id, id, attempt, &intent, clock) {
                    Ok(aid) => {
                        let _ = store.bind_external_ids(
                            aid,
                            Some(run_id),
                            Some(&format!("{id}-a{attempt}")),
                            None,
                        );
                        let _ = store.set_attempt_state(aid, AttemptState::Running);
                    }
                    Err(e) => {
                        final_state = NodeState::Failed;
                        reason = format!("intent persist failed: {e}");
                        break;
                    }
                }

                let outcome = executor.execute(node, attempt);
                tokens_used += outcome.tokens_used;
                wall_secs += outcome.elapsed_secs;
                clock += outcome.elapsed_secs as i64;
                let _ = store.append_event(
                    run_id,
                    "attempt_finished",
                    &format!(
                        "{id} a{attempt} {:?} tokens={}",
                        outcome.kind, outcome.tokens_used
                    ),
                    clock,
                );

                match outcome.kind {
                    ExecKind::Completed => {
                        states.insert(id.clone(), NodeState::Verifying);
                        // Success requires EVERY required gate proven by evidence.
                        let proven_set: BTreeSet<&str> = outcome
                            .gates
                            .iter()
                            .filter(|g| g.is_proven())
                            .map(|g| g.gate_id.as_str())
                            .collect();
                        let missing: Vec<&String> = required
                            .iter()
                            .filter(|g| !proven_set.contains(g.as_str()))
                            .collect();
                        if required.is_empty() {
                            // Capability node: success needs at least one proven gate
                            // OR an explicit artifact evidence; never bare NL.
                            if outcome.gates.iter().any(|g| g.is_proven())
                                || !outcome.artifacts.is_empty()
                            {
                                final_state = NodeState::Succeeded;
                                proven = outcome
                                    .gates
                                    .iter()
                                    .filter(|g| g.is_proven())
                                    .map(|g| g.gate_id.clone())
                                    .collect();
                                reason = "capability evidence present".into();
                            } else {
                                final_state = NodeState::Failed;
                                reason = "no gate evidence for capability success".into();
                            }
                        } else if missing.is_empty() {
                            final_state = NodeState::Succeeded;
                            proven = required.clone();
                            reason = "all required gates proven".into();
                        } else {
                            // A gate failed/unproven: repair only if retryable & idempotent.
                            let detail = format!("unproven gates: {missing:?}");
                            if idempotent && attempt < self.policy.max_attempts {
                                reason = format!("{detail}; repairing (attempt {attempt})");
                                let _ = store.append_event(
                                    run_id,
                                    "gate_repair",
                                    &format!("{id}: {detail}"),
                                    clock,
                                );
                                continue;
                            }
                            final_state = NodeState::Failed;
                            reason = detail;
                        }
                    }
                    ExecKind::NeedsInput => {
                        final_state = NodeState::WaitingInput;
                        reason = outcome.detail.clone();
                    }
                    ExecKind::RetryableFailure => {
                        if idempotent && attempt < self.policy.max_attempts {
                            reason =
                                format!("retryable failure: {}; attempt {attempt}", outcome.detail);
                            continue;
                        }
                        // Non-idempotent nodes are never blindly retried.
                        final_state = if idempotent {
                            NodeState::Failed
                        } else {
                            NodeState::Uncertain
                        };
                        reason = format!("retryable failure (no retry): {}", outcome.detail);
                    }
                    ExecKind::FatalFailure => {
                        final_state = NodeState::Failed;
                        reason = format!("fatal: {}", outcome.detail);
                    }
                    ExecKind::Uncertain => {
                        final_state = NodeState::Uncertain;
                        reason = format!("uncertain: {}", outcome.detail);
                    }
                }
                break;
            }

            if attempt >= self.policy.max_attempts
                && final_state == NodeState::Failed
                && reason.is_empty()
            {
                reason = "max attempts exhausted".into();
            }
            states.insert(id.clone(), final_state);
            reports.insert(
                id.clone(),
                NodeReport {
                    state: final_state,
                    attempts: attempt,
                    reason,
                    proven_gates: proven,
                },
            );
            let _ = store.append_event(
                run_id,
                "node_final",
                &format!("{id}: {final_state:?}"),
                clock,
            );
        }

        // Aggregate run state: success only if every node succeeded. A genuine
        // failure dominates a downstream block (the block is its consequence).
        let run_state = if reports.values().all(|r| r.state == NodeState::Succeeded) {
            NodeState::Succeeded
        } else if reports.values().any(|r| r.state == NodeState::Failed) {
            NodeState::Failed
        } else if reports.values().any(|r| r.state == NodeState::Uncertain) {
            NodeState::Uncertain
        } else if reports.values().any(|r| r.state == NodeState::WaitingInput) {
            NodeState::WaitingInput
        } else if reports.values().any(|r| r.state == NodeState::Cancelling) {
            NodeState::Cancelling
        } else if reports.values().any(|r| r.state == NodeState::Blocked) {
            NodeState::Blocked
        } else {
            NodeState::Failed
        };
        let _ = store.set_run_state(run_id, &format!("{run_state:?}").to_lowercase());
        let _ = store.append_event(run_id, "run_final", &format!("run: {run_state:?}"), clock);

        Ok(RunReport {
            state: run_state,
            nodes: reports,
            tokens_used,
            wall_secs,
            order,
        })
    }
}
