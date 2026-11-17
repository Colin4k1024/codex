# P0-4 / #67 结论：ext/goal 目标+预算+steering 与 ext/queue 空闲派发

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
features：goals=true、token_budget=true（config.toml [features]）

## goal API 往返（goal-transcript.json）
- thread/goal/set {objective, tokenBudget:60000} → goal{status:active, tokensUsed:0}。
- 真实修复任务 turn 后 thread/goal/get → status:complete、tokensUsed:23338、timeUsedSeconds:25
  （模型经 update_goal 工具记录进度；记账由引擎累计）。
- thread/goal/clear → {cleared:true}；全程 7 条 thread/goal/updated + 1 条 thread/goal/cleared 通知。
- 模型工具面：get_goal / create_goal / update_goal（ext/goal/src/spec.rs:9-11）。

## 预算超限行为（budget-limit.json）
- tokenBudget=500 + 长文 turn → turn 正常 completed，但 goal.status=**budgetLimited**、
  tokensUsed=21927 ≥ 500。SQL 在 tokens_used ≥ token_budget 时 active→budget_limited
  （state/src/runtime/goals.rs:296,331,625-630）。
- 超限后停止自动续跑（ext/goal/runtime.rs:366-372）。

## steering 注入
- steering 是内部 user-context 片段，**不是公开 RPC**：budgetLimited 触发一次性
  budget_limit.md 注入活动 turn（ext/goal/src/extension.rs:540-550、steering.rs:49-51,107-126），
  或作为续跑 turn 输入（steering.rs:59-71）。
- 本 spike 观测到触发条件（budgetLimited 状态迁移）成立；注入内容本身不经 wire 暴露，
  以代码路径为证。产品编排器无需自建 steering 通道。

## queue 空闲派发（queue-dispatch.json，experimentalApi=true）
- 活动 turn（sleep 15）期间 thread/queue/add 入队 → thread/queue/list 可见 1 条待派发项。
- 首个 turn/completed 后引擎自动派发队首（ext/queue/src/service.rs:440-508,588-603，
  turn_trigger:"queue"）→ 第二个 turn completed，回复精确 QUEUE_DISPATCH_OK，队列清空。
- RPC 面（均 experimental）：thread/queue/add/list/update/delete/reorder/start
  （protocol/v2/thread.rs:962-1061）；忙时 queue/start 报错"thread already has an active or pending turn"。

## 裁决
goal/预算/queue 全部经现有 API 可达，不触发 fork 变更。
queue 面需 experimentalApi capability（P0-9 清单记录）。