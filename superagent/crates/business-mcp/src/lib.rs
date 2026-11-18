//! SuperAgent business MCP server (P4-5 / #94).
//!
//! Business tools are exposed through our own MCP server; the server holds the
//! credentials and performs ALL trust decisions server-side: identity (IAM JWT
//! authenticate), parameters (schema/type validation), permission (IAM
//! resource-level authorize with a hard per-app tenant boundary) and an
//! idempotency key for writes (anti-replay: a repeated key never re-executes the
//! side effect). Read-only tools are first-class; writes require explicit
//! authorization + audit + dedupe.
//!
//! Each connected application (xiaohai / HCode / HR) gets its own tenant and a
//! channel identity bound to an IAM user, so one app's token can never call
//! another app's tools.
//!
//! BOUNDARY: an engine-side MCP *tool approval* (the execution sandbox letting a
//! tool run) is NOT business authorization. Even when a call carries
//! `tool_approved = true`, the server still enforces IAM identity/permission;
//! approval never bypasses `authorize`. The two layers do not replace each other.
//!
//! Model-free: pure policy computation over an in-memory fixture business
//! backend + IAM crypto. The fixture backend stands in for a live external
//! business system; the end-to-end gateway path (identity -> params -> permission
//! -> idempotency -> backend -> audit) is real and deterministic, but no live
//! external system is contacted (honest boundary, see evidence conclusion).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use superagent_iam::{Action, Claims, IamError, IamServer, ResourceKind, ResourceRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Number,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Write,
}

impl ToolKind {
    fn action(self) -> Action {
        match self {
            ToolKind::Read => Action::Read,
            ToolKind::Write => Action::Write,
        }
    }
}

/// A registered business tool: name, read/write kind, required parameter schema.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub kind: ToolKind,
    pub required_params: Vec<(String, ParamType)>,
}

/// A connected application: its own tenant + channel identity bound to an IAM
/// user. `app_id` is the backend namespace and the tenant boundary key.
#[derive(Debug, Clone)]
pub struct App {
    pub app_id: String,
    pub tenant: String,
    pub channel: String,
    pub external_id: String,
    pub user_id: String,
}

/// A single MCP tool invocation as it arrives at the gateway API.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub app: String,
    pub tool: String,
    pub params: Value,
    /// Required for writes; ignored for reads.
    pub idempotency_key: Option<String>,
    /// Engine-side sandbox tool approval. Deliberately does NOT grant business
    /// authorization — the server still runs IAM authorize regardless.
    pub tool_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub value: Value,
    /// True when a write was deduped by its idempotency key (no re-execution).
    pub replayed: bool,
}

#[derive(Debug)]
pub enum BusinessError {
    Iam(IamError),
    UnknownApp(String),
    UnknownTool(String),
    InvalidParams(String),
    MissingIdempotencyKey,
    CrossTenant(String),
    UnknownRecord(String),
}

impl std::fmt::Display for BusinessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusinessError::Iam(e) => write!(f, "iam: {e}"),
            BusinessError::UnknownApp(s) => write!(f, "unknown app: {s}"),
            BusinessError::UnknownTool(s) => write!(f, "unknown tool: {s}"),
            BusinessError::InvalidParams(s) => write!(f, "invalid params: {s}"),
            BusinessError::MissingIdempotencyKey => {
                write!(f, "write requires an idempotency key")
            }
            BusinessError::CrossTenant(s) => write!(f, "cross-tenant app access denied: {s}"),
            BusinessError::UnknownRecord(s) => write!(f, "unknown record: {s}"),
        }
    }
}

impl std::error::Error for BusinessError {}

impl From<IamError> for BusinessError {
    fn from(e: IamError) -> Self {
        BusinessError::Iam(e)
    }
}

pub type Result<T> = std::result::Result<T, BusinessError>;

/// In-memory fixture business backend. `records` is app -> record_id -> record;
/// `writes` is an append-only ledger of executed write side effects. This stands
/// in for a live external business system (honest boundary).
#[derive(Debug, Default)]
struct Backend {
    records: BTreeMap<String, BTreeMap<String, Value>>,
    writes: Vec<Value>,
}

pub struct BusinessMcpServer {
    iam: IamServer,
    apps: BTreeMap<String, App>,
    tools: BTreeMap<String, ToolDef>,
    backend: Backend,
    /// (app, idempotency_key) -> cached write result, for anti-replay dedupe.
    idempotency: BTreeMap<String, Value>,
    audit: Vec<Value>,
}

impl BusinessMcpServer {
    pub fn new(secret: &str) -> Self {
        Self {
            iam: IamServer::new(secret),
            apps: BTreeMap::new(),
            tools: BTreeMap::new(),
            backend: Backend::default(),
            idempotency: BTreeMap::new(),
            audit: Vec::new(),
        }
    }

    pub fn iam(&self) -> &IamServer {
        &self.iam
    }

    pub fn iam_mut(&mut self) -> &mut IamServer {
        &mut self.iam
    }

    /// Register a connected application with its own tenant + channel identity.
    /// Creates the IAM user and binds the channel identity to it.
    pub fn register_app(
        &mut self,
        app_id: &str,
        tenant: &str,
        channel: &str,
        external_id: &str,
        user_id: &str,
        role: &str,
    ) -> Result<()> {
        self.iam.add_user(user_id, role, tenant);
        self.iam.bind_channel(channel, external_id, user_id)?;
        self.apps.insert(
            app_id.into(),
            App {
                app_id: app_id.into(),
                tenant: tenant.into(),
                channel: channel.into(),
                external_id: external_id.into(),
                user_id: user_id.into(),
            },
        );
        self.backend.records.entry(app_id.into()).or_default();
        self.audit.push(json!({
            "event": "register_app",
            "app": app_id, "tenant": tenant, "channel": channel, "role": role,
        }));
        Ok(())
    }

    pub fn register_tool(
        &mut self,
        name: &str,
        kind: ToolKind,
        required_params: Vec<(String, ParamType)>,
    ) {
        self.tools.insert(
            name.into(),
            ToolDef {
                name: name.into(),
                kind,
                required_params,
            },
        );
    }

    /// Seed a fixture business record (read-only query target).
    pub fn seed_record(&mut self, app: &str, record_id: &str, record: Value) {
        self.backend
            .records
            .entry(app.into())
            .or_default()
            .insert(record_id.into(), record);
    }

    pub fn app(&self, app_id: &str) -> Option<&App> {
        self.apps.get(app_id)
    }

    /// Number of executed write side effects (replays do NOT increment this).
    pub fn write_count(&self) -> usize {
        self.backend.writes.len()
    }

    pub fn writes(&self) -> &[Value] {
        &self.backend.writes
    }

    pub fn audit(&self) -> &[Value] {
        &self.audit
    }

    fn validate_params(tool: &ToolDef, params: &Value) -> Result<()> {
        let obj = params
            .as_object()
            .ok_or_else(|| BusinessError::InvalidParams("params must be an object".into()))?;
        for (name, ty) in &tool.required_params {
            let v = obj
                .get(name)
                .ok_or_else(|| BusinessError::InvalidParams(format!("missing '{name}'")))?;
            let ok = match ty {
                ParamType::String => v.is_string(),
                ParamType::Number => v.is_number(),
                ParamType::Bool => v.is_boolean(),
            };
            if !ok {
                return Err(BusinessError::InvalidParams(format!(
                    "'{name}' must be {ty:?}"
                )));
            }
        }
        Ok(())
    }

    /// The gateway API entry point. Every trust decision is server-side:
    /// identity -> app tenant boundary -> params -> permission -> idempotency.
    /// `tool_approved` (engine sandbox approval) is recorded but never bypasses
    /// IAM authorization.
    pub fn call_tool(&mut self, token: &str, now: i64, call: &ToolCall) -> Result<ToolResult> {
        // 1. Identity: authenticate the JWT (signature + expiry).
        let claims: Claims = self.iam.authenticate(token, now)?;

        // 2. Resolve the app and enforce the per-app tenant boundary.
        let app = self
            .apps
            .get(&call.app)
            .ok_or_else(|| BusinessError::UnknownApp(call.app.clone()))?;
        if claims.tenant != app.tenant {
            self.audit.push(json!({
                "event": "call_denied", "app": call.app, "tool": call.tool,
                "sub": claims.sub, "reason": "cross-tenant",
            }));
            return Err(BusinessError::CrossTenant(call.app.clone()));
        }
        let app_tenant = app.tenant.clone();
        let app_id = app.app_id.clone();

        // 3. Resolve the tool and validate parameters server-side.
        let tool = self
            .tools
            .get(&call.tool)
            .ok_or_else(|| BusinessError::UnknownTool(call.tool.clone()))?;
        Self::validate_params(tool, &call.params)?;
        let kind = tool.kind;
        let tool_name = tool.name.clone();

        // 4. Permission: IAM resource-level authorize. tool_approved does NOT
        //    skip this — business authorization is independent of MCP approval.
        let record_id = call
            .params
            .get("record_id")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .to_string();
        let resource = ResourceRef {
            kind: ResourceKind::Artifact,
            tenant: app_tenant.clone(),
            id: format!("{app_id}:{record_id}"),
        };
        if let Err(e) = self.iam.authorize(&claims, &resource, kind.action()) {
            self.audit.push(json!({
                "event": "call_denied", "app": app_id, "tool": tool_name,
                "sub": claims.sub, "reason": "authorize", "toolApproved": call.tool_approved,
            }));
            return Err(e.into());
        }

        // 5. Execute against the fixture backend.
        let result = match kind {
            ToolKind::Read => {
                let rec = self
                    .backend
                    .records
                    .get(&app_id)
                    .and_then(|m| m.get(&record_id))
                    .ok_or_else(|| BusinessError::UnknownRecord(record_id.clone()))?;
                ToolResult {
                    value: rec.clone(),
                    replayed: false,
                }
            }
            ToolKind::Write => {
                let key = call
                    .idempotency_key
                    .as_ref()
                    .filter(|k| !k.is_empty())
                    .ok_or(BusinessError::MissingIdempotencyKey)?;
                let idem_key = format!("{app_id}:{key}");
                if let Some(cached) = self.idempotency.get(&idem_key) {
                    // Anti-replay: return the cached result, do NOT re-execute.
                    self.audit.push(json!({
                        "event": "write_replayed", "app": app_id, "tool": tool_name,
                        "sub": claims.sub, "idempotencyKey": key,
                    }));
                    return Ok(ToolResult {
                        value: cached.clone(),
                        replayed: true,
                    });
                }
                let value = json!({
                    "app": app_id, "record_id": record_id, "params": call.params,
                    "status": "written",
                });
                self.backend.writes.push(value.clone());
                self.backend
                    .records
                    .entry(app_id.clone())
                    .or_default()
                    .insert(record_id.clone(), call.params.clone());
                self.idempotency.insert(idem_key, value.clone());
                self.audit.push(json!({
                    "event": "write_executed", "app": app_id, "tool": tool_name,
                    "sub": claims.sub, "tenant": app_tenant, "idempotencyKey": key,
                }));
                ToolResult {
                    value,
                    replayed: false,
                }
            }
        };

        self.audit.push(json!({
            "event": "call_ok", "app": app_id, "tool": tool_name, "kind": kind,
            "sub": claims.sub, "tenant": app_tenant, "toolApproved": call.tool_approved,
            "replayed": result.replayed,
        }));
        Ok(result)
    }
}
