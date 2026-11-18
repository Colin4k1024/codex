//! Typed server-request (approval) handling. SECURITY INVARIANT: the adapter
//! NEVER auto-approves. The default handler declines every typed approval
//! (mirroring `p0_6.py decline_handler`, the proven P0-6 contract) and answers
//! any unknown server request with JSON-RPC error -32601. Granting anything
//! requires an explicit, caller-supplied handler decision or an explicit
//! `Adapter::respond_server_request` call — never a default.

use serde_json::{json, Value};

/// A server request received from the engine (has both `id` and `method`).
#[derive(Debug, Clone)]
pub struct ServerRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Explicit JSON-RPC result payload (caller-authored).
    Result(Value),
    /// Explicit JSON-RPC error reply.
    Error { code: i64, message: String },
    /// Hold the request open (`waiting_input`); the event pump keeps
    /// consuming. Answer later via `Adapter::respond_server_request`.
    Defer,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DecisionRecord {
    pub method: String,
    pub id: Value,
    pub payload: Value,
    pub deferred: bool,
    pub deferred_response: bool,
}

pub trait ApprovalHandler: Send + Sync {
    /// Must be non-blocking: return [`Decision::Defer`] for human-gated
    /// approvals so the reader pump never stalls (P0-6 trace B2).
    fn handle(&self, request: &ServerRequest) -> Decision;
}

/// Default-deny typed approval handler (P0-6 `decline_handler` parity).
#[derive(Debug, Default, Clone, Copy)]
pub struct DecliningApprovalHandler;

impl ApprovalHandler for DecliningApprovalHandler {
    fn handle(&self, request: &ServerRequest) -> Decision {
        let result = match request.method.as_str() {
            "item/commandExecution/requestApproval" => json!({"decision": "decline"}),
            "execCommandApproval" => json!({"decision": "deny"}),
            "item/fileChange/requestApproval" => json!({"decision": "decline"}),
            "applyPatchApproval" => json!({"decision": "deny"}),
            "item/permissions/requestApproval" => json!({"decision": "deny"}),
            "mcpServer/elicitation/request" => json!({"action": "decline"}),
            "item/tool/requestUserInput" => json!({"answers": {}}),
            unknown => {
                return Decision::Error {
                    code: -32601,
                    message: format!(
                        "unsupported server request: {unknown}; blocked, never auto-approved"
                    ),
                };
            }
        };
        Decision::Result(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str) -> ServerRequest {
        ServerRequest {
            id: json!(1),
            method: method.into(),
            params: json!({}),
        }
    }

    #[test]
    fn typed_approvals_all_declined() {
        let h = DecliningApprovalHandler;
        let cases = [
            (
                "item/commandExecution/requestApproval",
                json!({"decision":"decline"}),
            ),
            ("execCommandApproval", json!({"decision":"deny"})),
            (
                "item/fileChange/requestApproval",
                json!({"decision":"decline"}),
            ),
            ("applyPatchApproval", json!({"decision":"deny"})),
            (
                "item/permissions/requestApproval",
                json!({"decision":"deny"}),
            ),
            ("mcpServer/elicitation/request", json!({"action":"decline"})),
            ("item/tool/requestUserInput", json!({"answers":{}})),
        ];
        for (method, expected) in cases {
            assert_eq!(
                h.handle(&req(method)),
                Decision::Result(expected),
                "{method}"
            );
        }
    }

    #[test]
    fn unknown_server_request_is_32601_never_approved() {
        let h = DecliningApprovalHandler;
        match h.handle(&req("evil/grantEverything")) {
            Decision::Error { code, message } => {
                assert_eq!(code, -32601);
                assert!(message.contains("never auto-approved"));
            }
            other => panic!("unknown request must be -32601, got {other:?}"),
        }
    }
}
