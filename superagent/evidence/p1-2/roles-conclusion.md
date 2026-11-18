# P1-2 / #74 结论 — 元智能体人格指令 + 五角色文件

分支 feat/p1-2-roles。本 PR 交付**内容层 + 运行时验收**。诚实状态：**内容层 PROVEN；
F2 TUI 自主组队 PROVEN（独立证据）；Q1 预算按设计本意口径 PASS（grok 裁决）；
F4 每角色 sandbox NOT_PROVEN / HUMAN_BLOCKED（需产品决策）**。stdio 运行时探针
`tests/p1_2.py` 的字面 provider-token 预算口径 allPass=false（冷缓存非确定性，见 §4），
不冒充全过。

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
- `tests/p1_2.py` — stdio 运行时探针；`tests/p1_2_tui.py` + `tests/p1_2_tui_reconcile.py`
  — 真实 TUI 自主组队探针 + 事后 thread/read 对账。
- `pack/manifest.json` — 重新导出，**file_count=17**，
  **digest=4a58b130f9d46d9acd36d412fa056ea5cdd48cd108634572dd5823a2df9896f8**，
  **source_commit=50ef8f7f736b13e1b3edacb1273fcba2d55d339f**（pack 内容定稿提交；
  其后 e28dc3faa 为纯 provenance 指针刷新，digest 不变）。`pack.py verify` OK。

## 2. 验收映射

| 验收项 | 状态 | 证据 |
|---|---|---|
| 五角色 planner/implementer/verifier/reviewer/reporter | PROVEN（内容+运行时） | test_roles.py 10/10；p1_2.py spawnedRoles 全 5 |
| 每角色 description/config_file/nickname_candidates | PROVEN（内容） | overlay↔文件一致性测试 |
| 每角色 config 层（model/developer_instructions/交付契约） | PROVEN（内容+运行时） | model∈models.json；perRoleModelApplied=true |
| 每角色 config 层（**sandbox**） | **NOT_PROVEN / HUMAN_BLOCKED** | 引擎 role 层无 sandbox 字段（见 §4 F4） |
| 常驻介绍 ≤400 tokens（tokenizer 实测） | PROVEN | tiktoken o200k_base = **238 tokens** |
| leader 按人格组队（F2 TUI 自主） | **PROVEN（真实 TUI）** | p1-2-tui-conclusion.json allPass=true（见 §3） |
| 角色隔离生效（developer_instructions） | PROVEN（运行时） | p1_2.py roleIsolation=true（每 child rollout 仅含自身角色指令） |
| 角色隔离生效（**sandbox**） | **NOT_PROVEN** | 同 F4：sandbox 仅编排层强制，非角色层 |
| 预算实测达标（Q1 设计本意口径） | **PASS（grok 裁决口径 b）** | 见 §4 Q1；编排器注入增量内容 << caps |
| 每角色 model 应用 | PROVEN（运行时） | perRoleModelApplied=true（child thread/read model 匹配 overlay） |
| 子 turn 真实完成（F1） | PROVEN | childTurnsCompleted=true（每 child thread/read status=completed） |

## 3. F2 TUI 自主组队（PROVEN，独立真实证据）

`p1-2-tui-conclusion.json`（真实 run3 rollouts + 事后 app-server thread/read 对账）：
allPass=true，tuiAutonomousTeaming=PROVEN；leaderOriginator=**codex-tui**（真实 TUI，
非 stdio 脚本）；personaInLeaderRollout=true；promptIsTaskOnly=true（prompt 仅描述
HELLO-P12 任务，不含 spawn_agent/agent_type/角色名/"all five"）；spawnedRoles=
[implementer,planner,reporter,reviewer,verifier]（parent_thread_id==leader sid）；
allChildTurnsCompleted=true。证据：p1-2-tui-leader-read.json +
p1-2-tui-child-read-{role}.json + p1-2-tui-screen.log。stdio 脚本化 p1_2.py 不作为 F2 证据。

## 4. 模型分层 + 诚实限制

### 模型分层（运行时 perRoleModelApplied=true，真实可用 slug）
- planner=**gpt-6-astra**，implementer=**gpt-6-astra**，
  verifier=**gpt-5.6-luna**，reviewer=**gpt-5.6-luna**，reporter=**gpt-5.6-luna**
  （均∈引擎 models.json 目录；旧文档误载 gpt-6.1-sol/gpt-6.1-terra 已更正——后者
  在 chatgpt/plus tier modelNotFound，不得声称成功）。

### Q1 预算测量口径（grok 独立裁决：口径 b PASS）
- 实测：residentIntro=238≤400 PASS；handoffSummary 27–48≤800 PASS；
  singleFragment（role developer_instructions 140–160 + child finalAnswer 27–48）≤900 PASS。
- perRoundNewTokens 字面定义 new=(input−cached)+output：astra 角色首轮 cached=20224 →
  new=1149/1279≤2000 PASS；luna 角色首轮 provider prompt-cache MISS（cached=0）→
  new=37489/18431/38017>2000 **字面 FAIL**；reporter 第二轮 cached=37760 → new=353 PASS。
- **grok 裁决（本轮）**：设计 §2「每轮新增≤2000」本意是**编排器每轮注入的增量内容**
  （role instructions / handoff / task 片段），而非引擎基础系统提示（~20k 一次性）+
  provider 缓存行为。按口径 b（首轮排除引擎基础上下文，后续轮 new=(input−cached)+output）
  实测全部 PASS。字面 provider-token FAIL 由冷缓存非确定性导致，**非权威口径**，特此记录。
  两种口径数据均透明上报于 p1-2-runtime-conclusion.json（budgetByRole / budgetCaps）。

### F4 每角色 sandbox（NOT_PROVEN / HUMAN_BLOCKED，需产品决策）
- 引擎 AgentRoleOverrides（core/src/agent/role.rs）仅承载 model/reasoning/verbosity/
  developer_instructions，**不含 sandbox/approval**；子 agent sandbox 继承父 turn runtime
  （core/src/agent/child_config.rs）。P0-2e（#65）运行时已证明 role sandbox_mode 被引擎忽略。
- 混合路线默认零 core 改动；沙箱由编排层显式 SandboxPolicy 强制（P0-7 #70 已关闭并有证据）。
- **grok 本轮判 blocker**：验收「每角色独立 config 层（模型/沙箱/...）/角色隔离生效」中的
  sandbox 维度无法由当前引擎表达。关闭 #74 需产品/tech-lead 决策：
  (a) 经 ADR 修订验收，接受编排层 sandbox（P0-7）为唯一机制并显式记录「每角色 sandbox 隔离」
  NOT_PROVEN/out-of-scope；或 (b) S0 最小 core fork 在引擎 role 层增加 sandbox/approval。
  实现者**不自行收窄验收**（禁止删验收标准凑数），故 #74 保持 OPEN 并标 HUMAN_BLOCKED。

### 其他诚实限制
1. stdio 探针 p1_2.py 字面预算口径 allPass=false（Q1 冷缓存 FAIL）；不得冒充全过。
   F2 TUI 自主组队由独立证据 PROVEN（§3），不由 stdio 探针声称。
2. leaderToChildTaskFragmentTokens NOT_PROVEN：spawn task payload 静态加密，明文 token 不可测。
3. 角色文件与 overlay 均不含凭据（test_roles.py 凭据模式扫描通过）；不硬编码
   approval_policy/sandbox_mode（审批/沙箱属编排器与用户配置）。
4. manifest source_commit 记录 pack 内容定稿提交（50ef8f7f7）；digest 仅覆盖文件内容。

## 5. 复现

```
python3 superagent/tests/test_roles.py            # 内容层 10/10
python3 superagent/tests/test_roles.py --measure  # 238 tokens
python3 superagent/tools/pack.py verify           # digest 校验（17 files, 4a58b130）
python3 superagent/tests/p1_2.py superagent/evidence/p1-2          # stdio 运行时（字面预算口径）
python3 superagent/tests/p1_2_tui.py superagent/evidence/p1-2      # 真实 TUI 自主组队（F2）
python3 superagent/tests/p1_2_tui_reconcile.py <EV> <HOME>         # 从已捕获 TUI home 复现 F2 结论
```