# F4 / #74 裁决记录（ADR）：每角色 sandbox 隔离 — 接受编排层 SandboxPolicy 为唯一强制机制，引擎角色层 per-role sandbox 显式 out-of-scope

日期：2026-11-18 · 平台：Darwin arm64 · 路线：混合路线（默认零 core 改动）
决策来源：**用户产品决策（Option A）**——经 AskUserQuestion 明确选择「接受编排层沙箱+ADR」。
依据：#70（P0-7 exec gate，已关闭，grok APPROVE）+ #65（P0-2e role 运行时探针）+ 本 PR
内容层/运行时证据（roles-conclusion.md §2/§4）。本记录为 #74 验收的正式 ADR 修订交付。

## 1. 背景与缺口（事实，非主张）

#74 验收：「TUI leader 按人格组队；角色隔离生效；预算实测达标」，任务含「每角色独立
config 层：模型、沙箱、developer_instructions、交付契约」。其中 **sandbox 维度**存在引擎表达缺口：

- 引擎 `AgentRoleOverrides`（codex-rs/core/src/agent/role.rs）仅承载
  model / reasoning / verbosity / developer_instructions，**不含 sandbox / approval 字段**。
- 子 agent 的 sandbox 继承父 turn runtime（codex-rs/core/src/agent/child_config.rs），
  并非按角色独立强制。
- #65（P0-2e）运行时已实证：role 层 `sandbox_mode` 被引擎忽略（设了也不生效）。

因此「每角色独立 sandbox 隔离」**无法由当前引擎角色层表达**；要在引擎角色层原生强制
per-role sandbox，必须 S0 最小 core fork（改 role.rs / child_config.rs + schema 重导出）。

## 2. 已被证明的隔离与沙箱机制（本 HEAD 可查）

- **内容/配置层角色隔离 PROVEN**：五角色各自 `developer_instructions`（含 Delivery contract），
  运行时 `roleIsolation=true`（每 child rollout 仅含自身角色指令，p1_2.py）；
  `perRoleModelApplied=true`（child thread/read model 匹配 overlay）。
- **编排层沙箱强制 PROVEN（#70）**：沙箱由编排/执行层显式 `SandboxPolicy` 强制，而非引擎角色层。
  证据 superagent/evidence/p0-7/exec-gate-conclusion.md：
  - `semanticPolicyBlocked`：语义不可表达的 policy 被 SandboxPolicy Deserialize 拒绝，命令未执行；
  - `defaultPolicyNoHostWrite`：缺省回退会话 config.permissions，工作区外写仍被阻止；
  - 显式 sandboxPolicy 经 to_core（app-server-protocol/src/protocol/v2/permissions.rs:652）→
    set_legacy_sandbox_policy（core/src/config/mod.rs:526）→ can_set_permission_profile 约束层。
  即每次 command/exec 携带的 SandboxPolicy 在编排/执行层被真实强制（含拒绝与默认不授予宿主写）。

## 3. 裁决（Option A，用户授权）

1. **接受编排层 SandboxPolicy（#70）为 #74 的唯一沙箱强制机制**。#74 验收中「沙箱」维度
   由编排/执行层显式 SandboxPolicy 满足并已被证明；不要求引擎角色层原生 per-role sandbox。
2. **「角色隔离生效」的判定口径**：内容/配置层隔离（per-role developer_instructions + model，
   运行时 roleIsolation=true / perRoleModelApplied=true）+ 编排层 SandboxPolicy 强制。
   二者均已 PROVEN，故「角色隔离生效」在本口径下 PASS。
3. **引擎角色层 per-role sandbox 显式记录为 NOT_PROVEN / out-of-scope**（混合路线零 core 改动）。
   本 ADR **不声称**引擎角色层 sandbox 隔离已实现；该能力被明确划出 #74 范围。
4. **不删除/收窄验收文字凑数**：#74 原验收文字保留；本 ADR 以产品决策正式修订其**判定口径**，
   并透明记录被划出的引擎角色层 sandbox 维度及其重开条件（§5）。这是范围边界的正式裁决，
   而非实现者自行收窄。
5. **预算（Q1）**：按 grok 独立裁决口径 b（编排器每轮注入增量内容）PASS，见 roles-conclusion.md §4 Q1；
   字面 provider-token 口径冷缓存 FAIL 透明上报，不冒充全过。
6. **F2 TUI 自主组队 PROVEN**（独立真实 TUI 证据，roles-conclusion.md §3），不由 stdio 脚本声称。

## 4. 验收逐条对照（修订口径后）

| 验收项 | 状态 | 证据 |
|---|---|---|
| TUI leader 按人格组队 | PROVEN | p1-2-tui-conclusion.json allPass=true（真实 codex-tui） |
| 角色隔离生效（内容/配置层 + 编排层沙箱） | PROVEN（本 ADR 口径） | roleIsolation=true / perRoleModelApplied=true + #70 SandboxPolicy |
| 每角色 config 层：模型/developer_instructions/交付契约 | PROVEN | test_roles.py 10/10；model∈models.json |
| 每角色 config 层：沙箱 | 由编排层 SandboxPolicy 满足（#70 PROVEN）；引擎角色层 per-role sandbox **out-of-scope** | 本 ADR §3；evidence/p0-7 |
| 常驻介绍 ≤400 tokens | PROVEN | tiktoken o200k_base = 238 |
| 预算实测达标 | PASS（grok 裁决口径 b） | roles-conclusion.md §4 Q1 |

## 5. 后果与重开条件

- **后果**：#74 可在本 ADR 口径下关闭（内容层 + F2 TUI + Q1 预算 + 编排层沙箱均 PROVEN）。
  引擎角色层 per-role sandbox 不在 #74 交付内，且不被声称为已实现。
- **零 core 改动保持**：role.rs / child_config.rs / schema 均不动，上游合并性零影响。
- **重开 fork 条件**：若未来产品要求引擎原生 per-role sandbox/approval（例如不经编排层、
  由引擎角色层直接强制每角色不同沙箱），按 S0 最小 fork 实施（role.rs 增 sandbox/approval
  字段 + child_config.rs 按角色应用 + schema 重导出 + 运行时探针证明生效），届时另立 issue。
- **诚实边界**：本 ADR 是产品决策记录，不是引擎能力证明。引擎角色层 sandbox 仍 NOT_PROVEN；
  任何下游不得据此声称「引擎已支持每角色沙箱隔离」。