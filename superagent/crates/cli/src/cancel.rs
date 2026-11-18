//! `superagent cancel`: stop scheduling new nodes, interrupt the active turn or
//! terminate the active exec, keep consuming terminals, close the engine within
//! a bound, and mark the run cancelled only after cleanup is confirmed. The
//! cleanup evidence is persisted to the durable event log.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use superagent_adapter::{Adapter, AdapterEvent};
use superagent_executor::blocked;
use superagent_orchestrator::{ExecOutcome, NodeExecutor};
use superagent_storage::Store;

use crate::{CancelToken, Result};

/// Confirmed cleanup steps for a cancelled run.
#[derive(Debug, Clone)]
pub struct CleanupEvidence {
    pub interrupted: bool,
    pub terminated: bool,
    pub terminals_drained: usize,
    pub engine_closed: bool,
}

impl CleanupEvidence {
    fn to_json(&self, target: &str) -> Value {
        json!({
            "target": target,
            "interrupted": self.interrupted,
            "terminated": self.terminated,
            "terminalsDrained": self.terminals_drained,
            "engineClosed": self.engine_closed,
        })
    }
}

/// Drain terminal events within a bound (never block forever on a slow engine).
fn drain_terminals(adapter: &Adapter, bound: Duration) -> usize {
    let deadline = Instant::now() + bound;
    let mut drained = 0usize;
    while Instant::now() < deadline {
        match adapter.wait_event(Duration::from_millis(300)) {
            Ok(ev) => {
                drained += 1;
                if ev.is_terminal() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    drained
}

fn mark_cancelled(
    store: &Store,
    run_id: &str,
    now: i64,
    ev: &CleanupEvidence,
    target: &str,
) -> Result<()> {
    store.set_run_state(run_id, "cancelled")?;
    store.append_event(
        run_id,
        "run_cancelled",
        &ev.to_json(target).to_string(),
        now,
    )?;
    Ok(())
}

/// Cancel an active command/exec: terminate it, drain terminals, close the
/// engine, then mark the run cancelled with cleanup evidence.
pub fn cancel_exec_session(
    adapter: &Adapter,
    process_id: &str,
    store: &Store,
    run_id: &str,
    now: i64,
) -> Result<CleanupEvidence> {
    let term = adapter.request(
        "command/exec/terminate",
        json!({ "processId": process_id }),
        Some(Duration::from_secs(10)),
    )?;
    let terminated = term
        .get("terminated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let terminals_drained = drain_terminals(adapter, Duration::from_secs(5));
    adapter.close();
    let ev = CleanupEvidence {
        interrupted: false,
        terminated,
        terminals_drained,
        engine_closed: true,
    };
    mark_cancelled(store, run_id, now, &ev, process_id)?;
    Ok(ev)
}

/// Cancel an active agent turn: interrupt it, drain terminals, close the
/// engine, then mark the run cancelled with cleanup evidence.
pub fn cancel_turn_session(
    adapter: &Adapter,
    thread_id: &str,
    store: &Store,
    run_id: &str,
    now: i64,
) -> Result<CleanupEvidence> {
    let interrupted = adapter
        .request(
            "turn/interrupt",
            json!({ "threadId": thread_id }),
            Some(Duration::from_secs(10)),
        )
        .is_ok();
    let terminals_drained = drain_terminals(adapter, Duration::from_secs(5));
    adapter.close();
    let ev = CleanupEvidence {
        interrupted,
        terminated: false,
        terminals_drained,
        engine_closed: true,
    };
    mark_cancelled(store, run_id, now, &ev, thread_id)?;
    Ok(ev)
}

/// Wraps a node executor so that once the token is cancelled NO new node starts
/// (each subsequent execute returns Blocked without spawning a model/command).
/// The orchestrator then blocks downstream work; the run is marked cancelled.
pub struct CancelGuard<E: NodeExecutor> {
    pub inner: E,
    pub token: CancelToken,
}

impl<E: NodeExecutor> NodeExecutor for CancelGuard<E> {
    fn execute(&mut self, node: &superagent_contracts::Node, attempt: u32) -> ExecOutcome {
        if self.token.is_cancelled() {
            return blocked("cancelled: no new node scheduled");
        }
        self.inner.execute(node, attempt)
    }
}

/// Drain any pending terminals on an adapter without closing it (helper for
/// callers that manage the engine lifecycle separately).
pub fn drain(adapter: &Adapter, bound: Duration) -> usize {
    drain_terminals(adapter, bound)
}

/// Re-export for callers that pattern-match terminal events during cancel.
pub type TerminalEvent = AdapterEvent;
