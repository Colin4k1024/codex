# SuperAgent 元智能体：基于 Codex 的落地方案

日期：2026-11-17。状态：设计提案，未实现。
架构依据：`/Users/fanjia/Downloads/架构图.drawio`（智能体技术架构分层图）。
路线决策（用户已确认）：**混合路线**——元智能体与团队能力优先用现有引擎的公开配置/协议承载（不改 core）；编排、网关、记忆增强放独立产品服务；仅当 S0 验证证明公开 API 不足时，才在 fork 中加最小 crate。
MVP 形态（用户已确认）：**CLI/TUI 元智能体优先**，网关与桌面后置。
代码基线：fork main `79f2ab2dd`（已合并 upstream 2026-11 最新）。
与旧文档关系：HARNESS_INTEGRATION_DESIGN / HARNESS_IMPLEMENTATION_TASKS 保持历史作废状态；HARNESS_STANDALONE_DESIGN 的**执行契约、状态机、恢复与验收原则全部继承**，本文在其上扩展为 SuperAgent 平台方案并取代其范围定义。

## 1. 核心判断：架构图的大部分内核已经存在

对 codex-rs 全仓核查后，架构图各层与现有代码的映射如下。**复用优先，缺口才新建。**

| 架构图模块 | 现有代码（可直接复用） | 真实缺口 | 落地阶段 |
|---|---|---|---|
| SuperAgent 元智能体 | MAv2 子代理工具（`core/src/tools/handlers/multi_agents_v2/`：spawn/send_message/wait/list_agents/interrupt_agent/followup_task）；`agent-roles` 角色配置；`collaboration-mode-templates` | 元智能体人格与团队策略 = **能力包**（角色+技能+指令），不是代码 | P1 |
| Swarm Engine（Team Collaboration / Workspace / TeamAgents） | `message-board`（SQLite/内存/远程三后端，channel/thread/post/订阅，`permissions.rs` 通道级 ACL）；`message-board-mcp`（caller-bound MCP）；`agent-graph-store`（父子拓扑）；`ext/goal`（长期目标+token 预算+steering）；`ext/queue`（空闲派发队列）；`worktree`（隔离工作区） | 无 team 实体/leader-member 层级——用"leader 线程 + 角色团队 + 运行频道"组合表达，编排策略在外层编排器 | P1/P2 |
| Harness Engine（Context / Skills / SubAgents / Privilege / Rails / KV Mgmt / Compute Scheduling） | `core/context` 片段体系；`skills` + `ext/skills`（目录合并、提示注入、`skills/extraRoots/set`）；MAv2；`sandboxing` + `execpolicy` + permissions profiles；`hooks`（14 事件含 SubagentStart/Stop、Pre/PostCompact）；`rollout` 压缩/compaction；`agents.max_concurrent_threads_per_session` | 确定性工作流（DAG/门禁/产物/恢复）= 独立编排器（Swarm Flow） | P2 |
| RSI Engine（Self-evolving / Auto Harness） | `memories/write` 两阶段 LLM 精炼管线（rollout 提取→ consolidation）已是记忆蒸馏闭环 | 经验→评估→审批→晋升→回退的自进化闭环不存在 | P3 |
| 智能体记忆（Refinement / Hybrid Retrieval / Evolution） | `memories`（读写+引用注入）；`ext/memories`（list/read/search/add_ad_hoc_note 工具）；`thread-store`/`rollout` SQLite 持久化 | 检索仅文件子串匹配，**无 FTS/向量** | P3 |
| 智能体网关（Unified Entry / IAM / RBAC / Channels / Cron） | `app-server-daemon`（单用户引擎生命周期）；`remoteControl/*`（设备配对）；`workload-identity`（机器身份）；`connectors`（应用目录） | 多租户统一入口、用户级 IAM/RBAC、IM 渠道接入、cron 执行器全部不存在（plugin `ScheduledTaskSummary` 只是元数据） | P4 |
| 分布式运行时（Serving / Executor / Protocol） | `exec-server` + `exec-server-protocol`（认证远程执行、环境注册、远程 FS/进程/沙箱）；`environment/*` API；`agent-identity`（Ed25519/JWT 运行时身份）；message-board 远程后端 | agent 本体的跨机放置与注册发现服务不存在（agent 都在单进程 ThreadManager 内） | P5 |
| 系统服务：Sandbox / Memory Store / Agent Infer | `sandboxing`（seatbelt/landlock/bwrap/Windows）；`thread-store`；`model-provider` + `models-manager` | — | 已有 |
| 系统服务：Model Routing / Online Learning | spawn 级 model 覆盖（`agents.default_subagent_model`） | 成本/延迟/任务类路由层；在线学习 | P5/P3+ |
| 智能市场中心（Agent/Skill/MCP/Plugin Hub） | `core-plugins` marketplace（add/remove/upgrade、curated、npm 源、bundle、`plugin/share/*`）；`plugin` manifest（skills+MCP+hooks+apps+UI）；`rmcp-client`；`plugin-host`（V8 沙箱 UI 插件） | Agent 能力包作为市场一等公民 | P5 |
| 开发工作台（Low-code / Canvas / Resource Mgmt） | workflow schema（P2 产物）是画布的数据基础 | 全部 UI | P5+ |
| 智能体应用（xiaohai / HCode / HR Super Agent / Symbiosis） | — | 均为网关 API 的消费方 | P4 后接入 |

**结论：不需要重写智能体框架内核。需要新建的是四样东西——能力包（P1）、确定性编排器（P2）、记忆/进化服务（P3）、网关（P4），外加远期的分布式与市场扩展（P5）。**

## 2. 元智能体运行时设计（两层编排）

关键架构决策：**外层确定性、内层模型驱动**。

```
superagent CLI/编排器（Swarm Flow，独立产品仓库）
  │  DAG / 门禁 / 产物 / 状态机 / 恢复 / 租约        ← 确定性，代码保证
  │  app-server stdio JSON-RPC（thread/turn/command）
  ▼
leader 线程 = SuperAgent 元智能体（引擎内，能力包配置）
  │  MAv2 工具 spawn/send_message/wait/interrupt     ← 模型驱动，提示词+角色约束
  ├─ planner 角色线程 ─┐
  ├─ implementer 角色线程 ─┤  message-board #run-<id> 频道交接（产物引用+有界摘要）
  ├─ verifier 角色线程 ─┤  ext/goal 目标与 token 预算
  └─ reviewer 角色线程 ─┘  memories 精炼 + P3 检索 MCP
```

- **元智能体本体**是一个开启 MAv2 的 codex 会话：`[features.multi_agent_v2] enabled = true`，`[agents]` 并发上限/默认子代理模型，`[agents.<role>]` 声明角色（description + config_file + nickname_candidates），角色文件放 `<CODEX_HOME>/agents/*.toml` 自动发现，每个角色可带独立 config 层（模型、沙箱、developer_instructions）。
- **Swarm Skill** = 能力包中的技能（团队交付流程、委派策略、交接契约、验收标准），经现有 skills 加载路径注入，遵守目录 token 上限（默认上下文 2%）。
- **Swarm Memory** = 现有 memories 精炼管线 + 运行频道历史；P3 加混合检索服务。
- **Swarm Flow** = 外层编排器持有的 DAG。节点成功由**真实门禁证据**（command/exec 退出码、产物 schema 校验）判定，禁止以模型自报"完成了"判成功。继承 HARNESS_STANDALONE_DESIGN §7/§8 的 AgentExecutor/VerificationExecutor 契约、状态机（pending→ready→running→verifying→succeeded，另有 waiting_input/blocked/failed/uncertain/cancelling）、intent 先落盘、uncertain 对账、run 租约。
- 注意：`thread/start.multiAgentMode` 已标记 deprecated（"Use Ultra reasoning effort for proactive multi-agent behavior"）。S0 必须验证当前引擎下多代理行为的实际触发方式（features 开关 + reasoning effort + 角色提示），不依赖废弃字段。
- 上下文预算遵守仓库规则：常驻元智能体介绍 ≤400 tokens；节点交接摘要 ≤800；单片段硬上限 900；每轮新增 ≤2000；超 1k 的新片段按 P0 人工复核。
- **预算测量口径（#74 Q1 grok 独立裁决，2026-11-18）**：「每轮新增 ≤2000」测量的是**编排器每轮注入的增量内容**（role instructions / handoff 摘要 / task 片段），而非引擎基础系统提示（约 20k，一次性）叠加 provider prompt-cache 命中行为。按此口径：首轮排除引擎基础上下文，后续轮 new=(input_tokens−cached_input_tokens)+output_tokens。字面 provider-token 口径在冷缓存（cached=0）首轮会把引擎基础上下文计入「新增」而产生非确定性 FAIL（如 luna 角色 37489/18431/38017），该字面 FAIL **非权威口径**；编排器注入增量内容实测（role instr 140–160 + handoff 27–48）远小于 caps，PASS。两种口径数据均透明记录于 evidence/p1-2/p1-2-runtime-conclusion.json 与 roles-conclusion.md §4。

## 3. 仓库与组件布局

独立产品仓库（暂名 `superagent`，品牌待定）：

```text
superagent/
  crates/adapter/          # app-server stdio JSON-RPC 客户端；事件泵、审批 handler、资源上限
  crates/contracts/        # 产品类型 + 导出 TS schema；workflow/node/gate/artifact JSON schema
  crates/orchestrator/     # Swarm Flow：DAG 调度、门禁、重试、预算、恢复对账
  crates/storage/          # SQLite：runs/nodes/attempts/events/artifacts + 租约 + 事件序号
  crates/cli/              # superagent run/status/resume/cancel/report/doctor
  crates/gateway/          # P4：统一入口 HTTP/WS、IAM/RBAC、渠道、cron
  crates/memory-service/   # P3：FTS5/向量混合索引 + 检索 MCP server
  pack/                    # 能力包：roles/ skills/ workflows/ templates/ + manifest（digest/来源许可）
  packaging/               # 引擎清单（版本/摘要/平台）、sidecar 安装、品牌资源
  tests/fixtures/          # team-delivery fixture + 协议 trace
```

- 引擎 = 本 fork 构建的二进制，**重命名为 `superagent-engine`**（本机端点安全会按进程名 SIGKILL 名为 codex 的进程，已验证改名可运行；产品分发也借此与原版隔离）。
- 产品专属 `CODEX_HOME`（如 `~/Library/Application Support/SuperAgent/codex-home`），不触碰用户 `~/.codex`。能力包安装到该 CODEX_HOME 的 skills/agents 位置。
- adapter 内维护独立 serde DTO，用所选引擎 `generate-json-schema` 输出锁定 wire 契约；产品 Rust 类型单独导出 TS，不把 Codex DTO 当前端契约。
- fork（本仓库）默认**零改动**；触发式最小变更见 §6。

## 4. 分阶段路线图

### P0 / S0：接口验证（3–5 工程日，阻塞后续）

用 fork 构建的重命名引擎逐项验证并留 trace：

1. `[features.multi_agent_v2]` + `[agents.*]` 角色：TUI 里 leader 真实 spawn ≥2 个角色线程，send_message/wait/interrupt 往返；确认多代理行为触发方式（effort/提示）。
2. message-board 工具在会话内可用；`message-board-mcp` stdio/HTTP 从外部客户端按身份读写频道。
3. `ext/goal`：create_goal/update_goal 预算与 steering；`thread/goal/*` API。
4. skills：能力包目录经 `skills/extraRoots/set`（或配置）加载，重启后是否持久；`$` 提及与隐式触发。
5. app-server stdio 全链路：initialize→thread/start→turn/start→事件消费→turn/completed；审批/用户输入 typed handler；未知 server request 拒绝。
6. `command/exec` 门禁：成功/失败/超时/取消，真实退出码；确认其权限不自动继承 thread。
7. `thread/read`/`thread/resume` 对账能力：子代理状态能否从客户端事件（SubAgentActivityItem 等）充分观测——**这是决定是否需要 fork 加 agent/* RPC 的关键实验**。
8. 专属 CODEX_HOME、账号登录、管理员策略优先级。

退出条件：真实模型团队任务 + 失败 gate + 确认取消 + 断连对账四条可复现 trace；输出《接口决策与 fork 缺口清单》。

### P1：能力包 MVP——TUI 里的元智能体（3–5 工程日）

- `pack/`：元智能体人格指令、5 个角色（planner/implementer/verifier/reviewer/reporter，角色文件含各自 config 层）、team-delivery  swarm skill、交接契约与验收标准模板、manifest（digest/许可）。
- 安装器：把包写入产品 CODEX_HOME；`superagent` 启动器以专属 CODEX_HOME 拉起 superagent-engine TUI。
- 验收：fixture 任务上 leader 自主组队（≥3 角色）、频道交接、goal 预算内完成、产出报告；注入片段全部满足 §2 预算；普通 codex 行为零回归（不改引擎）。

### P2：Swarm Flow 编排器——确定性工作流（8–13 工程日）

- 继承 HARNESS_STANDALONE_TASKS T13–T24 的全部契约与故障测试（四个发送/落盘断点、uncertain 对账、产物摘要失效、租约竞争、慢消费者/大日志/未知事件）。
- `superagent run team-delivery --task ... [--dry-run]` / status / resume / cancel / report；串行 DAG 起步，worktree 并行作为可选扩展（合并冲突阻塞下游、合并后重验）。
- 验收：fixture 缺陷真实修复且测试通过；不可修复 fixture 停在 failed/blocked；缺验收命令 waiting_input；崩溃恢复不重复已确认成功节点。

**MVP = P0+P1+P2，约 14–23 工程日。** 首个可评审交付：P0 trace + P1 TUI 团队演示。

### P3：记忆增强与 RSI 基础（5–8 工程日）

- memory-service：SQLite FTS5 起步（rollout/memories/board  posts 索引），embeddings 可选后置；以 MCP server 暴露混合检索（复用 message-board-mcp 的 caller-bound 模式），任何角色线程可查。
- RSI 闭环 v1：memories 精炼产出的候选经验 → 固定 fixture 评估 → **人工审批** → 晋升为能力包新版本（digest 固定、run 绑定旧版本）→ 可回退。不自动改提示词/技能/工具。
- 验收：检索命中率/延迟基线；一次完整的"候选→评估→晋升→回退"演练；恶意 artifact 不能提权。

### P4：智能体网关（10–15 工程日）

- 统一入口 HTTP/WS 服务包装 app-server；租户 = 专属 CODEX_HOME + 引擎进程（池化、生命周期复用 app-server-daemon 思路但多实例）。
- IAM/RBAC 在网关自有表（用户/角色/策略），引擎侧继续用 sandbox/permissions 做执行约束；两层不互相替代。
- 渠道：Lark/Slack/webhook 入站 → 网关 → 元智能体会话；出站审批卡走渠道交互。
- cron 执行器在网关（tokio 定时 → thread/turn），**不进 fork**；plugin ScheduledTaskSummary 仅作声明元数据。
- xiaohai/HCode/HR Super Agent 作为 API 消费方接入；业务写操作经自有 MCP，服务端落实授权/幂等/审计。

### P5：分布式运行时、市场与工作台（远期，按需求启动）

- 远程工具执行：exec-server + `environment/*`（已有）；agent 跨机放置与注册发现服务（board 远程后端 + agent-identity 身份）新建。
- Model Routing：任务类→模型档位路由服务，消费 `model-provider` 配置；记录实际选择与用量。
- 市场：能力包（agent pack）进入 plugin marketplace 体系；工作台画布以 P2 workflow schema 为数据模型。
- 桌面端（Tauri + ClaudeCodeUI 派生前端）按 HARNESS_TAURI_DESKTOP_DESIGN 在此阶段复活，复用 gateway API。

## 5. 与旧 HARNESS 文档的继承关系

| 旧资产 | 处置 |
|---|---|
| HARNESS_STANDALONE §4 接入契约、§7 执行契约、§8 状态/恢复/取消、§10 验收边界 | **全部继承**，引擎版本换为 fork 79f2ab2dd 构建 |
| HARNESS_STANDALONE_TASKS T03–T07、T13–T24 | 并入 P0/P2，编号保留可追溯 |
| Tauri 桌面优先（T01/T08–T12/T25–T27） | 后置到 P5；MVP 改 CLI/TUI |
| "零 Codex 源码改动" | 放宽为"默认零改动 + 触发式最小 fork"（§6） |
| HARNESS_INTEGRATION_DESIGN（codex-harness 内置 crate 路线） | 保持作废；其 bundle/digest/解包/上下文预算原则被能力包设计吸收 |
| harness-demo 内容源 | 仍只导出精选角色/模板，Claude 专用 hooks 逐项转换，不整体打包 |

## 6. fork 最小变更清单（触发式，S0 后裁决）

原则：每项必须证明公开 API 确实不足；新增独立 crate 或 app-server v2 表面，**不往 codex-core 加代码**；单 PR ≤800 行；改协议跑 `just write-app-server-schema`；改配置跑 `just write-config-schema`；保持上游可合并性。

| 候选 | 触发条件 | 形态 |
|---|---|---|
| `agent/*` 观测/控制 RPC（list/read/interrupt/resume） | S0-7 证明客户端事件不足以观测子代理状态 | app-server v2 新 method family，包装现有 AgentRegistry/ThreadManager，只读优先 |
| skills extra roots 持久化 | S0-4 证明 `skills/extraRoots/set` 重启丢失且无配置键 | 配置键或 API 修正，最小 diff |
| 团队工作区隔离增强 | P2 并行阶段 worktree 与 MAv2 组合有缺陷 | 视缺陷定位，优先产品侧规避 |
| cron / IAM / 路由 / 检索 | 一律放独立服务，**不进 fork** | — |

## 7. 风险与对策

1. **上游漂移**：引擎版本按发布锁定（版本+摘要），schema fixtures 构建期生成；季度性合并 upstream（本次 1507 提交合并的经验：冲突集中在 fork 定制面，定制面越小越安全——这也是混合路线的核心理由）。
2. **本机端点安全按进程名杀 codex**：产品引擎一律重命名 superagent-engine；CI/开发脚本同样处理。
3. **多代理触发方式变化**：multiAgentMode 已废弃，S0 以实际行为为准，不写死废弃字段。
4. **提示词编排不可靠**：成功判定只认门禁证据；外层状态机代码保证，内层模型自由度被角色契约+预算约束。
5. **上下文膨胀**：能力包遵守 §2 预算与仓库 P0 规则；技能目录受 max_context_tokens 约束；长资料走文件读取/MCP，不灌提示词。
6. **并发写冲突**：MVP 串行单工作区；并行必须 worktree 隔离+显式合并+重验，不自动 git add/commit/reset。
7. **安全边界**：引擎沙箱不约束远端业务副作用；业务 MCP 写操作服务端授权+幂等+审计；低信任产物/经验不得经 developer 级注入提权。
8. ** experimental API 依赖**：environments/permissions/historyMode 等均为 experimental，S0 记录实际可用性，缺失则裁剪范围而非改源码补齐。

## 8. 工程量与首个交付

一名熟悉 Rust/codex 的工程师粗估（S0 后重估）：P0 3–5 日，P1 3–5 日，P2 8–13 日，P3 5–8 日，P4 10–15 日；P5 另议。MVP（P0–P2）约 **14–23 工程日**。

首个可评审交付 = P0 四条 trace + 《接口决策与 fork 缺口清单》+ P1 TUI 元智能体团队演示（fixture 真实交付）。不以"能聊天"或"能 spawn"作为元智能体上线依据；以门禁证据与恢复演练为准。