# P0-3 / #66 结论：message-board 双路径访问与权限边界

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
复现：`superagent/tests/p0-3-run.sh` + `p0-3b-run.sh`（后者重跑 board turn 并启动外部 MCP 探针）

## 会话内工具路径（board-turn-transcript.json）
- leader 经 create_channel 创建 `run-spike-p03`；planner 经 post 发布一行计划摘要；
  verifier 经 read_thread 读取并 post 确认回复（双代理产物交接，引用+有界摘要）。
- leader 经 subscribe 订阅频道，终态消息叙述 `/root` subscribed `enabled: true`。
- 独立佐证（不依赖模型叙述）：外部 MCP get_channels 显示 `run-spike-p03` created_by=/root、
  message_count=2、last_message_id 与 verifier 回复一致；外部 read_post 直接取回两帖原文
  （mcp-caller-bound.json trace.memberReadPost / memberPostReadback）；
  外部 subscribe 以成员身份调用返回成功（trace.memberSubscribe，原始工具结果，非模型叙述）。
- 观测面限制：MCP 工具调用不产生独立 item/completed 类型（transcript 仅 userMessage/agentMessage/
  subAgentActivity/collabAgentToolCall），故会话内 subscribe/read_thread 的原始结果不可直接捕获，
  以上外部 MCP 调用即为独立验证路径。

## 外部 MCP 路径（mcp-probe.txt、mcp-caller-bound.json）
- `codex-message-board-mcp --sqlite-home <产品HOME> --listen 127.0.0.1:7799`，
  bearer 来自 `CODEX_MESSAGE_BOARD_TOKEN`（spike 用假 token spike-token-p03，仅本机回环）。
- 无 token → HTTP 401；带 token initialize → HTTP 200。
- caller-bound 身份：bearer 只授权"可信宿主"，每次 tools/call 必须携带
  `_meta.threadId` + `_meta.sessionId`（message-board-mcp/src/http.rs:150-163）。
  caller 身份解析要求该 threadId 已持久化于 board SQLite（threads/thread_spawn_edges，
  persisted.rs:22-79）；未持久化的 caller 在成员树遍历之前即被拒绝。
- 实测七态（mcp-caller-bound.json summary allPass=true；ok() 要求 result.isError=false）：
  - memberReadOk：get_channels + list_threads + read_thread{thread_id} + read_post{message_id} 全部成功；
  - memberWriteOk：成员身份 post 成功，readback 取回原文含 P03B-EXTERNAL-WRITE-OK 与引用 message_id；
  - subscribeConfirmed：成员身份 subscribe 原始结果成功；
  - nonMemberDenied：随机 UUID threadId → 拒绝（no thread with id）；
  - crossBoardDenied：另一会话 leader threadId → 拒绝，**实际错误为
    "caller has no persisted agent path"**（caller 未在本 board 持久化，身份解析层拒绝；
    不是"已知 caller 被成员树拒绝"的情形——后者需要同 board 内两棵独立代理树，本 spike 未构造）；
  - foreignWriteDenied：外部 caller post → 拒绝，且 readback 中无该文本落地；
  - missingMetaDenied：缺 _meta → 拒绝（invalid_params）。

## 权限模型书面结论（对验收措辞的修正）
验收中"通道 ACL（BoardPermissions/ChannelPermissions）"的实际实现是**连接级有效能力**，
不是逐通道 ACL：
- `BoardPermissions{create_channel,edit_metadata,manage_permissions}` /
  `ChannelPermissions{read,post,reply,edit_metadata}` 由连接的 Access::ReadOnly|ReadWrite 推导
  （message-board/src/permissions.rs:13-26、local/connection.rs:114-131、local/catalog.rs:254-264）。
- create_channel 只接受 channel_name/description/subscribe（tools/arguments.rs:11-15），
  规格明确"all agents in this collaboration can read and post"（tools/spec.rs:18-22）；
  **不存在创建受限通道的参数**。
- 只读连接上 post/reply/create_channel → 结构化 `permission_denied`
  （connection.rs:124-128、permissions.rs:30-37）。
- 越权拒绝由两条边界承担：① MCP caller 身份必须已持久化于本 board（上面三态拒绝）；
  ② 连接级只读降权。read_thread 不做逐通道 ACL，仅校验 board 成员身份（local/queries.rs:501-512）。

## 裁决
不触发 fork 变更：现有成员边界 + 连接级权限已满足"授权可读、越权拒绝"，
读写双向均经外部 caller-bound 实测。若产品未来需要逐通道 ACL，属于新功能（另立 issue），不是 P0 缺口。