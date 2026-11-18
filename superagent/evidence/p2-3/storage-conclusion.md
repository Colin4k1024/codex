# P2-3 / #80 状态库（storage）验收结论

状态：**实现完成 + 16/16 集成测试 PASS（model-free）+ 依赖纯净**。合并待 #78 → #72。

## 1. 交付
独立 crate `superagent/crates/storage`（`superagent-storage` v0.1.0，edition 2021，`[workspace]` 独立根）。
依赖：`rusqlite`(bundled SQLite)、`serde`、`serde_json`、`superagent-contracts`(path)。
**零 codex-* crate 依赖、零 codex_ 符号、零进程/模型执行**；与引擎 rollout schema 完全分离（独立 DB 文件）。

## 2. 验收标准逐条映射
| 验收 | 实现 | 测试 |
|---|---|---|
| 并发租约竞争被拒绝 | `acquire_lease` 条件 UPDATE（owner 空/过期/同 owner 才成功），0 行→`LeaseHeld`；WAL+busy_timeout=5000 跨连接串行 | `exclusive_lease_contention_rejected`、`expired_lease_can_be_taken_over`、`lease_on_missing_run_is_not_found` |
| 事务一致性 | `persist_intent` 独立事务提交（intent_persisted）；`bind_external_ids` 另一事务（bound）；UNIQUE(run,node,attempt_no) | `intent_persists_then_binds_in_separate_transactions`、`fault_between_persist_and_bind_leaves_recoverable_intent`、`fault_after_persist_breakpoint_is_injectable`、`duplicate_attempt_no_rejected` |
| 发送/落盘断点可注入（供 #84） | `Breakpoint{AfterIntentPersist,BeforeExternalBind,BeforeEventAppend}` + `set_fault_hook`/`clear_fault_hook`，在真实提交边界调用，错误显式上抛不吞 | 上述两个 fault 测试 + `event_append_breakpoint_injectable` |

## 3. 任务清单映射
1. **SQLite 迁移 runs/nodes/attempts/events/artifacts**：`SCHEMA_V1` 五表 + 索引；`migrate()` 以 `user_version` pragma 升级；`schema_version()` 读回。测试 `migration_is_idempotent_and_versions`（重开不丢数据）。
2. **run 固定身份**：`RunSpec{workflow_digest,bundle_digest,input_summary,repo_identity,start_head,config_summary}`；`create_run` 写入、`get_run` 读回。测试 `run_fixed_identity_round_trips`（逐字段）、`duplicate_run_id_rejected`。
3. **attempt thread/turn/process 关联 + intent 先落盘后绑定**：`attempts.thread_id/turn_id/process_id`；`persist_intent`→`bind_external_ids`→`send_intent`（串联+断点）。崩溃在两提交之间留下可恢复未绑定 intent（`unbound_intents` 扫描 + 重绑）。
4. **排他租约 + 事件自增序号 + 有界摘要**：`events.seq AUTOINCREMENT` 单调，`events_after` 重放（`event_sequence_is_monotonic_and_replayable`）；`MAX_SUMMARY_BYTES=4096` 硬上限，超限 `SummaryTooLarge` 显式拒绝（`oversize_summaries_rejected_explicitly`，含恰好上限接受）；artifact 仅存 path/digest/size/schema_ref/bounded summary，blob 落文件（`artifact_reference_stored_without_blob`）。

## 4. 测试结果
`cargo test --release`：**16 passed; 0 failed**（evidence/p2-3/test-output.log）。
依赖纯净：`cargo tree` 无 codex-* crate（evidence/p2-3/purity-scan.txt；grep "codex" 仅命中仓库路径字符串）。
构建告警仅环境性（~/.cargo/config 弃用）+ ts-rs 解析 contracts 的 deny_unknown_fields（来自依赖 crate，非本 crate 代码）；storage 代码无 dead_code/unused 告警。

## 5. 诚实边界（NOT PROVEN / 范围外）
- 本 crate 为纯状态库，单元/集成测试 model-free，不依赖模型配额。
- 与真实引擎进程/rollout 的端到端集成属 #79(adapter)/#81(orchestrator)；跨进程真实并发与四断点故障注入属 #84。本 issue 不声称已端到端验证；**断点机制已就绪供 #84 驱动**。
- 并发租约测试以同文件多连接（WAL）模拟竞争，已覆盖排他/续租/过期接管/释放语义。

## 6. 复现
```
cd superagent/crates/storage
cargo test --release        # 16 passed
cargo tree --prefix depth   # 无 codex-* 依赖
```

## 7. 合并约束
堆叠 base=`feat/p2-1-contracts`（#78，APPROVE r2c）。合并顺序 **#72 → #78 → #80**。