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

use serde_json::{json, Value};

const MOCK_TOKEN: &str = "desktop-mock-token";
const MOCK_USER: &str = "demo";

fn fixture_project() -> Value {
    json!({
        "name": "superagent",
        "displayName": "SuperAgent",
        "fullPath": "/Users/fanjia/Desktop/code/codex",
        "path": "/Users/fanjia/Desktop/code/codex",
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
fn list_sessions(_project_name: String, _limit: i64, _offset: i64) -> Value {
    json!({ "sessions": [fixture_session()], "total": 1, "hasMore": false })
}

#[tauri::command]
fn get_messages(_session_id: String) -> Value {
    // U02 mock: empty conversation renders the chat empty state safely. Fixture
    // message payloads (NormalizedMessage shape) land with the U03 real adapter.
    json!([])
}

#[tauri::command]
fn send_input(_message: Value) -> Value {
    // U02 mock: accept input; real turn execution + events arrive in U03.
    json!({ "accepted": true })
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
            desktop_request
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
    fn list_sessions_paginates_fixture() {
        let s = list_sessions("superagent".into(), 20, 0);
        assert_eq!(s["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(s["total"], 1);
        assert_eq!(s["hasMore"], false);
    }

    #[test]
    fn get_messages_empty_renders_chat_empty_state() {
        assert_eq!(get_messages("session-welcome".into()), json!([]));
    }

    #[test]
    fn send_input_accepts() {
        assert_eq!(send_input(json!({ "text": "hi" }))["accepted"], true);
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
