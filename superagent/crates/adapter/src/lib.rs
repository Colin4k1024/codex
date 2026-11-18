//! SuperAgent stdio adapter (P2-2 / #79).
//!
//! The sole protocol boundary between the orchestrator and the codex engine
//! (an app-server speaking JSON-RPC over stdio). Product-side Rust port of the
//! proven P0-6 (#69) reference client `superagent/tests/spike_client.py`:
//!
//! - argv-array spawn (no shell), stdin/stdout/stderr all PIPE; stdout carries
//!   JSONL frames only, stderr is collected separately in a bounded ring.
//! - Byte-metered frames: single-frame limit 16 MiB (production default);
//!   oversize frames fault the transport with an explicit error — never a
//!   fabricated success.
//! - Per-run transcript log: 100 MiB retained limit with rotation (oldest
//!   segment dropped). Single artifact limit: 20 MiB.
//! - Continuous three-way event pump: response / notification / server-request.
//!   Request correlation by id; unbounded event queue (overload delays, never
//!   silently drops — terminal states always survive).
//! - Typed approval handler: default-deny on every known approval type;
//!   unknown server requests get JSON-RPC error -32601. SECURITY INVARIANT:
//!   the adapter NEVER auto-approves.
//! - Optional remote environment (P5-1 / #95): `environment/add` registration
//!   with an optional bearer token (redacted from the transcript log),
//!   `environment/info` capability discovery (cached), and the
//!   `thread/start.environments` entry. No local fallback: registration
//!   failure is an explicit error to the caller.
//! - Per-type timeouts (initialize 30s, environment/add 60s,
//!   environment/info 60s, thread/* 60s, turn/start 30s,
//!   turn/interrupt 30s, command/exec 90s, terminate 30s, fallback 60s),
//!   all clamped by a global run deadline. Model-task terminal waits and
//!   approval `waiting_input` waits use the run deadline; command/exec waits
//!   for program exit.
//! - UI coalescing: text deltas batched every 30–100 ms; terminals/approvals
//!   immediate.
//! - Crash detection: pending requests fail explicitly, in-flight turns get
//!   synthetic terminal events, and orphan-aware reconciliation classifies
//!   the outcome (P0-6 trace D parity).
//!
//! No codex-rs core dependencies, no model/network code. Tests are model-free
//! fault injection against a fake stdio engine (`tests/fake_engine.py`) with
//! SCALED thresholds injected through [`AdapterConfig`] — the same enforcement
//! mechanism runs at production limits.

pub mod adapter;
pub mod approval;
pub mod config;
pub mod error;
pub mod frame;
pub mod reconcile;
pub mod runlog;
pub mod ui;

pub use adapter::{write_artifact_with_limit, Adapter, AdapterStats, ExitInfo};
pub use approval::{
    ApprovalHandler, Decision, DecisionRecord, DecliningApprovalHandler, ServerRequest,
};
pub use config::{
    AdapterConfig, EnvironmentConfig, TimeoutPolicy, COALESCE_INTERVAL_RANGE_MS,
    DEFAULT_COALESCE_INTERVAL_MS, DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_FRAME_BYTES,
    DEFAULT_MAX_LOG_BYTES, DEFAULT_STDERR_RING_LINES,
};
pub use error::AdapterError;
pub use reconcile::{classify, is_terminal_status, reconcile_thread, Disposition, ReconcileReport};
pub use ui::{AdapterEvent, CoalescerStats, UiCoalescer, UiEvent};
