# P2-7 / #84 recovery — 实现结论（恢复核心 + 真实 adapter 运行时故障验收）

日期：2026-11-18 · crate：superagent-recovery v0.1.0（独立 [workspace]，edition 2021）
分支：feat/p2-7-recovery-main · 基线：origin/main da2db3903（含 contracts#78/storage#80/orchestrator#81/adapter#79/executor#82）

## 交付
确定性恢复/对账核心，基于已证明的 storage(#80)/orchestrator(#81)/contracts(#78)。
核心零 codex-* 依赖、零 codex_ 符号、无进程 spawn、无模型执行（purity-scan.txt：cargo tree 无 codex）。
外部确认输入（thread/read）由 adapter 层提供；核心只对给定确认做裁决。
**新增**：`tests/adapter_faults.rs` 把恢复核心桥接到**真实 #79 adapter**（model-free fake_engine
子进程，真实 stdio JSON-RPC 传输），证明运行时故障验收。dev-dependency 引入 superagent-adapter
（仅测试用；核心 lib 仍不依赖 adapter/引擎）。

## 验收映射（model-free 核心，22 测试）
| 任务 | 实现 | 测试 |
|---|---|---|
| 独占 resume：租约+包/输入/工作目录/artifact 摘要核对 | exclusive_resume()：acquire_lease + RunIdentity 六字段逐一核对；不匹配释放租约拒绝 | exclusive_resume_matching_identity_succeeds / mismatch_refused_and_lease_released / lease_contention_rejected / missing_run_not_found |
| 原成功节点仅产物匹配且依赖未变才复用 | resume_plan()：Succeeded 仅当 artifact digest 匹配且所有依赖 Reuse 才 Reuse；依赖变更传递失效 | resume_reuses_succeeded_with_matching_digest / reruns_when_artifact_digest_changed / invalidates_downstream_transitively / succeeded_without_artifact_digest_not_reused |
| uncertain 对账：无法证明则等待，不自动重放 | reconcile_unconfirmed()：Unproven→Wait；ConfirmedExecuted→BindSucceeded（不重放） | reconcile_unproven_waits / confirmed_executed_binds_no_replay / resume_uncertain_yields_wait |
| 非幂等节点不盲目重试 | 非幂等 ConfirmedNotExecuted→Wait（仅幂等允许 Retry） | reconcile_non_idempotent_never_blind_retry / confirmed_not_executed_idempotent_retry |
| 四断点注入 | RecoveryBreakpoint{BeforeSend,AfterSendBeforeResponse,AfterResponseBeforeBind,AfterGateBeforeCommit}→storage hooks | recovery_breakpoints_map_to_storage_hooks / before_send_fault_aborts_clean_no_durable_intent / fault_after_send_before_response_leaves_recoverable_intent / fault_after_response_before_bind_recoverable / after_gate_before_commit_fault_recoverable_not_lost / all_four_breakpoints_have_fault_coverage |
| 崩溃恢复不重复已确认成功节点 | resume_plan 复用已确认 Succeeded（digest 匹配），Uncertain 等待 | crash_recovery_does_not_repeat_confirmed_success |
| 缺失/失败依赖处理 | 缺失 prior→Rerun；依赖 Failed→下游 Rerun | resume_missing_prior_yields_rerun / reruns_when_dependency_failed |

## 验收映射（真实 adapter 运行时故障，5 测试，model-free）
`tests/adapter_faults.rs` 驱动真实 Adapter + fake_engine 子进程（真实传输，非模型自报）：
| 运行时验收 | 测试 | 证明 |
|---|---|---|
| 引擎崩溃 mid-turn → 新连接 thread/read 对账 → 无法证明则 Wait，非幂等不盲重放 | crash_mid_turn_reconciles_unproven_and_never_blind_replays | crash 场景 os._exit(9)；fresh adapter reconcile_thread→Uncertain；reconcile_unconfirmed(true/false,Unproven)==Wait |
| 崩溃后 thread/read 证明已完成 → BindSucceeded，不重放已确认成功 | crash_then_proven_completed_binds_success_without_replay | fresh reconcile_thread(completed)→ReconciledTerminal→ConfirmedExecuted→BindSucceeded |
| 旧审批 request ID 不回放；审批永不自动批准 | pending_approval_never_auto_answered_and_old_id_not_replayed | DeferHandler 保持审批 pending（adapter 不发明 grant）；fresh 连接 wait_pending_server_request 超时（无回放旧 ID） |
| SessionEnd（引擎退出）alone 不等于 run 成功 | session_end_alone_never_marks_run_succeeded | ignore_eof→close→terminal fault；Uncertain 节点 resume_plan→Wait（非 Reuse/Succeeded） |
| 超长帧 → FrameViolation 终态不静默丢失 | oversize_frame_surfaces_violation_terminal_not_silently_lost | max_frame=1024 + 200000B 帧→FrameViolation terminal；recovery Wait |

慢消费者/大日志/未知事件传输由 adapter #79 自身测试覆盖（acceptance.rs）；恢复核心对这些终态
（FrameViolation/EngineExited/TurnTerminal）一律不判成功，Uncertain→Wait。

## 测试结果
27/27 PASS（test-output.log）：22 model-free 核心 + 5 真实 adapter 故障。
全 6 crate CI 门禁（fmt/clippy -D warnings/test，toolchain 1.99.1）CLEAN。

## grok r1 整改（model-free 可修项，已并入）
- major（四断点测试覆盖）：drive_send_with_fault / drive_gate_commit_with_fault 驱动器 +
  BeforeSend（清洁中止无 durable intent）与 AfterGateBeforeCommit（终态事件不静默丢失、可重新 append）测试。四断点全覆盖。
- minor（RunIdentity 字段映射）：doc 明确六字段语义；节点级 artifact 摘要核对由 resume_plan current_artifacts digest 门控。

## 诚实边界
- 运行时故障验收以**真实 adapter 传输 + model-free fake_engine**证明（崩溃/审批/超长帧/SessionEnd/对账），
  非真实模型崩溃；这满足 #84「传输故障 + 恢复裁决」验收（恢复逻辑与模型无关，确认输入由 adapter 提供）。
- 真实模型端到端崩溃恢复属 #82/#85 集成范畴；本 crate 不声称已证明真实模型崩溃。
- 核心 lib  purity 保持：无 codex 依赖/符号、无进程 spawn、无模型调用（adapter 仅 dev-dep）。