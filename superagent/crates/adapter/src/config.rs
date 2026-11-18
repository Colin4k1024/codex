//! Adapter configuration: production resource limits (trace-calibrated in
//! P0-6 / #69), per-type timeout policy, spawn wiring.
//!
//! Tests inject SCALED thresholds through this same config (mirroring
//! `superagent/tests/p0_6b.py`); the enforcement mechanism is identical at
//! production limits and at scaled limits.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::AdapterError;

/// Production single-frame limit: 16 MiB (P0-6 trace-calibrated).
pub const DEFAULT_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// Production per-run log limit: 100 MiB (rotation drops the oldest segment).
pub const DEFAULT_MAX_LOG_BYTES: usize = 100 * 1024 * 1024;
/// Production single-artifact limit: 20 MiB (trace-calibrated).
pub const DEFAULT_MAX_ARTIFACT_BYTES: usize = 20 * 1024 * 1024;
/// stderr ring buffer capacity (lines), matching the reference client.
pub const DEFAULT_STDERR_RING_LINES: usize = 2000;
/// Default UI delta coalescing interval; must stay within 30..=100 ms.
pub const DEFAULT_COALESCE_INTERVAL_MS: u64 = 50;
/// Allowed coalescing interval range (issue #79 task 5).
pub const COALESCE_INTERVAL_RANGE_MS: std::ops::RangeInclusive<u64> = 30..=100;

/// Per-type request timeouts, mirroring `spike_client.Client.DEFAULT_TIMEOUTS`
/// (the proven P0-6 contract). Connection/query methods are bounded; model
/// task terminal waits and approval `waiting_input` waits use the run deadline
/// via [`crate::Adapter::wait_notification`] / `wait_pending_server_request`;
/// `command/exec` waits for program exit (bounded by the run deadline).
#[derive(Debug, Clone)]
pub struct TimeoutPolicy {
    pub by_method: BTreeMap<String, Duration>,
    pub fallback: Duration,
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        let entries = [
            ("initialize", 30),
            ("environment/add", 60),
            ("environment/info", 60),
            ("thread/start", 60),
            ("thread/read", 60),
            ("thread/list", 60),
            ("thread/resume", 60),
            ("turn/start", 30),
            ("turn/interrupt", 30),
            ("command/exec", 90),
            ("command/exec/terminate", 30),
        ];
        Self {
            by_method: entries
                .into_iter()
                .map(|(m, s)| (m.to_string(), Duration::from_secs(s)))
                .collect(),
            fallback: Duration::from_secs(60),
        }
    }
}

impl TimeoutPolicy {
    /// Explicit timeout overrides the per-type default; unknown methods use
    /// the fallback. The caller additionally clamps against the run deadline.
    pub fn effective(&self, method: &str, explicit: Option<Duration>) -> Duration {
        explicit.unwrap_or_else(|| self.by_method.get(method).copied().unwrap_or(self.fallback))
    }
}

/// Optional remote execution environment (P5-1 / #95).
///
/// Registration happens through the app-server `environment/add` RPC using
/// `id` + `exec_server_url` (+ an optional bearer token). `cwd` and
/// `workspace_roots` are ENVIRONMENT-NATIVE values sent in the `thread/start`
/// `environments` entry; when `cwd` is `None` the executor workspace is used.
///
/// There is deliberately no local fallback: when registration fails the
/// executor surfaces an explicit fatal/blocked outcome instead of running the
/// turn against the host.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EnvironmentConfig {
    pub id: String,
    pub exec_server_url: String,
    /// Raw bearer token for exec-server authentication. Never logged: the
    /// adapter redacts it on the transcript-log path and this `Debug` impl
    /// prints only a marker.
    pub auth_bearer_token: Option<String>,
    /// Environment-native working directory (`thread/start.environments[].cwd`).
    pub cwd: Option<PathBuf>,
    /// Environment-native runtime workspace roots. Empty omits the field, so
    /// the server defaults it to `cwd`.
    pub workspace_roots: Vec<PathBuf>,
}

impl fmt::Debug for EnvironmentConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentConfig")
            .field("id", &self.id)
            .field("exec_server_url", &self.exec_server_url)
            .field(
                "auth_bearer_token",
                &self.auth_bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("cwd", &self.cwd)
            .field("workspace_roots", &self.workspace_roots)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Full argv array for the engine subprocess (no shell). Tests point this
    /// at a fake engine; production uses `[engine, "app-server", "--listen",
    /// "stdio://"]`.
    pub argv: Vec<String>,
    /// CODEX_HOME passed to the engine (the only env var the contract needs).
    pub codex_home: Option<PathBuf>,
    /// Extra env vars applied on top of the inherited environment.
    pub env: BTreeMap<String, String>,
    /// Inherit the parent environment (reference client does; tests may clear).
    pub inherit_env: bool,
    pub cwd: Option<PathBuf>,
    /// Optional remote execution environment (P5-1). `None` (default) keeps
    /// every pre-P5-1 wire shape byte-for-byte unchanged.
    pub environment: Option<EnvironmentConfig>,
    /// Single stdout JSONL frame limit (bytes, excluding newline).
    pub max_frame_bytes: usize,
    /// Per-run transcript log path; rotation enabled when set.
    pub run_log_path: Option<PathBuf>,
    /// Per-run log retained-bytes limit (rotation drops oldest segment).
    pub max_log_bytes: usize,
    /// Single artifact file limit (bytes).
    pub max_artifact_bytes: usize,
    /// stderr ring buffer capacity in lines.
    pub stderr_ring_lines: usize,
    /// Global run deadline; clamps every request/notification/approval wait.
    pub run_deadline: Duration,
    pub timeouts: TimeoutPolicy,
    /// Grace period on close before SIGKILL of an unresponsive engine.
    pub close_grace: Duration,
    /// UI text-delta coalescing interval (30..=100 ms).
    pub coalesce_interval_ms: u64,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            argv: Vec::new(),
            codex_home: None,
            env: BTreeMap::new(),
            inherit_env: true,
            cwd: None,
            environment: None,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            run_log_path: None,
            max_log_bytes: DEFAULT_MAX_LOG_BYTES,
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            stderr_ring_lines: DEFAULT_STDERR_RING_LINES,
            run_deadline: Duration::from_secs(120),
            timeouts: TimeoutPolicy::default(),
            close_grace: Duration::from_secs(10),
            coalesce_interval_ms: DEFAULT_COALESCE_INTERVAL_MS,
        }
    }
}

impl AdapterConfig {
    pub fn validate(&self) -> Result<(), AdapterError> {
        if self.argv.is_empty() {
            return Err(AdapterError::InvalidConfig("argv must be non-empty".into()));
        }
        if self.max_frame_bytes == 0 || self.max_log_bytes == 0 || self.max_artifact_bytes == 0 {
            return Err(AdapterError::InvalidConfig(
                "resource limits must be > 0".into(),
            ));
        }
        if self.stderr_ring_lines == 0 {
            return Err(AdapterError::InvalidConfig(
                "stderr_ring_lines must be > 0".into(),
            ));
        }
        if !COALESCE_INTERVAL_RANGE_MS.contains(&self.coalesce_interval_ms) {
            return Err(AdapterError::InvalidConfig(format!(
                "coalesce_interval_ms {} outside 30..=100",
                self.coalesce_interval_ms
            )));
        }
        // Environment config is only checked for non-emptiness: transport
        // security (TLS vs loopback) is enforced by the app-server/exec-server,
        // not guessed here.
        if let Some(env) = &self.environment {
            if env.id.trim().is_empty() {
                return Err(AdapterError::InvalidConfig(
                    "environment.id must be non-empty".into(),
                ));
            }
            if env.exec_server_url.trim().is_empty() {
                return Err(AdapterError::InvalidConfig(
                    "environment.exec_server_url must be non-empty".into(),
                ));
            }
            if env.auth_bearer_token.as_deref() == Some("") {
                return Err(AdapterError::InvalidConfig(
                    "environment.auth_bearer_token must be non-empty when set".into(),
                ));
            }
        }
        Ok(())
    }
}
