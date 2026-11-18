//! UI delivery layer (issue #79 task 5): text deltas coalesce every 30–100 ms;
//! terminal states and approvals are emitted immediately. The queue is
//! unbounded by design — overload may delay delivery but can NEVER silently
//! drop a terminal state (or any event). Deterministic tests drive a synthetic
//! clock (`now_ms`); production callers pass monotonic milliseconds.

use serde_json::Value;

use crate::config::COALESCE_INTERVAL_RANGE_MS;
use crate::error::AdapterError;

/// Events produced by the adapter pump. All variants are retained; nothing is
/// dropped under overload.
#[derive(Debug, Clone, PartialEq)]
pub enum AdapterEvent {
    /// Engine notification (includes malformed frames recorded as `_raw`).
    Notification(Value),
    /// A server request deferred to `waiting_input` (approval pending).
    ApprovalPending(Value),
    /// A stdout frame exceeded the frame limit; transport faulted.
    FrameViolation { bytes: usize },
    /// Engine process exited (crash or clean). Terminal fault signal.
    EngineExited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// Synthetic terminal for an in-flight turn when the engine died before
    /// emitting `turn/completed|failed|aborted`. Guarantees the upper layer
    /// always observes a terminal state (no silent drop on crash).
    TurnTerminal {
        thread_id: String,
        status: String,
        reason: String,
    },
}

impl AdapterEvent {
    pub fn is_terminal(&self) -> bool {
        match self {
            Self::EngineExited { .. } | Self::TurnTerminal { .. } | Self::FrameViolation { .. } => {
                true
            }
            Self::Notification(n) => matches!(
                n.get("method").and_then(Value::as_str),
                Some("turn/completed" | "turn/failed" | "turn/aborted")
            ),
            Self::ApprovalPending(_) => false,
        }
    }

    pub fn is_approval(&self) -> bool {
        matches!(self, Self::ApprovalPending(_))
    }

    fn delta_text(&self) -> Option<String> {
        let Self::Notification(n) = self else {
            return None;
        };
        let method = n.get("method").and_then(Value::as_str)?;
        if method != "item/agentMessage/delta" {
            return None;
        }
        Some(
            n.get("params")
                .and_then(|p| p.get("delta"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )
    }
}

/// What the upper layer consumes.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// Batch of coalesced text deltas (content preserved, order preserved).
    CoalescedDeltas { count: usize, text: String },
    /// Anything non-delta, delivered immediately (terminals/approvals first
    /// flush any buffered deltas so ordering is preserved).
    Immediate(AdapterEvent),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoalescerStats {
    pub deltas_seen: usize,
    pub deltas_emitted: usize,
    pub terminals_emitted: usize,
    pub approvals_emitted: usize,
    /// Always zero: the coalescer has no drop path.
    pub dropped: usize,
}

pub struct UiCoalescer {
    interval_ms: u64,
    buffer_count: usize,
    buffer_text: String,
    buffer_start_ms: Option<u64>,
    pub stats: CoalescerStats,
}

impl UiCoalescer {
    pub fn new(interval_ms: u64) -> Result<Self, AdapterError> {
        if !COALESCE_INTERVAL_RANGE_MS.contains(&interval_ms) {
            return Err(AdapterError::InvalidConfig(format!(
                "coalesce interval {interval_ms}ms outside 30..=100ms"
            )));
        }
        Ok(Self {
            interval_ms,
            buffer_count: 0,
            buffer_text: String::new(),
            buffer_start_ms: None,
            stats: CoalescerStats::default(),
        })
    }

    /// Feed one adapter event at monotonic time `now_ms`. Returns whatever
    /// must be delivered now. Terminals/approvals are always returned
    /// immediately, regardless of buffer state or load.
    pub fn push(&mut self, event: AdapterEvent, now_ms: u64) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if let Some(text) = event.delta_text() {
            // Flush an already-due batch first, then buffer the new delta.
            if self.flush_due(now_ms, &mut out) {
                // batch emitted
            }
            self.stats.deltas_seen += 1;
            if self.buffer_start_ms.is_none() {
                self.buffer_start_ms = Some(now_ms);
            }
            self.buffer_count += 1;
            self.buffer_text.push_str(&text);
            return out;
        }
        // Non-delta: preserve order by flushing buffered deltas first.
        self.flush_buffer(&mut out);
        if event.is_terminal() {
            self.stats.terminals_emitted += 1;
        }
        if event.is_approval() {
            self.stats.approvals_emitted += 1;
        }
        out.push(UiEvent::Immediate(event));
        out
    }

    /// Time-driven flush: emits the buffered delta batch once the interval
    /// has elapsed. Call periodically (e.g. every UI frame).
    pub fn tick(&mut self, now_ms: u64) -> Vec<UiEvent> {
        let mut out = Vec::new();
        self.flush_due(now_ms, &mut out);
        out
    }

    /// Force-flush remaining deltas (shutdown / end of turn).
    pub fn flush(&mut self) -> Vec<UiEvent> {
        let mut out = Vec::new();
        self.flush_buffer(&mut out);
        out
    }

    fn flush_due(&mut self, now_ms: u64, out: &mut Vec<UiEvent>) -> bool {
        match self.buffer_start_ms {
            Some(start) if now_ms.saturating_sub(start) >= self.interval_ms => {
                self.flush_buffer(out);
                true
            }
            _ => false,
        }
    }

    fn flush_buffer(&mut self, out: &mut Vec<UiEvent>) {
        if self.buffer_count == 0 {
            return;
        }
        self.stats.deltas_emitted += self.buffer_count;
        out.push(UiEvent::CoalescedDeltas {
            count: self.buffer_count,
            text: std::mem::take(&mut self.buffer_text),
        });
        self.buffer_count = 0;
        self.buffer_start_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn delta(i: usize) -> AdapterEvent {
        AdapterEvent::Notification(json!({
            "method": "item/agentMessage/delta",
            "params": {"delta": format!("d{i} ")}
        }))
    }

    #[test]
    fn interval_bounds_enforced() {
        assert!(UiCoalescer::new(29).is_err());
        assert!(UiCoalescer::new(101).is_err());
        assert!(UiCoalescer::new(30).is_ok());
        assert!(UiCoalescer::new(100).is_ok());
        assert!(UiCoalescer::new(50).is_ok());
    }

    #[test]
    fn deltas_coalesce_on_interval_terminal_immediate() {
        let mut c = UiCoalescer::new(50).unwrap();
        for i in 0..100 {
            assert!(c.push(delta(i), 10).is_empty());
        }
        assert!(c.tick(59).is_empty()); // 49ms elapsed: not due
        let batch = c.tick(60);
        assert_eq!(batch.len(), 1);
        match &batch[0] {
            UiEvent::CoalescedDeltas { count, text } => {
                assert_eq!(*count, 100);
                let expected: String = (0..100).map(|i| format!("d{i} ")).collect();
                assert_eq!(text, &expected);
            }
            other => panic!("expected coalesced batch, got {other:?}"),
        }
        // Terminal bypasses the buffer and is immediate.
        let term = AdapterEvent::Notification(json!({"method":"turn/completed","params":{}}));
        let out = c.push(term.clone(), 61);
        assert_eq!(out, vec![UiEvent::Immediate(term)]);
        assert_eq!(c.stats.terminals_emitted, 1);
        assert_eq!(c.stats.dropped, 0);
        assert_eq!(c.stats.deltas_seen, c.stats.deltas_emitted);
    }

    #[test]
    fn overload_never_drops_terminal_or_deltas() {
        let mut c = UiCoalescer::new(50).unwrap();
        // 50k deltas with NO consumer ticks (overload), then a terminal.
        for i in 0..50_000 {
            c.push(delta(i), 0);
        }
        let term = AdapterEvent::TurnTerminal {
            thread_id: "t".into(),
            status: "aborted".into(),
            reason: "engine exited".into(),
        };
        let out = c.push(term.clone(), 0);
        // Buffered deltas flush first (order), terminal delivered immediately.
        assert_eq!(out.len(), 2);
        assert!(matches!(
            out[0],
            UiEvent::CoalescedDeltas { count: 50_000, .. }
        ));
        assert_eq!(out[1], UiEvent::Immediate(term));
        assert_eq!(c.stats.dropped, 0);
        assert_eq!(c.stats.deltas_seen, 50_000);
        assert_eq!(c.stats.deltas_emitted, 50_000);
        assert_eq!(c.stats.terminals_emitted, 1);
    }

    #[test]
    fn approval_immediate_and_flushes_pending_deltas() {
        let mut c = UiCoalescer::new(50).unwrap();
        c.push(delta(0), 0);
        let approval = AdapterEvent::ApprovalPending(json!({"method":"execCommandApproval"}));
        let out = c.push(approval.clone(), 1);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], UiEvent::CoalescedDeltas { count: 1, .. }));
        assert_eq!(out[1], UiEvent::Immediate(approval));
        assert_eq!(c.stats.approvals_emitted, 1);
    }
}
