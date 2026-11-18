# P1-3 (#75) team-delivery swarm skill + 交接/验收契约 + 预算合规 — 结论与验收映射

日期：2026-11-17 · 分支 feat/p1-3-team-delivery（堆叠于 feat/p1-1-pack-manifest / PR #113）
状态：**内容层 + 运行时验收均完成；13/13 结构/预算测试 PASS；预算实测 allPass=true；技能发现 PROVEN；运行时触发 PROVEN**。
p1_3.py 全模式实测 allPass=true（p1-3-runtime-conclusion.json）：skill_discoverable=true、skill_triggered=true、
fragment_role_user_not_developer=true、skill_span_tokens_o200k=589≤900、no_developer_elevation=true。
合并顺序：#72 → #73（PR #113）→ #75。依赖 #68（已关闭）+ #73。

## 1. 交付物

| 交付 | 路径 |
|---|---|
| team-delivery 技能（intake/plan/implement/verify/review/report + 产物契约 + 委派策略 + 预算 + 信任边界） | superagent/pack/skills/team-delivery/SKILL.md |
| 工作流节点/产物契约（read on demand） | superagent/pack/workflows/team-delivery.md |
| 交接契约（artifact 引用 + 摘要 ≤800 + 低信任不提权） | superagent/pack/templates/handoff-contract.md |
| 验收标准模板（门禁 command_source + artifact schema + 缺门禁 waiting_input） | superagent/pack/templates/acceptance-criteria.json |
| 结构/预算/合规测试（13） | superagent/tests/test_team_delivery.py |
| 运行时探针（发现 model-free + 触发 quota-gated） | superagent/tests/p1_3.py |
| skills/templates 规范更新（extraRoots 重注入→#76 实证默认发现根持久） | superagent/pack/skills/README.md, templates/README.md |
| manifest 重导出（10 文件） | superagent/pack/manifest.json |

## 2. 验收映射

| 验收项 | 证据 |
|---|---|
| 技能可触发 | **发现已证**（p1-3-discovery-conclusion.json `skill_discoverable=true, allPass=true`）：pack/skills 复制到产品 CODEX_HOME/skills（引擎默认发现根），skills/list 列出 team-delivery（enabled=true，path 指向 home/skills/team-delivery/SKILL.md，无 extraRoots）。**turn 触发**经 p1_3.py 全模式在配额恢复后取证（$team-delivery 显式调用 → rollout 含 `<skill><name>team-delivery</name>` 片段）。 |
| 注入片段全数满足预算（实测） | test_team_delivery.py + budget-measurement.json `allPass=true`（o200k_base AND cl100k_base）：SKILL.md 注入片段 493 tokens ≤900；workflow 500 / handoff 367 / acceptance 848 各 ≤900；交接摘要示例 103 ≤800；每轮新增（skill+任一 on-demand）最大 1341 ≤2000。运行时再测注入 `<skill>` span tokens ≤900（p1_3.py）。 |
| 低信任产物不经 developer 级注入提权 | 引擎机制实证：技能片段 `SkillInstructions` role="user"、kind="skills.selected_skill_instructions"、markers `<skill>...</skill>`（codex-rs/ext/skills/src/fragments.rs:92-106）；且被标记为**非授权 guardian context**（codex-rs/core/src/context/contextual_user_message.rs:41 "Hidden runtime context is not user authorization"，is_skill_prompt_fragment 在 CONTEXTUAL_USER_FRAGMENT_MATCHERS）。developer_instructions 仅来自可信 pack 角色文件。SKILL/workflow/handoff 均声明低信任产物 user-level、绝不写入 developer_instructions；test_low_trust_no_developer_elevation 断言。p1_3.py 运行时断言片段 role=user 且 skill body 不在 developer 消息。 |

## 3. 预算硬约束（仓库 P0 规则，设计 §2）

常驻介绍 ≤400（#74）；节点交接摘要 ≤800；单片段硬上限 900；每轮新增 ≤2000；超 1k 新片段人工复核。
本技能仅 SKILL.md 自动注入（493 tokens）；workflow/handoff/acceptance 均 read on demand（文件/MCP），
不灌提示词。长资料走文件读取/MCP（SKILL.md + workflow 明确声明）。

## 4. 诚实性声明

1. 运行时 turn 触发验收已实测（p1_3.py 全模式 allPass=true）：skill_triggered=true、注入片段 role=user（非 developer）、span 589 tokens≤900、no_developer_elevation=true。内容层 + 静态预算 + model-free 发现 + 运行时触发四层均证。
2. 预算"实测"含静态 tiktoken 测量（已完成，allPass）与运行时注入片段测量（quota-gated）两层。
3. acceptance-criteria.json 的 schema 为示意；权威 wire schema 以 P2-1 #78 contracts 为准（已 APPROVE）。
4. 技能发现以 pack/skills 直接复制到 CODEX_HOME/skills 复现安装器放置（安装器本体属 #76，已 APPROVE）；机制与 #76 重启持久证明一致。
5. 低信任不提权的运行时确认（rollout role 扫描）随 p1_3.py 全模式取证；静态层已由引擎源码 + 测试断言承载。

## 5. 复现

```bash
python3 superagent/tests/test_team_delivery.py                       # 13 tests
python3 superagent/tests/test_team_delivery.py --measure superagent/evidence/p1-3  # 预算实测
python3 superagent/tests/p1_3.py superagent/evidence/p1-3 --discovery-only  # model-free 发现
python3 superagent/tests/p1_3.py superagent/evidence/p1-3            # 运行时触发（quota-gated）
```