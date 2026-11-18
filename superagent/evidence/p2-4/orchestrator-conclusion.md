# P2-4 / #81 确定性编排器（orchestrator）验收结论

状态：**实现完成 + 18/18 集成测试 PASS（model-free）+ 依赖纯净**。合并待 #80 → #78 → #72。

## 1. 交付
独立 crate `superagent/crates/orchestrator`（`superagent-orchestrator` v0.1.0，edition 2021，`[workspace]` 独立根）。
依赖：`superagent-contracts`(path)、`superagent-storage`(path)、`serde`、`serde_json`。
**零 codex-* crate 依赖、零进程/模型执行**（`NodeExecutor` 注入）；成功仅由门禁证据判定。

## 2. 验收标准逐条映射
| 验收 | 实现 | 测试 |
|---|---|---|
| DAG 循环/失败传播 | `topo_order` Kahn 串行（确定性 tie-break）；`plan` 先 contracts `dry_run`；循环/未知/自依赖→PlanError；依赖未 Succeeded→下游 Blocked；`downstream` 传递闭包 | `dag_cycle_rejected_at_plan`、`unknown_dependency_rejected`、`failure_propagates_to_downstream`、`downstream_invalidation_is_transitive` |
| 失败 gate 阻止 succeeded | `GateOutcome::is_proven`=passed && evidence_ref 非空；Verification 需每个 required gate proven；Capability 需 proven gate 或 artifact 证据 | `failed_gate_blocks_succeeded`、`gate_pass_without_evidence_is_not_success`、`capability_requires_evidence_not_nl`、`all_gates_proven_succeeds` |
| 预算耗尽 blocked/failed | token 预算超限→Blocked（attempts=0）；wall 超限→后续 Cancelling；重试耗尽→Failed | `token_budget_exhaustion_blocks`、`wall_budget_exhaustion_cancels_remaining`、`max_attempts_exhausted_is_failed` |

## 3. 任务清单映射
1. **串行 DAG**：`topo_order`/`plan`（`topo_order_serial_and_deterministic`）。
2. **状态机**：`NodeState{Pending,Ready,Running,Verifying,Succeeded,WaitingInput,Blocked,Failed,Uncertain,Cancelling}`；逐节点转移并持久化到 #80 Store（`run_persists_events_and_intents_to_store`：intent 落盘+绑定、attempt_finished/node_final/run_final 事件、run state=succeeded）。
3. **有限重试/预算/wall_timeout**：`Policy{max_attempts=3(1+2修复),max_tokens,max_wall_secs}`；验证失败幂等且有剩余→重跑（`verification_failure_repaired_then_succeeds` attempts=2）；耗尽→Failed attempts=3。
4. **可重试才生成下一 attempt；非幂等不盲目重试**：`RetryableFailure` 仅 idempotent && attempt<max 重试；`FatalFailure` 不重试（attempts=1）；非幂等节点 RetryableFailure→Uncertain attempts=1（`non_idempotent_node_never_blindly_retried`、`fatal_failure_not_retried`）。
5. **依赖失效传播**：`downstream` 传递闭包 + 依赖未成功→Blocked（`downstream_invalidation_is_transitive`、`failure_propagates_to_downstream`）。

## 4. 测试结果
`cargo test --release`：**18 passed; 0 failed**（evidence/p2-4/test-output.log）。
superagent-ci 门禁（toolchain 1.99.1）：`cargo +1.99.1 fmt -- --check` 0 diffs、`cargo +1.99.1 clippy --all-targets -- -D warnings` clean、`cargo +1.99.1 test` 18 passed（test-output.log 已含 1.99.1 门禁输出）。合并 main 后修复两处 clippy lint：`GateOutcome::is_proven` 的 `map_or(false,..)`→`is_some_and(..)`；测试中 `Policy::default()` 后字段重赋→结构体初始化器 `Policy{..Default::default()}`（field_reassign_with_default）。
依赖纯净：`cargo tree` 无 codex-* crate（evidence/p2-4/purity-scan.txt）。storage 代码无 unused/dead_code 告警。
开发中修复两处逻辑缺陷：(a) attempt 计数在预算检查前自增→预算阻断时 attempts 应为 0（已将自增移到预算检查后）；(b) run 聚合让 Blocked 掩盖真实 Failed→改为 Failed 优先于 Blocked（失败是下游阻断的根因）。

## 5. 诚实边界（NOT PROVEN / 范围外）
- 纯确定性编排核心，`NodeExecutor` 注入，model-free；不依赖模型配额。
- 真实 AgentExecutor/VerificationExecutor 接入属 #82；adapter stdio 属 #79；恢复对账/四断点故障注入属 #84。本 issue 不声称已端到端跑真实引擎。
- wall_timeout 用逻辑时钟（executor 上报 elapsed_secs 累加），非真实墙钟；真实进程超时属 #79/#82。
- 失效传播 v1：downstream 传递闭包 + 依赖未成功→Blocked；输入/bundle 变化主动触发重算属 #84 恢复对账扩展。

## 6. 复现
```
cd superagent/crates/orchestrator
cargo test --release        # 18 passed
cargo tree --prefix depth   # 无 codex-* 依赖
```

## 7. 合并约束
#72/#78/#80 均已合并入 main；本分支已 merge 最新 main（storage 取 main 权威版本，orchestrator 为 #81 交付），base=main，可直接合并。