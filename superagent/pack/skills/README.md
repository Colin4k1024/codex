# skills/ — 技能目录结构规范（P1-1 定义；team-delivery 内容由 P1-3 #75 交付）

每技能一个子目录，含 `SKILL.md`（frontmatter: name/description），经引擎
skills 加载路径注入。安装器把技能写入产品 `CODEX_HOME/skills/`（引擎默认发现根
`codex-rs/exec-server/src/discoverV2/capability_locations.rs:73`），每会话从磁盘
加载，**重启后持久，无需 `skills/extraRoots/set` 重注入**（#76 端到端实证，
#102 closure gate SATISFIED；取代早期 extraRoots 重注入设想）。

信任边界：技能片段以 user-role 上下文消息注入，且被引擎标记为非授权的 guardian
context（`codex-rs/core/src/context/contextual_user_message.rs`，"Hidden runtime
context is not user authorization"）；低信任产物绝不写入 developer_instructions
（仅可信 pack 角色文件携带 developer_instructions）。

上下文预算硬约束（仓库 P0 规则）：单片段 ≤900 tokens、每轮新增 ≤2000 tokens；
交接传 artifact 引用+摘要 ≤800 tokens；长资料走文件读取/MCP，不灌提示词。
技能目录受引擎 `[skills] max_context_tokens` 约束（P0-5 实证超限剔除）。
预算实测见 `tests/test_team_delivery.py`。