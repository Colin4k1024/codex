# P2-2 (#79) — stdio adapter crate: acceptance conclusion

- Crate: `superagent/crates/adapter` (`superagent-adapter`, standalone cargo
  workspace, edition 2021, deps: `serde` + `serde_json` only — same convention
  as `superagent-contracts` / P2-1).
- Branch: `feat/p2-2-adapter` (from `origin/main`).
- Reference contract ported: `superagent/tests/spike_client.py`, `p0_6.py`,
  `p0_6b.py` (P0-6 / #69, merged). No codex-rs core changes; everything under
  `superagent/`.
- Tests: **34 total, all PASS** (15 unit + 19 integration), two consecutive
  runs green; `cargo fmt --check` clean; `cargo clippy --all-targets -D warnings`
  clean. Real output: `test-output.log`. Purity: `purity-scan.txt`.
- All tests are **model-free fault injection** against a fake stdio engine
  (`tests/fake_engine.py`, deliberately not named "codex"): no real engine, no
  model, no network, no quota. Scaled thresholds (64 KiB frame / 256 KiB log /
  4 KiB artifact) are injected through `AdapterConfig` — the **same enforcement
  mechanism** runs at the documented production defaults (16 MiB / 100 MiB /
  20 MiB), mirroring `p0_6b.py`'s scaled-threshold approach.

## Acceptance mapping (criterion -> test -> result)

### Task 1 — stdio subprocess management

| Criterion | Test(s) | Result |
|---|---|---|
| argv-array spawn (no shell), arbitrary argv accepted | `spawn_argv_env_and_pipe_wiring` (engine reports its argv verbatim; `AdapterConfig.argv` is the spawn path) | PASS |
| stdin/stdout/stderr PIPE; CODEX_HOME env wiring | `spawn_argv_env_and_pipe_wiring` | PASS |
| stdout JSONL / stderr separated, stderr size-bounded (2000-line ring) | `stderr_ring_bounded` (3000-line flood -> tail <= 2000, oldest dropped, newest kept) | PASS |
| exit cleanup (EOF -> exit -> reap; unresponsive -> SIGKILL) | `exit_cleanup_reaps_child` (exit 0, reaped, no zombie), `close_kills_unresponsive_engine` (grace 300ms -> signal 9) | PASS |

### Task 2 — correlation, three-way pump, typed approvals

| Criterion | Test(s) | Result |
|---|---|---|
| request correlation by id | `request_correlation_three_way_pump` | PASS |
| continuous pump: response / notification / server-request | `request_correlation_three_way_pump` (all three ways observed on one connection) | PASS |
| typed approval handler (7 typed methods, P0-6 parity) | `typed_approval_never_auto_approves`, unit `typed_approvals_all_declined` | PASS |
| unknown server request -> -32601, NEVER auto-approved | `typed_approval_never_auto_approves` (engine echoes the actual reply: error -32601 "never auto-approved"), unit `unknown_server_request_is_32601_never_approved`; decision audit asserts no grant | PASS |
| deferred approval (`waiting_input`), pump keeps consuming | `deferred_approval_pump_keeps_consuming` (concurrent turn completes while approval held; explicit decline observed by engine) | PASS |

### Task 3 — resource limits (explicit error, never fabricate success)

| Criterion | Test(s) | Result |
|---|---|---|
| single frame 16 MiB (prod default; tested scaled 64 KiB, 200 KB frame) | `oversize_frame_faults_with_explicit_error` (violation recorded, transport faulted, subsequent requests fail explicitly), unit `oversize_line_metered_and_drained`, `exact_cap_is_allowed`; default constant asserted in `big_log_rotation_scaled_threshold` | PASS |
| per-run log 100 MiB (prod default; tested scaled 256 KiB, ~1.2 MiB traffic) | `big_log_rotation_scaled_threshold` (rotations >= 1, retained <= limit on disk), unit `rotation_drops_oldest_and_bounds_retained`, `entry_larger_than_limit_is_truncated_notice` | PASS |
| artifact single-file 20 MiB (prod default; tested scaled 4 KiB) | `artifact_limit_explicit_error` (5000 B > 4096 B -> explicit error, file NOT written; 3000 B ok) | PASS |

### Task 4 — per-type timeouts

| Criterion | Test(s) | Result |
|---|---|---|
| per-type default map = P0-6 contract (initialize 30s, thread/* 60s, turn/start 30s, turn/interrupt 30s, command/exec 90s, terminate 30s, fallback 60s) | `per_type_timeout_defaults_match_p0_6_contract` | PASS |
| connection/query bounded; explicit override | `command_exec_timeout_then_terminate_reaps_child` (500ms explicit timeout fires in <3s while `sleep 20` runs) | PASS |
| command/exec waits program exit; recovery via terminate reaps child | `command_exec_timeout_then_terminate_reaps_child` (child alive after client timeout -> terminate -> child reaped; late response reports real non-zero exit, never success) | PASS |
| model tasks use run deadline (terminal wait) | `request_correlation_three_way_pump` (terminal wait uses `deadline_remaining()`), `notification_wait_clamped_by_run_deadline` (60s wait clamped to 500ms deadline) | PASS |
| global run deadline clamps every wait | `global_deadline_clamps_long_request` (60s explicit timeout clamped to 400ms deadline) | PASS |
| approval waiting_input bounded by run deadline | `approval_waiting_input_bounded_by_deadline`, `deferred_approval_pump_keeps_consuming` | PASS |

### Task 5 — UI coalescing, no silent terminal drop

| Criterion | Test(s) | Result |
|---|---|---|
| text deltas coalesce every 30–100ms (interval bounds enforced) | unit `deltas_coalesce_on_interval_terminal_immediate`, `interval_bounds_enforced` (29/101ms rejected, 30/50/100 accepted) | PASS |
| terminal states + approvals handled immediately | unit `deltas_coalesce_on_interval_terminal_immediate`, `approval_immediate_and_flushes_pending_deltas` | PASS |
| overload must NOT silently drop terminal states | unit `overload_never_drops_terminal_or_deltas` (50k deltas, no consumer ticks, terminal delivered immediately, dropped == 0, content conserved); integration `slow_consumer_retains_all_events_and_terminal` (5000-delta flood + 300ms slow consumer: all events retained, terminal survives, coalescer conserves content) | PASS |

### Six acceptance fault cases (model-free)

| Fault case | Test | Result |
|---|---|---|
| slow consumer | `slow_consumer_retains_all_events_and_terminal` | PASS |
| oversize line (> frame limit) | `oversize_frame_faults_with_explicit_error` | PASS |
| big log (> log limit, scaled) | `big_log_rotation_scaled_threshold` | PASS |
| unknown event (+ malformed JSONL frame) | `unknown_event_and_malformed_frame_keep_pump_alive` (unknown notification retained, `_raw` recorded, pump survives, correlation still works) | PASS |
| unknown server request (-32601, never auto-approved) | `typed_approval_never_auto_approves` | PASS |
| engine crash mid-turn (reconcile, no silent terminal drop) | `engine_crash_mid_turn_explicit_terminal_and_reconcile` (exit 9 detected; pending request fails explicitly with `EngineExited`; synthetic `TurnTerminal{aborted}` delivered for the in-flight turn; fresh-connection `thread/read` reconciliation: inProgress -> Uncertain/no blind replay, failed+orphan-free -> ReconciledTerminal, failed+orphan -> Uncertain), unit `terminal_and_orphan_free_is_reconciled`, `terminal_with_orphan_is_uncertain`, `non_terminal_or_missing_is_uncertain` | PASS |

## Security invariant

The adapter NEVER auto-approves. The default handler (`DecliningApprovalHandler`)
declines/denies all 7 typed approval methods and returns JSON-RPC error -32601
for anything unknown. Granting requires an explicit caller-supplied handler
decision or an explicit `respond_server_request` payload (orchestrator policy
gate). Proven by round-trip: the fake engine echoes the exact reply it received
(`typed_approval_never_auto_approves`), plus a decision-log audit asserting no
grant ever leaked.

## Honesty / scope notes

- Production limits (16 MiB / 100 MiB / 20 MiB) are asserted as config defaults
  and enforced by the identical code path proven at scaled thresholds; the
  tests do not push 100 MiB of real traffic (same trade-off as p0_6b.py).
- Crash reconciliation is proven against the fake engine's persisted state
  (model-free). The orphan census input (`orphan_present`) is supplied by the
  orchestrator; the adapter's classification logic is proven for all four
  quadrants (terminal/non-terminal x orphan/no-orphan). Real-engine orphan
  behavior was already proven in P0-6 trace D and is out of scope here.
- The 30–100ms coalescing cadence is proven with a synthetic clock
  (deterministic); wall-clock scheduling jitter is the UI loop's concern.
- No credentials/secrets in code, tests, or evidence (purity-scan.txt §8).
- Not merged, not pushed; issue #79 left open.