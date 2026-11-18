# P0-8 / #71 裁决：子代理外部可观测性 — SUFFICIENT_NO_FORK（F-1 不触发）

日期：2026-11-17（整改重跑）· 数据源：p0-2 已提交真实团队 turn transcript
（feat/p0-2-mav2-roles:superagent/evidence/p0-2/team-turn-transcript.json）+ 本次公开 RPC 实测探针。
复现：`python3 superagent/tests/p0_8.py`（探针不需要模型调用；transcript 缺失时才跑 fresh team turn）。

## 观测面实测（event-samples.json、verdict.json）
- 24 个子代理相关 item：SubAgentActivity kind started×6 / interacted×4 / completed×6，
  CollabAgentToolCall wait×8（含 agentsStates 拓扑快照）。
- 生命周期三段全观测：spawnObserved / activityObserved / terminalObserved = true。
- 4 个 agentThreadId（leader + planner/implementer/verifier）；逐个 thread/read 成功
  → **childHistoryReadableViaPublicRpc = true**（子代理完整历史经公开 RPC 可读）。
- thread/items/list 服务端 UNSUPPORTED_SERVER_SIDE（P0-9 探针结论）；leader 历史以 rollout 文件为准。
- hooks 补充面：HookEventName::SubagentStart/SubagentStop 在协议中**静态存在**
  （hooks-events.txt，含源文件路径：protocol/src/hooks.rs:30-32,132-133、
  protocol/src/protocol.rs:1841-1842、core/src/hook_runtime.rs:139-149,384-414）。
  本 spike **未**追踪运行时 hook 触发，仅证明协议存在性。

## 控制面评估
- 生成的 ClientRequest schema 全量枚举 **110 个方法**（no-agent-rpc.txt）：
  **agent/* = 0，graph/store = 0**；turn/* 含 turn/interrupt；thread/* 含 thread/resume。
- agent-graph-store 负面证据（store-negative-evidence.txt）：app-server 内仅内部接线
  （message_processor.rs:414、mcp_refresh.rs:575 将 core 的
  local_agent_graph_store_from_state_db，core/src/thread_manager.rs:640，传入会话管理器），
  无任何公开 RPC 读写 store；事件流已携带等价拓扑信息，产品无需直读 store。
- **拓扑读取方式（修正归因）**：拓扑经 SubAgentActivity items +
  CollabAgentToolCall.agentsStates（每代理 threadId）+ 子线程 thread/read 重建。
  **thread/list 不枚举代理线程**：probes["thread/list"].agentThreadsListed = []（实测）。
- 外部中断：turn/interrupt 取消整个 leader turn。运行时实证见
  interrupt-cross-evidence.json（引自 feat/p0-6-stdio-contract:superagent/evidence/p0-6/summary.json
  trace C，PR #108：interrupt 前 sleep 子进程存在、interrupt 后 pgrep 为空）。
  精确到单个子代理的中断由 leader 经 interrupt_agent 模型工具执行（编排器经 turn 输入指令驱动）。
- **resume 评估（静态代码评估，非运行时 trace）**：thread/resume
  （app-server/src/request_processors/thread_processor.rs:640 → thread_resume_inner:3925）
  可将任意持久化线程（含子代理线程）冷恢复为**独立顶层会话**，不会把子代理重新挂回
  运行中 leader 的代理树。树内单代理 resume 由 leader 的 resume_agent 工具承担
  （仅 MAv1；MAv2 无 resume 工具，P0-2 实证）。外部编排器如需"不经 leader 的树内
  单代理 resume"，属 F-1 形态新增；MVP 不需要（leader 中介 resume 已够）。

## 裁决
**充分（不改 fork）**：外部编排器仅凭 thread 事件/items + thread/read + turn/interrupt
即可观测子代理生命周期与历史、并实施取消。最小 RPC 提案清单为空。
F-1（#101）触发条件不成立 → #101 以"未触发"关闭。
若未来需要外部直接 interrupt/resume 单个子代理（不经 leader），再按 #101 形态重开。

## 审查整改记录
首轮 grok 审查 CHANGES_REQUESTED（4 项）：①补 resume 评估；②补 store 负面证据；
③修正 topology 对 thread/list 的错误归因；④hooks 证据补源路径并标注静态存在性。
本次整改全部落实，另补 interrupt 跨 issue 运行时证据指针与全量方法枚举（110），
证据 JSON 统一 PII scrub（installationId/serverName/codexHome/authMode/planType →
<scrubbed>；主机名/用户名 → <host>/<user>）。