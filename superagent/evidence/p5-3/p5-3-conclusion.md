# P5-3 (#97) Model Routing 任务类→模型档位路由服务 — 结论

Refs #97

## 交付
`superagent/crates/model-routing`（standalone crate，serde/serde_json，模型自由/进程自由/网络自由）。`Router` 消费 model-provider 形状的 catalog（`ModelSpec{slug,tier,input_cost_per_mtok,output_cost_per_mtok,latency_ms}`）+ 可配置 `RoutingPolicy{tier_order,rules,default_rule,allowed_models,strict}`：
- **任务类→档位路由**：`route()` 按 task_class 规则选 preferred_tier 内最便宜模型（受 max_tier 上限钳制），无匹配档位时回退到上限内最便宜可用模型。
- **spawn override 兼容 + 不扩大限制**：显式 `spawn_override` 仅在 admin `allowed_models` 集合内 AND 不超过任务 max_tier 时放行（`overridden=true`）；越界 → `OverrideExceedsLimit`；未知 slug → `UnknownModel`。路由与 override 都受同一 admin 上限约束。
- **可配置可审计**：`from_config(Value)` 从 JSON 装载 catalog+policy；每次 `route()` 写 `{event:"route",taskClass,slug,tier,overridden,reason}` 审计；`record_usage()` 写 `{event:"usage",...,inputTokens,outputTokens,latencyMs,cost}` 并累计 `total_cost()`。
- **同 fixture 成本/延迟对比**：`compare_policies(catalog,fixture,policies)` 对同一组 `FixtureRun` 在多个命名策略下逐条路由，按 catalog 成本/延迟剖面计算 totalCost/totalLatencyMs + perRun，确定性对比报告。

## 验收映射（issue #97）
| 验收 | 测试（tests/routing.rs，6 integration） | 证据 |
|---|---|---|
| 路由策略可配置可审计 | `configurable_policy_audited`：from_config 装载 JSON 策略，code_fix→balanced / summarize→fast，audit 2 条 route 事件含 slug/tier/reason | test-output.log |
| 同 fixture 成本/延迟对比报告 | `cost_latency_comparison_report`：同一 3-run fixture，always-frontier=240 / task-routed=98 / always-fast=9，task-routed 成本介于两者之间、延迟低于 always-frontier，perRun 长度一致 | test-output.log |
| 与 spawn override 兼容；路由不扩大用户/管理员限制 | `spawn_override_compat_and_ceiling`（summarize 上限 balanced：frontier override 拒绝、balanced 放行、ghost 未知拒绝、research 自动选最便宜 frontier）+ `routing_does_not_expand_limits`（admin allowed_models 排除 frontier：research 回退 fast-cheap 绝不选被排除模型，frontier override 拒绝，allowed override 放行） | test-output.log |
| 记录实际选择与用量 | `usage_recorded`：record_usage 写 usage 事件 inputTokens/tier/cost=(1e6*5+1e6*10)/1e6=15.0，total_cost=15.0，未知 slug 拒绝 | test-output.log |
| 配置/查询边界 | `unknown_task_class_and_empty_config`：strict 模式未映射 task_class → UnknownTaskClass（已映射仍放行）；空 catalog → EmptyConfig | test-output.log |

## 门禁（toolchain 1.99.1，与 CI 一致）
- `cargo +1.99.1 fmt --check`：exit 0（fmt-clippy.log）
- `cargo +1.99.1 clippy --all-targets -- -D warnings`：exit 0，error-lines=0（fmt-clippy.log）
- `cargo +1.99.1 test`：0 unit + 6 integration（routing）passed / 0 failed；5 次连续全套运行均 6 passed（test-output.log）

## 边界
- 模型自由验收：路由/上限/审计/成本延迟对比均由确定性 catalog 剖面 + fixture 计算证明，不执行模型 turn、不访问网络。成本/延迟为 catalog 声明剖面的确定性计算（非真实计费 API），符合“同 fixture 对比报告”验收口径。
- 关闭门禁：grok 对最新 SHA 独立审查 APPROVE + 上述验收全部 PASS。