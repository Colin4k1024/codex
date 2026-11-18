//! Permissions translation (P2-5 / #82).
//!
//! A [`PermissionsConfig`] is the caller's sandbox intent. It must be translated
//! into something the execution path can ACTUALLY enforce:
//!
//! - Agent turns run inside the engine; the intent becomes an explicit
//!   `sandboxPolicy` on `turn/start` (P0-7 / #70 proven enforcement).
//! - Verification gate programs run as direct child processes
//!   (`std::process::Command`, no shell). A direct child can only enforce cwd
//!   confinement, timeout kill and output caps — it CANNOT enforce an OS
//!   sandbox. Only an explicit [`SandboxRequest::LocalProcess`] authorization
//!   is expressible there; anything demanding kernel enforcement returns
//!   [`Blocked`].
//!
//! SECURITY INVARIANT: when the intent cannot be expressed, the executor
//! returns BLOCKED. It never falls back to a host shell, `dangerFullAccess`,
//! or `externalSandbox`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Explicit refusal: the requested sandbox policy cannot be expressed on this
/// execution path. Carries the reason; never silently downgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked(pub String);

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BLOCKED: {}", self.0)
    }
}

/// The caller's sandbox intent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SandboxRequest {
    /// Engine-enforced workspace-write sandbox. Every writable root must be
    /// absolute (relative roots are inexpressible — P0-7 case 7 parity).
    WorkspaceWrite {
        writable_roots: Vec<PathBuf>,
        #[serde(default)]
        network_access: bool,
    },
    /// Engine-enforced read-only sandbox.
    ReadOnly {
        #[serde(default)]
        network_access: bool,
    },
    /// Explicit caller authorization for a TRUSTED local gate program with no
    /// OS sandbox. Constraints actually applied by the gate runner: cwd
    /// confinement, timeout kill, bounded output. This is a deliberate
    /// configuration, never a fallback; agent turns refuse it.
    LocalProcess,
    /// A request the executor refuses to express (external sandbox, full
    /// access, unknown/legacy shapes). Always Blocked.
    Inexpressible { reason: String },
}

/// Permissions configuration handed to an executor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionsConfig {
    pub sandbox: SandboxRequest,
}

impl PermissionsConfig {
    pub fn workspace_write(writable_roots: Vec<PathBuf>, network_access: bool) -> Self {
        PermissionsConfig {
            sandbox: SandboxRequest::WorkspaceWrite {
                writable_roots,
                network_access,
            },
        }
    }

    pub fn local_process() -> Self {
        PermissionsConfig {
            sandbox: SandboxRequest::LocalProcess,
        }
    }

    /// Translate to the engine `turn/start` `sandboxPolicy` JSON (v2
    /// camelCase contract). Inexpressible intents return [`Blocked`] —
    /// never `dangerFullAccess` / `externalSandbox` / host fallback.
    pub fn engine_sandbox_policy(&self) -> Result<Value, Blocked> {
        match &self.sandbox {
            SandboxRequest::WorkspaceWrite {
                writable_roots,
                network_access,
            } => {
                for root in writable_roots {
                    if !root.is_absolute() {
                        return Err(Blocked(format!(
                            "writable root '{}' is not absolute; sandbox policy inexpressible \
                             (no host-shell fallback)",
                            root.display()
                        )));
                    }
                }
                Ok(json!({
                    "type": "workspaceWrite",
                    "writableRoots": writable_roots,
                    "networkAccess": network_access,
                }))
            }
            SandboxRequest::ReadOnly { network_access } => Ok(json!({
                "type": "readOnly",
                "networkAccess": network_access,
            })),
            SandboxRequest::LocalProcess => Err(Blocked(
                "agent turns require an engine-enforced sandbox; LocalProcess is gate-only \
                 and never maps to dangerFullAccess (no host fallback)"
                    .into(),
            )),
            SandboxRequest::Inexpressible { reason } => Err(Blocked(format!(
                "sandbox policy inexpressible: {reason} (no externalSandbox/host fallback)"
            ))),
        }
    }

    /// Whether a DIRECT gate child process may run under this intent. Only the
    /// explicit `LocalProcess` authorization is expressible without an OS
    /// sandbox; kernel-enforcement intents return [`Blocked`].
    pub fn gate_exec_allowed(&self) -> Result<(), Blocked> {
        match &self.sandbox {
            SandboxRequest::LocalProcess => Ok(()),
            SandboxRequest::WorkspaceWrite { .. } => Err(Blocked(
                "policy requires OS sandbox enforcement (workspaceWrite); a direct gate child \
                 process cannot express it; refusing host-shell fallback"
                    .into(),
            )),
            SandboxRequest::ReadOnly { .. } => Err(Blocked(
                "policy requires OS sandbox enforcement (readOnly); a direct gate child \
                 process cannot express it; refusing host-shell fallback"
                    .into(),
            )),
            SandboxRequest::Inexpressible { reason } => Err(Blocked(format!(
                "sandbox policy inexpressible: {reason} (no host-shell fallback)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_write_maps_to_engine_camel_case_policy() {
        let p = PermissionsConfig::workspace_write(vec![PathBuf::from("/tmp/ws")], false);
        let v = p.engine_sandbox_policy().unwrap();
        assert_eq!(v["type"], "workspaceWrite");
        assert_eq!(v["writableRoots"][0], "/tmp/ws");
        assert_eq!(v["networkAccess"], false);
    }

    #[test]
    fn relative_writable_root_is_blocked() {
        let p = PermissionsConfig::workspace_write(vec![PathBuf::from("relative/dir")], false);
        let err = p.engine_sandbox_policy().unwrap_err();
        assert!(err.0.contains("not absolute"));
    }

    #[test]
    fn read_only_maps_to_engine_policy() {
        let p = PermissionsConfig {
            sandbox: SandboxRequest::ReadOnly {
                network_access: false,
            },
        };
        let v = p.engine_sandbox_policy().unwrap();
        assert_eq!(v["type"], "readOnly");
    }

    #[test]
    fn local_process_never_becomes_danger_full_access() {
        let p = PermissionsConfig::local_process();
        assert!(p.engine_sandbox_policy().is_err());
        assert!(p.gate_exec_allowed().is_ok());
    }

    #[test]
    fn kernel_enforcement_intents_block_direct_gate_exec() {
        let p = PermissionsConfig::workspace_write(vec![PathBuf::from("/tmp/ws")], false);
        let err = p.gate_exec_allowed().unwrap_err();
        assert!(err.0.contains("refusing host-shell fallback"));
        let ro = PermissionsConfig {
            sandbox: SandboxRequest::ReadOnly {
                network_access: false,
            },
        };
        assert!(ro.gate_exec_allowed().is_err());
    }

    #[test]
    fn inexpressible_requests_block_both_paths() {
        let p = PermissionsConfig {
            sandbox: SandboxRequest::Inexpressible {
                reason: "externalSandbox requested".into(),
            },
        };
        assert!(p.engine_sandbox_policy().is_err());
        assert!(p.gate_exec_allowed().is_err());
    }
}
