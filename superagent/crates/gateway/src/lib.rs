//! SuperAgent unified gateway (P4-1 / #90): multi-tenant entry + engine pool.
//!
//! Model-free: the only subprocess is the per-tenant engine (caller-supplied
//! argv; `fake_engine.py` in tests) speaking the app-server JSONL stdio
//! contract. Each tenant gets an isolated `CODEX_HOME`; the pool bounds
//! concurrency, recovers crashed engines, reaps on release (no leak), and
//! writes an append-only audit log + run manifest. A TCP unified entry
//! forwards app-server JSON-RPC (session/turn/approval/event) to the correct
//! tenant engine, routing by `tenantId`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[derive(Debug)]
pub enum GatewayError {
    ConcurrencyLimit(usize),
    UnknownTenant(String),
    Engine(String),
    Io(std::io::Error),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatewayError::ConcurrencyLimit(n) => {
                write!(f, "tenant concurrency limit reached ({n})")
            }
            GatewayError::UnknownTenant(t) => write!(f, "unknown tenant: {t}"),
            GatewayError::Engine(e) => write!(f, "engine: {e}"),
            GatewayError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for GatewayError {}

impl From<std::io::Error> for GatewayError {
    fn from(e: std::io::Error) -> Self {
        GatewayError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, GatewayError>;

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Append-only JSONL audit trail of tenant lifecycle events.
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(
        &self,
        tenant: &str,
        event: &str,
        pid: Option<u32>,
        codex_home: Option<&Path>,
    ) -> Result<()> {
        let line = json!({
            "ts": now_ms(),
            "tenant": tenant,
            "event": event,
            "pid": pid,
            "codexHome": codex_home.map(|p| p.display().to_string()),
        });
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(&line).unwrap_or_default())?;
        Ok(())
    }
}

/// Run manifest: product version / engine digest / platform / protocol summary.
#[derive(Debug, Clone)]
pub struct RunManifest {
    pub product_version: String,
    pub engine_digest: String,
    pub platform: String,
    pub protocol_summary: String,
}

impl RunManifest {
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let v = json!({
            "productVersion": self.product_version,
            "engineDigest": self.engine_digest,
            "platform": self.platform,
            "protocolSummary": self.protocol_summary,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&v).unwrap_or_default())?;
        Ok(())
    }
}

/// Command template spawned per tenant; `CODEX_HOME` is injected per tenant.
#[derive(Debug, Clone)]
pub struct EngineTemplate {
    pub argv: Vec<String>,
}

struct TenantEngine {
    codex_home: PathBuf,
    child: Child,
    reader: BufReader<ChildStdout>,
}

/// Multi-tenant engine process pool with isolated `CODEX_HOME` per tenant.
pub struct GatewayPool {
    base: PathBuf,
    template: EngineTemplate,
    max_tenants: usize,
    audit: AuditLog,
    tenants: RefCell<BTreeMap<String, TenantEngine>>,
}

impl GatewayPool {
    pub fn new(
        base: &Path,
        template: EngineTemplate,
        max_tenants: usize,
        audit: AuditLog,
    ) -> Result<Self> {
        std::fs::create_dir_all(base)?;
        Ok(Self {
            base: base.to_path_buf(),
            template,
            max_tenants: max_tenants.max(1),
            audit,
            tenants: RefCell::new(BTreeMap::new()),
        })
    }

    pub fn active_count(&self) -> usize {
        self.tenants.borrow().len()
    }

    pub fn max_tenants(&self) -> usize {
        self.max_tenants
    }

    pub fn codex_home(&self, tenant: &str) -> PathBuf {
        self.base.join(tenant)
    }

    fn spawn(&self, tenant: &str) -> Result<TenantEngine> {
        let home = self.codex_home(tenant);
        std::fs::create_dir_all(&home)?;
        let Some((prog, rest)) = self.template.argv.split_first() else {
            return Err(GatewayError::Engine("empty engine argv".into()));
        };
        let mut child = Command::new(prog)
            .args(rest)
            .env("CODEX_HOME", &home)
            .current_dir(&home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GatewayError::Engine("no stdout pipe".into()))?;
        Ok(TenantEngine {
            codex_home: home,
            child,
            reader: BufReader::new(stdout),
        })
    }

    /// Acquire (or return the existing) tenant engine; returns its pid.
    pub fn acquire(&self, tenant: &str) -> Result<u32> {
        if let Some(eng) = self.tenants.borrow().get(tenant) {
            return Ok(eng.child.id());
        }
        if self.tenants.borrow().len() >= self.max_tenants {
            return Err(GatewayError::ConcurrencyLimit(self.max_tenants));
        }
        let eng = self.spawn(tenant)?;
        let pid = eng.child.id();
        let home = eng.codex_home.clone();
        self.tenants.borrow_mut().insert(tenant.to_string(), eng);
        self.audit.record(tenant, "spawn", Some(pid), Some(&home))?;
        Ok(pid)
    }

    pub fn is_alive(&self, tenant: &str) -> Result<bool> {
        let mut t = self.tenants.borrow_mut();
        let eng = t
            .get_mut(tenant)
            .ok_or_else(|| GatewayError::UnknownTenant(tenant.into()))?;
        Ok(eng.child.try_wait()?.is_none())
    }

    /// Reap a crashed/existing engine and respawn it; audits crash + restart.
    pub fn restart(&self, tenant: &str) -> Result<u32> {
        let (old_pid, home) = {
            let mut t = self.tenants.borrow_mut();
            let eng = t
                .get_mut(tenant)
                .ok_or_else(|| GatewayError::UnknownTenant(tenant.into()))?;
            let old_pid = eng.child.id();
            let home = eng.codex_home.clone();
            let _ = eng.child.kill();
            let _ = eng.child.wait();
            (old_pid, home)
        };
        self.audit
            .record(tenant, "crash", Some(old_pid), Some(&home))?;
        let eng = self.spawn(tenant)?;
        let pid = eng.child.id();
        self.tenants.borrow_mut().insert(tenant.to_string(), eng);
        self.audit
            .record(tenant, "restart", Some(pid), Some(&home))?;
        Ok(pid)
    }

    /// Kill + reap a tenant engine (no orphan/leak); audits release.
    pub fn release(&self, tenant: &str) -> Result<()> {
        let mut t = self.tenants.borrow_mut();
        if let Some(mut eng) = t.remove(tenant) {
            let pid = eng.child.id();
            let home = eng.codex_home.clone();
            let _ = eng.child.kill();
            let _ = eng.child.wait();
            self.audit
                .record(tenant, "release", Some(pid), Some(&home))?;
        }
        Ok(())
    }

    pub fn release_all(&self) -> Result<()> {
        let ids: Vec<String> = self.tenants.borrow().keys().cloned().collect();
        for id in ids {
            self.release(&id)?;
        }
        Ok(())
    }

    /// Forward one app-server JSON-RPC request to a tenant engine over stdio,
    /// skipping streamed notifications until the matching response id arrives.
    pub fn forward(&self, tenant: &str, request: &Value) -> Result<Value> {
        let req_id = request.get("id");
        if req_id.is_none() {
            return Err(GatewayError::Engine("request must carry an id".into()));
        }
        let mut t = self.tenants.borrow_mut();
        let eng = t
            .get_mut(tenant)
            .ok_or_else(|| GatewayError::UnknownTenant(tenant.into()))?;
        {
            let stdin = eng
                .child
                .stdin
                .as_mut()
                .ok_or_else(|| GatewayError::Engine("no stdin pipe".into()))?;
            writeln!(
                stdin,
                "{}",
                serde_json::to_string(request).unwrap_or_default()
            )?;
            stdin.flush()?;
        }
        loop {
            let mut line = String::new();
            let n = eng.reader.read_line(&mut line)?;
            if n == 0 {
                return Err(GatewayError::Engine("engine closed stdout".into()));
            }
            let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if msg.get("id") == req_id {
                return Ok(msg);
            }
        }
    }
}

/// TCP unified entry: accept one connection and forward each `tenantId`-tagged
/// app-server JSON-RPC line to the matching tenant engine, writing responses
/// back. Single-threaded accept loop (the pool is intentionally !Sync).
pub fn serve_connection(listener: &TcpListener, pool: &GatewayPool) -> Result<()> {
    let (stream, _addr) = listener.accept()?;
    handle_connection(stream, pool)
}

pub fn handle_connection(stream: TcpStream, pool: &GatewayPool) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        let Ok(req) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let tenant = req
            .get("tenantId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut fwd = req.clone();
        if let Some(o) = fwd.as_object_mut() {
            o.remove("tenantId");
        }
        let resp = pool.forward(&tenant, &fwd)?;
        writeln!(
            writer,
            "{}",
            serde_json::to_string(&resp).unwrap_or_default()
        )?;
        writer.flush()?;
    }
}
