## 背景
验证 Swarm Engine 复用核心：MAv2 子代理工具 + agent-roles 角色体系。注意 thread/start.multiAgentMode 已废弃（"Use Ultra reasoning effort for proactive multi-agent behavior"），需实测当前触发方式。

## 任务
- [ ] 配置 [features.multi_agent_v2] enabled=true；[agents] max_concurrent_threads_per_session / default_subagent_model
- [ ] [agents.planner/implementer/verifier] 声明角色（description/config_file/nickname_candidates），角色文件放 <CODEX_HOME>/agents/*.toml 验证自动发现
- [ ] 角色独立 config 层（模型、沙箱、developer_instructions）生效验证
- [ ] TUI 启动 leader，真实 spawn ≥2 个角色线程；send_message/wait_agent/interrupt_agent/close_agent 往返
- [ ] 记录多代理行为实际触发方式（effort/角色提示/工具提示）

## 验收
leader 自主组队完成一个子任务的 trace；角色 config 层生效；触发方式书面结论。

## 代码
core/src/tools/handlers/multi_agents_v2/；agent-roles/src/loader.rs；config/src/config_toml.rs（AgentsToml）。依赖 #64。

