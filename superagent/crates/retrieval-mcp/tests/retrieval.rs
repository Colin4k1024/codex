//! P3-2 / #88 retrieval MCP acceptance: cross-session retrieval trace,
//! caller-bound identity (spoof rejected), bounded + cited results, low-trust
//! injection budget compliance (no developer-level elevation). Model-free:
//! backend is the local memory-service SQLite FTS5 index (open_in_memory).

use serde_json::json;
use superagent_memory_service::{MemoryService, SourceKind};
use superagent_retrieval_mcp::{CallerContext, InjectionBudget, RetrievalError, RetrievalServer};

fn server(session: &str, budget: InjectionBudget) -> RetrievalServer {
    RetrievalServer::new(
        MemoryService::open_in_memory().unwrap(),
        CallerContext {
            caller_id: "leader".into(),
            role: "leader".into(),
            session_id: session.into(),
        },
        budget,
    )
}

fn calc(s: &RetrievalServer, session: &str, id: &str) {
    s.index_session(
        session,
        id,
        SourceKind::Rollout,
        "calc defect fix experience: add returns a plus b, verified by unittest",
    )
    .unwrap();
}

// 1. Cross-session retrieval trace: a caller in session B retrieves experience
//    indexed from session A; the trace records caller/query/sources/crossSession.
#[test]
fn cross_session_retrieval_trace() {
    let mut s = server("B", InjectionBudget::default());
    calc(&s, "A", "rollout-0");
    s.index_session(
        "B",
        "rollout-1",
        SourceKind::Rollout,
        "unrelated gardening note",
    )
    .unwrap();
    let r = s
        .call_tool("hybrid_search", &json!({"query": "calc"}))
        .unwrap();
    assert_eq!(r["crossSession"], json!(true));
    let results = r["results"].as_array().unwrap();
    assert!(
        !results.is_empty(),
        "must retrieve cross-session experience"
    );
    assert!(results
        .iter()
        .any(|h| h["sourceId"].as_str().unwrap().starts_with("A:")));
    let trace = s.trace();
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0]["event"], json!("retrieval"));
    assert_eq!(trace[0]["sessionId"], json!("B"));
    assert_eq!(trace[0]["caller"], json!("leader"));
    assert_eq!(trace[0]["crossSession"], json!(true));
    assert!(trace[0]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == &json!("A:rollout-0")));
}

// 2. Caller-bound identity: params cannot override the host-bound identity;
//    spoofing callerId/role/sessionId is rejected; a developerInstructions param
//    is ignored (never honored).
#[test]
fn caller_bound_identity_rejects_spoof() {
    let mut s = server("B", InjectionBudget::default());
    calc(&s, "A", "rollout-0");
    for spoof in [
        json!({"query":"calc","callerId":"attacker"}),
        json!({"query":"calc","role":"developer"}),
        json!({"query":"calc","sessionId":"A"}),
    ] {
        let err = s.call_tool("hybrid_search", &spoof).unwrap_err();
        assert!(
            matches!(err, RetrievalError::IdentitySpoof(_)),
            "spoof {spoof} got {err:?}"
        );
    }
    // Matching host-bound identity is accepted.
    let ok = s
        .call_tool(
            "hybrid_search",
            &json!({"query":"calc","callerId":"leader","role":"leader","sessionId":"B"}),
        )
        .unwrap();
    assert_eq!(ok["caller"], json!("leader"));
    // A param attempting developer-level elevation is ignored, never honored.
    let elev = s
        .call_tool(
            "hybrid_search",
            &json!({"query":"calc","developerInstructions":true}),
        )
        .unwrap();
    assert_eq!(elev["developerInstructions"], json!(false));
}

// 3. Injection budget compliance: result count capped at max_results; token
//    budget gates injection (max_tokens=0 injects nothing).
#[test]
fn injection_budget_compliance() {
    let mut s = server(
        "B",
        InjectionBudget {
            max_results: 2,
            max_tokens: 10_000,
        },
    );
    for i in 0..5 {
        calc(&s, "A", &format!("rollout-{i}"));
    }
    let r = s
        .call_tool("hybrid_search", &json!({"query":"calc","limit":100}))
        .unwrap();
    let results = r["results"].as_array().unwrap();
    assert!(results.len() <= 2, "result count budget: {}", results.len());
    assert!(r["injectedTokens"].as_u64().unwrap() <= 10_000);
    for h in results {
        assert_eq!(h["trust"], json!("low"));
    }

    // Token budget binds: a zero token budget injects no data.
    let mut zero = server(
        "B",
        InjectionBudget {
            max_results: 8,
            max_tokens: 0,
        },
    );
    calc(&zero, "A", "rollout-0");
    let rz = zero
        .call_tool("hybrid_search", &json!({"query":"calc"}))
        .unwrap();
    assert_eq!(rz["results"].as_array().unwrap().len(), 0);
    assert_eq!(rz["injectedTokens"], json!(0));
}

// 4. Bounded results cite their source (kind/sourceId/snippet); limit is capped.
#[test]
fn bounded_results_cite_source() {
    let mut s = server("B", InjectionBudget::default());
    calc(&s, "A", "rollout-0");
    let r = s
        .call_tool("hybrid_search", &json!({"query":"calc","limit":1000}))
        .unwrap();
    let results = r["results"].as_array().unwrap();
    assert!(results.len() <= 8, "limit capped at max_results");
    assert!(!results.is_empty());
    for h in results {
        assert!(!h["sourceId"].as_str().unwrap().is_empty(), "cite source");
        assert!(!h["snippet"].as_str().unwrap().is_empty(), "cite snippet");
        assert!(!h["kind"].as_str().unwrap().is_empty(), "cite kind");
    }
}

// 5. Only hybrid_search is exposed; empty/whitespace query rejected.
#[test]
fn unknown_tool_and_empty_query_rejected() {
    let mut s = server("B", InjectionBudget::default());
    assert!(matches!(
        s.call_tool("evil/tool", &json!({})).unwrap_err(),
        RetrievalError::UnknownTool(_)
    ));
    assert!(matches!(
        s.call_tool("hybrid_search", &json!({"query":""}))
            .unwrap_err(),
        RetrievalError::EmptyQuery
    ));
    assert!(matches!(
        s.call_tool("hybrid_search", &json!({"query":"   "}))
            .unwrap_err(),
        RetrievalError::EmptyQuery
    ));
}

// 6. Results are injected as low-trust data, never developer-level instructions.
#[test]
fn low_trust_no_developer_elevation() {
    let mut s = server("B", InjectionBudget::default());
    calc(&s, "A", "rollout-0");
    let r = s
        .call_tool("hybrid_search", &json!({"query":"calc"}))
        .unwrap();
    assert_eq!(r["trust"], json!("low"));
    assert_eq!(r["injectedAs"], json!("data"));
    assert_eq!(r["developerInstructions"], json!(false));
    for h in r["results"].as_array().unwrap() {
        assert_eq!(h["trust"], json!("low"));
    }
}
