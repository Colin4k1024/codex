//! Core stdio adapter: argv-array subprocess spawn, byte-metered JSONL frame
//! pump (response / notification / server-request three ways), request
//! correlation, per-type timeouts clamped by a global run deadline, typed
//! approval handling (never auto-approve), resource limits, crash detection
//! with explicit terminal signals, and exit cleanup.
//!
//! Port of the proven P0-6 reference client (`superagent/tests/spike_client.py`)
//! to product-side Rust. No codex-rs core dependencies; no model/network code.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::approval::{
    ApprovalHandler, Decision, DecisionRecord, DecliningApprovalHandler, ServerRequest,
};
use crate::config::AdapterConfig;
use crate::error::AdapterError;
use crate::frame::{read_line_metered, LineOutcome};
use crate::runlog::RunLog;
use crate::ui::AdapterEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AdapterStats {
    pub spawn_argv: Vec<String>,
    pub frame_limit_bytes: usize,
    pub max_frame_seen: usize,
    pub frame_violations: Vec<usize>,
    pub run_log_limit_bytes: usize,
    pub run_log_rotations: usize,
    pub run_log_retained_bytes: usize,
    pub artifact_limit_bytes: usize,
    pub exit: Option<ExitInfo>,
    pub pending_server_requests: usize,
    pub queued_events: usize,
    pub server_request_decisions: Vec<DecisionRecord>,
}

#[derive(Default)]
struct State {
    responses: HashMap<u64, Value>,
    /// Unbounded event queue: overload delays delivery, never drops it.
    events: VecDeque<AdapterEvent>,
    pending_server_requests: BTreeMap<String, ServerRequest>,
    decisions: Vec<DecisionRecord>,
    /// turn/start request id -> threadId, for synthetic terminals on crash.
    inflight_turns: HashMap<u64, String>,
    max_frame_seen: usize,
    frame_violations: Vec<usize>,
    /// First oversize frame faults the transport (explicit errors afterwards).
    fault: Option<usize>,
    exit: Option<ExitInfo>,
}

struct Shared {
    config: AdapterConfig,
    deadline: Instant,
    state: Mutex<State>,
    cv: Condvar,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pid: u32,
    runlog: Mutex<Option<RunLog>>,
    stderr: Mutex<VecDeque<String>>,
    next_id: AtomicU64,
    handler: Arc<dyn ApprovalHandler>,
}

pub struct Adapter {
    shared: Arc<Shared>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    closed: AtomicBool,
}

fn exit_info(status: std::process::ExitStatus) -> ExitInfo {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitInfo {
            code: status.code(),
            signal: status.signal(),
        }
    }
    #[cfg(not(unix))]
    {
        ExitInfo {
            code: status.code(),
            signal: None,
        }
    }
}

fn log_frame(shared: &Shared, direction: &str, line: &str) {
    if let Some(log) = shared.runlog.lock().unwrap().as_mut() {
        // A log write failure must never fabricate success nor kill the pump;
        // it is recorded on stderr ring for diagnostics.
        if let Err(e) = log.record(direction, line) {
            let mut ring = shared.stderr.lock().unwrap();
            ring.push_back(format!("<run-log error: {e}>"));
            while ring.len() > shared.config.stderr_ring_lines {
                ring.pop_front();
            }
        }
    }
}

fn send_value(shared: &Shared, msg: &Value) -> Result<(), AdapterError> {
    let data = serde_json::to_string(msg).map_err(|e| AdapterError::Protocol(e.to_string()))?;
    let mut guard = shared.stdin.lock().unwrap();
    let stdin = guard.as_mut().ok_or(AdapterError::ConnectionClosed)?;
    stdin
        .write_all(data.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|e| AdapterError::Io(format!("stdin write: {e}")))?;
    drop(guard);
    log_frame(shared, "send", &data);
    Ok(())
}

fn record_exit(shared: &Shared, info: ExitInfo) {
    let mut st = shared.state.lock().unwrap();
    if st.exit.is_some() {
        return;
    }
    st.exit = Some(info);
    // No silent terminal drop: every in-flight turn gets an explicit
    // synthetic terminal, then the connection-level terminal fault signal.
    let orphaned_turns: Vec<String> = st.inflight_turns.drain().map(|(_, tid)| tid).collect();
    for thread_id in orphaned_turns {
        st.events.push_back(AdapterEvent::TurnTerminal {
            thread_id,
            status: "aborted".into(),
            reason: "engine exited before emitting a terminal turn event".into(),
        });
    }
    st.events.push_back(AdapterEvent::EngineExited {
        code: info.code,
        signal: info.signal,
    });
    shared.cv.notify_all();
}

fn dispatch_server_request(shared: &Shared, msg: Value) {
    let req = ServerRequest {
        id: msg.get("id").cloned().unwrap_or(Value::Null),
        method: msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        params: msg.get("params").cloned().unwrap_or(json!({})),
    };
    // Handler runs WITHOUT the state lock; it must be non-blocking (Defer
    // for human-gated approvals) so the pump keeps consuming (trace B2).
    let decision = shared.handler.handle(&req);
    let (payload, deferred) = match &decision {
        Decision::Defer => {
            let mut st = shared.state.lock().unwrap();
            st.pending_server_requests
                .insert(req.id.to_string(), req.clone());
            st.decisions.push(DecisionRecord {
                method: req.method.clone(),
                id: req.id.clone(),
                payload: json!({"deferred": true}),
                deferred: true,
                deferred_response: false,
            });
            st.events.push_back(AdapterEvent::ApprovalPending(msg));
            shared.cv.notify_all();
            return;
        }
        Decision::Result(v) => (json!({"result": v}), false),
        Decision::Error { code, message } => {
            (json!({"error": {"code": code, "message": message}}), false)
        }
    };
    let mut reply = json!({"jsonrpc": "2.0", "id": req.id});
    if let Some(obj) = payload.as_object() {
        for (k, v) in obj {
            reply[k] = v.clone();
        }
    }
    let send_result = send_value(shared, &reply);
    let mut st = shared.state.lock().unwrap();
    st.decisions.push(DecisionRecord {
        method: req.method.clone(),
        id: req.id.clone(),
        payload: match send_result {
            Ok(()) => payload,
            Err(e) => json!({"sendError": e.to_string()}),
        },
        deferred,
        deferred_response: false,
    });
    shared.cv.notify_all();
}

fn read_pump(shared: Arc<Shared>, stdout: std::process::ChildStdout) {
    let mut reader = BufReader::new(stdout);
    let cap = shared.config.max_frame_bytes;
    // On pipe error the loop ends; the exit waiter reports the real status.
    while let Ok(outcome) = read_line_metered(&mut reader, cap) {
        match outcome {
            LineOutcome::Eof => break,
            LineOutcome::Oversize { bytes } => {
                log_frame(&shared, "recv", &format!("<oversize frame: {bytes} bytes>"));
                let mut st = shared.state.lock().unwrap();
                st.max_frame_seen = st.max_frame_seen.max(bytes);
                st.frame_violations.push(bytes);
                if st.fault.is_none() {
                    st.fault = Some(bytes);
                }
                st.events.push_back(AdapterEvent::FrameViolation { bytes });
                shared.cv.notify_all();
                // Keep draining (metering further violations) but never
                // dispatch frames on a faulted transport.
            }
            LineOutcome::Line(buf) => {
                let faulted = shared.state.lock().unwrap().fault.is_some();
                if faulted {
                    continue;
                }
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                log_frame(&shared, "recv", line);
                {
                    let mut st = shared.state.lock().unwrap();
                    st.max_frame_seen = st.max_frame_seen.max(line.len());
                }
                let msg: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => {
                        // Malformed frame: record raw (bounded), keep pumping.
                        let raw: String = line.chars().take(4000).collect();
                        let mut st = shared.state.lock().unwrap();
                        st.events
                            .push_back(AdapterEvent::Notification(json!({"_raw": raw})));
                        shared.cv.notify_all();
                        continue;
                    }
                };
                let has_id = msg.get("id").is_some();
                let has_method = msg.get("method").is_some();
                if has_id && has_method {
                    dispatch_server_request(&shared, msg);
                    continue;
                }
                let mut st = shared.state.lock().unwrap();
                if has_id {
                    if let Some(id) = msg["id"].as_u64() {
                        st.responses.insert(id, msg);
                    }
                } else {
                    // Terminal turn notifications clear the in-flight marker
                    // so a later clean exit does not double-emit terminals.
                    if matches!(
                        msg.get("method").and_then(Value::as_str),
                        Some("turn/completed" | "turn/failed" | "turn/aborted")
                    ) {
                        if let Some(tid) = msg
                            .get("params")
                            .and_then(|p| p.get("threadId"))
                            .and_then(Value::as_str)
                        {
                            st.inflight_turns.retain(|_, v| v != tid);
                        }
                    }
                    st.events.push_back(AdapterEvent::Notification(msg));
                }
                shared.cv.notify_all();
            }
        }
    }
}

fn stderr_pump(shared: Arc<Shared>, stderr: std::process::ChildStderr) {
    let reader = BufReader::new(stderr);
    let cap = shared.config.stderr_ring_lines;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let mut ring = shared.stderr.lock().unwrap();
        ring.push_back(line);
        while ring.len() > cap {
            ring.pop_front();
        }
    }
}

fn exit_waiter(shared: Arc<Shared>) {
    loop {
        let status = {
            let mut guard = shared.child.lock().unwrap();
            match guard.as_mut() {
                Some(child) => child.try_wait(),
                None => return,
            }
        };
        match status {
            Ok(Some(status)) => {
                *shared.child.lock().unwrap() = None; // reaped
                record_exit(&shared, exit_info(status));
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                *shared.child.lock().unwrap() = None;
                record_exit(
                    &shared,
                    ExitInfo {
                        code: None,
                        signal: None,
                    },
                );
                return;
            }
        }
    }
}

/// Write an artifact with an explicit single-file size limit. On exceed:
/// explicit error, nothing written (never fabricate success).
pub fn write_artifact_with_limit(
    path: &Path,
    bytes: &[u8],
    limit: usize,
) -> Result<(), AdapterError> {
    if bytes.len() > limit {
        return Err(AdapterError::ArtifactLimitExceeded {
            limit,
            observed: bytes.len(),
        });
    }
    std::fs::write(path, bytes).map_err(|e| AdapterError::Io(format!("artifact write: {e}")))
}

impl Adapter {
    /// Spawn the engine with the configured argv array (no shell) and the
    /// default-deny approval handler.
    pub fn spawn_default(config: AdapterConfig) -> Result<Self, AdapterError> {
        Self::spawn(config, Arc::new(DecliningApprovalHandler))
    }

    pub fn spawn(
        config: AdapterConfig,
        handler: Arc<dyn ApprovalHandler>,
    ) -> Result<Self, AdapterError> {
        config.validate()?;
        let mut cmd = Command::new(&config.argv[0]);
        cmd.args(&config.argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !config.inherit_env {
            cmd.env_clear();
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        if let Some(home) = &config.codex_home {
            cmd.env("CODEX_HOME", home);
        }
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AdapterError::Spawn(e.to_string()))?;
        let pid = child.id();
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let runlog = match &config.run_log_path {
            Some(p) => Some(RunLog::open(p, config.max_log_bytes)?),
            None => None,
        };
        let deadline = Instant::now() + config.run_deadline;
        let shared = Arc::new(Shared {
            config,
            deadline,
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(Some(child)),
            pid,
            runlog: Mutex::new(runlog),
            stderr: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            handler,
        });
        let mut handles = Vec::new();
        {
            let s = Arc::clone(&shared);
            handles.push(
                thread::Builder::new()
                    .name("adapter-read-pump".into())
                    .spawn(move || read_pump(s, stdout))
                    .map_err(|e| AdapterError::Spawn(e.to_string()))?,
            );
        }
        {
            let s = Arc::clone(&shared);
            handles.push(
                thread::Builder::new()
                    .name("adapter-stderr".into())
                    .spawn(move || stderr_pump(s, stderr))
                    .map_err(|e| AdapterError::Spawn(e.to_string()))?,
            );
        }
        {
            let s = Arc::clone(&shared);
            handles.push(
                thread::Builder::new()
                    .name("adapter-exit-waiter".into())
                    .spawn(move || exit_waiter(s))
                    .map_err(|e| AdapterError::Spawn(e.to_string()))?,
            );
        }
        Ok(Self {
            shared,
            handles: Mutex::new(handles),
            closed: AtomicBool::new(false),
        })
    }

    /// initialize + initialized handshake (reference client parity).
    pub fn handshake(&self) -> Result<Value, AdapterError> {
        let result = self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "superagent-adapter", "title": "SuperAgent Adapter", "version": "0.1.0"},
                "capabilities": {}
            }),
            None,
        )?;
        self.notify("initialized", json!({}))?;
        Ok(result)
    }

    pub fn spawn_argv(&self) -> &[String] {
        &self.shared.config.argv
    }

    pub fn pid(&self) -> u32 {
        self.shared.pid
    }

    pub fn deadline_remaining(&self) -> Duration {
        self.shared
            .deadline
            .saturating_duration_since(Instant::now())
    }

    fn check_sendable(&self) -> Result<(), AdapterError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(AdapterError::ConnectionClosed);
        }
        let st = self.shared.state.lock().unwrap();
        if let Some(observed) = st.fault {
            return Err(AdapterError::FrameLimitExceeded {
                limit: self.shared.config.max_frame_bytes,
                observed,
            });
        }
        if let Some(e) = st.exit {
            return Err(AdapterError::EngineExited {
                code: e.code,
                signal: e.signal,
            });
        }
        Ok(())
    }

    /// Send a request; returns the correlation id. Per-type timeout applies
    /// when the response is awaited via [`Self::request`]/[`Self::wait_response`].
    pub fn send_request(&self, method: &str, params: Value) -> Result<u64, AdapterError> {
        self.check_sendable()?;
        let id = self.shared.next_id.fetch_add(1, Ordering::SeqCst);
        let thread_id = if method == "turn/start" {
            params
                .get("threadId")
                .and_then(Value::as_str)
                .map(str::to_string)
        } else {
            None
        };
        send_value(
            &self.shared,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        if let Some(tid) = thread_id {
            self.shared
                .state
                .lock()
                .unwrap()
                .inflight_turns
                .insert(id, tid);
        }
        Ok(id)
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), AdapterError> {
        self.check_sendable()?;
        send_value(
            &self.shared,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
    }

    /// Await a correlated response. Bounded by min(explicit/per-type timeout,
    /// remaining run deadline). Engine exit / frame fault fail explicitly.
    pub fn wait_response(&self, id: u64, timeout: Duration) -> Result<Value, AdapterError> {
        let wait_until = self.shared.deadline.min(Instant::now() + timeout);
        let mut st = self.shared.state.lock().unwrap();
        loop {
            if let Some(resp) = st.responses.remove(&id) {
                return Ok(resp);
            }
            if let Some(observed) = st.fault {
                return Err(AdapterError::FrameLimitExceeded {
                    limit: self.shared.config.max_frame_bytes,
                    observed,
                });
            }
            if let Some(e) = st.exit {
                return Err(AdapterError::EngineExited {
                    code: e.code,
                    signal: e.signal,
                });
            }
            let remaining = wait_until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(if Instant::now() >= self.shared.deadline {
                    AdapterError::DeadlineExceeded {
                        method: format!("response id={id}"),
                    }
                } else {
                    AdapterError::Timeout {
                        method: format!("response id={id}"),
                        id,
                        waited: timeout,
                    }
                });
            }
            st = self.shared.cv.wait_timeout(st, remaining).unwrap().0;
        }
    }

    /// Request with per-type timeout (explicit overrides), clamped by the run
    /// deadline. JSON-RPC error objects propagate as [`AdapterError::Rpc`].
    pub fn request(
        &self,
        method: &str,
        params: Value,
        explicit_timeout: Option<Duration>,
    ) -> Result<Value, AdapterError> {
        let timeout = self
            .shared
            .config
            .timeouts
            .effective(method, explicit_timeout);
        let id = self.send_request(method, params)?;
        let msg = self.wait_response(id, timeout)?;
        if let Some(err) = msg.get("error") {
            return Err(AdapterError::Rpc {
                code: err.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Await a notification matching `predicate` (consumed on match). Bounded
    /// by min(timeout, run deadline) — model-task terminal waits pass the run
    /// deadline here.
    pub fn wait_notification(
        &self,
        predicate: impl Fn(&Value) -> bool,
        timeout: Duration,
    ) -> Result<Value, AdapterError> {
        let wait_until = self.shared.deadline.min(Instant::now() + timeout);
        let mut st = self.shared.state.lock().unwrap();
        loop {
            if let Some(pos) = st.events.iter().position(|ev| match ev {
                AdapterEvent::Notification(n) => predicate(n),
                _ => false,
            }) {
                if let Some(AdapterEvent::Notification(n)) = st.events.remove(pos) {
                    return Ok(n);
                }
            }
            if let Some(e) = st.exit {
                return Err(AdapterError::EngineExited {
                    code: e.code,
                    signal: e.signal,
                });
            }
            let remaining = wait_until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(if Instant::now() >= self.shared.deadline {
                    AdapterError::DeadlineExceeded {
                        method: "notification".into(),
                    }
                } else {
                    AdapterError::Timeout {
                        method: "notification".into(),
                        id: 0,
                        waited: timeout,
                    }
                });
            }
            st = self.shared.cv.wait_timeout(st, remaining).unwrap().0;
        }
    }

    /// Pop the next event (unbounded queue; nothing is ever dropped).
    pub fn wait_event(&self, timeout: Duration) -> Result<AdapterEvent, AdapterError> {
        let wait_until = self.shared.deadline.min(Instant::now() + timeout);
        let mut st = self.shared.state.lock().unwrap();
        loop {
            if let Some(ev) = st.events.pop_front() {
                return Ok(ev);
            }
            if let Some(e) = st.exit {
                return Err(AdapterError::EngineExited {
                    code: e.code,
                    signal: e.signal,
                });
            }
            let remaining = wait_until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AdapterError::Timeout {
                    method: "event".into(),
                    id: 0,
                    waited: timeout,
                });
            }
            st = self.shared.cv.wait_timeout(st, remaining).unwrap().0;
        }
    }

    pub fn drain_events(&self) -> Vec<AdapterEvent> {
        self.shared.state.lock().unwrap().events.drain(..).collect()
    }

    /// `waiting_input`: await a deferred server request, bounded by the run
    /// deadline (approval waits are not short query timeouts).
    pub fn wait_pending_server_request(
        &self,
        timeout: Duration,
    ) -> Result<ServerRequest, AdapterError> {
        let wait_until = self.shared.deadline.min(Instant::now() + timeout);
        let mut st = self.shared.state.lock().unwrap();
        loop {
            if let Some(req) = st.pending_server_requests.values().next() {
                return Ok(req.clone());
            }
            if let Some(e) = st.exit {
                return Err(AdapterError::EngineExited {
                    code: e.code,
                    signal: e.signal,
                });
            }
            let remaining = wait_until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(if Instant::now() >= self.shared.deadline {
                    AdapterError::DeadlineExceeded {
                        method: "waiting_input".into(),
                    }
                } else {
                    AdapterError::Timeout {
                        method: "waiting_input".into(),
                        id: 0,
                        waited: timeout,
                    }
                });
            }
            st = self.shared.cv.wait_timeout(st, remaining).unwrap().0;
        }
    }

    /// Explicitly answer a deferred server request. The adapter never invents
    /// an approval: the payload is caller-authored (orchestrator policy gate).
    pub fn respond_server_request(
        &self,
        id: &Value,
        decision: Decision,
    ) -> Result<(), AdapterError> {
        let payload = match decision {
            Decision::Result(v) => json!({"result": v}),
            Decision::Error { code, message } => {
                json!({"error": {"code": code, "message": message}})
            }
            Decision::Defer => {
                return Err(AdapterError::InvalidConfig(
                    "cannot defer an already-deferred request".into(),
                ));
            }
        };
        let key = id.to_string();
        let req = {
            let mut st = self.shared.state.lock().unwrap();
            st.pending_server_requests.remove(&key)
        };
        let req = req.ok_or_else(|| {
            AdapterError::Protocol(format!("no deferred server request id={key}"))
        })?;
        let mut reply = json!({"jsonrpc": "2.0", "id": id});
        if let Some(obj) = payload.as_object() {
            for (k, v) in obj {
                reply[k] = v.clone();
            }
        }
        send_value(&self.shared, &reply)?;
        let mut st = self.shared.state.lock().unwrap();
        st.decisions.push(DecisionRecord {
            method: req.method,
            id: req.id,
            payload,
            deferred: false,
            deferred_response: true,
        });
        self.shared.cv.notify_all();
        Ok(())
    }

    pub fn write_artifact(&self, path: &Path, bytes: &[u8]) -> Result<(), AdapterError> {
        write_artifact_with_limit(path, bytes, self.shared.config.max_artifact_bytes)
    }

    pub fn stderr_tail(&self, n: usize) -> Vec<String> {
        let ring = self.shared.stderr.lock().unwrap();
        ring.iter().rev().take(n).rev().cloned().collect()
    }

    pub fn stats(&self) -> AdapterStats {
        let st = self.shared.state.lock().unwrap();
        let log = self.shared.runlog.lock().unwrap();
        AdapterStats {
            spawn_argv: self.shared.config.argv.clone(),
            frame_limit_bytes: self.shared.config.max_frame_bytes,
            max_frame_seen: st.max_frame_seen,
            frame_violations: st.frame_violations.clone(),
            run_log_limit_bytes: self.shared.config.max_log_bytes,
            run_log_rotations: log.as_ref().map_or(0, RunLog::rotations),
            run_log_retained_bytes: log.as_ref().map_or(0, RunLog::retained_bytes),
            artifact_limit_bytes: self.shared.config.max_artifact_bytes,
            exit: st.exit,
            pending_server_requests: st.pending_server_requests.len(),
            queued_events: st.events.len(),
            server_request_decisions: st.decisions.clone(),
        }
    }

    /// Close stdin (engine should exit), wait up to `close_grace`, then kill.
    /// Always reaps the child and joins the pump threads (exit cleanup).
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            // Dropping stdin sends EOF to the engine.
            let mut guard = self.shared.stdin.lock().unwrap();
            *guard = None;
        }
        let grace_until = Instant::now() + self.shared.config.close_grace;
        {
            let mut st = self.shared.state.lock().unwrap();
            while st.exit.is_none() {
                let remaining = grace_until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                st = self.shared.cv.wait_timeout(st, remaining).unwrap().0;
            }
        }
        if self.shared.state.lock().unwrap().exit.is_none() {
            if let Some(child) = self.shared.child.lock().unwrap().as_mut() {
                let _ = child.kill();
            }
            let kill_until = Instant::now() + Duration::from_secs(5);
            let mut st = self.shared.state.lock().unwrap();
            while st.exit.is_none() {
                let remaining = kill_until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                st = self.shared.cv.wait_timeout(st, remaining).unwrap().0;
            }
        }
        for handle in self.shared_handles_drain() {
            let _ = handle.join();
        }
    }

    fn shared_handles_drain(&self) -> Vec<JoinHandle<()>> {
        self.handles.lock().unwrap().drain(..).collect()
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        self.close();
    }
}
