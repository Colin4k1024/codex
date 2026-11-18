# P0-4 / #67 结论：ext/goal 目标+预算+steering 与 ext/queue 空闲派发

日期：2026-11-17（审查整改版）· 引擎：superagent-engine · 平台：Darwin arm64
features：goals=true、token_budget=true（config.toml [features]）

## goal API 往返（goal-transcript.json）
- thread/goal/set {objective, tokenBudget:60000} → goal{status:active, tokensUsed:0}。
- 真实修复任务 turn 后 thread/goal/get → status:complete、tokensUsed:23338、timeUsedSeconds:25
  （记账由引擎累计）。
- thread/goal/clear → {cleared:true}；全程 7 条 thread/goal/updated + 1 条 thread/goal/cleared 通知。
- 模型工具面：get_goal / create_goal / update_goal
  （codex-rs/ext/goal/src/spec.rs:7-13；handler 注册 codex-rs/core/src/tools/handlers/extension_tools.rs:140）。
- **模型工具往返（F1 整改）**：首轮审查指出 goal-transcript.json 未证明模型实际调用
  create_goal/get_goal/update_goal（NOT PROVEN）。新增探针 superagent/tests/p0_4c.py：
  真实 turn 指示模型依次调用三个工具，随后扫描 leader rollout JSONL 的
  function_call/function_call_output 对（spike_client.rollout_function_calls），
  要求每个工具 ≥1 次调用且带输出。运行证据 goal-tool-calls.json / p0-4c-summary.json
  回填后本节才视为已证明；在此之前模型工具往返标记为 **待证（pending p0_4c run）**。

## 预算超限行为（budget-limit.json）
- tokenBudget=500 + 长文 turn → turn 正常 completed，但 goal.status=**budgetLimited**、
  tokensUsed=21927 ≥ 500。SQL 在 tokens_used ≥ token_budget 时 active→budget_limited
  （codex-rs/state/src/runtime/goals.rs:296,331,625-630）。
- 超限后的状态迁移守卫（Active/BudgetLimited→UsageLimited 才允许 stop，否则清空活动 goal）：
  codex-rs/ext/goal/src/runtime.rs:366-372。

## steering 注入
- steering 是内部 user-context 片段，**不是公开 RPC**：budgetLimited 触发一次性
  budget_limit.md 注入活动 turn（codex-rs/ext/goal/src/extension.rs:538-552、
  codex-rs/ext/goal/src/steering.rs:49-51,107-126），
  或作为续跑 turn 输入（codex-rs/ext/goal/src/steering.rs:59-71）。
- 本 spike 观测到触发条件（budgetLimited 状态迁移）成立；注入内容本身不经 wire 暴露，
  以代码路径为证。产品编排器无需自建 steering 通道。

## queue 空闲派发（queue-dispatch.json，experimentalApi=true）
- 活动 turn（sleep 15）期间 thread/queue/add 入队 → thread/queue/list 可见 1 条待派发项。
- 首个 turn/completed 后引擎自动派发队首（codex-rs/ext/queue/src/service.rs:438-508,588-603，
  turn_trigger:"queue"）→ 第二个 turn completed，回复精确 QUEUE_DISPATCH_OK，队列清空。
- RPC 面（均 experimental，定义于 codex-rs/app-server-protocol/src/protocol/common.rs:666-701，
  #[experimental] 属性标注；**不在生成的 v2 schema JSON 中**，需 experimentalApi capability）：
  thread/queue/add/list/update/delete/reorder/start。
- 忙时拒绝（**静态代码证据，非运行时 trace**）：thread/queue/start 在
  NotIdle/PendingTriggerTurn 时返回 invalid_request "thread already has an active or pending turn"
  （codex-rs/app-server/src/request_processors/thread_queue_processor.rs:228-237）。
  首轮结论中 protocol/v2/thread.rs:962-1061 的引用有误，已更正为上述两处实际位置。

## 裁决
goal/预算/queue 全部经现有 API 可达，不触发 fork 变更。
queue 面需 experimentalApi capability（P0-9 清单记录）。
模型工具往返以 p0_4c.py 运行证据为准（见上）。

## 审查整改记录
首轮 grok CHANGES_REQUESTED：F1（blocker）模型工具往返未证明 → 新增 p0_4c.py 探针，
证据待配额恢复后运行回填；F2（minor）queue/start 忙时错误断言无证据且引用错误 →
改为静态代码证据并更正引用（common.rs:666-701、thread_queue_processor.rs:228-237）；
F3（minor）引用不可验证 → 全部改为仓库相对完整路径并逐段核对 HEAD 内容。