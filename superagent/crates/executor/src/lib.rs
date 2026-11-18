//! SuperAgent node executors (P2-5 / #82).
//!
//! Two executor contracts over the #79 stdio [`Adapter`], wired into the #81
//! [`NodeExecutor`] trait:
//!
//! - [`AgentExecutor`] runs ONE model turn for a capability node. A completed
//!   turn means EXECUTION ENDED, not success: the orchestrator only marks the
//!   node succeeded when durable artifact/gate evidence is present. The turn
//!   result (threadId / turnId / status / usage / artifact refs) is written as a
//!   digest-backed evidence artifact.
//! - [`VerificationExecutor`] runs a verification node's gates as REAL
//!   processes. Command gates run through the engine `command/exec` RPC (engine
//!   sandbox enforced) or, only under an explicit `LocalProcess` authorization,
//!   as a direct child via [`program`]. The ACTUAL exit code is captured — a
//!   model's self-reported exit code is never consulted. ArtifactSchema gates
//!   validate the artifact against its registered JSON Schema; Approval gates
//!   return `NeedsInput` (never auto-approved).
//!
//! SECURITY INVARIANT: when the requested sandbox policy cannot be expressed on
//! the execution path, the executor returns [`ExecKind::Blocked`]. It NEVER
//! falls back to a host shell, `dangerFullAccess`, or `externalSandbox`.
//!
//! [`SwarmExecutor`] dispatches a node to the right executor by its kind. Tests
//! are model-free fault injection against the #79 fake stdio engine; a real
//! runtime probe (`superagent/tests/p2_5.py`) exercises a live model node that
//! modifies a fixture plus a real-exit-code gate.

pub mod agent;
pub mod evidence;
pub mod permissions;
pub mod program;
pub mod verification;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use superagent_adapter::{Adapter, AdapterConfig, AdapterError};
use superagent_contracts::{Node, NodeKind};
use superagent_orchestrator::{ExecKind, ExecOutcome, GateOutcome, NodeExecutor};

pub use agent::AgentExecutor;
pub use evidence::{sha256_hex, EvidenceRecorder};
pub use permissions::{Blocked, PermissionsConfig, SandboxRequest};
pub use program::{log_program_run, run_program, ProgramRun, ProgramSpec};
pub use superagent_adapter::EnvironmentConfig;
pub use verification::VerificationExecutor;

/// How to spawn the engine app-server for an agent turn / command gate.
#[derive(Debug, Clone)]
pub struct EngineSpec {
    /// Full argv (no shell), e.g. `[engine, "app-server", "--listen", "stdio://"]`.
    pub argv: Vec<String>,
    pub codex_home: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    /// Optional remote execution environment (P5-1 / #95). `None` keeps the
    /// pre-P5-1 local behavior exactly; `Some` makes the agent path register
    /// the environment and address the turn to it, with no local fallback.
    pub environment: Option<EnvironmentConfig>,
    pub run_deadline: Duration,
    pub max_artifact_bytes: usize,
}

impl EngineSpec {
    /// Spawn an adapter with production resource limits and a per-attempt run
    /// log. The adapter NEVER auto-approves (default-deny handler).
    pub fn adapter(&self, run_log: Option<PathBuf>) -> Result<Adapter, AdapterError> {
        if let Some(p) = &run_log {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| AdapterError::InvalidConfig(format!("run log dir: {e}")))?;
            }
        }
        let config = AdapterConfig {
            argv: self.argv.clone(),
            codex_home: self.codex_home.clone(),
            cwd: self.cwd.clone(),
            environment: self.environment.clone(),
            run_deadline: self.run_deadline,
            max_artifact_bytes: self.max_artifact_bytes,
            run_log_path: run_log,
            ..AdapterConfig::default()
        };
        Adapter::spawn_default(config)
    }
}

/// A terminal refusal: policy inexpressible, no host fallback (#82).
pub fn blocked(detail: impl Into<String>) -> ExecOutcome {
    ExecOutcome {
        kind: ExecKind::Blocked,
        tokens_used: 0,
        elapsed_secs: 0,
        gates: vec![],
        artifacts: vec![],
        detail: detail.into(),
    }
}

pub fn fatal(detail: impl Into<String>) -> ExecOutcome {
    ExecOutcome {
        kind: ExecKind::FatalFailure,
        tokens_used: 0,
        elapsed_secs: 0,
        gates: vec![],
        artifacts: vec![],
        detail: detail.into(),
    }
}

pub fn uncertain(detail: impl Into<String>) -> ExecOutcome {
    ExecOutcome {
        kind: ExecKind::Uncertain,
        tokens_used: 0,
        elapsed_secs: 0,
        gates: vec![],
        artifacts: vec![],
        detail: detail.into(),
    }
}

/// An evidence-backed gate pass/fail. `passed` alone is never proof — the
/// orchestrator's [`GateOutcome::is_proven`] also requires `evidence_ref`.
pub fn gate(
    gate_id: &str,
    passed: bool,
    evidence_ref: Option<String>,
    exit_code: Option<i32>,
) -> GateOutcome {
    GateOutcome {
        gate_id: gate_id.to_string(),
        passed,
        evidence_ref,
        exit_code,
    }
}

/// Extract the engine-reported total token usage for a thread from a
/// `thread/read` result (`thread.tokenUsage.total.totalTokens`) or a turn's
/// `usage.totalTokens`. Never model-supplied; defaults to 0 when absent.
pub fn usage_from_thread_read(read: &Value) -> u64 {
    let thread = read.get("thread").unwrap_or(read);
    if let Some(n) = thread
        .get("tokenUsage")
        .and_then(|u| u.get("total"))
        .and_then(|t| t.get("totalTokens"))
        .and_then(Value::as_u64)
    {
        return n;
    }
    if let Some(turns) = thread.get("turns").and_then(Value::as_array) {
        let mut sum = 0u64;
        for t in turns {
            if let Some(n) = t
                .get("usage")
                .and_then(|u| u.get("totalTokens"))
                .and_then(Value::as_u64)
            {
                sum += n;
            }
        }
        if sum > 0 {
            return sum;
        }
    }
    0
}

/// Dispatches each node to the executor for its kind. Capability nodes go to
/// the agent (model turn); Verification and Loop nodes go to the gate runner.
pub struct SwarmExecutor {
    pub agent: AgentExecutor,
    pub verification: VerificationExecutor,
}

impl NodeExecutor for SwarmExecutor {
    fn execute(&mut self, node: &Node, attempt: u32) -> ExecOutcome {
        match &node.kind {
            NodeKind::Capability { .. } => self.agent.execute(node, attempt),
            NodeKind::Verification { .. } | NodeKind::Loop { .. } => {
                self.verification.execute(node, attempt)
            }
        }
    }
}
