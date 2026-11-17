# P0-8 / #71 裁决：子代理外部可观测性 — SUFFICIENT_NO_FORK（F-1 不触发）

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 数据源：p0-2 真实团队 turn transcript

## 观测面实测（event-samples.json、verdict.json）
- 24 个子代理相关 item：SubAgentActivity kind started×6 / interacted×4 / completed×6，
  CollabAgentToolCall wait×8（含 agentsStates 拓扑快照）。
- 生命周期三段全观测：spawnObserved / activityObserved / terminalObserved = true。
- 4 个 agentThreadId（leader + planner/implementer/verifier）；逐个 thread/read 成功
  → **childHistoryReadableViaPublicRpc = true**（子代理完整历史经公开 RPC 可读）。
- hooks 补充面：HookEventName::SubagentStart/SubagentStop 存在
  （hooks-events.txt，core hooks 协议），可作进程级补充观测。

## 控制面评估
- 生成的 ClientRequest schema 中 **agent/* 方法数 = 0**（no-agent-rpc.txt）。
- 外部中断：turn/interrupt 取消整个 leader turn（P0-6 trace C 实证）；
  精确到单个子代理的中断由 leader 经 interrupt_agent 模型工具执行（编排器经 turn 输入指令驱动）。
- 拓扑：CollabAgentToolCall.agentsStates + thread/list 可读；agent-graph-store 无公开读取 RPC，
  但事件流已携带等价拓扑信息，产品无需直读 store。

## 裁决
**充分（不改 fork）**：外部编排器仅凭 thread 事件/items + thread/read + turn/interrupt
即可观测子代理生命周期与历史、并实施取消。最小 RPC 提案清单为空。
F-1（#101）触发条件不成立 → #101 以"未触发"关闭。
若未来需要外部直接 interrupt/resume 单个子代理（不经 leader），再按 #101 形态重开。