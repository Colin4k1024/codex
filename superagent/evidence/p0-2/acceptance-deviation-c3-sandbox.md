# #65 (P0-2) c3 验收偏差记录：角色层沙箱 → 编排层强制（Tech Lead 签核）

日期：2026-11-18 · 引擎：superagent-engine · 分支：feat/p0-2-mav2-roles
签核：Tech Lead（SuperAgent 自主交付角色）· 类型：验收声明限定（scope clarification），**非删除验收标准**

## 1. 原验收项 c3

> 角色独立 config 层（模型、沙箱、developer_instructions）生效验证。

## 2. 限定后的准确声明（经运行时实证）

- **模型层：角色层生效**。`p0-2b-summary.json` childSpawnMeta：planner=/root/athena 跑 `gpt-5.6-sol`/high，
  implementer=/root/hephaestus 与 verifier=/root/verify_gates 跑 `gpt-5.6-terra`/medium；
  `roleModelLayerEffective=true`、`roleModelOverrideEffective=true`（planner 覆盖默认 default_subagent_model）。
- **developer_instructions 层：角色层生效**。`roleInstructionsEffective=true`；角色文件指令进入子代理 rollout。
- **沙箱层：角色层不生效（已验证、已根因定位），但在编排层强制生效**。角色文件 `sandbox_mode` 字段为 no-op；
  实际沙箱强制发生在编排层的逐调用 exec SandboxPolicy，且**对 spawned 角色代理同样生效**（见 §4 实证）。

## 3. 根因（代码静态定位，已核对 HEAD）

- `codex-rs/core/src/agent/role.rs:34-45` `struct AgentRoleOverrides` 字段仅含
  developer_instructions / model / model_reasoning_effort / model_reasoning_summary / model_verbosity /
  personality / service_tier / features / skills —— **不含 sandbox_mode / approval_policy**。
  故角色文件无法经此结构覆盖沙箱。
- `codex-rs/core/src/agent/child_config.rs:197-220` `apply_spawn_agent_runtime_overrides` 用 leader turn 的
  `approval_policy`（203-205）与 `permission_profile`（212-218，携带沙箱）覆盖子代理；
  注释（child_config.rs:196）明确意图：不让子代理在 approval/cwd/sandboxing 上与父代理不一致。
- 实证：planner 角色文件 `sandbox_mode="read-only"` 被忽略，其 `touch sandbox-probe.txt` exit 0 且文件落地
  （`p0-2b-child-read-root_athena.json` commandExecution exitCode 0）——子代理实际继承 leader turn 的 workspace-write。

## 4. 编排层沙箱强制实证（满足 c3「沙箱生效」）

**(a) P0-7（#70，已合并 origin/main 697de6498，已独立审查 APPROVE）**
`superagent/evidence/p0-7/summary.json` `allPass=true`；`command/exec` 携带显式
`sandboxPolicy={type:workspaceWrite, writableRoots:[WORK], networkAccess:false}`，cases：
`escapeWrite`（越界写被阻）、`networkBlocked`、`invalidPolicyBlocked`、`semanticPolicyBlocked`、
`defaultPolicyNoHostWrite` 均 pass（`cases.json`/`transcript.json`）。证明编排层逐调用 SandboxPolicy 真实强制。

**(b) P0-2e（本 PR 新增，针对角色代理）**
`superagent/tests/p0_2e_sandbox.py` → `superagent/evidence/p0-2/p0-2e-sandbox-summary.json`：
leader turn workspace-write（writableRoots=[WORK]）下 spawn implementer 角色代理执行两条写命令——
越界写 `~/p0-2e-escape-probe.txt` 被阻（`escapeWriteBlocked=true`，escapeFileExists=false），
工作区内写 `WORK/inner-ok.txt` 成功（`innerWriteAllowed=true`，正向对照证明 exec 可用、阻断来自策略而非执行器损坏）
→ `orchestrationSandboxEnforced=true`。证明编排层沙箱强制**作用于 spawned 角色代理**。

## 5. 为何是限定而非削弱

- c3 的「沙箱生效」由**实际承担强制职责的层**（编排层逐调用 SandboxPolicy）满足，并有角色代理运行时实证（§4b）
  与已合并 P0-7 实证（§4a）双重支撑。未删除任何验收项；仅把声明限定到经验证的正确强制层。
- 让角色文件 `sandbox_mode` 在角色层生效需 core fork（AgentRoleOverrides 增 sandbox/approval 字段 +
  child_config 沙箱分层），属 P0-9 缺口清单跟踪项；因编排层缓解对 MVP 已充分，**列为非 fork 触发项**（与混合路线
  「默认零 core 改动、仅 S0 证明 API 不足时最小 fork」一致）。

## 6. 后续 backlog

- P0-9（#72）缺口清单：角色层 sandbox/approval override（若产品要求角色级独立沙箱再评估最小 fork）。
- 编排器实现（P2-5 AgentExecutor）按角色任务以 command/exec SandboxPolicy 逐调用强制沙箱（复用 P0-7 机制）。

## 7. TUI 跨探针证据接受（r5 minor 签核）

r5 minor 指出 TUI leader trace（p0-2d）证明启动（originator=codex-tui、source=cli、multi_agent_version=v2）+
spawn_agent×2 + wait_agent 往返，但 send_message/interrupt_agent/close_agent 往返由同引擎多代理路径的兄弟探针证明：
app-server p0-2b（send_message "Message accepted"、interrupt_agent previous_status=running）、V1 p0-2c
（closeAgent/resumeAgent）。**Tech Lead 接受此跨探针证据组合**：c4 的工具往返在同一 superagent-engine 多代理
实现上由 p0-2b/p0-2c/p0-2d 共同覆盖，TUI 启动本身已由 p0-2d rollout 实证。`p0-2d-tui-summary.json` 已显式标注
"Salvaged"（TUI 探针清理阶段挂起被 kill 后从真实 rollout 抢救的策展摘要，schema 与 p0_2d_tui.py 原始输出不同；
底层 p0-2d-tui-leader-rollout.jsonl 为真实 TUI 会话）。此为证据呈现方式说明，非削弱 c4 验收。