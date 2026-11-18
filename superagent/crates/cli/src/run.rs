//! `superagent run`: drive the deterministic orchestrator over the real
//! executor (agent turns + real-exit-code gates) and persist to the store.
//! Non-interactive runs that need an authorization/policy return `blocked`
//! with a stable exit code; an unproven outcome returns `uncertain`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::Value;
use superagent_contracts::{DryRunContext, Workflow};
use superagent_executor::{
    AgentExecutor, EngineSpec, EvidenceRecorder, PermissionsConfig, SwarmExecutor,
    VerificationExecutor,
};
use superagent_orchestrator::{NodeState, Orchestrator, Policy, RunReport};
use superagent_storage::{RunSpec, Store};

use crate::cancel::CancelGuard;
use crate::{CancelToken, CliError, Result};

pub struct RunOptions {
    pub workflow: Workflow,
    pub ctx: DryRunContext,
    pub policy: Policy,
    pub engine: EngineSpec,
    pub workspace: PathBuf,
    pub evidence_dir: PathBuf,
    pub run_id: String,
    pub spec: RunSpec,
    pub now: i64,
    pub schema_registry: BTreeMap<String, Value>,
    pub gate_timeout: Duration,
    pub output_cap_bytes: usize,
    pub task_override: Option<String>,
    pub cancel: CancelToken,
}

/// Execute the workflow serially. Success is decided only by gate evidence.
/// A cancelled token, a node needing authorization (Blocked/WaitingInput) in
/// non-interactive mode, or an unproven outcome maps to a stable error/exit.
pub fn execute_run(store: Rc<Store>, opts: RunOptions) -> Result<RunReport> {
    if opts.cancel.is_cancelled() {
        return Err(CliError::Cancelled("run cancelled before start".into()));
    }
    let rec = EvidenceRecorder::new(
        store.clone(),
        opts.run_id.clone(),
        opts.evidence_dir.clone(),
    );
    let perms = PermissionsConfig::workspace_write(vec![opts.workspace.clone()], false);
    let agent = AgentExecutor {
        engine: opts.engine.clone(),
        permissions: perms.clone(),
        workspace: opts.workspace.clone(),
        evidence: rec.clone(),
        task_override: opts.task_override.clone(),
    };
    let verification = VerificationExecutor {
        wf: opts.workflow.clone(),
        engine: opts.engine.clone(),
        permissions: perms,
        workspace: opts.workspace.clone(),
        evidence: rec,
        schema_registry: opts.schema_registry.clone(),
        gate_timeout: opts.gate_timeout,
        output_cap_bytes: opts.output_cap_bytes,
    };
    let swarm = SwarmExecutor {
        agent,
        verification,
    };
    let mut guard = CancelGuard {
        inner: swarm,
        token: opts.cancel.clone(),
    };
    let orch = Orchestrator::new(opts.workflow.clone(), opts.ctx.clone(), opts.policy.clone());
    let report = orch.run(&mut guard, &store, &opts.run_id, &opts.spec, opts.now)?;

    if opts.cancel.is_cancelled() {
        return Err(CliError::Cancelled(
            "run cancelled; cleanup required".into(),
        ));
    }
    // Non-interactive: a node that needs an authorization or whose policy is
    // inexpressible is a terminal refusal, not a silent success.
    if let Some((id, nr)) = report
        .nodes
        .iter()
        .find(|(_, nr)| matches!(nr.state, NodeState::Blocked | NodeState::WaitingInput))
    {
        return Err(CliError::Blocked(format!(
            "node '{id}' needs authorization/input ({:?}): {}",
            nr.state, nr.reason
        )));
    }
    if report.state == NodeState::Uncertain {
        return Err(CliError::Uncertain("run outcome unproven".into()));
    }
    Ok(report)
}

/// Stable exit code for a completed (non-error) run report.
pub fn exit_code_for_report(report: &RunReport) -> i32 {
    match report.state {
        NodeState::Succeeded => crate::exit::OK,
        NodeState::Uncertain => crate::exit::UNCERTAIN,
        NodeState::Cancelling => crate::exit::CANCELLED,
        _ => crate::exit::FAILED,
    }
}
