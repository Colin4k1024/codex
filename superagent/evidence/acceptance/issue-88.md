## 背景
检索能力以 MCP server 暴露给所有角色线程，复用 message-board-mcp 的 caller-bound 身份模式。

## 任务
- [ ] 检索 MCP server：hybrid search 工具（有界结果、引用来源）
- [ ] caller-bound 身份：身份由 host 固定，不由参数声明；越权拒绝
- [ ] leader/角色在任务中完成一次跨会话经验检索
- [ ] 检索结果作为低信任数据注入，不经 developer 级提权

## 验收
跨会话检索 trace；越权身份被拒；注入预算合规。

依赖 #87、#66。

