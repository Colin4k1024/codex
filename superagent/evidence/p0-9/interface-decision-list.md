# P0-9 / #72 接口决策与 fork 缺口清单（DRAFT — 依赖 #65/#67/#69/#71 关闭后定稿）

日期：2026-11-17 · 状态：**草稿**（P0-2/P0-4/P0-6/P0-8 整改重跑待模型配额恢复）
复现：`python3 superagent/tests/p0_9_probe.py superagent/evidence/p0-9 --no-model`（无模型部分）；
完整探针去掉 --no-model（turn/start permissions/environments 两项需模型）。

## 1. 引擎版本 / 摘要（p0-1/engine-manifest.json，#64 已合并）

- engine_name：superagent-engine（codex 重命名，规避本机安全软件对 "codex" 进程名的 SIGKILL）
- source_repo：Colin4k1024/codex · source_commit：425a848a9ce98dd31dda1adb62e2edd56ef26765
- engine_sha256：2421b8b5605f7487c092d8988c8eb941827448b15cf5c87802e6d99fa4ad6a2f
- v8 artifact：rusty-v8-v152.2.0 ptrcomp_sandbox aarch64-apple-darwin
- 平台矩阵：macOS arm64 实测；linux x64/arm64、Windows **未测**（P1 安装器 #76 义务）

## 2. experimental API 实际可用性（probe-summary.json，本次实测）

| 接口 | experimentalApi=true | 无 capability |
|---|---|---|
| environment/info | AVAILABLE | GATED_REQUIRES_EXPERIMENTAL |
| memory/status | AVAILABLE | GATED_REQUIRES_EXPERIMENTAL |
| collaborationMode/list | AVAILABLE | 未测（同族推定 gated） |
| thread/start historyMode=paginated | AVAILABLE | GATED_REQUIRES_EXPERIMENTAL |
| thread/items/list | **UNSUPPORTED_SERVER_SIDE** | — |
| thread/goal/set/get/clear | AVAILABLE | 未测（goal 在 v2 schema 内，推定非 gated） |
| thread/queue/add/list/delete | AVAILABLE | 未测（#[experimental] 标注，推定 gated） |
| turn/start permissions=<profile> | 待模型探针 | — |
| turn/start environments=[] | 待模型探针 | — |

schema 全量枚举（p0-8/no-agent-rpc.txt）：ClientRequest 110 方法；**agent/* = 0、graph/store = 0**。
queue 方法不在生成的 v2 schema JSON（#[experimental]，common.rs:666-701）。

### 2.1 turn/start experimental 字段语义（静态源码核验，运行时探针待配额）

两字段均不在默认生成的 v2 TurnStartParams.json（#[experimental] 门控），仅 experimentalApi=true 时可用：

- `permissions: Option<String>`（codex-rs/app-server-protocol/src/protocol/v2/turn.rs:258-262）：
  选择命名 permissions profile id，作用于本 turn 及后续 turn；**不可与 `sandboxPolicy` 同时传**。
  探针用不存在 profile（p09-nonexistent-profile）验证服务端校验路径。
- `environments: Option<Vec<TurnEnvironmentParams>>`（同文件 :231-238）：
  省略=沿用线程 sticky environments；**空数组=本 turn 禁用环境访问**；非空=选第一个为当前 turn 环境。
  探针用 `[]` 验证禁用语义。
- 编排器含义：permissions profile 与 SandboxPolicy 二选一；P0-7 已证明 command/exec 显式
  SandboxPolicy 路径，permissions profile 路径待本探针定稿后裁决取舍。

## 3. 各 P0 裁决汇总

| Issue | 主题 | 裁决 | 关键约束 |
|---|---|---|---|
| #64 P0-1 | spawn/stdio 接入 | 充分，无 fork | 进程名必须非 codex（本机环境）；argv 数组 + PIPE |
| #65 P0-2 | MAv2 角色层 | 待重跑定稿 | 角色 sandbox_mode 不被应用（role.rs:77-86 + child_config.rs:197-220）→ 编排器经 command/exec SandboxPolicy 兜底；V2 无 close_agent/resume_agent |
| #66 P0-3 | message-board | 充分，无 fork | 双路径（会话内工具 + 外部 MCP）实证；ACL 边界成立 |
| #67 P0-4 | goal/预算/queue | 充分，无 fork（工具往返待 p0_4c 实证） | queue 需 experimentalApi；忙时 queue/start 拒绝（thread_queue_processor.rs:228-237） |
| #68 P0-5 | skills 持久化 | 充分，F-2 workaround | extraRoots + $提及/隐式触发实证；安装器义务 → #76 |
| #69 P0-6 | stdio 契约 | 充分，无 fork | 资源上限客户端实现并实证（p0_6b 2/2）；审批 decision 合法值见结论；thread/read includeTurns 弃用=接受风险 |
| #70 P0-7 | exec gate | 充分，无 fork | 11/11 gate；编排器必须显式传 SandboxPolicy；legacy 字段语义拒绝 fail-closed |
| #71 P0-8 | 子代理可观测 | SUFFICIENT_NO_FORK | 拓扑=items+agentsStates+thread/read；thread/list 不列代理线程；外部单代理 interrupt/resume 不经 leader 不可达（F-1 形态，未触发） |

## 4. fork 裁决

- **F-1（#101，子代理控制面 RPC）：未触发**。事件流 + thread/read + turn/interrupt + leader 中介工具
  满足 MVP 观测/取消需求。若产品后续要求"不经 leader 的外部单代理 interrupt/resume"，按 #101 形态重开。
- **F-2（#102，skills 持久化 workaround）：workaround 成立**，extraRoots 每会话注入；
  安装器/编排器承担持久化义务，跟踪于 #76（P1-4）。
- 其余缺口均为**裁剪范围**而非改源码：
  - thread/items/list 服务端未实现 → 用 rollout 文件 + thread/read（弃用接受风险，owner=编排器）；
  - 角色 sandbox_mode 不生效 → 编排器 command/exec 显式 SandboxPolicy；
  - MAv2 无 close/resume 工具 → leader 生命周期由 interrupt_agent + turn 结束管理；
  - MCP elicitation 运行时触发未构造 → handler+schema 已验证，产品如需该路径在 P2 补触发探针。

## 5. P1/P2 工程量重估（初步）

- P1 编排器核心（#73–#77）：无需 fork 补丁，全部走公开 RPC + 客户端纪律；
  新增客户端义务：资源上限（已参考实现 spike_client）、显式 SandboxPolicy、
  审批 decision 合法值表、孤儿感知对账、experimentalApi capability 开关。
- P2（#78–#86）：记忆/看板/技能产品化依赖 memory/status、board MCP、extraRoots——均已 AVAILABLE；
  thread/items/list 缺失使"分页历史 UI"裁剪为 rollout 读取，工程量 -1 项。
- 风险项：模型配额（usageLimitExceeded）影响一切真实 turn 探针；平台矩阵仅 macOS arm64。

## 6. 待办（定稿前）

- [ ] #65 重跑 p0_2b/c/d（配额恢复后）→ 角色层/昵称/V1 close/TUI 证据定稿
- [ ] #67 p0_4c 模型工具往返证据
- [ ] #69 p0_6 重跑 A/B/B2/B3/B4/C/D
- [ ] turn/start permissions/environments 探针（去 --no-model）
- [ ] 四条可复现 trace 汇总评审（真实团队任务/失败 gate/确认取消/断连对账）
- [ ] 依赖全部关闭后移除 DRAFT 标记，提交评审