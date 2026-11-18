//! SuperAgent recovery / reconciliation core (P2-7 / #84).
//!
//! Deterministic, model-free recovery logic over the proven storage (#80),
//! orchestrator (#81) and contracts (#78) crates:
//! - exclusive resume: lease + fixed-identity digest match before any reuse;
//! - digest-gated reuse: a prior Succeeded node is reused only when its artifact
//!   digest matches and every dependency is itself reused (transitive invalidation);
//! - uncertain reconciliation: an unconfirmed intent is never auto-replayed;
//!   non-idempotent nodes are never blindly retried;
//! - four-breakpoint fault injection leaves recoverable (not silently lost) state.
//!
//! No process spawning and no model calls anywhere in this crate. The external
//! confirmation input (thread/read etc.) is supplied by the adapter layer; this
//! crate only decides what to do with a given confirmation.

use std::collections::{BTreeMap, BTreeSet};

use superagent_contracts::Workflow;
use superagent_orchestrator::{topo_order, NodeState, PlanError};
use superagent_storage::{RunSpec, Store, StoreError};

#[derive(Debug)]
pub enum RecoveryError {
    Store(StoreError),
    Plan(PlanError),
    IdentityMismatch { field: String },
    UnknownNode(String),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoveryError::Store(e) => write!(f, "store: {e}"),
            RecoveryError::Plan(e) => write!(f, "plan: {}", e.0),
            RecoveryError::IdentityMismatch { field } => {
                write!(f, "resume identity mismatch on '{field}'")
            }
            RecoveryError::UnknownNode(id) => write!(f, "unknown node '{id}'"),
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<StoreError> for RecoveryError {
    fn from(e: StoreError) -> Self {
        RecoveryError::Store(e)
    }
}

impl From<PlanError> for RecoveryError {
    fn from(e: PlanError) -> Self {
        RecoveryError::Plan(e)
    }
}

pub type Result<T> = std::result::Result<T, RecoveryError>;

/// Fixed run identity that must match exactly before an exclusive resume.
///
/// Field mapping to #84's "包/输入/工作目录/artifact 摘要核对":
/// - bundle_digest = 能力包摘要; input_summary = 输入摘要;
/// - repo_identity + start_head = 工作目录身份（仓库标识 + 初始 HEAD）;
/// - config_summary = 配置摘要; workflow_digest = 工作流摘要.
///
/// Node-level artifact 摘要核对 is performed separately by [`resume_plan`] via
/// `current_artifacts` digests (per-node reuse gate), not at the run level here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunIdentity {
    pub workflow_digest: String,
    pub bundle_digest: String,
    pub input_summary: String,
    pub repo_identity: String,
    pub start_head: String,
    pub config_summary: String,
}

impl RunIdentity {
    pub fn from_spec(spec: &RunSpec) -> Self {
        Self {
            workflow_digest: spec.workflow_digest.clone(),
            bundle_digest: spec.bundle_digest.clone(),
            input_summary: spec.input_summary.clone(),
            repo_identity: spec.repo_identity.clone(),
            start_head: spec.start_head.clone(),
            config_summary: spec.config_summary.clone(),
        }
    }
}

/// Acquire an exclusive lease and verify the persisted run identity matches the
/// expected identity field-by-field. On mismatch the lease is released and the
/// resume is refused (never reuse state from a different run identity).
pub fn exclusive_resume(
    store: &Store,
    run_id: &str,
    owner: &str,
    now: i64,
    ttl_secs: i64,
    expected: &RunIdentity,
) -> Result<()> {
    store.acquire_lease(run_id, owner, now, ttl_secs)?;
    let (spec, _state) = match store.get_run(run_id) {
        Ok(v) => v,
        Err(e) => {
            let _ = store.release_lease(run_id, owner);
            return Err(e.into());
        }
    };
    let actual = RunIdentity::from_spec(&spec);
    for (field, a, b) in [
        (
            "workflow_digest",
            &actual.workflow_digest,
            &expected.workflow_digest,
        ),
        (
            "bundle_digest",
            &actual.bundle_digest,
            &expected.bundle_digest,
        ),
        (
            "input_summary",
            &actual.input_summary,
            &expected.input_summary,
        ),
        (
            "repo_identity",
            &actual.repo_identity,
            &expected.repo_identity,
        ),
        ("start_head", &actual.start_head, &expected.start_head),
        (
            "config_summary",
            &actual.config_summary,
            &expected.config_summary,
        ),
    ] {
        if a != b {
            let _ = store.release_lease(run_id, owner);
            return Err(RecoveryError::IdentityMismatch {
                field: field.into(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    /// Prior success reused (artifact digest matched, dependencies unchanged).
    Reuse,
    /// Must run again.
    Rerun,
    /// Cannot prove prior execution outcome; wait for reconciliation, no replay.
    Wait,
}

#[derive(Debug, Clone)]
pub struct PriorNode {
    pub state: NodeState,
    pub artifact_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResumeContext {
    pub prior: BTreeMap<String, PriorNode>,
    /// Current artifact digests observed in the workspace (node -> digest).
    pub current_artifacts: BTreeMap<String, String>,
    pub non_idempotent: BTreeSet<String>,
}

/// Compute a per-node resume plan in topological order. A prior Succeeded node
/// is reused only if its artifact digest still matches AND every dependency is
/// reused; any changed dependency invalidates downstream transitively. Uncertain
/// prior nodes yield Wait (never auto-replay). Missing prior state yields Rerun.
pub fn resume_plan(wf: &Workflow, ctx: &ResumeContext) -> Result<BTreeMap<String, ResumeDecision>> {
    let order = topo_order(wf)?;
    let mut decisions: BTreeMap<String, ResumeDecision> = BTreeMap::new();
    for id in &order {
        let node = wf
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .ok_or_else(|| RecoveryError::UnknownNode(id.clone()))?;
        let dep_changed = node
            .depends_on
            .iter()
            .any(|d| decisions.get(d) != Some(&ResumeDecision::Reuse));
        let decision = if dep_changed {
            ResumeDecision::Rerun
        } else {
            match ctx.prior.get(id) {
                Some(p) if p.state == NodeState::Succeeded => {
                    let matches = match (&p.artifact_digest, ctx.current_artifacts.get(id)) {
                        (Some(old), Some(cur)) => old == cur,
                        _ => false,
                    };
                    if matches {
                        ResumeDecision::Reuse
                    } else {
                        ResumeDecision::Rerun
                    }
                }
                Some(p) if p.state == NodeState::Uncertain => ResumeDecision::Wait,
                _ => ResumeDecision::Rerun,
            }
        };
        decisions.insert(id.clone(), decision);
    }
    Ok(decisions)
}

/// External confirmation of an unconfirmed intent, supplied by the adapter via a
/// public read interface (e.g. thread/read). This crate never fetches it itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalConfirmation {
    ConfirmedExecuted,
    ConfirmedNotExecuted,
    Unproven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileAction {
    /// Bind the already-executed intent as succeeded; do not replay.
    BindSucceeded,
    /// Proven not executed: safe to retry.
    Retry,
    /// Cannot prove outcome: wait for reconciliation, never auto-replay.
    Wait,
}

/// Reconcile an unconfirmed intent. An unproven outcome always waits (no blind
/// replay); a non-idempotent node is never blindly retried even on retryable
/// failure — only a positive confirmation of non-execution permits a retry.
pub fn reconcile_unconfirmed(non_idempotent: bool, conf: ExternalConfirmation) -> ReconcileAction {
    match conf {
        ExternalConfirmation::ConfirmedExecuted => ReconcileAction::BindSucceeded,
        ExternalConfirmation::ConfirmedNotExecuted => {
            if non_idempotent {
                ReconcileAction::Wait
            } else {
                ReconcileAction::Retry
            }
        }
        ExternalConfirmation::Unproven => ReconcileAction::Wait,
    }
}

/// The four recovery fault breakpoints (#84). These map onto the storage fault
/// hooks (#80) plus the orchestrator gate-commit boundary; injecting at any of
/// them must leave a recoverable state, never a silently lost terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryBreakpoint {
    BeforeSend,
    AfterSendBeforeResponse,
    AfterResponseBeforeBind,
    AfterGateBeforeCommit,
}

impl RecoveryBreakpoint {
    /// The storage breakpoint a recovery breakpoint injects through, if any.
    pub fn storage_hook(self) -> Option<superagent_storage::Breakpoint> {
        match self {
            RecoveryBreakpoint::BeforeSend => None,
            RecoveryBreakpoint::AfterSendBeforeResponse => {
                Some(superagent_storage::Breakpoint::AfterIntentPersist)
            }
            RecoveryBreakpoint::AfterResponseBeforeBind => {
                Some(superagent_storage::Breakpoint::BeforeExternalBind)
            }
            RecoveryBreakpoint::AfterGateBeforeCommit => {
                Some(superagent_storage::Breakpoint::BeforeEventAppend)
            }
        }
    }
}

/// Outcome of driving the send sequence with a fault injected at a breakpoint.
#[derive(Debug)]
pub enum SendFaultOutcome {
    /// BeforeSend: aborted before any durable intent was written (clean state).
    AbortedBeforeSend,
    /// A storage boundary fault aborted the send; state is recoverable.
    FaultedRecoverable(StoreError),
    /// No fault fired; intent sent and bound.
    Sent(i64),
}

/// Drive `send_intent` with a fault injected at the given recovery breakpoint.
/// BeforeSend short-circuits before persisting (nothing durable is written); the
/// other breakpoints inject through the mapped storage hook and always clear the
/// hook afterwards so the store remains usable for recovery.
pub fn drive_send_with_fault(
    store: &Store,
    run_id: &str,
    node_id: &str,
    attempt_no: u32,
    intent_json: &str,
    now: i64,
    bp: RecoveryBreakpoint,
) -> SendFaultOutcome {
    let Some(hook_bp) = bp.storage_hook() else {
        return SendFaultOutcome::AbortedBeforeSend;
    };
    store.set_fault_hook(move |b| {
        if b == hook_bp {
            Err(StoreError::FaultInjected(b))
        } else {
            Ok(())
        }
    });
    let r = store.send_intent(
        run_id,
        node_id,
        attempt_no,
        intent_json,
        now,
        Some("t"),
        Some("turn"),
        None,
    );
    store.clear_fault_hook();
    match r {
        Ok(id) => SendFaultOutcome::Sent(id),
        Err(e) => SendFaultOutcome::FaultedRecoverable(e),
    }
}

/// Drive a terminal-event append with a fault at AfterGateBeforeCommit. Returns
/// the append result; the hook is cleared afterwards so recovery can re-append.
pub fn drive_gate_commit_with_fault(
    store: &Store,
    run_id: &str,
    kind: &str,
    summary: &str,
    now: i64,
) -> std::result::Result<i64, StoreError> {
    store.set_fault_hook(|b| {
        if b == superagent_storage::Breakpoint::BeforeEventAppend {
            Err(StoreError::FaultInjected(b))
        } else {
            Ok(())
        }
    });
    let r = store.append_event(run_id, kind, summary, now);
    store.clear_fault_hook();
    r
}
