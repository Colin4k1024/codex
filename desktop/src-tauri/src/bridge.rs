//! U03: mapping between Codex app-server protocol shapes and the derived
//! frontend's view contracts (ProjectSession list + NormalizedMessage history).
//!
//! These are pure functions so the mapping is unit-testable without an engine.
//! Unknown item types fall back to a safe text rendering rather than dropping
//! data or guessing semantics.

use serde_json::{json, Value};

/// Unix seconds → RFC3339 UTC string (frontend `new Date(...)` parses this).
pub fn iso_from_secs(secs: Option<i64>) -> String {
    match secs.and_then(|s| chrono::DateTime::from_timestamp(s, 0)) {
        Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        None => "1970-01-01T00:00:00Z".to_string(),
    }
}

/// Unix milliseconds → RFC3339 UTC string.
pub fn iso_from_ms(ms: Option<i64>) -> String {
    match ms.and_then(chrono::DateTime::from_timestamp_millis) {
        Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        None => "1970-01-01T00:00:00Z".to_string(),
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Map a protocol `Thread` to the frontend ProjectSession shape.
pub fn thread_to_session(thread: &Value, project_name: &str) -> Value {
    let id = str_field(thread, "id");
    let name = str_field(thread, "name");
    let preview = str_field(thread, "preview");
    let title = if name.is_empty() {
        preview.clone()
    } else {
        name.clone()
    };
    let created = iso_from_secs(thread.get("createdAt").and_then(Value::as_i64));
    let updated = iso_from_secs(thread.get("updatedAt").and_then(Value::as_i64));
    json!({
        "id": id,
        "title": if title.is_empty() { "Untitled".to_string() } else { title },
        "name": name,
        "summary": preview,
        "createdAt": created,
        "created_at": created,
        "updated_at": updated,
        "lastActivity": updated,
        "messageCount": 0,
        "__provider": "codex",
        "__projectName": project_name,
    })
}

/// Concatenate the text parts of a `userMessage` content array (UserInput[]).
fn user_message_text(item: &Value) -> String {
    let mut out = String::new();
    if let Some(parts) = item.get("content").and_then(Value::as_array) {
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                out.push_str(part.get("text").and_then(Value::as_str).unwrap_or(""));
            }
        }
    }
    out
}

/// Map a protocol `ThreadItem` to a frontend NormalizedMessage.
pub fn item_to_message(item: &Value, thread_id: &str, ts_ms: Option<i64>) -> Value {
    let id = str_field(item, "id");
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    let timestamp = iso_from_ms(ts_ms);
    let base = json!({
        "id": id,
        "sessionId": thread_id,
        "timestamp": timestamp,
        "provider": "codex",
    });

    let mut msg = match kind {
        "userMessage" => json!({
            "kind": "text",
            "role": "user",
            "content": user_message_text(item),
        }),
        "agentMessage" => json!({
            "kind": "text",
            "role": "assistant",
            "content": str_field(item, "text"),
        }),
        "reasoning" => {
            let summary = item
                .get("summary")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            json!({ "kind": "thinking", "text": summary })
        }
        "plan" => {
            json!({ "kind": "text", "role": "assistant", "content": str_field(item, "text") })
        }
        "commandExecution" => {
            let exit = item.get("exitCode").and_then(Value::as_i64);
            let output = str_field(item, "aggregatedOutput");
            json!({
                "kind": "tool_use",
                "toolName": "commandExecution",
                "toolId": id,
                "toolInput": { "command": str_field(item, "command"), "cwd": str_field(item, "cwd") },
                "toolResult": { "content": output, "isError": exit.map(|c| c != 0).unwrap_or(false) },
                "exitCode": exit,
            })
        }
        "fileChange" | "mcpToolCall" | "functionCallOutput" => json!({
            "kind": "tool_use",
            "toolName": kind,
            "toolId": id,
            "toolInput": item,
        }),
        // Safe fallback: preserve the raw item as text rather than dropping it.
        _ => json!({ "kind": "text", "role": "assistant", "content": item.to_string() }),
    };

    if let (Some(dst), Some(src)) = (msg.as_object_mut(), base.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    msg
}

/// Map a `thread/items/list` response (`data: ThreadItemEntry[]`) to messages.
pub fn items_response_to_messages(resp: &Value, thread_id: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(entries) = resp.get("data").and_then(Value::as_array) {
        for entry in entries {
            let item = entry.get("item").unwrap_or(entry);
            let ts = entry
                .get("completedAtMs")
                .and_then(Value::as_i64)
                .or_else(|| entry.get("startedAtMs").and_then(Value::as_i64));
            out.push(item_to_message(item, thread_id, ts));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_maps_to_session() {
        let thread = json!({
            "id": "thr_1", "name": "My task", "preview": "hello",
            "createdAt": 1_750_000_000_i64, "updatedAt": 1_750_000_100_i64
        });
        let s = thread_to_session(&thread, "superagent");
        assert_eq!(s["id"], "thr_1");
        assert_eq!(s["title"], "My task");
        assert_eq!(s["__provider"], "codex");
        assert_eq!(s["__projectName"], "superagent");
        assert!(s["createdAt"].as_str().unwrap().starts_with("2025-"));
    }

    #[test]
    fn thread_without_name_uses_preview() {
        let thread = json!({ "id": "t", "name": "", "preview": "preview text", "createdAt": 0, "updatedAt": 0 });
        assert_eq!(thread_to_session(&thread, "p")["title"], "preview text");
    }

    #[test]
    fn user_message_extracts_text() {
        let item = json!({
            "type": "userMessage", "id": "i1",
            "content": [{ "type": "text", "text": "hi " }, { "type": "text", "text": "there" }]
        });
        let m = item_to_message(&item, "thr_1", Some(1_750_000_000_000));
        assert_eq!(m["kind"], "text");
        assert_eq!(m["role"], "user");
        assert_eq!(m["content"], "hi there");
        assert_eq!(m["sessionId"], "thr_1");
        assert_eq!(m["provider"], "codex");
    }

    #[test]
    fn agent_message_maps_to_assistant_text() {
        let item = json!({ "type": "agentMessage", "id": "i2", "text": "answer" });
        let m = item_to_message(&item, "t", None);
        assert_eq!(m["kind"], "text");
        assert_eq!(m["role"], "assistant");
        assert_eq!(m["content"], "answer");
    }

    #[test]
    fn command_execution_maps_to_tool_with_exit_code() {
        let item = json!({
            "type": "commandExecution", "id": "i3", "command": "ls", "cwd": "/tmp",
            "aggregatedOutput": "file.txt", "exitCode": 0
        });
        let m = item_to_message(&item, "t", None);
        assert_eq!(m["kind"], "tool_use");
        assert_eq!(m["toolName"], "commandExecution");
        assert_eq!(m["toolInput"]["command"], "ls");
        assert_eq!(m["toolResult"]["isError"], false);
        assert_eq!(m["exitCode"], 0);
    }

    #[test]
    fn command_failure_flags_error() {
        let item = json!({ "type": "commandExecution", "id": "i", "command": "x", "exitCode": 2, "aggregatedOutput": "err" });
        assert_eq!(
            item_to_message(&item, "t", None)["toolResult"]["isError"],
            true
        );
    }

    #[test]
    fn unknown_item_falls_back_to_text() {
        let item = json!({ "type": "someFutureItem", "id": "i9", "data": 1 });
        let m = item_to_message(&item, "t", None);
        assert_eq!(m["kind"], "text");
        assert!(m["content"].as_str().unwrap().contains("someFutureItem"));
    }

    #[test]
    fn items_response_unwraps_entries() {
        let resp = json!({
            "data": [
                { "turnId": "u", "item": { "type": "agentMessage", "id": "a", "text": "x" }, "completedAtMs": 1_750_000_000_000_i64 },
                { "turnId": "u", "item": { "type": "userMessage", "id": "b", "content": [{ "type": "text", "text": "y" }] }, "startedAtMs": 1_750_000_000_000_i64 }
            ],
            "nextCursor": null
        });
        let msgs = items_response_to_messages(&resp, "t");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["content"], "x");
        assert_eq!(msgs[1]["content"], "y");
    }
}
