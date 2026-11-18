# P0-6 结论：app-server stdio 全链路接入契约（issue #69）

日期：2026-11-17（审查整改版）· 引擎：superagent-engine（codex-cli 0.0.0，source_commit 425a848a9，sha256 见 p0-1/engine-manifest.json）· 平台：macOS arm64

## 总结论

stdio JSON-RPC 接入契约成立，**无需 fork 改动**。
已提交运行时证据：trace A/B/C/D 四条 PASS（summary.json，首轮 run）。
整改新增：trace B2（延迟审批+事件泵）、B3（fileChange 拒绝）、B4（userInput）探针代码已就绪，
**运行时 trace 待模型配额恢复后重跑回填**；资源上限与 spawn wiring 已用无模型探针 p0_6b.py 实证
（resource-limits.json allPass=true，2/2）。
复现：`python3 superagent/tests/p0_6b.py`（无模型）；`python3 superagent/tests/p0_6.py`（需模型）。

## spawn 与管道接线（blocker 整改）

- argv 数组：`[<engine>, "app-server", "--listen", "stdio://"]`；stdin/stdout/stderr 全部 PIPE，
  stdout 专用 JSONL 帧，stderr 独立收集（环形保留 2000 行）；env 仅注入 CODEX_HOME。
- 实证记录：`spawn-wiring.json`（来自活客户端 transcript，含 frameLimit/timeout 映射）；
  spawn 路径亦见 #64（PR #103，已合并）superagent/evidence/p0-1/。

## Trace A — 真实模型任务（PASS，已提交）

- `initialize → initialized → thread/start → turn/start → 事件泵 → turn/completed` 全链路走通。
- `turn/start` 立即返回（inProgress），完成以 `turn/completed` 通知为准。
- 模型真实写文件 `p06.txt`，内容逐字节等于 `SUPERAGENT_P06_OK`（无尾换行）。
- 32 条通知；单帧最大 1695 字节，16MiB 帧上限 0 违例。证据：`trace-a-transcript.json`。

## Trace B — typed 审批拒绝（PASS，已提交；decision 值整改）

- 强制审批配置：`approval_policy = "on-request"` + `sandbox_mode = "read-only"`。
  上游已移除 `approval_policy = "untrusted"`（core/src/config/mod.rs:252 报错退出），不得使用。
- **合法 decision 值（schema 核实）**：
  - item/commandExecution/requestApproval：accept/acceptForSession/**decline**/cancel
    （schema/json/CommandExecutionRequestApprovalResponse.json）；
  - item/fileChange/requestApproval：accept/acceptForSession/**decline**/cancel；
  - execCommandApproval / applyPatchApproval（v1 形态）：allow/**deny**/approved/approved_for_session/
    approved_mcp_policy_amendment/timed_out/abort —— 首轮 handler 用的 "denied" **不合法**，已改为 "deny"；
  - item/permissions/requestApproval：**deny**（+read/write 授权形态）；
  - mcpServer/elicitation/request：action accept/**decline**/cancel；
  - item/tool/requestUserInput：answers 映射，空 {} 表示不提供输入。
- handler 覆盖矩阵（p0_6.py decline_handler）：
  | server request | typed 分支 | 运行时实证 |
  |---|---|---|
  | item/commandExecution/requestApproval | decline | trace B PASS |
  | execCommandApproval | deny | 待重跑（v1 形态，当前配置走 v2） |
  | item/fileChange/requestApproval | decline | trace B3 待配额 |
  | applyPatchApproval | deny | 待重跑（v1 形态） |
  | item/permissions/requestApproval | deny | 待触发 |
  | mcpServer/elicitation/request | action decline | **未构造运行时触发**（需 elicitation MCP server）；handler+schema 已验证，结论限于已证明路径 |
  | item/tool/requestUserInput | answers {} | trace B4 待配额（工具若不可用则记录 UNAVAILABLE 并收窄结论） |
  | 其余（attestation/generate、account/chatgptAuthTokens/refresh、item/tool/call 等） | **-32601 协议错误，绝不自动批准** | 默认分支代码在 spike_client.py；运行时出现待重跑记录 |
- 实测（首轮）：1 次 commandExecution 审批，decline 后模型回复 DENIED，副作用文件不存在。

## Trace B2 — 等待审批时事件泵不阻塞（新增，待配额）

- 客户端支持延迟答复：handler 返回 `{"__defer__": true}` → 请求挂起（pending_server_requests），
  读线程继续消费通知；随后 `respond_server_request(id, payload)` 补发 decline。
- 断言：审批挂起 8s 窗口内通知数严格增加（无关事件被消费），decline 后无副作用文件。
- 证据（待回填）：`trace-b2-transcript.json`。

## Trace C — 确认取消（PASS；核心最小 fork 修复 yielded-exec 孤儿）

- 先 pgrep 确认 `sleep 60` 子进程存在 → `turn/interrupt` → 轮询进程消失。
- **根因（已核对 HEAD）**：one-shot 完成路径 `unified_exec/oneshot.rs:41-124`
  （`exec_command_to_completion`）本就在取消时 terminate（child_token + drop_guard +
  `tokio::select!` 75-97 → `terminate_confirmed`）；但 **yielded 路径**
  （`exec_command`，completion=None）把进程存入 manager（`store_process`），
  `collect_output_until_deadline` 只 race 进程退出 token + deadline，turn 中断时 future 被 drop，
  子进程组存活成为孤儿。`unified_exec_options`（runtimes/unified_exec.rs:111-122）只挂
  DefaultTimeout + network_denial token，无 turn token；orchestrator.rs:106 不 race 取消。
  CodeModeInterrupt（features default_enabled）只 terminate code-mode cell，不杀 yielded 子进程。
- **修复（最小 fork）**：`core/src/unified_exec/process_manager.rs` 新增
  `terminate_process_on_turn_cancellation(process, turn_cancellation)` watcher，镜像
  `terminate_process_on_network_denial`；在 `exec_command_inner` 中仅对 yielded 路径
  （`completion.is_none()`）spawn：select turn_cancellation.cancelled() vs
  process.cancellation_token().cancelled()，中断则 `process.fail_and_terminate("interrupted")`
  （走 exec.rs terminate/kill process group + 宽限期；子进程为组 leader，child_command.rs:281 process_group(0)）。
- **实证**：`trace-c-rerun.json`（新引擎）pass=true，sleepBeforeInterrupt="5412 sleep 60"，
  sleepAfterInterrupt=""，cleanupSeconds=0（即时回收），turnStatus=interrupted。
- 时序陷阱：子进程未 spawn 或清理窗口内采样会误判孤儿；必须"确认存在→interrupt→轮询消失"。

## Trace D — 断连对账（PASS，已提交；孤儿感知整改）

- turn 进行中 SIGKILL 引擎；新连接 thread/read 对账，首轮最后一轮状态 interrupted（终态），
  且崩溃后 pgrep 无孤儿（orphanSleepAfterCrash 为空）。
- **整改**：terminalProven 现在要求"终态 **且** 孤儿子进程已消失"（轮询上限 20s）；
  若孤儿仍在，即使状态终态也判 **uncertain**（不能证明执行已终止），不盲目重放，等待操作者决策。
  删除首轮"可证明执行已终止"的无条件表述。
- 证据：`trace-d-precrash.json`、`trace-d-reconcile.json`（重跑后含 orphanPollSeconds/orphanCleared）。

## 资源上限初值（blocker 整改：已实现并实证，非仅设计值）

| 项 | 初值 | 实现/验证 |
|---|---|---|
| 单帧上限 | 16 MiB | spike_client.py max_frame_bytes，逐帧计量+违例记录；trace A 0 违例 |
| 日志单 run | 100 MiB | spike_client.py run_log_path + max_log_bytes（默认 100MiB）：超限轮转并丢弃最旧段，保留日志 ≤ 上限。**机制实证**：p0_6b 以 1MiB 缩放阈值驱动 >1MiB 真实流量，rotations≥1 且 retained≤limit（resource-limits.json logRotation PASS）；100MiB 为同一机制的生产初值 |
| 请求超时按类型 | Client.DEFAULT_TIMEOUTS | initialize 30s；thread/* 60s；turn/start 30s、turn/interrupt 30s；command/exec 90s、terminate 30s；fallback 60s；全局 deadline 兜底。**实证**：p0_6b command/exec sleep 20 + 显式 2s 超时 → TimeoutError（elapsed<5s）→ terminate 回收，pgrep 确认进程消失（requestTimeout PASS） |

## thread/read 弃用（minor 整改：接受风险）

- includeTurns 全量 hydration 已弃用（schema/json/v2/ThreadReadParams.json 描述：
  prefer metadata-only read + thread/turns/list + thread/items/list 分页）。
- 但 thread/items/list 当前 UNSUPPORTED_SERVER_SIDE（P0-9 探针）→ 分页迁移暂不可行。
- **接受风险**：spike/编排器 MVP 继续用 thread/read includeTurns（小历史量），
  owner=SuperAgent 编排器；上游分页可用后迁移，跟踪于 #72（P0-9 接口决策清单）。

## fork 裁决

审批/对账/资源上限均可用现有 API + 客户端纪律完成，无需 core 改动。
**唯一最小 fork（S0 证明 API 不足）**：yielded-exec 子进程在 turn 中断时不被回收（孤儿）。
现有 API 无 turn 级取消信号挂到 yielded 进程；one-shot 路径已正确而 yielded 路径缺失。
修复限于 `core/src/unified_exec/process_manager.rs` 新增 turn-cancellation watcher（镜像既有
network-denial watcher），仅对 yielded 路径生效，不改协议/不动 one-shot 正确路径。
证据：trace-c-rerun.json（修复后 cleanupSeconds=0）。该 fork 满足混合路线「仅 S0 证明不足时最小 fork」。

## 审查整改记录

首轮 grok CHANGES_REQUESTED（4 blocker + 1 major + 2 minor）：
①typed handler 覆盖不全 → 覆盖矩阵 + B3/B4 探针 + elicitation 明确收窄；
②等待审批事件泵/超时无 trace → B2 延迟审批设计 + p0_6b 超时实证；
③100MiB/按类型超时仅设计值 → 客户端实现 + p0_6b 实证（2/2 PASS）；
④spawn wiring 未记录 → spawn-wiring.json；
⑤trace D 孤儿感知 → terminalProven 加孤儿消失条件；
⑥thread/read 弃用 → 接受风险+owner+跟踪；
⑦"denied" 非法值 → schema 核实改 "deny"。
配额恢复后已重跑 p0_6.py 回填 A/B/B2/B3/C/D 运行时证据（summary.json）：
- Trace C/D：核心最小 fork（process_manager.rs turn-cancellation watcher）修复 yielded-exec 孤儿，
  trace C cleanupSeconds=0、trace D 孤儿感知对账 PASS。
- Trace B2：单 turn 阻塞审批期间无无关事件 → 改为同连接并发第二 thread 在挂起窗口发通知，
  断言 unrelatedFromOtherThread 非空（pumpConsumedWhilePending=true），证明事件泵不阻塞。
- Trace B4（user-input）：本 build 未向模型暴露 user-input 工具（userInputRequestObserved=false），
  记 UNAVAILABLE 并收窄结论；B4 单独报告，不计入 allPass（allPass=A/B/B2/B3/C/D）。
- elicitation：需 elicitation MCP server 构造运行时触发，handler+schema 已验证，结论限于已证明路径。