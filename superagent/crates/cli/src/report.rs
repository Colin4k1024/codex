//! `superagent report`: separate verified from unverified items, pre-existing
//! from this-run changes, and record version / usage / evidence. The builder is
//! a pure function so its completeness is testable without a model.

use serde_json::{json, Value};
use superagent_orchestrator::RunReport;
use superagent_storage::Store;

use crate::Result;

#[derive(Debug, Clone)]
pub struct NodeOutcome {
    pub id: String,
    pub state: String,
    pub proven_gates: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub path: String,
    pub digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReportInput {
    pub run_id: String,
    pub version: String,
    pub run_state: String,
    pub nodes: Vec<NodeOutcome>,
    pub tokens_used: u64,
    pub wall_secs: u64,
    /// Changes that existed before this run (baseline workspace state).
    pub baseline_changes: Vec<Change>,
    /// Changes produced by this run.
    pub run_changes: Vec<Change>,
    pub evidence: Vec<String>,
}

fn change_json(c: &Change) -> Value {
    json!({ "path": c.path, "digest": c.digest })
}

/// Build the complete report JSON. Verified = nodes whose success is backed by
/// gate/artifact evidence (state Succeeded by orchestrator construction);
/// unverified = every other node, with its reason. Never merges the two.
pub fn build_report(inp: &ReportInput) -> Value {
    let mut verified = Vec::new();
    let mut unverified = Vec::new();
    for n in &inp.nodes {
        let item = json!({
            "id": n.id,
            "state": n.state,
            "provenGates": n.proven_gates,
            "reason": n.reason,
        });
        if n.state == "Succeeded" {
            verified.push(item);
        } else {
            unverified.push(item);
        }
    }
    json!({
        "runId": inp.run_id,
        "version": inp.version,
        "state": inp.run_state,
        "verified": verified,
        "unverified": unverified,
        "changes": {
            "baseline": inp.baseline_changes.iter().map(change_json).collect::<Vec<_>>(),
            "thisRun": inp.run_changes.iter().map(change_json).collect::<Vec<_>>(),
        },
        "usage": { "tokensUsed": inp.tokens_used, "wallSecs": inp.wall_secs },
        "evidence": inp.evidence,
    })
}

/// Adapt an in-process orchestrator [`RunReport`] into report input.
pub fn from_run_report(
    run_id: &str,
    version: &str,
    report: &RunReport,
    baseline_changes: Vec<Change>,
    run_changes: Vec<Change>,
    evidence: Vec<String>,
) -> ReportInput {
    let nodes = report
        .nodes
        .iter()
        .map(|(id, nr)| NodeOutcome {
            id: id.clone(),
            state: format!("{:?}", nr.state),
            proven_gates: nr.proven_gates.clone(),
            reason: nr.reason.clone(),
        })
        .collect();
    ReportInput {
        run_id: run_id.to_string(),
        version: version.to_string(),
        run_state: format!("{:?}", report.state),
        nodes,
        tokens_used: report.tokens_used,
        wall_secs: report.wall_secs,
        baseline_changes,
        run_changes,
        evidence,
    }
}

/// Derive per-node outcomes from the durable event log (`node_final` events),
/// for a post-hoc report read from the store. Proven-gate detail is not in the
/// event log; the state still separates verified (Succeeded) from unverified.
pub fn outcomes_from_events(store: &Store, run_id: &str) -> Result<Vec<NodeOutcome>> {
    let events = store.events_after(run_id, 0)?;
    let mut out = Vec::new();
    for e in events {
        if e.kind == "node_final" {
            if let Some((id, state)) = e.summary.split_once(": ") {
                out.push(NodeOutcome {
                    id: id.to_string(),
                    state: state.to_string(),
                    proven_gates: vec![],
                    reason: String::new(),
                });
            }
        }
    }
    Ok(out)
}
