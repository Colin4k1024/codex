//! Crash reconciliation (P0-6 trace D parity, model-free). After an engine
//! crash mid-turn, a fresh connection reads the thread's last turn status.
//! Disposition is orphan-aware: a terminal status alone is NOT proof the
//! execution ended if an orphaned child process may remain — that case is
//! `Uncertain` (await operator decision, no blind replay).

use serde_json::Value;

use crate::adapter::Adapter;
use crate::error::AdapterError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Turn status terminal AND no orphaned child process remains.
    ReconciledTerminal,
    /// Cannot prove execution outcome; await operator decision, no blind replay.
    Uncertain { reason: String },
}

pub fn is_terminal_status(status: &str) -> bool {
    matches!(status, "completed" | "interrupted" | "failed" | "aborted")
}

/// Pure disposition logic (trace D): terminal + orphan-free => reconciled;
/// anything else => uncertain.
pub fn classify(last_turn_status: Option<&str>, orphan_present: bool) -> Disposition {
    match last_turn_status {
        Some(status) if is_terminal_status(status) && !orphan_present => {
            Disposition::ReconciledTerminal
        }
        Some(status) if is_terminal_status(status) => Disposition::Uncertain {
            reason: format!(
                "turn status '{status}' terminal but orphaned child process still present; \
                 execution outcome not provably terminated; await operator decision, no blind replay"
            ),
        },
        other => Disposition::Uncertain {
            reason: format!(
                "cannot prove execution outcome (last turn status: {other:?}); \
                 await operator decision, no blind replay"
            ),
        },
    }
}

#[derive(Debug, Clone)]
pub struct ReconcileReport {
    pub thread_id: String,
    pub last_turn_status: Option<String>,
    pub disposition: Disposition,
}

/// Reconcile over a FRESH adapter connection: `thread/read` with turns, then
/// orphan-aware classification. `orphan_present` comes from the orchestrator's
/// process census (the adapter itself never spawns engine children).
pub fn reconcile_thread(
    adapter: &Adapter,
    thread_id: &str,
    orphan_present: bool,
) -> Result<ReconcileReport, AdapterError> {
    let read = adapter.request(
        "thread/read",
        serde_json::json!({"threadId": thread_id, "includeTurns": true}),
        None,
    )?;
    let status = read
        .get("thread")
        .and_then(|t| t.get("turns"))
        .and_then(Value::as_array)
        .and_then(|turns| turns.last())
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ReconcileReport {
        thread_id: thread_id.to_string(),
        disposition: classify(status.as_deref(), orphan_present),
        last_turn_status: status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_orphan_free_is_reconciled() {
        for s in ["completed", "interrupted", "failed", "aborted"] {
            assert_eq!(classify(Some(s), false), Disposition::ReconciledTerminal);
        }
    }

    #[test]
    fn terminal_with_orphan_is_uncertain() {
        assert!(matches!(
            classify(Some("failed"), true),
            Disposition::Uncertain { .. }
        ));
    }

    #[test]
    fn non_terminal_or_missing_is_uncertain() {
        assert!(matches!(
            classify(Some("inProgress"), false),
            Disposition::Uncertain { .. }
        ));
        assert!(matches!(
            classify(None, false),
            Disposition::Uncertain { .. }
        ));
    }
}
