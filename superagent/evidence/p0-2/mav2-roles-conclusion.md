# P0-2 / #65 结论：MAv2 子代理工具 + agent-roles 角色体系

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
复现：`superagent/tests/p0-2-run.sh`（leader 自主组队修复 calc.add）+ `python3 superagent/tests/p0_2b.py`（工具往返/角色层补充探针）

## 1. leader 自主组队 trace（PASS，p0-2-run.sh）

- 配置：`[features.multi_agent_v2] enabled=true` + `subagent_usage_hint_text`；`[agents] enabled=true,
  max_concurrent_threads_per_session=4`；planner/implementer/verifier 三角色声明
  （description/config_file/nickname_candidates），角色文件 `<CODEX_HOME>/agents/*.toml` 自动发现生效。
- leader 在无人工干预下依次 spawn planner→implementer→verifier（subAgentActivity started/completed，
  agentPath /root/plan、/root/implement、/root/verify），wait_agent 交接，verifier 运行验收命令。
- 结果：fixture 基线测试失败 → 团队修复 calc.py（a-b → a+b）→  post-test 通过（post-test.txt、
  calc-after.py、workspace-diff.patch）。

## 2. 工具往返（PASS，p0_2b.py，p0-2b-summary.json allPass=true）

| 工具 | 证据 |
|---|---|
| spawn_agent | 2× subAgentActivity kind=started（/root/athena、/root/hephaestus），child thread/read 可读 |
| list_agents | leader 工具结果叙述列出 /root、/root/athena、/root/hephaestus 及状态；thread/list 独立佐证（p0-2b-thread-list.json） |
| send_message | subAgentActivity kind=interacted（hephaestus），leader 报告 message accepted |
| interrupt_agent | subAgentActivity kind=interrupted；leader 报告返回 `{"previous_status":"running"}` |
| wait_agent | collabAgentToolCall tool=wait status=completed；planner completed / implementer interrupted |
| close_agent | **V2 不暴露**：leader 报告工具不存在；代码佐证 multi_agents_v2/ 无 close handler，close_agent/resume_agent 仅 V1（core/src/tools/spec_plan.rs:794、handlers/multi_agents/close_agent.rs）。V2 生命周期终止 = 完成/中断，编排器清理走 turn/interrupt（P0-6 trace C） |

观测面限制：collabAgentToolCall item 仅流式化 `wait`；spawn/send/interrupt/list 的原始工具结果不单独成 item，
以 subAgentActivity kind + leader 工具结果叙述 + 外部 thread/read、thread/list 交叉佐证。

## 3. 角色独立 config 层生效验证

- **developer_instructions：生效**。planner/implementer 角色文件中的指令分别出现在对应子线程 rollout JSONL
  （p0-2b-summary.json roleProof.*InstructionsInRollout=true；thread/read items 不含系统级指令，故查 rollout）。
- **模型层：生效**。planner 角色文件 `model_reasoning_effort="high"` → 子代理 subAgentActivity
  reasoningEffort=high（leader 与 implementer 为 low）；model 覆盖同属 AgentRoleOverrides
  （core/src/agent/role.rs:77-86：developer_instructions/model/model_reasoning_effort/summary/verbosity/
  personality/service_tier/features）。
- **nickname_candidates：生效**。spawn 未显式传 nickname，子线程 thread/read 返回
  agentNickname=Athena/Hephaestus、agentRole=planner/implementer（候选自动指派）。
- **沙箱：角色层不生效（真实缺口，书面记录）**。planner 角色文件 `sandbox_mode="read-only"` 被忽略：
  其 `touch sandbox-probe.txt` exit 0 且文件落地。代码原因：AgentRoleOverrides 不含 sandbox/approval
  （role.rs:77-86），且 spawn 末尾 apply_spawn_agent_runtime_overrides 用 leader turn 的
  approval_policy/permission_profile/cwd 覆盖（child_config.rs:197-220）。
  **编排器缓解**：按角色的沙箱约束必须在编排层以 command/exec SandboxPolicy 逐调用强制（P0-7 已验证），
  或为角色任务使用独立 permissions profile 的 leader turn。列入 P0-9 缺口清单（非 fork 触发项）。

## 4. 多代理行为实际触发方式（书面结论）

- `thread/start.multiAgentMode` 已废弃（上游提示 "Use Ultra reasoning effort for proactive multi-agent behavior"）。
- 当前确定性触发 = 配置 `[features.multi_agent_v2] enabled=true`（工具面注册 spawn_agent/wait_agent/
  send_message/interrupt_agent/list_agents/followup_task）+ `[agents]` 角色声明 +
  `subagent_usage_hint_text` 系统提示 + 用户任务提示中的显式委派指令。
- 本次两条 trace 均由"配置开关 + 角色提示 + 显式委派 prompt"触发，未依赖 reasoning effort 档位的自发行为；
  effort 档位（Ultra）只影响主动性，不是工具可用性的开关。
- `default_subagent_model` 字段仍存在于 AgentsToml（config/src/config_toml.rs:819，core 映射
  config/mod.rs:4230）；未设置时子代理继承 leader 模型（实测 gpt-6-astra）。

## 裁决

MAv2 + 角色体系可复用，不触发 fork。唯一行为缺口（角色级沙箱不生效）以编排层缓解，
已纳入 P0-9 清单；close_agent 缺失为 V1/V2 设计差异，V2 用完成/中断 + turn/interrupt 覆盖。