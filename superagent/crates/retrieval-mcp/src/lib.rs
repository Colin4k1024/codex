//! SuperAgent retrieval MCP server (P3-2 / #88): caller-bound hybrid search.
//!
//! Wraps the memory-service FTS5 index as an MCP `hybrid_search` tool exposed to
//! role threads. Identity is fixed by the host ([`CallerContext`]), never
//! declared by tool params; a param attempting to override it is rejected
//! (越权拒绝). Results are bounded, cite their source, and are injected as
//! LOW-TRUST data within a token budget — never as developer-level instructions.
//! Each retrieval emits a trace event (caller / query / hits / sources /
//! cross-session flag). Model-free: the only backend is the local SQLite index.

use serde_json::{json, Value};
use superagent_memory_service::{MemoryError, MemoryService, SourceKind};

#[derive(Debug)]
pub enum RetrievalError {
    IdentitySpoof(String),
    EmptyQuery,
    UnknownTool(String),
    Memory(MemoryError),
}

impl std::fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetrievalError::IdentitySpoof(e) => write!(f, "identity spoof rejected: {e}"),
            RetrievalError::EmptyQuery => write!(f, "empty query"),
            RetrievalError::UnknownTool(t) => write!(f, "unknown tool: {t}"),
            RetrievalError::Memory(e) => write!(f, "memory: {e}"),
        }
    }
}

impl std::error::Error for RetrievalError {}

impl From<MemoryError> for RetrievalError {
    fn from(e: MemoryError) -> Self {
        RetrievalError::Memory(e)
    }
}

pub type Result<T> = std::result::Result<T, RetrievalError>;

/// Host-bound caller identity, fixed at connection time. Tool params cannot set
/// or override it (the message-board-mcp caller-bound pattern).
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub caller_id: String,
    pub role: String,
    pub session_id: String,
}

/// Bounds on low-trust injection: result count + approximate token budget.
#[derive(Debug, Clone)]
pub struct InjectionBudget {
    pub max_results: usize,
    pub max_tokens: usize,
}

impl Default for InjectionBudget {
    fn default() -> Self {
        Self {
            max_results: 8,
            max_tokens: 900,
        }
    }
}

/// Model-free approximate token count (whitespace tokens) for budget gating.
fn approx_tokens(s: &str) -> usize {
    s.split_whitespace().count()
}

pub struct RetrievalServer {
    svc: MemoryService,
    caller: CallerContext,
    budget: InjectionBudget,
    trace: Vec<Value>,
}

impl RetrievalServer {
    pub fn new(svc: MemoryService, caller: CallerContext, budget: InjectionBudget) -> Self {
        Self {
            svc,
            caller,
            budget,
            trace: Vec::new(),
        }
    }

    pub fn caller(&self) -> &CallerContext {
        &self.caller
    }

    pub fn trace(&self) -> &[Value] {
        &self.trace
    }

    /// Index a session source on the host write path. The source id is prefixed
    /// with the session so retrieval can attribute hits to their origin session.
    pub fn index_session(
        &self,
        session: &str,
        source_id: &str,
        kind: SourceKind,
        content: &str,
    ) -> Result<()> {
        let sid = format!("{session}:{source_id}");
        self.svc
            .index(kind, &sid, content, &json!({"sessionId": session}))?;
        Ok(())
    }

    /// MCP tool dispatch. Only `hybrid_search` is exposed.
    pub fn call_tool(&mut self, name: &str, params: &Value) -> Result<Value> {
        match name {
            "hybrid_search" => self.hybrid_search(params),
            other => Err(RetrievalError::UnknownTool(other.to_string())),
        }
    }

    /// Reject any param that declares an identity differing from the host-bound
    /// context. Identity is never taken from params.
    fn check_identity(&self, params: &Value) -> Result<()> {
        let bound = [
            ("callerId", self.caller.caller_id.as_str()),
            ("role", self.caller.role.as_str()),
            ("sessionId", self.caller.session_id.as_str()),
        ];
        for (key, expected) in bound {
            if let Some(v) = params.get(key).and_then(Value::as_str) {
                if v != expected {
                    return Err(RetrievalError::IdentitySpoof(format!(
                        "param {key}='{v}' cannot override host-bound '{expected}'"
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn hybrid_search(&mut self, params: &Value) -> Result<Value> {
        self.check_identity(params)?;
        let query = params.get("query").and_then(Value::as_str).unwrap_or("");
        if query.trim().is_empty() {
            return Err(RetrievalError::EmptyQuery);
        }
        let requested = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(self.budget.max_results as u64) as usize;
        let limit = requested.min(self.budget.max_results).max(1);
        let hits = self.svc.search(query, limit)?;

        let mut data = Vec::new();
        let mut tokens = 0usize;
        let mut cross_session = false;
        for h in hits {
            let session = h.source_id.split(':').next().unwrap_or("");
            if session != self.caller.session_id {
                cross_session = true;
            }
            let snippet_tokens = approx_tokens(&h.snippet);
            if tokens + snippet_tokens > self.budget.max_tokens {
                break;
            }
            tokens += snippet_tokens;
            data.push(json!({
                "kind": h.kind,
                "sourceId": h.source_id,
                "snippet": h.snippet,
                "score": h.score,
                "trust": "low",
            }));
        }

        let sources: Vec<Value> = data.iter().map(|d| d["sourceId"].clone()).collect();
        let result = json!({
            "trust": "low",
            "injectedAs": "data",
            "developerInstructions": false,
            "query": query,
            "caller": self.caller.caller_id,
            "sessionId": self.caller.session_id,
            "results": data,
            "injectedTokens": tokens,
            "budget": {
                "maxResults": self.budget.max_results,
                "maxTokens": self.budget.max_tokens,
            },
            "crossSession": cross_session,
        });
        self.trace.push(json!({
            "event": "retrieval",
            "caller": self.caller.caller_id,
            "role": self.caller.role,
            "sessionId": self.caller.session_id,
            "query": query,
            "hitCount": sources.len(),
            "sources": sources,
            "crossSession": cross_session,
            "injectedTokens": tokens,
        }));
        Ok(result)
    }
}
