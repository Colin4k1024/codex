//! YourAgent desktop host. The WebView only touches product commands; it never
//! spawns processes, reads auth files, or sends arbitrary engine RPC. This
//! bootstrap command is the product-state entry point (design §5); the gateway
//! API / engine sidecar wiring lands in later P5-6 steps.

use serde_json::{json, Value};

#[tauri::command]
fn bootstrap() -> Value {
    json!({
        "schemaVersion": 1,
        "product": "YourAgent",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": "codex-app-server",
        "gateway": "reused (not engine-direct)",
        "workspaces": [],
        "account": { "connected": false },
        "pendingApprovals": 0
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![bootstrap])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}