//! P4-5 / #94 acceptance tests: server-side identity/param/permission/idempotency
//! validation, read-only query end-to-end, write authorization+audit+anti-replay,
//! per-app tenant/channel identity, and the MCP-approval != business-authorization
//! boundary. All model-free over an in-memory fixture business backend + IAM.

use serde_json::json;
use superagent_business_mcp::{BusinessError, BusinessMcpServer, ParamType, ToolCall, ToolKind};
use superagent_iam::{scan_for_secrets, IamError};

const SECRET: &str = "business-mcp-signing-key";
const NOW: i64 = 1_700_000_000;

/// Server with three connected apps (xiaohai / HCode / HR), each its own tenant
/// + channel identity, a read tool and a write tool, and one seeded record each.
fn server() -> BusinessMcpServer {
    let mut s = BusinessMcpServer::new(SECRET);
    s.register_app("xiaohai", "t-xiaohai", "lark", "ext-x", "u-x", "member")
        .unwrap();
    s.register_app("hcode", "t-hcode", "slack", "ext-h", "u-h", "member")
        .unwrap();
    s.register_app("hr", "t-hr", "webhook", "ext-r", "u-r", "viewer")
        .unwrap();
    s.register_tool(
        "query_record",
        ToolKind::Read,
        vec![("record_id".into(), ParamType::String)],
    );
    s.register_tool(
        "update_record",
        ToolKind::Write,
        vec![
            ("record_id".into(), ParamType::String),
            ("value".into(), ParamType::String),
        ],
    );
    s.seed_record("xiaohai", "r1", json!({"name": "客户A", "tier": "gold"}));
    s.seed_record("hcode", "r1", json!({"repo": "core", "lang": "rust"}));
    s.seed_record("hr", "r1", json!({"employee": "E001", "dept": "eng"}));
    s
}

fn token(s: &BusinessMcpServer, user: &str) -> String {
    s.iam().issue_token(user, NOW, 3600).unwrap()
}

fn read_call(app: &str, record_id: &str) -> ToolCall {
    ToolCall {
        app: app.into(),
        tool: "query_record".into(),
        params: json!({"record_id": record_id}),
        idempotency_key: None,
        tool_approved: true,
    }
}

fn write_call(app: &str, record_id: &str, key: Option<&str>, approved: bool) -> ToolCall {
    ToolCall {
        app: app.into(),
        tool: "update_record".into(),
        params: json!({"record_id": record_id, "value": "v"}),
        idempotency_key: key.map(String::from),
        tool_approved: approved,
    }
}

#[test]
fn readonly_query_end_to_end_app_onboarding() {
    let mut s = server();
    // App onboarding: channel identity resolves to the IAM user/tenant.
    let u = s.iam().map_channel_identity("lark", "ext-x").unwrap();
    assert_eq!(u.user_id, "u-x");
    assert_eq!(u.tenant, "t-xiaohai");

    // Real read-only business query end-to-end through the gateway API path:
    // identity -> tenant boundary -> params -> permission -> fixture backend.
    let t = token(&s, "u-x");
    let res = s.call_tool(&t, NOW, &read_call("xiaohai", "r1")).unwrap();
    assert_eq!(res.value, json!({"name": "客户A", "tier": "gold"}));
    assert!(!res.replayed);
    assert!(s
        .audit()
        .iter()
        .any(|e| e["event"] == "call_ok" && e["app"] == "xiaohai"));
}

#[test]
fn server_side_identity_param_permission_validation() {
    let mut s = server();
    let t = token(&s, "u-x");

    // Identity: a malformed token is rejected server-side.
    let err = s
        .call_tool("garbage", NOW, &read_call("xiaohai", "r1"))
        .unwrap_err();
    assert!(matches!(err, BusinessError::Iam(IamError::MalformedToken)));

    // Params: missing required field.
    let mut bad = read_call("xiaohai", "r1");
    bad.params = json!({});
    assert!(matches!(
        s.call_tool(&t, NOW, &bad).unwrap_err(),
        BusinessError::InvalidParams(_)
    ));

    // Params: wrong type (record_id must be a string).
    let mut bad = read_call("xiaohai", "r1");
    bad.params = json!({"record_id": 123});
    assert!(matches!(
        s.call_tool(&t, NOW, &bad).unwrap_err(),
        BusinessError::InvalidParams(_)
    ));

    // Unknown tool / unknown app.
    let mut bad = read_call("xiaohai", "r1");
    bad.tool = "nope".into();
    assert!(matches!(
        s.call_tool(&t, NOW, &bad).unwrap_err(),
        BusinessError::UnknownTool(_)
    ));
    let mut bad = read_call("ghost", "r1");
    bad.app = "ghost".into();
    assert!(matches!(
        s.call_tool(&t, NOW, &bad).unwrap_err(),
        BusinessError::UnknownApp(_)
    ));
}

#[test]
fn write_idempotency_anti_replay() {
    let mut s = server();
    let t = token(&s, "u-x");

    let r1 = s
        .call_tool(&t, NOW, &write_call("xiaohai", "r1", Some("k1"), true))
        .unwrap();
    assert!(!r1.replayed);
    assert_eq!(s.write_count(), 1);

    // Same idempotency key: deduped, NO re-execution, identical result.
    let r2 = s
        .call_tool(&t, NOW, &write_call("xiaohai", "r1", Some("k1"), true))
        .unwrap();
    assert!(r2.replayed);
    assert_eq!(r2.value, r1.value);
    assert_eq!(s.write_count(), 1);
    assert!(s.audit().iter().any(|e| e["event"] == "write_replayed"));

    // A different key executes a new side effect.
    let r3 = s
        .call_tool(&t, NOW, &write_call("xiaohai", "r1", Some("k2"), true))
        .unwrap();
    assert!(!r3.replayed);
    assert_eq!(s.write_count(), 2);
}

#[test]
fn write_requires_idempotency_key() {
    let mut s = server();
    let t = token(&s, "u-x");
    let err = s
        .call_tool(&t, NOW, &write_call("xiaohai", "r1", None, true))
        .unwrap_err();
    assert!(matches!(err, BusinessError::MissingIdempotencyKey));
    assert_eq!(s.write_count(), 0);
}

#[test]
fn write_authorization_and_audit_hygiene() {
    let mut s = server();
    // HR app user is a viewer: Artifact Write is not granted.
    let viewer = token(&s, "u-r");
    let err = s
        .call_tool(&viewer, NOW, &write_call("hr", "r1", Some("k"), true))
        .unwrap_err();
    assert!(matches!(err, BusinessError::Iam(IamError::Forbidden(_))));
    assert_eq!(s.write_count(), 0);

    // member writes are authorized and audited.
    let member = token(&s, "u-x");
    s.call_tool(&member, NOW, &write_call("xiaohai", "r1", Some("k"), true))
        .unwrap();
    assert_eq!(s.write_count(), 1);
    assert!(s.audit().iter().any(|e| e["event"] == "write_executed"));

    // Audit never leaks the signing secret or tokens.
    let raw = serde_json::to_string(s.audit()).unwrap();
    assert!(scan_for_secrets(&raw, &[SECRET, &member, &viewer]).is_empty());
}

#[test]
fn per_app_tenant_channel_identity_isolation() {
    let mut s = server();
    // Each app maps its own channel identity to its own tenant.
    assert_eq!(
        s.iam()
            .map_channel_identity("slack", "ext-h")
            .unwrap()
            .tenant,
        "t-hcode"
    );
    assert_eq!(
        s.iam()
            .map_channel_identity("webhook", "ext-r")
            .unwrap()
            .tenant,
        "t-hr"
    );

    // xiaohai's token cannot call HCode's tools: hard per-app tenant boundary.
    let t = token(&s, "u-x");
    let err = s.call_tool(&t, NOW, &read_call("hcode", "r1")).unwrap_err();
    assert!(matches!(err, BusinessError::CrossTenant(_)));

    // HCode's own token reads its own record fine.
    let th = token(&s, "u-h");
    let res = s.call_tool(&th, NOW, &read_call("hcode", "r1")).unwrap();
    assert_eq!(res.value, json!({"repo": "core", "lang": "rust"}));
}

#[test]
fn mcp_tool_approval_does_not_replace_business_authorization() {
    let mut s = server();
    // viewer + engine-side tool_approved=true STILL denied business write.
    let viewer = token(&s, "u-r");
    let err = s
        .call_tool(&viewer, NOW, &write_call("hr", "r1", Some("k"), true))
        .unwrap_err();
    assert!(matches!(err, BusinessError::Iam(IamError::Forbidden(_))));
    assert_eq!(s.write_count(), 0);

    // member + tool_approved=false STILL allowed: business authorization is
    // independent of the engine sandbox approval flag.
    let member = token(&s, "u-x");
    s.call_tool(&member, NOW, &write_call("xiaohai", "r1", Some("k"), false))
        .unwrap();
    assert_eq!(s.write_count(), 1);
}
