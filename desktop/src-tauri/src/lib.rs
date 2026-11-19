//! YourAgent desktop host. The WebView only touches product commands; it never
//! spawns processes, reads auth files, or sends arbitrary engine RPC.
//!
//! U02 (HARNESS_CLAUDECODEUI_MIGRATION.md): self-contained mock DesktopClient.
//! These commands return fixture data so the derived ClaudeCodeUI frontend boots
//! into the workspace WITHOUT the upstream Express/WebSocket backend. They are the
//! typed product boundary; U03 replaces the fixtures with the real Codex app-server
//! adapter. The generic `desktop_request` is a graceful-degradation stub for
//! endpoints not yet migrated to typed commands — it is NOT a fake Express API and
//! makes no claim of real backend behavior.

mod bridge;
mod engine;
mod workspace;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use engine::EngineClient;
use serde_json::{json, Value};
use tauri::Emitter;

const MOCK_TOKEN: &str = "desktop-mock-token";
const MOCK_USER: &str = "demo";

/// Lazily-connected real Codex app-server bridge (U03). None until the frontend
/// requests a connection or the engine is unavailable; commands fall back to the
/// U02 fixtures so the shell still boots without an engine configured.
struct EngineState(Mutex<Option<Arc<EngineClient>>>);

/// Tracks the active turn id per thread so `abort-session` can interrupt it.
struct ActiveTurns(Mutex<HashMap<String, String>>);

#[tauri::command]
fn engine_connect(app: tauri::AppHandle, state: tauri::State<EngineState>) -> Value {
    let mut guard = state.0.lock().unwrap();
    if let Some(client) = guard.as_ref() {
        return json!({ "connected": true, "info": client.info() });
    }
    let path = match engine::resolve_engine_path() {
        Some(p) => p,
        None => {
            return json!({ "connected": false, "reason": "engine not configured (set YOURAGENT_ENGINE_PATH)" });
        }
    };
    let home = engine::resolve_codex_home();
    let emitter = app.clone();
    let sink: engine::EventSink = Arc::new(move |v| {
        let _ = emitter.emit(engine::EVENT_NAME, v);
    });
    match EngineClient::start(&path, &home, sink) {
        Ok(client) => {
            let info = client.info();
            *guard = Some(client);
            json!({ "connected": true, "info": info })
        }
        Err(e) => json!({ "connected": false, "reason": e }),
    }
}

#[tauri::command]
fn engine_status(state: tauri::State<EngineState>) -> Value {
    match state.0.lock().unwrap().as_ref() {
        Some(client) => json!({ "connected": true, "info": client.info() }),
        None => json!({ "connected": false }),
    }
}

#[tauri::command]
fn engine_disconnect(state: tauri::State<EngineState>) -> Value {
    if let Some(client) = state.0.lock().unwrap().take() {
        client.shutdown();
    }
    json!({ "connected": false })
}

/// Resolve a server approval/elicitations request with an explicit user decision.
/// Never called automatically — only from a typed frontend action.
#[tauri::command]
fn respond_to_request(state: tauri::State<EngineState>, request_id: Value, result: Value) -> Value {
    match state.0.lock().unwrap().as_ref() {
        Some(client) => match client.respond(request_id, result) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e }),
        },
        None => json!({ "ok": false, "error": "engine not connected" }),
    }
}

fn fixture_project() -> Value {
    json!({
        "name": "superagent",
        "displayName": "SuperAgent",
        "fullPath": workspace::FIXTURE_WORKSPACE_ROOT,
        "path": workspace::FIXTURE_WORKSPACE_ROOT,
        "sessions": [fixture_session()],
        "codexSessions": [fixture_session()],
        "cursorSessions": [],
        "geminiSessions": [],
        "sessionMeta": { "total": 1, "hasMore": false },
        "taskmaster": { "hasTaskmaster": false }
    })
}

fn fixture_session() -> Value {
    json!({
        "id": "session-welcome",
        "title": "Welcome to YourAgent",
        "summary": "Self-contained desktop session (U02 mock).",
        "name": "Welcome",
        "createdAt": "2026-11-18T00:00:00Z",
        "created_at": "2026-11-18T00:00:00Z",
        "updated_at": "2026-11-18T00:00:00Z",
        "lastActivity": "2026-11-18T00:00:00Z",
        "messageCount": 0,
        "__provider": "codex",
        "__projectName": "superagent"
    })
}

#[tauri::command]
fn bootstrap() -> Value {
    json!({
        "schemaVersion": 1,
        "product": "YourAgent",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": "codex-app-server",
        "gateway": "reused (not engine-direct)",
        "user": { "username": MOCK_USER },
        "token": MOCK_TOKEN,
        "hasCompletedOnboarding": true,
        "workspaces": [fixture_project()],
        "account": { "connected": true },
        "pendingApprovals": 0
    })
}

#[tauri::command]
fn auth_status() -> Value {
    json!({ "needsSetup": false, "isAuthenticated": true })
}

#[tauri::command]
fn login(username: String, _password: String) -> Value {
    json!({ "token": MOCK_TOKEN, "user": { "username": username } })
}

#[tauri::command]
fn register(username: String, _password: String) -> Value {
    json!({ "token": MOCK_TOKEN, "user": { "username": username } })
}

#[tauri::command]
fn auth_user() -> Value {
    json!({ "user": { "username": MOCK_USER } })
}

#[tauri::command]
fn logout() -> Value {
    json!({})
}

#[tauri::command]
fn onboarding_status() -> Value {
    json!({ "hasCompletedOnboarding": true })
}

#[tauri::command]
fn complete_onboarding() -> Value {
    json!({})
}

#[tauri::command]
fn git_config() -> Value {
    json!({ "gitName": "YourAgent", "gitEmail": "agent@youragent.local" })
}

#[tauri::command]
fn update_git_config(_git_name: String, _git_email: String) -> Value {
    json!({})
}

#[tauri::command]
fn list_projects() -> Value {
    json!([fixture_project()])
}

#[tauri::command]
fn list_sessions(
    project_name: String,
    limit: i64,
    _offset: i64,
    state: tauri::State<EngineState>,
) -> Value {
    let client = state.0.lock().unwrap().clone();
    if let Some(client) = client {
        match client.request("thread/list", json!({ "limit": limit.max(1) })) {
            Ok(resp) => {
                let sessions: Vec<Value> = resp
                    .get("data")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|t| bridge::thread_to_session(t, &project_name))
                            .collect()
                    })
                    .unwrap_or_default();
                let has_more = resp
                    .get("nextCursor")
                    .map(|c| !c.is_null())
                    .unwrap_or(false);
                let total = sessions.len();
                return json!({ "sessions": sessions, "total": total, "hasMore": has_more });
            }
            Err(e) => return json!({ "sessions": [], "total": 0, "hasMore": false, "error": e }),
        }
    }
    json!({ "sessions": [fixture_session()], "total": 1, "hasMore": false })
}

#[tauri::command]
fn get_messages(session_id: String, state: tauri::State<EngineState>) -> Value {
    let client = state.0.lock().unwrap().clone();
    if let Some(client) = client {
        match client.request(
            "thread/items/list",
            json!({ "threadId": session_id, "limit": 200 }),
        ) {
            Ok(resp) => return json!(bridge::items_response_to_messages(&resp, &session_id)),
            Err(e) => return json!({ "error": e }),
        }
    }
    // U02 fallback: empty conversation renders the chat empty state safely.
    json!([])
}

#[tauri::command]
fn send_input(
    message: Value,
    state: tauri::State<EngineState>,
    active: tauri::State<ActiveTurns>,
) -> Value {
    let client = match state.0.lock().unwrap().clone() {
        Some(c) => c,
        None => return json!({ "accepted": true }),
    };
    let mtype = message.get("type").and_then(Value::as_str).unwrap_or("");
    match mtype {
        "codex-command" => {
            let text = message.get("command").and_then(Value::as_str).unwrap_or("");
            let opts = message.get("options").cloned().unwrap_or_else(|| json!({}));
            let cwd = opts.get("cwd").and_then(Value::as_str).unwrap_or("");
            let session_id = message
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let reusable = !session_id.is_empty()
                && !session_id.starts_with("temp")
                && !session_id.starts_with("pending");
            let thread_id = if reusable {
                session_id.to_string()
            } else {
                let cwd_val = if cwd.is_empty() {
                    Value::Null
                } else {
                    json!(cwd)
                };
                match client.request("thread/start", json!({ "cwd": cwd_val })) {
                    Ok(r) => r["thread"]["id"].as_str().unwrap_or("").to_string(),
                    Err(e) => return json!({ "accepted": false, "error": e }),
                }
            };
            if thread_id.is_empty() {
                return json!({ "accepted": false, "error": "could not resolve thread" });
            }
            let input = json!([{ "type": "text", "text": text, "text_elements": [] }]);
            match client.request(
                "turn/start",
                json!({ "threadId": thread_id, "input": input }),
            ) {
                Ok(r) => {
                    let turn_id = r["turn"]["id"].as_str().unwrap_or("").to_string();
                    active
                        .0
                        .lock()
                        .unwrap()
                        .insert(thread_id.clone(), turn_id.clone());
                    json!({ "accepted": true, "threadId": thread_id, "turnId": turn_id })
                }
                Err(e) => json!({ "accepted": false, "error": e }),
            }
        }
        "abort-session" => {
            let sid = message
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let turn_id = active.0.lock().unwrap().get(sid).cloned();
            match turn_id {
                Some(tid) => match client
                    .request("turn/interrupt", json!({ "threadId": sid, "turnId": tid }))
                {
                    Ok(_) => json!({ "accepted": true }),
                    Err(e) => json!({ "accepted": false, "error": e }),
                },
                None => json!({ "accepted": false, "error": "no active turn for session" }),
            }
        }
        "claude-permission-response" | "permission-response" => {
            let req_id = message.get("requestId").cloned().unwrap_or(Value::Null);
            let decision = resolve_decision(&message);
            match client.respond(req_id, json!({ "decision": decision })) {
                Ok(()) => json!({ "accepted": true }),
                Err(e) => json!({ "accepted": false, "error": e }),
            }
        }
        _ => json!({ "accepted": true }),
    }
}

/// Extract an approval decision from a frontend permission-response message.
/// Accepts a string decision, a top-level `allow` bool, or `decision.allow`.
fn resolve_decision(message: &Value) -> Value {
    if let Some(d) = message.get("decision").and_then(Value::as_str) {
        return json!(d);
    }
    let allow = message.get("allow").and_then(Value::as_bool).or_else(|| {
        message
            .get("decision")
            .and_then(|d| d.get("allow"))
            .and_then(Value::as_bool)
    });
    json!(match allow {
        Some(true) => "accept",
        _ => "decline",
    })
}

#[tauri::command]
fn cancel_turn(thread_id: String, turn_id: String, state: tauri::State<EngineState>) -> Value {
    match state.0.lock().unwrap().clone() {
        Some(client) => match client.request(
            "turn/interrupt",
            json!({ "threadId": thread_id, "turnId": turn_id }),
        ) {
            Ok(_) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "error": e }),
        },
        None => json!({ "ok": false, "error": "engine not connected" }),
    }
}

// ─── U04 workspace file + read-only Git commands ─────────────────────────────
// All paths are confined to a backend-resolved workspace root (workspace.rs).
// Git is read-only; write operations are disabled in the frontend desktop build.

#[tauri::command]
fn list_files(project_name: String) -> Value {
    workspace::list_files(&project_name)
}

#[tauri::command]
fn read_file(project_name: String, file_path: String) -> Value {
    workspace::read_file(&project_name, &file_path)
}

#[tauri::command]
fn save_file(
    project_name: String,
    file_path: String,
    content: String,
    expected_mtime_ms: Option<i64>,
) -> Value {
    workspace::save_file(&project_name, &file_path, &content, expected_mtime_ms)
}

#[tauri::command]
fn git_status(project_name: String) -> Value {
    workspace::git_status(&project_name)
}

#[tauri::command]
fn git_diff(project_name: String, file_path: String) -> Value {
    workspace::git_diff(&project_name, &file_path)
}

#[tauri::command]
fn git_branches(project_name: String) -> Value {
    workspace::git_branches(&project_name)
}

#[tauri::command]
fn git_remote_status(project_name: String) -> Value {
    workspace::git_remote_status(&project_name)
}

#[tauri::command]
fn git_commits(project_name: String, limit: i64) -> Value {
    workspace::git_commits(&project_name, limit)
}

#[tauri::command]
fn git_commit_diff(project_name: String, commit: String) -> Value {
    workspace::git_commit_diff(&project_name, &commit)
}

#[tauri::command]
fn git_file_with_diff(project_name: String, file_path: String) -> Value {
    workspace::git_file_with_diff(&project_name, &file_path)
}

#[tauri::command]
fn desktop_request(_method: String, endpoint: String, _body: Value) -> Value {
    // Graceful-degradation stub for unmigrated endpoints. Returns an empty list
    // for collection-like paths and an empty object otherwise, so secondary
    // features degrade rather than crash. NOT a real backend implementation.
    let list_like = [
        "files",
        "sessions",
        "projects",
        "templates",
        "tasks",
        "branches",
        "changes",
    ];
    if list_like.iter().any(|k| endpoint.contains(k)) {
        json!([])
    } else {
        json!({})
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(EngineState(Mutex::new(None)))
        .manage(ActiveTurns(Mutex::new(HashMap::new())))
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            auth_status,
            login,
            register,
            auth_user,
            logout,
            onboarding_status,
            complete_onboarding,
            git_config,
            update_git_config,
            list_projects,
            list_sessions,
            get_messages,
            send_input,
            desktop_request,
            engine_connect,
            engine_status,
            engine_disconnect,
            respond_to_request,
            cancel_turn,
            list_files,
            read_file,
            save_file,
            git_status,
            git_diff,
            git_branches,
            git_remote_status,
            git_commits,
            git_commit_diff,
            git_file_with_diff
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_provisions_self_contained_session() {
        let b = bootstrap();
        assert_eq!(b["schemaVersion"], 1);
        assert_eq!(b["product"], "YourAgent");
        assert_eq!(b["token"], MOCK_TOKEN);
        assert_eq!(b["user"]["username"], MOCK_USER);
        assert_eq!(b["hasCompletedOnboarding"], true);
        assert_eq!(b["workspaces"][0]["name"], "superagent");
        assert_eq!(b["workspaces"][0]["sessions"][0]["id"], "session-welcome");
    }

    #[test]
    fn auth_status_is_authenticated_without_setup() {
        let s = auth_status();
        assert_eq!(s["needsSetup"], false);
        assert_eq!(s["isAuthenticated"], true);
    }

    #[test]
    fn list_projects_embeds_sessions() {
        let p = list_projects();
        let arr = p.as_array().expect("projects array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["codexSessions"][0]["__provider"], "codex");
        assert_eq!(arr[0]["sessionMeta"]["total"], 1);
    }

    #[test]
    fn resolve_decision_string_passthrough() {
        assert_eq!(
            resolve_decision(&json!({ "decision": "accept" })),
            json!("accept")
        );
        assert_eq!(
            resolve_decision(&json!({ "decision": "acceptForSession" })),
            json!("acceptForSession")
        );
    }

    #[test]
    fn resolve_decision_allow_bool() {
        assert_eq!(resolve_decision(&json!({ "allow": true })), json!("accept"));
        assert_eq!(
            resolve_decision(&json!({ "allow": false })),
            json!("decline")
        );
        assert_eq!(
            resolve_decision(&json!({ "decision": { "allow": true } })),
            json!("accept")
        );
    }

    #[test]
    fn resolve_decision_defaults_to_decline() {
        // Never auto-accept: an absent/ambiguous decision declines.
        assert_eq!(resolve_decision(&json!({})), json!("decline"));
        assert_eq!(
            resolve_decision(&json!({ "decision": {} })),
            json!("decline")
        );
    }

    #[test]
    fn desktop_request_degrades_collections_to_empty_list() {
        assert_eq!(
            desktop_request("GET".into(), "/api/files".into(), json!({})),
            json!([])
        );
        assert_eq!(
            desktop_request("GET".into(), "/api/user/preferences".into(), json!({})),
            json!({})
        );
    }
}
