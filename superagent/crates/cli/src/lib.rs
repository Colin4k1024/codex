//! SuperAgent product CLI (P2-6 / #83).
//!
//! The deterministic outer loop over the proven crates: contracts (#78),
//! storage (#80), orchestrator (#81), executor (#82), adapter (#79) and
//! recovery (#84). Success is decided only by gate evidence, never by a model
//! self-report. `dry-run` starts no model and no command; `cancel` stops new
//! scheduling, interrupts/terminates the active work, drains terminals, closes
//! the engine within a bound and marks the run cancelled with cleanup evidence;
//! `report` separates verified from unverified items and pre-existing from
//! this-run changes, recording version/usage/evidence. Non-interactive runs that
//! need an authorization return `blocked` with a stable exit code.

pub mod cancel;
pub mod plan;
pub mod report;
pub mod resume;
pub mod run;
pub mod status;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Stable process exit codes (contract for scripts / CI).
pub mod exit {
    pub const OK: i32 = 0;
    pub const FAILED: i32 = 1;
    pub const USAGE: i32 = 2;
    /// A required authorization/policy is inexpressible in non-interactive mode.
    pub const BLOCKED: i32 = 3;
    /// The run was cancelled with confirmed cleanup.
    pub const CANCELLED: i32 = 4;
    /// Outcome unproven; reconciliation required (never auto-replayed).
    pub const UNCERTAIN: i32 = 5;
}

#[derive(Debug)]
pub enum CliError {
    Usage(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Store(superagent_storage::StoreError),
    Plan(superagent_orchestrator::PlanError),
    Recovery(superagent_recovery::RecoveryError),
    Adapter(superagent_adapter::AdapterError),
    Blocked(String),
    Cancelled(String),
    Uncertain(String),
    Failed(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Usage(m) => write!(f, "usage: {m}"),
            CliError::Io(e) => write!(f, "io: {e}"),
            CliError::Json(e) => write!(f, "json: {e}"),
            CliError::Store(e) => write!(f, "store: {e}"),
            CliError::Plan(e) => write!(f, "plan: {}", e.0),
            CliError::Recovery(e) => write!(f, "recovery: {e}"),
            CliError::Adapter(e) => write!(f, "adapter: {e}"),
            CliError::Blocked(m) => write!(f, "blocked: {m}"),
            CliError::Cancelled(m) => write!(f, "cancelled: {m}"),
            CliError::Uncertain(m) => write!(f, "uncertain: {m}"),
            CliError::Failed(m) => write!(f, "failed: {m}"),
        }
    }
}

impl std::error::Error for CliError {}

impl CliError {
    /// Map an error to its stable exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage(_) => exit::USAGE,
            CliError::Blocked(_) => exit::BLOCKED,
            CliError::Cancelled(_) => exit::CANCELLED,
            CliError::Uncertain(_) => exit::UNCERTAIN,
            _ => exit::FAILED,
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}
impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::Json(e)
    }
}
impl From<superagent_storage::StoreError> for CliError {
    fn from(e: superagent_storage::StoreError) -> Self {
        CliError::Store(e)
    }
}
impl From<superagent_orchestrator::PlanError> for CliError {
    fn from(e: superagent_orchestrator::PlanError) -> Self {
        CliError::Plan(e)
    }
}
impl From<superagent_recovery::RecoveryError> for CliError {
    fn from(e: superagent_recovery::RecoveryError) -> Self {
        CliError::Recovery(e)
    }
}
impl From<superagent_adapter::AdapterError> for CliError {
    fn from(e: superagent_adapter::AdapterError) -> Self {
        CliError::Adapter(e)
    }
}

pub type Result<T> = std::result::Result<T, CliError>;

/// A cooperative cancellation signal. Once set, no new node is scheduled; the
/// active work is interrupted/terminated and the engine closed within a bound.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Product version recorded in every report (engine + pack identity).
pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
