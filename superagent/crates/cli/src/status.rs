//! `superagent status <run-id> --json`: read the durable run state, identity,
//! per-node outcomes and event log from the store. Read-only; never executes.

use serde_json::{json, Value};
use superagent_storage::Store;

use crate::report::outcomes_from_events;
use crate::Result;

pub fn status_json(store: &Store, run_id: &str) -> Result<Value> {
    let (spec, state) = store.get_run(run_id)?;
    let nodes = outcomes_from_events(store, run_id)?;
    let events = store.events_after(run_id, 0)?;
    let event_json: Vec<Value> = events
        .iter()
        .map(|e| {
            json!({
                "seq": e.seq,
                "kind": e.kind,
                "summary": e.summary,
                "createdAt": e.created_at,
            })
        })
        .collect();
    let node_json: Vec<Value> = nodes
        .iter()
        .map(|n| json!({ "id": n.id, "state": n.state }))
        .collect();
    Ok(json!({
        "runId": run_id,
        "state": state,
        "identity": {
            "workflowDigest": spec.workflow_digest,
            "bundleDigest": spec.bundle_digest,
            "inputSummary": spec.input_summary,
            "repoIdentity": spec.repo_identity,
            "startHead": spec.start_head,
            "configSummary": spec.config_summary,
        },
        "nodes": node_json,
        "events": event_json,
    }))
}
