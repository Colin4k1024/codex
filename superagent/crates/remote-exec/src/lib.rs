//! SuperAgent remote tool execution (P5-1 / #95).
//!
//! Distributed-runtime step one: a task's tool execution is dispatched to a
//! registered REMOTE environment through an authenticated exec-server
//! connection, with environment bootstrap, capability discovery (FS / process /
//! sandbox) wired into the #81 orchestrator, and a hard permission boundary.
//! The agent itself stays local (cross-machine placement is #96).
//!
//! Integration, not rebuild: `RemoteExecExecutor` implements the orchestrator's
//! `NodeExecutor`, so a normal `Orchestrator::run` drives a node's tool execution
//! through capability discovery -> permission boundary -> remote dispatch ->
//! evidence-bearing gate outcomes. A refused capability/sandbox yields
//! `ExecKind::Blocked` (never retried, never fallen back to a host shell).
//!
//! Model-free: pure crypto (hmac/sha2) + policy computation over an in-memory
//! fixture exec-server backend. The fixture stands in for a live remote
//! exec-server process; the integration path (auth -> bootstrap -> discovery ->
//! permission -> dispatch -> evidence -> orchestrator state) is real and
//! deterministic, but no live cross-machine exec-server is contacted (honest
//! boundary, see evidence conclusion; mirrors the #94 fixture-backend pattern).

use std::collections::{BTreeMap, BTreeSet};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use superagent_contracts::{Node, NodeKind};
use superagent_orchestrator::{ExecKind, ExecOutcome, GateOutcome, NodeExecutor};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    FileSystem,
    Process,
    Sandbox,
}

/// Sandbox strength ordering: ReadOnly < WorkspaceWrite < FullAccess. A tool
/// requiring a stronger mode than the environment grants is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

#[derive(Debug, Clone)]
pub struct Environment {
    pub id: String,
    pub capabilities: BTreeSet<CapabilityKind>,
    pub sandbox: SandboxMode,
    pub bootstrapped: bool,
}

/// A tool to execute remotely: the capability it needs, its argv, and the
/// minimum sandbox mode the environment must grant.
#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub tool_id: String,
    pub required_capability: CapabilityKind,
    pub argv: Vec<String>,
    pub min_sandbox: SandboxMode,
}

/// An authenticated connection bound to one environment.
#[derive(Debug, Clone)]
pub struct Connection {
    pub env_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub exit_code: i32,
    pub evidence_ref: String,
    pub output: String,
}

#[derive(Debug)]
pub enum RemoteError {
    Auth,
    Expired,
    UnknownEnvironment(String),
    NotBootstrapped(String),
    CapabilityDenied(String),
    SandboxDenied(String),
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteError::Auth => write!(f, "invalid connection token"),
            RemoteError::Expired => write!(f, "connection token expired"),
            RemoteError::UnknownEnvironment(s) => write!(f, "unknown environment: {s}"),
            RemoteError::NotBootstrapped(s) => write!(f, "environment not bootstrapped: {s}"),
            RemoteError::CapabilityDenied(s) => write!(f, "capability denied: {s}"),
            RemoteError::SandboxDenied(s) => write!(f, "sandbox boundary denied: {s}"),
        }
    }
}

impl std::error::Error for RemoteError {}

pub type Result<T> = std::result::Result<T, RemoteError>;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sign(secret: &str, msg: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(msg.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// Fixture exec-server backend: environment registry/bootstrap, authenticated
/// connections, capability discovery and remote tool dispatch with a permission
/// boundary. Stands in for a live remote exec-server process (honest boundary).
pub struct RemoteExecServer {
    secret: String,
    environments: BTreeMap<String, Environment>,
    audit: Vec<Value>,
}

impl RemoteExecServer {
    pub fn new(secret: &str) -> Self {
        Self {
            secret: secret.to_string(),
            environments: BTreeMap::new(),
            audit: Vec::new(),
        }
    }

    /// Register + bootstrap a remote environment with its capability inventory
    /// and sandbox mode.
    pub fn register_environment(
        &mut self,
        env_id: &str,
        capabilities: BTreeSet<CapabilityKind>,
        sandbox: SandboxMode,
    ) {
        self.environments.insert(
            env_id.into(),
            Environment {
                id: env_id.into(),
                capabilities,
                sandbox,
                bootstrapped: true,
            },
        );
        self.audit.push(json!({
            "event": "environment_bootstrap", "env": env_id, "sandbox": sandbox,
        }));
    }

    pub fn environment(&self, env_id: &str) -> Option<&Environment> {
        self.environments.get(env_id)
    }

    /// Issue an authenticated connection token bound to an environment.
    pub fn issue_token(&self, env_id: &str, now: i64, ttl: i64) -> Result<String> {
        if !self.environments.contains_key(env_id) {
            return Err(RemoteError::UnknownEnvironment(env_id.into()));
        }
        let exp = now + ttl;
        let msg = format!("{env_id}:{exp}");
        let sig = sign(&self.secret, &msg);
        Ok(format!("{msg}:{sig}"))
    }

    /// Authenticate a connection token (signature + expiry + bootstrap).
    pub fn connect(&self, token: &str, now: i64) -> Result<Connection> {
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 3 {
            return Err(RemoteError::Auth);
        }
        let (env_id, exp_str, sig) = (parts[0], parts[1], parts[2]);
        let exp: i64 = exp_str.parse().map_err(|_| RemoteError::Auth)?;
        let expected = sign(&self.secret, &format!("{env_id}:{exp}"));
        if expected != sig {
            return Err(RemoteError::Auth);
        }
        if exp <= now {
            return Err(RemoteError::Expired);
        }
        let env = self
            .environments
            .get(env_id)
            .ok_or_else(|| RemoteError::UnknownEnvironment(env_id.into()))?;
        if !env.bootstrapped {
            return Err(RemoteError::NotBootstrapped(env_id.into()));
        }
        Ok(Connection {
            env_id: env_id.into(),
        })
    }

    /// Capability discovery: the remote environment's FS/process/sandbox
    /// inventory, consumed by the orchestrator before dispatch.
    pub fn capability_discovery(&self, conn: &Connection) -> Result<BTreeSet<CapabilityKind>> {
        let env = self
            .environments
            .get(&conn.env_id)
            .ok_or_else(|| RemoteError::UnknownEnvironment(conn.env_id.clone()))?;
        if !env.bootstrapped {
            return Err(RemoteError::NotBootstrapped(conn.env_id.clone()));
        }
        Ok(env.capabilities.clone())
    }

    /// Dispatch a tool to the remote environment. Enforces the permission
    /// boundary: the required capability must be present AND the environment
    /// sandbox must be at least the tool's minimum. Refusal is explicit (no
    /// host-shell fallback). The fixture exec is deterministic (exit 0 +
    /// evidence reference).
    pub fn exec_tool(&mut self, conn: &Connection, req: &ToolRequest) -> Result<ExecResult> {
        let env = self
            .environments
            .get(&conn.env_id)
            .ok_or_else(|| RemoteError::UnknownEnvironment(conn.env_id.clone()))?;
        if !env.bootstrapped {
            return Err(RemoteError::NotBootstrapped(conn.env_id.clone()));
        }
        if !env.capabilities.contains(&req.required_capability) {
            return Err(RemoteError::CapabilityDenied(format!(
                "{} needs {:?}",
                req.tool_id, req.required_capability
            )));
        }
        if env.sandbox < req.min_sandbox {
            return Err(RemoteError::SandboxDenied(format!(
                "{} needs {:?}, environment grants {:?}",
                req.tool_id, req.min_sandbox, env.sandbox
            )));
        }
        let evidence_ref = format!("ev:{}:{}", conn.env_id, req.tool_id);
        self.audit.push(json!({
            "event": "remote_exec", "env": conn.env_id, "tool": req.tool_id,
            "argv": req.argv, "capability": req.required_capability,
        }));
        Ok(ExecResult {
            exit_code: 0,
            evidence_ref,
            output: format!("executed {} on {}", req.tool_id, conn.env_id),
        })
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }
}

fn node_gates(node: &Node) -> Vec<String> {
    match &node.kind {
        NodeKind::Verification { gates, .. } => gates.clone(),
        NodeKind::Loop { gate, .. } => vec![gate.clone()],
        NodeKind::Capability { .. } => vec![],
    }
}

/// Orchestrator integration: drives a node's tool execution through the remote
/// environment. Implements `NodeExecutor` so a standard `Orchestrator::run`
/// dispatches remotely with capability discovery + permission boundary.
pub struct RemoteExecExecutor {
    server: RemoteExecServer,
    conn: Connection,
    tools: BTreeMap<String, ToolRequest>,
}

impl RemoteExecExecutor {
    pub fn new(server: RemoteExecServer, conn: Connection) -> Self {
        Self {
            server,
            conn,
            tools: BTreeMap::new(),
        }
    }

    pub fn bind_tool(&mut self, node_id: &str, req: ToolRequest) {
        self.tools.insert(node_id.into(), req);
    }

    pub fn server(&self) -> &RemoteExecServer {
        &self.server
    }

    pub fn server_mut(&mut self) -> &mut RemoteExecServer {
        &mut self.server
    }

    fn blocked(&self, detail: String) -> ExecOutcome {
        ExecOutcome {
            kind: ExecKind::Blocked,
            tokens_used: 0,
            elapsed_secs: 0,
            gates: vec![],
            artifacts: vec![],
            detail,
        }
    }
}

impl NodeExecutor for RemoteExecExecutor {
    fn execute(&mut self, node: &Node, _attempt: u32) -> ExecOutcome {
        let req = match self.tools.get(&node.id) {
            Some(r) => r.clone(),
            None => {
                return ExecOutcome {
                    kind: ExecKind::FatalFailure,
                    tokens_used: 0,
                    elapsed_secs: 0,
                    gates: vec![],
                    artifacts: vec![],
                    detail: format!("no remote tool bound to node {}", node.id),
                }
            }
        };

        // Capability discovery feeds the dispatch decision.
        let caps = match self.server.capability_discovery(&self.conn) {
            Ok(c) => c,
            Err(e) => return self.blocked(e.to_string()),
        };
        if !caps.contains(&req.required_capability) {
            return self.blocked(format!("capability denied: {:?}", req.required_capability));
        }

        // Remote dispatch with the permission boundary enforced server-side.
        let res = match self.server.exec_tool(&self.conn, &req) {
            Ok(r) => r,
            Err(e) => return self.blocked(e.to_string()),
        };

        let gates: Vec<GateOutcome> = node_gates(node)
            .into_iter()
            .map(|g| GateOutcome {
                gate_id: g,
                passed: res.exit_code == 0,
                evidence_ref: Some(res.evidence_ref.clone()),
                exit_code: Some(res.exit_code),
            })
            .collect();

        let mut out = ExecOutcome::completed(gates, 1, 1);
        // Capability nodes require artifact evidence (no NL success).
        if matches!(node.kind, NodeKind::Capability { .. }) {
            out.artifacts = vec![res.evidence_ref.clone()];
        }
        out.detail = res.output;
        out
    }
}
