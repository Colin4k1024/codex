# skills/ — 技能目录结构规范（P1-1 定义，team-delivery 内容由 P1-3 #75 交付）

每技能一个子目录，含 `SKILL.md`（frontmatter: name/description），经引擎
skills 加载路径注入；安装器每次会话启动经 `skills/extraRoots/set` 重注入
（F-2 裁决，#102/#76）。

上下文预算硬约束（仓库 P0 规则）：单片段 ≤900 tokens、每轮新增 ≤2000 tokens；
交接传 artifact 引用+摘要 ≤800 tokens；长资料走文件读取/MCP，不灌提示词。
技能目录受引擎 `[skills] max_context_tokens` 约束（P0-5 实证超限剔除）。