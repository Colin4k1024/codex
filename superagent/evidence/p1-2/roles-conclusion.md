# P1-2 / #74 结论 — 元智能体人格指令 + 五角色文件

分支 feat/p1-2-roles（stacked on feat/p1-1-pack-manifest）。本 PR 交付**内容层**；
运行时验收（TUI leader 按人格组队 / 角色隔离 / 预算实测）由 quota-gated 探针
`tests/p1_2.py` 在配额恢复后证明，**Issue 在运行时证据回填前保持 OPEN**。

## 1. 交付物

- `pack/roles/leader-persona.md` — leader 完整人格（INTAKE→DELEGATE→VERIFY→REPORT）
  + §2 常驻介绍（resident intro）。
- `pack/roles/{planner,implementer,verifier,reviewer,reporter}.toml` — 五角色 config 层：
  description / model / developer_instructions（含 Delivery contract）。
- `pack/config/agents-overlay.toml` — `[features.multi_agent_v2]` +
  `subagent_usage_hint_text`（= 常驻介绍）+ `[agents]` + 五个 `[agents.<role>]`
  （description / config_file="./agents/<role>.toml" / nickname_candidates）。
- `tests/test_roles.py` — 内容层 10 项测试（结构 / overlay↔文件一致性 / 模型∈引擎目录 /
  常驻介绍 token 预算 / 交付契约 / 凭据 absence / 沙箱约束声明）。
- `tests/p1_2.py` — 运行时验收探针（app-server stdio，与 TUI 同引擎+配置路径）。
- `pack/manifest.json` — 重新导出，file_count=13，
  digest=d5690d7004ee4a6e75603ea1375dec313e587d6f3d795d55d5dfe646d1b6e697。

## 2. 验收映射（内容层已证）

| 验收项 | 状态 | 证据 |
|---|---|---|
| 五角色 planner/implementer/verifier/reviewer/reporter | PROVEN（内容） | test_roles.py 10/10；五 toml 存在且解析 |
| 每角色 description/config_file/nickname_candidates | PROVEN（内容） | overlay↔文件一致性测试 |
| 每角色 config 层（model/developer_instructions/交付契约） | PROVEN（内容） | model∈models.json 目录；Delivery contract 测试 |
| 常驻介绍 ≤400 tokens（tokenizer 实测） | PROVEN | tiktoken o200k_base = **238 tokens**（tokenizer-measurement.json） |
| TUI leader 按人格组队 | PENDING（运行时） | p1_2.py：persona 注入 + 五角色 spawn |
| 角色隔离生效 | PENDING（运行时） | p1_2.py：每 child rollout 仅含自身角色指令 |
| 预算实测达标 | PENDING（运行时） | p1_2.py：每 child token 用量实测 |

## 3. 模型分层（预算 tiering，声明值；运行时应用待 p1_2.py 验证）

- planner=gpt-6.1-sol，implementer=gpt-6-astra，verifier=gpt-5.6-luna，
  reviewer=gpt-6.1-terra，reporter=gpt-5.6-luna（均∈引擎 models.json 目录）。

## 4. 诚实限制

1. **内容 ≠ 运行时验收**：本 PR 不声称 leader 已按人格组队/隔离/预算达标；这些由
   p1_2.py 在配额恢复后实测，证据回填前 Issue 保持 OPEN（完整性教训）。
2. **角色级 sandbox_mode 不被引擎应用**（P0-2 实证，core/src/agent/role.rs）；
   沙箱兜底由编排器经显式 SandboxPolicy（P0-7）承担。角色文件不含 sandbox_mode。
3. **生产 overlay 不硬编码 approval_policy/sandbox_mode**（区别于 P0-2 spike overlay）；
   审批/沙箱属编排器与用户配置。
4. **运行时组队经 app-server stdio 驱动**（与 TUI 同引擎+配置路径）；TUI 人格渲染为
   另一较轻断言，p1_2.py 不声称 TUI 自主组队。
5. 角色文件与 overlay 均不含凭据（test_roles.py 凭据模式扫描通过）。
6. manifest source_commit 记录导出时 HEAD（23f5ffc99）；digest 仅覆盖文件内容，
   与提交顺序无关。

## 5. 复现

```
python3 superagent/tests/test_roles.py            # 内容层 10/10
python3 superagent/tests/test_roles.py --measure  # 238 tokens
python3 superagent/tools/pack.py verify           # digest 校验
# 运行时（配额恢复后，post-quota chain 自动执行）：
python3 superagent/tests/p1_2.py superagent/evidence/p1-2
```