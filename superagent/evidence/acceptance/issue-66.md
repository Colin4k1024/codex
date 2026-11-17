## 背景
message-board 是团队通信基座（SQLite/内存/远程三后端，通道级 ACL）。验证会话内工具与外部 MCP 两条访问路径。

## 任务
- [ ] 会话内 create_channel/post/read_thread/subscribe 等工具可用
- [ ] 两个角色线程经 #run-<id> 频道完成一次产物交接（引用+有界摘要）
- [ ] message-board-mcp stdio/HTTP 外部客户端按 caller-bound 身份读写
- [ ] 通道 ACL（BoardPermissions/ChannelPermissions）：越权身份被拒绝

## 验收
双代理频道交接 trace；外部 MCP 授权可读、越权拒绝。

## 代码
message-board/；message-board-mcp/；agent-message-board-host/。依赖 #65。

