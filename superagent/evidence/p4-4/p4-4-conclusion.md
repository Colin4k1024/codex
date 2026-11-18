# P4-4 (#93) cron 执行器：定时触发元智能体任务 — 结论

Refs #93

## 交付
`superagent/crates/cron`（standalone crate，serde/serde_json/tokio）。网关侧定时触发元智能体任务，调度核心确定性、模型自由：
- **定时调度**：`Schedule{Hourly,Daily{hour},Weekdays{hour},Weekly{weekday,hour}}`（镜像 plugin ScheduledTaskSummary 元数据）。`next_fire(after)` 用 civil-time 数学（epoch 日起算 weekday，1970-01-01=Thursday）计算严格大于 after 的下一个整点触发时刻。tokio interval 驱动 `run_interval(sched,clock,interval,ticks,on_fire)` 按 clock 泵 `tick(now)` 触发任务（运行时绑定层）。
- **调度定义 CRUD + 租户隔离**：`add_def/update_def/remove_def/list_defs(tenant)/get_def(tenant,id)`；key 以 `tenant/id`  scoped，list 仅返回本租户，跨租户 id 查询 → UnknownDef，重复 id → DuplicateDef。
- **misfire 策略**：`Skip` 仅补跑最近一个 missed slot；`CatchUp` 逐个补跑（上限 MAX_CATCHUP=1000）。
- **防重复触发（租约/幂等键）**：幂等键 `tenant/id/fire_time` 入 `fired` 集合；并发 tick / 重启重载后同一 slot 绝不二次触发。`export_state/import_state` 持久化 defs+next_due+fired 供重启恢复。
- **触发审计**：每次 fire 写 `{event:"fire",tenant,defId,fireTime,taskClass,idempotencyKey,misfire}`。

## 验收映射（issue #93）
| 验收 | 测试（tests/cron.rs，6 integration，含 1 tokio） | 证据 |
|---|---|---|
| 定时任务按时触发且可审计 | `schedules_fire_on_time_audited`：Hourly next_fire=T0+3600；Daily/Weekdays/Weekly 触发时刻 hour/weekday 正确且有界；tick 到点触发 1 次，audit fire 事件含 fireTime/taskClass/idempotencyKey | test-output.log |
| 并发不重复触发 | `no_duplicate_on_concurrent_tick`：tick(T0+2h)→2 fires；同 now 再 tick→0；更晚 now 仅触发新 slot（不重触发旧 slot），fired_keys=3 | test-output.log |
| 重启不重复触发 | `no_duplicate_on_restart`：s1 tick→2 fires，export_state；s2 import_state 重载后同 now tick→0（fired 键去重），新 slot 触发 1 次 | test-output.log |
| tokio 调度器触发（运行时绑定） | `tokio_driver_triggers_and_dedups`（#[tokio::test]）：run_interval 固定 clock=T0+1h、3 ticks，首 tick 触发 1 次、后续 tick 幂等去重，collected=1，audit=1 | test-output.log |
| CRUD + 租户隔离 | `crud_and_tenant_isolation`：双租户同 id 共存、list 各 1、跨租户查询 UnknownDef、重复 DuplicateDef、update 生效、remove true→false | test-output.log |
| misfire 补跑/跳过 | `misfire_skip_vs_catchup`：hourly 落后 5h，Skip→1（最近 slot）、CatchUp→5（T0+1h..T0+5h） | test-output.log |

## 门禁（toolchain 1.99.1，与 CI 一致）
- `cargo +1.99.1 fmt --check`：exit 0（fmt-clippy.log）
- `cargo +1.99.1 clippy --all-targets -- -D warnings`：exit 0，error-lines=0（fmt-clippy.log）
- `cargo +1.99.1 test`：0 unit + 6 integration（cron，含 1 tokio async）passed / 0 failed；5 次连续全套运行均 6 passed（test-output.log）

## 边界
- 调度核心为确定性 civil-time 计算 + 幂等去重，模型自由/进程自由/网络自由；tokio interval 驱动以受控 clock（ManualClock）测试，证明运行时绑定按 clock 触发且跨 tick 去重，无真实时间 flake。
- “thread/turn 触发元智能体任务”的下游 thread/turn 调用由网关 #90 的 engine pool 承载；本 crate 产出 `Fire{tenant,def_id,fire_time,task_class,idempotency_key}` 触发事件作为对接契约，不在本 crate 内执行模型 turn（模型自由验收口径）。
- 关闭门禁：grok 对最新 SHA 独立审查 APPROVE + 上述验收全部 PASS。