//! `superagent resume`: exclusive lease + fixed-identity match before any reuse
//! (recovery #84), then a per-node Reuse/Rerun/Wait plan. An unproven prior node
//! is never auto-replayed.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use superagent_contracts::Workflow;
use superagent_executor::sha256_hex;
use superagent_recovery::{exclusive_resume, resume_plan, ResumeContext, RunIdentity};
use superagent_storage::Store;

use crate::Result;

/// Exclusive-lease parameters for a resume attempt.
pub struct ResumeLease<'a> {
    pub owner: &'a str,
    pub now: i64,
    pub ttl_secs: i64,
}

/// Build the EXPECTED run identity from the caller's CURRENT resume inputs —
/// never from the stored run spec. This is what makes the #84 identity guard
/// meaningful on the CLI path: a changed workflow / bundle / input / repo / head
/// / config produces a different digest and is refused by `exclusive_resume`.
/// Mirrors the identity construction used by `run` so a resume of the same
/// inputs matches the original run.
pub fn expected_identity(
    wf: &Workflow,
    task: Option<&str>,
    bundle_digest: &str,
    repo_identity: &str,
    start_head: &str,
    config_summary: &str,
) -> Result<RunIdentity> {
    let workflow_digest = sha256_hex(&serde_json::to_vec(wf)?);
    let input_summary = sha256_hex(task.unwrap_or_default().as_bytes());
    Ok(RunIdentity {
        workflow_digest,
        bundle_digest: bundle_digest.to_string(),
        input_summary,
        repo_identity: repo_identity.to_string(),
        start_head: start_head.to_string(),
        config_summary: config_summary.to_string(),
    })
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
