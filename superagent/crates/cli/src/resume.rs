//! `superagent resume`: exclusive lease + fixed-identity match before any reuse
//! (recovery #84), then a per-node Reuse/Rerun/Wait plan. An unproven prior node
//! is never auto-replayed.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use superagent_contracts::Workflow;
use superagent_recovery::{exclusive_resume, resume_plan, ResumeContext, RunIdentity};
use superagent_storage::Store;

use crate::Result;

/// Exclusive-lease parameters for a resume attempt.
pub struct ResumeLease<'a> {
    pub owner: &'a str,
    pub now: i64,
    pub ttl_secs: i64,
}

/// Acquire the exclusive lease, verify the run identity field-by-field, and
/// compute the resume plan. On identity mismatch the lease is released and the
/// resume refused (recovery core guarantees this).
pub fn resume_run(
    store: &Store,
    run_id: &str,
    lease: &ResumeLease<'_>,
    expected: &RunIdentity,
    wf: &Workflow,
    ctx: &ResumeContext,
) -> Result<Value> {
    exclusive_resume(
        store,
        run_id,
        lease.owner,
        lease.now,
        lease.ttl_secs,
        expected,
    )?;
    let decisions = resume_plan(wf, ctx)?;
    let decision_json: BTreeMap<String, String> = decisions
        .iter()
        .map(|(k, v)| (k.clone(), format!("{v:?}")))
        .collect();
    Ok(json!({
        "runId": run_id,
        "resumed": true,
        "identityMatched": true,
        "leaseOwner": lease.owner,
        "decisions": decision_json,
    }))
}
