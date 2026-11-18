//! AgentExecutor (P2-5 / #82): one model turn for a capability node.
//!
//! A completed turn is EXECUTION END, not success. The executor writes a
//! digest-backed turn-result artifact (threadId / turnId / status / usage /
//! produced files) and returns it as evidence; the #81 orchestrator decides
//! node success from that evidence. A failed/aborted turn is a retryable
//! failure; a transport fault / crash mid-turn is Uncertain (no blind replay).
//!
//! The turn runs under an explicit engine `sandboxPolicy` (P0-7 / #70 enforced
//! inside the engine). An inexpressible policy returns [`ExecKind::Blocked`] —
//! never a host-shell / dangerFullAccess fallback.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{json, Value};
use superagent_contracts::{Node, NodeKind};
use superagent_orchestrator::{ExecKind, ExecOutcome, NodeExecutor};

use crate::evidence::EvidenceRecorder;
use crate::permissions::PermissionsConfig;
use crate::{blocked, fatal, gate, uncertain, usage_from_thread_read, EngineSpec};

pub struct AgentExecutor {
    pub engine: EngineSpec,
    pub permissions: PermissionsConfig,
    pub workspace: PathBuf,
    pub evidence: EvidenceRecorder,
    /// Resolved command text for the turn. When None, derived from the node's
    /// command_ref + declared inputs (the pack command catalog resolution).
    pub task_override: Option<String>,
}

impl AgentExecutor {
    fn task_text(&self, node: &Node) -> String {
        if let Some(t) = &self.task_override {
            return t.clone();
        }
        match &node.kind {
            NodeKind::Capability {
                command_ref,
                inputs,
                outputs,
            } => format!(
                "Execute team command '{command_ref}' for node '{}'. Read inputs {inputs:?} \
                 and produce the declared output artifacts {outputs:?}. Report what you produced.",
                node.id
            ),
            _ => format!("Execute node '{}'.", node.id),
        }
    }

    fn run_turn(&self, node: &Node, attempt: u32) -> ExecOutcome {
        let policy = match self.permissions.engine_sandbox_policy() {
            Ok(p) => p,
            Err(b) => return blocked(b.0),
        };
        let start = Instant::now();
        let log = self
            .evidence
            .dir()
            .join(format!("{}-a{attempt}-engine.jsonl", node.id));
        let adapter = match self.engine.adapter(Some(log)) {
            Ok(a) => a,
            Err(e) => return fatal(format!("engine spawn failed: {e}")),
        };
        let result = self.drive_turn(&adapter, node, &policy);
        adapter.close();
        let elapsed = start.elapsed().as_secs();
        match result {
            Ok(mut outcome) => {
                outcome.elapsed_secs = elapsed;
                outcome
            }
            Err(e) => {
                let mut o = e;
                o.elapsed_secs = elapsed;
                o
            }
        }
    }

    fn drive_turn(
        &self,
        adapter: &superagent_adapter::Adapter,
        node: &Node,
        policy: &Value,
    ) -> Result<ExecOutcome, ExecOutcome> {
        adapter
            .handshake()
            .map_err(|e| fatal(format!("handshake failed: {e}")))?;
        let mut start_params = json!({ "cwd": self.workspace });
        if let Some(model) = &node.model {
            start_params["model"] = json!(model);
        }
        let ts = adapter
            .request("thread/start", start_params, None)
            .map_err(|e| fatal(format!("thread/start failed: {e}")))?;
        let thread_id = ts
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| fatal("thread/start returned no thread id"))?
            .to_string();

        let input = self.task_text(node);
        let turn_params = json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": input}],
            "sandboxPolicy": policy,
        });
        // turn/start returns promptly; the terminal state arrives as a
        // notification bounded by the run deadline (model-task wait). A fault
        // here (e.g. engine crash) leaves the turn outcome unprovable.
        adapter
            .request("turn/start", turn_params, None)
            .map_err(|e| uncertain(format!("turn/start faulted ({e}); outcome not provable")))?;

        // The engine streams `thread/tokenUsage/updated` before the terminal
        // turn notification; capture the cumulative total as those events pass
        // (thread/read does not always populate usage for this engine build).
        let usage_cell = Cell::new(0u64);
        let terminal = adapter
            .wait_notification(
                |n| {
                    let m = n.get("method").and_then(Value::as_str).unwrap_or("");
                    let tid = n
                        .get("params")
                        .and_then(|p| p.get("threadId"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if m == "thread/tokenUsage/updated" && tid == thread_id {
                        if let Some(t) = n
                            .get("params")
                            .and_then(|p| p.get("tokenUsage"))
                            .and_then(|u| u.get("total"))
                            .and_then(|t| t.get("totalTokens"))
                            .and_then(Value::as_u64)
                        {
                            if t > usage_cell.get() {
                                usage_cell.set(t);
                            }
                        }
                    }
                    matches!(m, "turn/completed" | "turn/failed" | "turn/aborted")
                        && tid == thread_id
                },
                adapter.deadline_remaining(),
            )
            .map_err(|e| {
                uncertain(format!(
                    "no terminal turn state ({e}); outcome not provable"
                ))
            })?;

        let method = terminal.get("method").and_then(Value::as_str).unwrap_or("");
        let turn = terminal.get("params").and_then(|p| p.get("turn"));
        let status = turn
            .and_then(|t| t.get("status"))
            .and_then(Value::as_str)
            .unwrap_or(if method == "turn/completed" {
                "completed"
            } else {
                "unknown"
            });
        let turn_id = turn
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let last_message = turn
            .and_then(|t| t.get("lastAgentMessage"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // Authoritative usage + produced items via thread/read on this thread.
        let read = adapter
            .request(
                "thread/read",
                json!({"threadId": thread_id, "includeTurns": true}),
                None,
            )
            .unwrap_or(Value::Null);
        let usage = usage_cell.get().max(usage_from_thread_read(&read));

        let turn_result = json!({
            "nodeId": node.id,
            "threadId": thread_id,
            "turnId": turn_id,
            "status": status,
            "usageTotalTokens": usage,
            "lastAgentMessage": last_message,
            "sandboxPolicy": policy,
        });
        let bytes = serde_json::to_vec_pretty(&turn_result).unwrap_or_default();
        let evidence_ref = self
            .evidence
            .record(
                Some(&node.id),
                &format!("{}-turn-result.json", node.id),
                &bytes,
                "application/json",
                None,
                Some(&format!("turn {status} thread={thread_id}")),
            )
            .map_err(|e| fatal(format!("turn evidence write failed: {e}")))?;

        if status != "completed" {
            // Execution ended but not successfully: retryable, no success evidence.
            return Ok(ExecOutcome {
                kind: ExecKind::RetryableFailure,
                tokens_used: usage,
                elapsed_secs: 0,
                gates: vec![],
                artifacts: vec![],
                detail: format!("turn {status} thread={thread_id} turn={turn_id}"),
            });
        }

        // Completed turn: execution ended. Success is the orchestrator's call
        // based on this durable artifact evidence (never bare NL "done").
        let mut outcome = ExecOutcome::completed(
            vec![gate(
                &format!("{}:turn", node.id),
                true,
                Some(evidence_ref.clone()),
                None,
            )],
            usage,
            0,
        );
        outcome.artifacts = vec![evidence_ref];
        outcome.detail = format!("turn completed thread={thread_id} turn={turn_id} usage={usage}");
        Ok(outcome)
    }
}

impl NodeExecutor for AgentExecutor {
    fn execute(&mut self, node: &Node, attempt: u32) -> ExecOutcome {
        if !matches!(node.kind, NodeKind::Capability { .. }) {
            return fatal("AgentExecutor only executes capability nodes");
        }
        self.run_turn(node, attempt)
    }
}
