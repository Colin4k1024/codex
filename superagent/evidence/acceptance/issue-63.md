# SuperAgent 元智能体落地路线图（Epic）

基于 codex fork（基线 `79f2ab2dd`）搭建 SuperAgent 元智能体。完整设计见仓库根目录 `SUPERAGENT_LANDING_DESIGN.md`（本地待提交）；本 issue 为 GitHub 权威跟踪入口。

## 路线决策（已确认）
- **混合路线**：元智能体与团队能力用现有引擎公开配置/协议承载（默认不改 core）；编排、网关、记忆增强放独立产品服务；仅当 P0 验证证明公开 API 不足时才在 fork 加最小 crate（见 fork-trigger issues）。
- **MVP 形态**：CLI/TUI 元智能体优先，网关与桌面后置。

## 核心判断
架构图内核大部分已存在：Swarm Engine ≈ MAv2 子代理工具 + agent-roles + message-board + ext/goal + ext/queue；Harness Engine ≈ skills/hooks/sandboxing/execpolicy/rollout；记忆精炼 ≈ memories 两阶段管线；市场 ≈ plugin marketplace。需新建：能力包（P1）、确定性编排器（P2）、记忆/进化服务（P3）、网关（P4）、分布式与市场扩展（P5）。

## 两层编排架构
外层 Swarm Flow 编排器（独立仓库，DAG/门禁/状态机/恢复，成功只认真实退出码）经 app-server stdio 驱动内层 leader 线程；leader = 开启 MAv2 的 codex 会话，用 [agents.*] 角色 + swarm skills + message-board 运行频道 + goal 预算组队。

## Issue 地图

### P0 接口验证（3–5 工程日，阻塞后续）
- #64 构建 superagent-engine + 专属 CODEX_HOME/认证/策略
- #65 MAv2 + 角色团队验证
- #66 message-board 工具 + 外部 MCP/ACL
- #67 goal 预算/steering + queue 空闲派发
- #68 skills 加载与持久化
- #69 app-server stdio 全链路 + 审批
- #70 command/exec 门禁
- #71 子代理可观测性实验 → fork 裁决（关键）
- #72 接口决策与 fork 缺口清单（退出交付）

### P1 能力包 MVP（3–5 工程日）
- #73 能力包结构与 manifest
- #74 元智能体人格 + 五角色文件
- #75 team-delivery swarm skill + 交接契约（预算合规）
- #76 安装器 + superagent 启动器/doctor
- #77 TUI 团队 fixture 验收

### P2 Swarm Flow 编排器（8–13 工程日）
- #78 contracts schema
- #79 adapter
- #80 storage
- #81 orchestrator DAG/状态机
- #82 AgentExecutor/VerificationExecutor
- #83 CLI
- #84 恢复对账与故障注入
- #85 fixture 端到端验收（MVP 退出）
- #86 （可选）worktree 并行

### P3 记忆与 RSI（5–8 工程日）
- #87 FTS5 混合索引
- #88 检索 MCP server
- #89 RSI 闭环 v1（候选→评估→审批→晋升→回退）

### P4 智能体网关（10–15 工程日）
- #90 统一入口 + 租户进程池
- #91 IAM/RBAC
- #92 渠道接入
- #93 cron 执行器
- #94 业务 MCP + 应用接入

### P5 分布式与市场（远期）
- #95 exec-server 远程执行
- #96 agent 注册/发现
- #97 Model Routing
- #98 能力包市场
- #99 workflow 画布工作台
- #100 Tauri 桌面复活

### fork 触发式最小变更（未触发不实施）
- #101 agent/* RPC（待 #71 裁决）
- #102 skills extraRoots 持久化（待 #68 裁决）

## 工程量
MVP（P0–P2）约 14–23 工程日；P3 5–8 日；P4 10–15 日；P5 另议。P0 后重估。

## 关键风险
1. 上游漂移：引擎版本按发布锁定，schema fixtures 构建期生成，季度合并。
2. 本机端点安全按进程名 SIGKILL codex：引擎一律重命名 superagent-engine。
3. thread/start.multiAgentMode 已废弃：以 P0 实测触发方式为准。
4. 提示词编排不可靠：成功判定只认门禁证据。
5. 上下文预算：常驻 ≤400 / 交接 ≤800 / 单片段 ≤900 / 每轮新增 ≤2000 tokens。

## 首个可评审交付
P0 四条 trace（真实团队任务/失败 gate/确认取消/断连对账）+ 接口决策清单 + P1 TUI 元智能体团队演示。

