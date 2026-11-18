# P2-6 / #83 — CLI run/status/resume/cancel/report/dry-run

## Scope
New standalone crate `superagent/crates/cli` (bin `superagent`), a thin product
entry point over the existing service crates. No codex engine DTOs, no direct
process spawn / network in CLI source — engine access is exclusively via the
#79 adapter and #82 executor. Deterministic guarantees live in the crates.

Commands: `run <workflow.json> [--task|--task-file] [--dry-run]`,
`status <run-id> --json`, `resume`, `cancel`, `report`, `version`.
Stable exit codes: OK=0 FAILED=1 USAGE=2 BLOCKED=3 CANCELLED=4 UNCERTAIN=5.

## Acceptance mapping (issue #83 验收)
1. **dry-run 不启动模型/命令** — `plan::dry_run_plan` calls the PURE
   `contracts::dry_run` + `orchestrator::topo_order`; it never constructs an
   adapter/executor. `to_json` reports `startedModel:false, startedCommand:false,
   validated:true`. Proven by:
   - test `dry_run_starts_no_model_or_command` (asserts both flags false);
   - test `dry_run_rejects_cycle_before_execution` (DAG cycle → PlanError before
     any execution);
   - **real binary smoke** (`cli-binary-smoke.log`): `superagent run wf-smoke.json
     --dry-run` → `startedModel:false startedCommand:false validated:true`, exit 0,
     with NO engine binary present/invoked.
2. **取消有清理证据** — `cancel::cancel_exec_session` sends
   `command/exec/terminate`, drains terminals within a bound, closes the engine,
   then `mark_cancelled` persists `set_run_state("cancelled")` + a durable
   `run_cancelled` event carrying the cleanup JSON. `cancel_turn_session` does the
   same via `turn/interrupt`. `CancelGuard` schedules NO new node once the token
   is cancelled (returns Blocked). Proven by:
   - test `cancel_exec_session_cleanup_evidence_and_cancelled_mark` over GENUINE
     stdio transport (fake_engine subprocess): a live `command/exec` (sleep 30,
     caller processId, pidFile sync) is terminated (`terminated:true`), engine
     closed, run state `cancelled`, durable `run_cancelled` event present;
   - test `cancel_guard_stops_new_nodes` (PanicExecutor inner never invoked after
     cancel; outcome Blocked + "cancelled").
3. **report 内容完整** — `report::build_report` separates verified
   (`state=="Succeeded"`) vs unverified nodes, baseline vs this-run changes, and
   records version / usage (tokensUsed, wallSecs) / evidence. Proven by test
   `report_separates_verified_unverified_and_changes`.
4. **无交互模式遇所需授权返回 blocked 和稳定退出码** — a Verification node with an
   Approval gate yields `ExecKind::NeedsInput` (executor never auto-approves) →
   orchestrator `WaitingInput` → `execute_run` returns `CliError::Blocked`
   (exit 3). Proven by test
   `non_interactive_approval_returns_blocked_stable_exit`.
5. **status / resume** — `status_json` reads durable state/identity/nodes/events
   (test `status_json_reads_durable_state`); `resume_run` enforces the #84
   exclusive lease + six-field identity match (mismatch refused, lease released)
   then a per-node Reuse/Rerun/Wait plan (test
   `resume_identity_match_reuse_and_mismatch_refused`).
6. **run success only by gate evidence** — test `run_succeeds_via_real_gate_evidence`
   drives a Capability + Verification workflow over fake_engine; success requires
   a proven (evidence-backed) gate, not a model self-report.

## Test results
`cargo +1.99.1 test` (crate cli): **11 passed; 0 failed** (model-free; genuine
stdio JSON-RPC transport via fake_engine subprocess for cancel/run/approval).
`cargo +1.99.1 fmt --check` clean; `cargo +1.99.1 clippy --all-targets -- -D
warnings` clean. Full multi-crate gate (7 crates, all GATE_PASS): see
test-output.log.

## grok independent review remediation (round 2)
The first grok review of head 736cefdc0 returned CHANGES_REQUESTED (1 major +
2 minor). All three are remediated and re-proven:
- **MAJOR — resume identity tautology.** `cmd_resume` previously built the
  expected identity from the stored spec (`RunIdentity::from_spec`), so
  `exclusive_resume` compared stored-vs-itself and always matched. Fixed:
  `resume::expected_identity` now derives the expected `RunIdentity` from the
  CALLER's current inputs (loaded `--workflow` digest + `--bundle-digest` /
  `--repo-identity` / `--start-head` / `--config-summary` / task), and
  `cmd_resume` uses it. New test
  `resume_cli_expected_from_caller_inputs_refuses_changed_workflow_or_bundle`
  proves a changed workflow OR changed bundle is refused (RecoveryError) and the
  lease is released so a fresh owner can acquire it.
- **MINOR — report usage / proven-gate evidence empty.** `execute_run` now
  persists a durable `run_report` event carrying the live orchestrator report
  (state, tokensUsed, wallSecs, per-node provenGates/reason).
  `report::report_input_from_store` prefers that event and falls back to
  `node_final` states + `attempt_finished` token sum + event-log wall span, so
  `cmd_report` emits real usage/evidence rather than zeros.
- **MINOR — turn cancel unconfirmed.** fake_engine now implements
  `turn/interrupt` (marks the turn aborted, emits a terminal `turn/aborted`,
  responds `interrupted:true`). `cancel_turn_session` no longer silently marks a
  run cancelled when the interrupt is unconfirmed: it leaves the run `uncertain`
  with a distinct `run_cancel_unconfirmed` event. New test
  `cancel_turn_session_interrupts_and_marks_cancelled` proves the confirmed path
  (interrupted:true, engine closed, state cancelled, durable run_cancelled).

## Purity
`cargo tree -e normal`: only superagent-* crates (contracts/storage/orchestrator/
executor/adapter/recovery); NO codex engine crate. CLI src has no
`std::process::Command` / network / `use codex`. The `codex_home` identifiers are
the engine-home CONFIG field (CODEX_HOME path handed to the adapter), not engine
DTO coupling. See purity-scan.txt.

## Honesty note
All acceptance claims are backed by passing tests over genuine transport or by
the real binary smoke log. dry-run is pure (no engine spawned). Cancel cleanup is
confirmed before the run is marked cancelled. No model self-report is ever used
as proof of success.