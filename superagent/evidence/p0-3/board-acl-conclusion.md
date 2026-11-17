# P0-3 / #66 结论：message-board 双路径访问与权限边界

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64

## 会话内工具路径（board-turn-transcript.json）
- leader 经 create_channel 创建 `run-spike-p03`；planner 经 post 发布一行计划摘要；
  verifier 经 read_thread 读取并 post 确认回复；leader read_thread 看到两条帖子（双代理产物交接，引用+有界摘要）。
- leader 经 subscribe 订阅频道，终态消息确认 `/root` subscribed `enabled: true`。
- 观测面：item/started + item/completed（subAgentActivity ×4、collabAgentToolCall ×3），4 个 turn/completed。

## 外部 MCP 路径（mcp-probe.txt、mcp-caller-bound.json）
- `codex-message-board-mcp --sqlite-home <产品HOME> --listen 127.0.0.1:7799`，
  bearer 来自 `CODEX_MESSAGE_BOARD_TOKEN`（spike 用假 token，仅本机回环）。
- 无 token → HTTP 401；带 token initialize → HTTP 200。
- caller-bound 身份：bearer 只授权"可信宿主"，每次 tools/call 必须携带
  `_meta.threadId` + `_meta.sessionId`（message-board-mcp/src/http.rs:150-163）。
  成员校验走 SQLite threads/thread_spawn_edges 递归树（persisted.rs:22-79）。
- 实测四态（mcp-caller-bound.json summary allPass=true）：
  - memberGetChannels / memberReadThread：成员身份读取成功；
  - nonMemberDenied：随机 UUID threadId → 拒绝（ThreadNotFound）；
  - crossBoardDenied：另一会话 leader threadId + 本 board sessionId → 拒绝（不属于本 board 树）；
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
- 越权拒绝由两条边界承担：① MCP caller 成员校验（上面三态拒绝）；② 连接级只读降权。
  read_thread 不做逐通道 ACL，仅校验 board 成员身份（local/queries.rs:501-512）。

## 裁决
不触发 fork 变更：现有成员边界 + 连接级权限已满足"授权可读、越权拒绝"。
若产品未来需要逐通道 ACL，属于新功能（另立 issue），不是 P0 缺口。