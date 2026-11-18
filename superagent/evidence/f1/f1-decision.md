# F-1 / #101 裁决记录：子代理观测/控制 RPC — 触发条件不成立，不实施 fork

日期：2026-11-18（定稿）· 引擎：superagent-engine · 平台：Darwin arm64 · 依据 HEAD：origin/main 5797520a5
依据：#71（P0-8）已合并证据（SUFFICIENT_NO_FORK，grok APPROVE r2）；本记录为 #101 的正式裁决交付。
前置：#65（P0-2）、#69（P0-6）已合并（#71 的声明依赖）。

## 1. 触发条件评估

#101 触发条件："#71 裁决：外部客户端仅凭 thread 事件/items 不足以观测/控制子代理"。

**#71 裁决为充分（SUFFICIENT_NO_FORK）**，触发条件不成立。关键实证
（superagent/evidence/p0-8/，均在本 HEAD 可查）：

- 观测面：24 个子代理相关 item（SubAgentActivity started×6 / interacted×4 / completed×6，
  CollabAgentToolCall wait×8 含 agentsStates 拓扑快照）；生命周期三段全观测
  （spawnObserved/activityObserved/terminalObserved=true，verdict.json）。
- 子代理历史：4 个 agentThreadId 逐个 thread/read 成功 →
  childHistoryReadableViaPublicRpc=true（event-samples.json、verdict.json）。
- 拓扑重建：SubAgentActivity items + CollabAgentToolCall.agentsStates + 子线程 thread/read；
  thread/list 不枚举代理线程（实测 agentThreadsListed=[]，如实记录）。
- 控制面：turn/interrupt 取消整个 leader turn 的运行时实证见
  superagent/evidence/p0-6/summary.json traceC（interrupt 前 sleep 子进程存在、之后 pgrep 为空）；
  单子代理精确中断由 leader 的 interrupt_agent 工具承担，**运行时已实证**：
  superagent/evidence/p0-2/p0-2b-summary.json interruptObserved=true，
  mav2-roles-conclusion.md §2 表：subAgentActivity kind=interrupted、leader 工具返回
  {"previous_status":"running"}（编排器经 turn 输入这一公开 RPC 驱动 leader 执行）。
- RPC 面枚举：ClientRequest 110 方法，agent/*=0、graph/store=0（no-agent-rpc.txt）；
  agent-graph-store 无公开 RPC 读写（store-negative-evidence.txt），事件流携带等价拓扑。

## 2. 已识别边界（如实记录，非缺口隐瞒）

- thread/items/list 服务端 UNSUPPORTED_SERVER_SIDE → 历史读取走 thread/read
  （includeTurns 弃用=接受风险，owner=编排器，P0-9 清单跟踪）。
- 外部"不经 leader 的单子代理 interrupt/resume"不可达：thread/resume 冷恢复为独立顶层会话
  （thread_processor.rs:640 → thread_resume_inner:3925，静态评估），不挂回 leader 代理树；
  MAv2 无树内 resume 工具。**MVP 不需要该能力**（leader 中介已够），故不构成触发。
- hooks SubagentStart/SubagentStop 仅静态存在性证明，未追踪运行时触发（如实标注）。

## 3. 裁决

- **不实施 fork 变更**：不新增 agent/* method family；最小 RPC 提案清单为空。
- **重开条件**：产品要求"不经 leader 的外部单代理 interrupt/resume"或等效直接控制面时，
  按 #101 原形态重开（agent/list、agent/read 只读优先，必要时 agent/interrupt、agent/resume；
  包装 AgentRegistry/ThreadManager，单 PR ≤800 行，schema 重导出 + 协议集成测试）。

## 4. 验收逐条对照

| #101 验收 | 状态 | 证据 |
|---|---|---|
| 编排器经公开 RPC 观测/控制子代理 | 满足（无需新 RPC）：观测=事件流+thread/read（运行时）；控制=turn/interrupt（运行时 traceC）+leader interrupt_agent（运行时 interruptObserved=true），编排器经 turn 输入驱动 | p0-8 verdict.json/event-samples.json；p0-6 summary.json traceC；p0-2 p0-2b-summary.json |
| schema/TS 导出更新 | 不适用：未实施 fork（形态条款仅在新增 RPC 时要求） | 本 PR diff 零引擎改动 |
| 测试通过 | 不适用：无新代码；支撑证据来自已审查合并的 #65/#69/#71 探针 | 各 PR grok-review.json |

## 5. 诚实性声明

本记录不声称存在 agent/* 公开 RPC（实测为 0）；不声称 thread/items/list 可用（实测 UNSUPPORTED）；
resume 评估为静态代码评估而非运行时 trace，均已如实标注。全部引用证据来自已合并、已独立审查的 PR。