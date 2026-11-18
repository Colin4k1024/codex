//! Explicit adapter errors. Every limit violation / fault surfaces as an
//! error here — the adapter never fabricates success (issue #79 task 3).

use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// Configuration rejected before spawn.
    InvalidConfig(String),
    /// Subprocess spawn failed.
    Spawn(String),
    /// Transport I/O failure.
    Io(String),
    /// stdin was closed / adapter shut down; no further requests possible.
    ConnectionClosed,
    /// Engine process exited while requests were in flight. Carries the real
    /// exit status; pending work fails explicitly instead of hanging.
    EngineExited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// Per-type (or explicit) request timeout elapsed.
    Timeout {
        method: String,
        id: u64,
        waited: Duration,
    },
    /// Global run deadline elapsed (clamps every wait).
    DeadlineExceeded { method: String },
    /// A single stdout frame exceeded `max_frame_bytes`. Transport is faulted;
    /// subsequent requests fail explicitly.
    FrameLimitExceeded { limit: usize, observed: usize },
    /// An artifact payload exceeded `max_artifact_bytes`; nothing was written.
    ArtifactLimitExceeded { limit: usize, observed: usize },
    /// Engine returned a JSON-RPC error object (propagated verbatim).
    Rpc { code: i64, message: String },
    /// Frame was not valid JSON (recorded as raw, pump keeps running).
    Protocol(String),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(m) => write!(f, "invalid adapter config: {m}"),
            Self::Spawn(m) => write!(f, "engine spawn failed: {m}"),
            Self::Io(m) => write!(f, "transport io error: {m}"),
            Self::ConnectionClosed => write!(f, "connection closed"),
            Self::EngineExited { code, signal } => {
                write!(f, "engine exited (code={code:?}, signal={signal:?})")
            }
            Self::Timeout { method, id, waited } => {
                write!(f, "request {method} id={id} timed out after {waited:?}")
            }
            Self::DeadlineExceeded { method } => {
                write!(f, "run deadline exceeded while waiting for {method}")
            }
            Self::FrameLimitExceeded { limit, observed } => write!(
                f,
                "frame limit exceeded: {observed} bytes > {limit} bytes limit; transport faulted"
            ),
            Self::ArtifactLimitExceeded { limit, observed } => write!(
                f,
                "artifact limit exceeded: {observed} bytes > {limit} bytes limit; not written"
            ),
            Self::Rpc { code, message } => write!(f, "engine rpc error {code}: {message}"),
            Self::Protocol(m) => write!(f, "protocol error: {m}"),
        }
    }
}

impl std::error::Error for AdapterError {}
