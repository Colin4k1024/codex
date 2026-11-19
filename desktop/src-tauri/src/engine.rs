//! U03: stdio JSON-RPC client bridging the unmodified Codex app-server.
//!
//! The desktop host spawns the engine binary with an ISOLATED `CODEX_HOME`
//! (never the user's `~/.codex`), performs the `initialize` handshake, correlates
//! request/response by id, and forwards notifications and server-initiated
//! requests to the WebView as ordered product events over [`EVENT_NAME`].
//!
//! Security: server approval requests (`item/*/requestApproval`, elicitations,
//! token refresh) are NEVER auto-answered. They are forwarded to the frontend and
//! only resolved by an explicit typed user action via [`EngineClient::respond`].
//! The engine path and CODEX_HOME are configurable; nothing here writes to the
//! user's real Codex home or logs credentials.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

/// Tauri event channel carrying ordered engine events to the WebView.
pub const EVENT_NAME: &str = "desktop-event";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Sink for product events emitted to the frontend. Decoupled from Tauri so the
/// client is testable without an app handle.
pub type EventSink = Arc<dyn Fn(Value) + Send + Sync + 'static>;

/// Classify an incoming JSON-RPC message.
#[derive(Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// Response to a client request (has id + result/error).
    Response,
    /// Server-initiated request expecting a client response (has id + method).
    ServerRequest,
    /// Server notification (method, no id).
    Notification,
    /// Unrecognized frame.
    Unknown,
}

pub fn classify(msg: &Value) -> MessageKind {
    let has_id = msg.get("id").map(|v| !v.is_null()).unwrap_or(false);
    let has_method = msg.get("method").and_then(Value::as_str).is_some();
    let has_result = msg.get("result").is_some() || msg.get("error").is_some();
    if has_id && has_result {
        MessageKind::Response
    } else if has_id && has_method {
        MessageKind::ServerRequest
    } else if has_method {
        MessageKind::Notification
    } else {
        MessageKind::Unknown
    }
}

/// Build the `initialize` params for this product client.
pub fn initialize_params() -> Value {
    json!({
        "clientInfo": { "name": "youragent-desktop", "title": "YourAgent", "version": env!("CARGO_PKG_VERSION") },
        "capabilities": { "experimentalApi": false, "requestAttestation": false }
    })
}

pub struct EngineClient {
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
    next_id: AtomicU64,
    info: Mutex<Value>,
}

impl EngineClient {
    /// Spawn the engine and complete the initialize handshake.
    pub fn start(
        engine_path: &str,
        codex_home: &str,
        sink: EventSink,
    ) -> Result<Arc<EngineClient>, String> {
        std::fs::create_dir_all(codex_home).map_err(|e| format!("create CODEX_HOME: {e}"))?;

        let mut child = Command::new(engine_path)
            .arg("app-server")
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn engine {engine_path}: {e}"))?;

        let stdin = child.stdin.take().ok_or("engine stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("engine stdout unavailable")?;
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        let reader_sink = Arc::clone(&sink);

        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match classify(&msg) {
                    MessageKind::Response => {
                        if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                            if let Some(tx) = reader_pending.lock().unwrap().remove(&id) {
                                let _ = tx.send(msg);
                            }
                        }
                    }
                    MessageKind::ServerRequest => {
                        // Forward approvals/elicitations; never auto-answer.
                        reader_sink(json!({
                            "kind": "server_request",
                            "requestId": msg.get("id").cloned().unwrap_or(Value::Null),
                            "method": msg.get("method").cloned().unwrap_or(Value::Null),
                            "params": msg.get("params").cloned().unwrap_or(Value::Null),
                        }));
                    }
                    MessageKind::Notification => {
                        reader_sink(json!({
                            "kind": "notification",
                            "method": msg.get("method").cloned().unwrap_or(Value::Null),
                            "params": msg.get("params").cloned().unwrap_or(Value::Null),
                        }));
                    }
                    MessageKind::Unknown => {}
                }
            }
            // Engine stdout closed: signal pending requests so they fail fast.
            reader_pending.lock().unwrap().clear();
        });

        if let Some(stderr) = stderr {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for _ in reader.lines().map_while(Result::ok) {
                    // Drain to avoid blocking the child; do not log (may carry
                    // sensitive config paths). Diagnostics surface via RPC errors.
                }
            });
        }

        let client = Arc::new(EngineClient {
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
            next_id: AtomicU64::new(1),
            info: Mutex::new(Value::Null),
        });

        let result = client.request("initialize", initialize_params())?;
        client.notify("initialized", json!({}))?;
        *client.info.lock().unwrap() = result;
        Ok(client)
    }

    /// The initialize result (userAgent, codexHome, platformFamily, platformOs).
    pub fn info(&self) -> Value {
        self.info.lock().unwrap().clone()
    }

    fn write_line(&self, msg: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().unwrap();
        let mut line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("engine write: {e}"))?;
        stdin.flush().map_err(|e| format!("engine flush: {e}"))?;
        Ok(())
    }

    /// Send a request and wait for its correlated response.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.write_line(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;
        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(resp) => {
                if let Some(err) = resp.get("error") {
                    Err(format!("engine error: {err}"))
                } else {
                    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("engine request timeout: {method}"))
            }
        }
    }

    /// Send a notification (no response expected).
    pub fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Resolve a server-initiated request (e.g. an approval) with a typed user
    /// decision. Only called from an explicit frontend action — never automatic.
    pub fn respond(&self, request_id: Value, result: Value) -> Result<(), String> {
        self.write_line(&json!({ "jsonrpc": "2.0", "id": request_id, "result": result }))
    }

    /// Terminate the engine process.
    pub fn shutdown(&self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

impl Drop for EngineClient {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

/// Resolve the engine binary path: explicit override, then PATH lookup.
pub fn resolve_engine_path() -> Option<String> {
    if let Ok(p) = std::env::var("YOURAGENT_ENGINE_PATH") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    None
}

/// Resolve the isolated CODEX_HOME (never the user's ~/.codex).
pub fn resolve_codex_home() -> String {
    if let Ok(p) = std::env::var("YOURAGENT_CODEX_HOME") {
        if !p.is_empty() {
            return p;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    format!("{home}/.youragent/codex-home")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_response() {
        assert_eq!(
            classify(&json!({"id":1,"result":{}})),
            MessageKind::Response
        );
        assert_eq!(
            classify(&json!({"id":1,"error":{"code":-1}})),
            MessageKind::Response
        );
    }

    #[test]
    fn classify_server_request_vs_notification() {
        assert_eq!(
            classify(&json!({"id":9,"method":"item/commandExecution/requestApproval","params":{}})),
            MessageKind::ServerRequest
        );
        assert_eq!(
            classify(&json!({"method":"item/agentMessage/delta","params":{}})),
            MessageKind::Notification
        );
        assert_eq!(classify(&json!({"foo":1})), MessageKind::Unknown);
    }

    #[test]
    fn initialize_params_shape() {
        let p = initialize_params();
        assert_eq!(p["clientInfo"]["name"], "youragent-desktop");
        assert_eq!(p["capabilities"]["experimentalApi"], false);
        assert_eq!(p["capabilities"]["requestAttestation"], false);
    }

    /// End-to-end against a mock stdio server (portable /bin/sh), exercising
    /// spawn → initialize handshake → request/response correlation → notification
    /// and server-request forwarding. No real engine or network required.
    #[test]
    fn mock_server_handshake_and_events() {
        use std::os::unix::fs::PermissionsExt;
        let script = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"mock/0.0","codexHome":"/tmp/mock-home","platformFamily":"unix","platformOs":"linux"}}'
      ;;
    *'"thread/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"t","turnId":"u","itemId":"i","delta":"hi"}}'
      printf '%s\n' '{"jsonrpc":"2.0","id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"t","itemId":"i"}}'
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[],"nextCursor":null}}'
      ;;
  esac
done
"#;
        let mock_path =
            std::env::temp_dir().join(format!("youragent-mock-{}.sh", std::process::id()));
        std::fs::write(&mock_path, script).expect("write mock");
        std::fs::set_permissions(&mock_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod mock");

        let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_events = Arc::clone(&events);
        let sink: EventSink = Arc::new(move |v| sink_events.lock().unwrap().push(v));

        let client = EngineClient::start(
            mock_path.to_str().unwrap(),
            "/tmp/youragent-mock-home",
            sink,
        )
        .expect("mock start");
        let _ = std::fs::remove_file(&mock_path);
        // start() ran initialize (id 1); next request gets id 2.
        let info = client.info();
        assert_eq!(info["codexHome"], "/tmp/mock-home");
        assert_eq!(info["userAgent"], "mock/0.0");

        let list = client
            .request("thread/list", json!({"limit":10}))
            .expect("thread/list");
        assert_eq!(list["data"], json!([]));

        // Give the reader thread a moment to forward the notification + request.
        thread::sleep(Duration::from_millis(100));
        let ev = events.lock().unwrap();
        assert!(ev.iter().any(|e| e["kind"] == "notification"
            && e["method"] == "item/agentMessage/delta"
            && e["params"]["delta"] == "hi"));
        assert!(ev.iter().any(|e| e["kind"] == "server_request"
            && e["method"] == "item/commandExecution/requestApproval"
            && e["requestId"] == 7));
        client.shutdown();
    }

    /// Real engine handshake. Ignored by default (needs a renamed engine binary
    /// and isolated home); run with:
    ///   YOURAGENT_ENGINE_PATH=/tmp/youragent-engine \
    ///   YOURAGENT_CODEX_HOME=/tmp/youragent-codex-home \
    ///   cargo test real_engine_handshake -- --ignored
    #[test]
    #[ignore]
    fn real_engine_handshake() {
        let path = std::env::var("YOURAGENT_ENGINE_PATH").expect("YOURAGENT_ENGINE_PATH");
        let home = std::env::var("YOURAGENT_CODEX_HOME").expect("YOURAGENT_CODEX_HOME");
        let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_events = Arc::clone(&events);
        let sink: EventSink = Arc::new(move |v| sink_events.lock().unwrap().push(v));
        let client = EngineClient::start(&path, &home, sink).expect("real start");
        let info = client.info();
        assert!(info["userAgent"]
            .as_str()
            .unwrap()
            .contains("youragent-desktop"));
        let reported_home = info["codexHome"].as_str().unwrap();
        let canon = |p: &str| {
            std::fs::canonicalize(p)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(canon(reported_home), canon(&home));
        // Security: the engine must use the isolated home, never the user's ~/.codex.
        assert!(!reported_home.ends_with("/.codex"));
        let list = client
            .request("thread/list", json!({"limit":5}))
            .expect("thread/list");
        assert!(list["data"].is_array());
        client.shutdown();
    }
}
