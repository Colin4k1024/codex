# P0-6 结论：app-server stdio 全链路接入契约（issue #69）

日期：2026-11-17 · 引擎：superagent-engine（codex-cli 0.0.0，source_commit 425a848a9，sha256 见 p0-1/engine-manifest.json）· 平台：macOS arm64

## 总结论

四条 trace 全部 PASS（`summary.json allPass=true`），stdio JSON-RPC 接入契约成立，**无需 fork 改动**。
复现：`python3 superagent/tests/p0_6.py`（证据写入本目录）。

## Trace A — 真实模型任务（PASS）

- `initialize → initialized → thread/start → turn/start → 事件泵 → turn/completed` 全链路走通。
- `turn/start` 立即返回（inProgress），完成以 `turn/completed` 通知为准（`{threadId, turn:{id,status}}`）。
- 模型真实写文件 `p06.txt`，内容逐字节等于 `SUPERAGENT_P06_OK`（无尾换行）。
- 32 条通知；单帧最大 1695 字节，16MiB 帧上限 0 违例。
- 证据：`trace-a-transcript.json`。

## Trace B — typed 审批拒绝（PASS）

- 强制审批配置：`approval_policy = "on-request"` + `sandbox_mode = "read-only"`（写命令升级触发审批）。
  注意：上游已移除 `approval_policy = "untrusted"`（core/src/config/mod.rs:252 直接报错退出），不得再使用。
- typed handler 对 `item/commandExecution/requestApproval` 返回 `{"decision":"decline"}`；
  对 `execCommandApproval`/`applyPatchApproval`/`item/fileChange/requestApproval`/`mcpServer/elicitation/request`
  均有对应 typed 分支；**未知 server request 一律返回 -32601 协议错误并阻塞，绝不自动批准**（p0_6.py decline_handler）。
- 实测收到 1 次 `item/commandExecution/requestApproval`，decline 后模型回复 DENIED，
  副作用文件 `approval-probe.txt` 不存在。
- 等待审批期间事件泵持续消费（后台读线程），审批后正常收到 `turn/completed`。
- 证据：`trace-b-transcript.json`、`home-b/config.toml`。

## Trace C — 确认取消（PASS）

- 模型执行 `sleep 60`；**先 pgrep 确认子进程已存在**再发 `turn/interrupt {threadId, turnId}`。
- interrupt 响应无错误；`turn/completed` 状态 `interrupted`；`sleep 60` 进程在 **3 秒内**消失（cleanupSeconds=3）。
- 代码路径佐证：core/src/exec.rs:1043-1099 `ExecExpirationOutcome::Cancelled → terminate_process_group +
  CANCELLATION_TERMINATION_GRACE_PERIOD + kill_process_group`；core/src/tasks/mod.rs:1220-1234
  优雅中断 100ms 宽限后 abort task handle。
- 时序陷阱（早期 run 的假孤儿）：若在子进程尚未 spawn 或清理窗口内采样，pgrep 会误判。
  编排器取消确认必须：确认子进程存在 → interrupt → 轮询进程消失（本 probe 轮询上限 23s）。
- 证据：`trace-c-transcript.json`。

## Trace D — 断连对账（PASS）

- turn 进行中 `SIGKILL` 引擎进程（模拟崩溃），记录崩溃前通知快照与 stderr 尾部。
- 新连接 `thread/read {includeTurns:true}` 成功：最后一轮状态 `interrupted`（终态），
  判定 `reconciled-terminal` —— 可证明执行已终止，无需盲目重放。
- 若对账得到非终态，处置策略为 **uncertain：不盲目重放，等待操作者决策**（probe 中显式分支）。
- 证据：`trace-d-precrash.json`、`trace-d-reconcile.json`。

## 资源上限初值（编排器侧）

| 项 | 初值 | 实现/验证 |
|---|---|---|
| 单帧上限 | 16 MiB | spike_client.py `max_frame_bytes=16*1024*1024`，逐帧计量，违例记录；trace A 0 违例 |
| 日志单 run | 100 MiB（设计初值） | spike 客户端内存 stderr 环形保留最近 1000 行，全量落盘 run.log；100MiB 轮转在 P1 编排器实现 |
| 请求超时 | 按类型区分 | initialize/thread 类 60s；turn 类 180–360s；interrupt/read 30–60s；全局 deadline 兜底 |

## fork 裁决

无缺口。审批/取消/对账均可用现有 API 完成；唯一上游行为变更（untrusted 移除）已在配置层适配。