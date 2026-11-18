# P0-9 / #72 接口决策与 fork 缺口清单（定稿）

日期：2026-11-18 · 状态：**定稿**（依赖 #64–#71 全部关闭，四条 trace 验收满足）
复现：`python3 superagent/tests/p0_9_probe.py superagent/evidence/p0-9`（完整探针，含 turn/start 两项模型探针；
实测见 probe-summary.json，workdir=p09-probe-e8vo3ta3，model=gpt-6-astra）。

## 1. 引擎版本 / 摘要（p0-1/engine-manifest.json，#64 已合并）

- engine_name：superagent-engine（codex 重命名，规避本机安全软件对 "codex" 进程名的 SIGKILL）
- source_repo：Colin4k1024/codex · source_commit（manifest 基线）：425a848a9ce98dd31dda1adb62e2edd56ef26765
- engine_sha256（manifest 基线）：2421b8b5605f7487c092d8988c8eb941827448b15cf5c87802e6d99fa4ad6a2f
- v8 artifact：rusty-v8-v152.2.0 ptrcomp_sandbox aarch64-apple-darwin
- 平台矩阵：macOS arm64 实测；linux x64/arm64、Windows **未测**（P1 安装器 #76 义务）
- **引擎重建（#69 最小 fork）**：含 process_manager.rs turn-cancellation watcher 的 release 构建
  （cargo build --release --bin codex，0 error），inode 替换（rm+cp）至 superagent/packaging/superagent-engine；
  trace C/D 实证基于该重建引擎。manifest 基线 digest 对应 #64 接入证明，#69 fork 后以本清单 §4 裁决为准。

## 2. experimental API 实际可用性（probe-summary.json，实测）

| 接口 | experimentalApi=true | 无 capability |
|---|---|---|
| environment/info | AVAILABLE | GATED_REQUIRES_EXPERIMENTAL |
| memory/status | AVAILABLE | GATED_REQUIRES_EXPERIMENTAL |
| collaborationMode/list | AVAILABLE | 未测（同族推定 gated） |
| thread/start historyMode=paginated | AVAILABLE | GATED_REQUIRES_EXPERIMENTAL |
| thread/items/list | **UNSUPPORTED_SERVER_SIDE** | — |
| thread/goal/set/get/clear | AVAILABLE | 未测（goal 在 v2 schema 内，推定非 gated） |
| thread/queue/add/list/delete | AVAILABLE | 未测（#[experimental] 标注，推定 gated） |
| turn/start permissions=<profile> | **AVAILABLE_REQUIRES_CONFIG**（实测） | — |
| turn/start environments=[] | **AVAILABLE**（实测，空数组禁用本 turn 环境） | — |

schema 全量枚举（p0-8/no-agent-rpc.txt）：ClientRequest 110 方法；**agent/* = 0、graph/store = 0**。
queue 方法不在生成的 v2 schema JSON（#[experimental]，common.rs:666-701）。

### 2.1 turn/start experimental 字段语义（静态源码核验 + 运行时探针实测）

两字段均不在默认生成的 v2 TurnStartParams.json（#[experimental] 门控），仅 experimentalApi=true 时可用：

- `permissions: Option<String>`（codex-rs/app-server-protocol/src/protocol/v2/turn.rs:258-262）：
  选择命名 permissions profile id，作用于本 turn 及后续 turn；**不可与 `sandboxPolicy` 同时传**。
- `environments: Option<Vec<TurnEnvironmentParams>>`（同文件 :231-238）：
  省略=沿用线程 sticky environments；**空数组=本 turn 禁用环境访问**；非空=选第一个为当前 turn 环境。
- 未知 profile **实测报错**（probe-summary.json id=6）：JSON-RPC error code=-32600，
  message="failed to load configuration: default_permissions requires a `[permissions]` table"。
  接口被服务端处理，但当前 codex-home 缺 `[permissions]` 表，先在配置加载层 fail-closed。
  classify() 映射 **AVAILABLE_REQUIRES_CONFIG**：走 permissions profile 路径前必须注入 `[permissions]` 表；
  否则用 P0-7 已实证的显式 SandboxPolicy 路径（编排器裁决取后者）。
- environments=[] **实测**（probe-summary.json id=7）：turn/start 返回 result.turn.status=inProgress，
  空数组被接受（本 turn 禁用环境访问语义成立），状态 AVAILABLE。

## 3. 各 P0 裁决汇总（#64–#71 全部 CLOSED）

| Issue | 主题 | 裁决 | 关键约束 |
|---|---|---|---|
| #64 P0-1 | spawn/stdio 接入 | 充分，无 fork | 进程名必须非 codex（本机环境）；argv 数组 + PIPE |
| #65 P0-2 | MAv2 角色层 | 充分（model+developer_instructions 角色层生效）；sandbox 角色层不生效已根因+编排层兜底实证 | 角色 sandbox_mode 不被应用（role.rs:34-45 + child_config.rs:197-220）→ 编排层 command/exec SandboxPolicy 强制（P0-2e：越界写 exit_code=1 'operation not permitted'、区内写 exit_code=0）；V2 无 close_agent/resume_agent。grok r7 APPROVE |
| #66 P0-3 | message-board | 充分，无 fork | 双路径（会话内工具 + 外部 MCP）实证；ACL 边界成立 |
| #67 P0-4 | goal/预算/queue | 充分，无 fork | create_goal/get_goal/update_goal 模型工具往返实证（p0-4c）；queue 需 experimentalApi；忙时 queue/start 拒绝（thread_queue_processor.rs:228-237）。grok r6 APPROVE |
| #68 P0-5 | skills 持久化 | 充分，F-2 workaround | extraRoots + $提及/隐式触发实证；安装器义务 → #76 |
| #69 P0-6 | stdio 契约 | 充分 + **一处最小 fork**（yielded-exec 孤儿回收） | A/B/B2/B3/C/D allPass；资源上限客户端实现并实证（p0_6b 2/2）；B2 并发第二 thread 证明审批挂起期事件泵不阻塞；thread/read includeTurns 弃用=接受风险。grok r3 APPROVE |
| #70 P0-7 | exec gate | 充分，无 fork | 11/11 gate；编排器必须显式传 SandboxPolicy；legacy 字段语义拒绝 fail-closed |
| #71 P0-8 | 子代理可观测 | SUFFICIENT_NO_FORK | 拓扑=items+agentsStates+thread/read；thread/list 不列代理线程；外部单代理 interrupt/resume 不经 leader 不可达（F-1 形态，未触发）。grok r2 APPROVE |

## 4. fork 裁决

- **已实施最小 fork（#69，S0 证明 API 不足）**：yielded-exec 子进程在 turn 中断时不被回收（孤儿）。
  one-shot 完成路径（unified_exec/oneshot.rs:41-124）本就在取消时 terminate；yielded 路径
  （exec_command，completion=None）把进程存 manager，turn 中断 future 被 drop → 子进程组存活。
  现有 API 无 turn 级取消信号挂到 yielded 进程。修复限于 core/src/unified_exec/process_manager.rs
  新增 terminate_process_on_turn_cancellation watcher（镜像既有 network-denial watcher），仅对 yielded 路径生效。
  实证：trace C cleanupSeconds=0（sleep 子进程即时回收）。满足混合路线「仅 S0 证明不足时最小 fork」。
- **F-1（#101，子代理控制面 RPC）：未触发**。事件流 + thread/read + turn/interrupt + leader 中介工具
  满足 MVP 观测/取消需求。若产品后续要求"不经 leader 的外部单代理 interrupt/resume"，按 #101 形态重开。
- **F-2（#102，skills 持久化 workaround）：workaround 成立**，extraRoots 每会话注入；
  安装器/编排器承担持久化义务，跟踪于 #76（P1-4）。#102 裁决 PR#112 已合并，issue 保持 open blocked-on-#76。
- 其余缺口均为**裁剪范围**而非改源码：
  - thread/items/list 服务端未实现 → 用 rollout 文件 + thread/read（弃用接受风险，owner=编排器）；
  - 角色 sandbox_mode 不生效 → 编排器 command/exec 显式 SandboxPolicy（P0-2e 实证强制）；
  - MAv2 无 close/resume 工具 → leader 生命周期由 interrupt_agent + turn 结束管理；
  - MCP elicitation 运行时触发未构造 → handler+schema 已验证，产品如需该路径在 P2 补触发探针；
  - user-input 工具本 build 未向模型暴露 → 记 UNAVAILABLE，结论收窄（#69 trace B4）。

## 5. P1/P2 工程量重估（定稿）

- P1 编排器核心（#73–#77）：除 #69 已实施的最小 fork 外无需新增 fork 补丁，全部走公开 RPC + 客户端纪律；
  客户端义务：资源上限（参考实现 spike_client）、显式 SandboxPolicy、审批 decision 合法值表、
  孤儿感知对账（#69 fork 后 turn 中断孤儿即时回收，崩溃孤儿仍判 uncertain）、experimentalApi capability 开关。
- P2（#78–#86）：记忆/看板/技能产品化依赖 memory/status、board MCP、extraRoots——均已 AVAILABLE；
  thread/items/list 缺失使"分页历史 UI"裁剪为 rollout 读取，工程量 -1 项。
- 风险项：平台矩阵仅 macOS arm64（linux/Windows 待 #76 安装器验证）；CI Bazel 红为 main 基线环境问题（见 #122）。

## 6. 四条可复现 trace 汇总（验收）

| trace | 来源（已合并证据） | 复现 | 结果 |
|---|---|---|---|
| 真实团队任务 | #65 p0-2 team-turn-transcript.json（leader 自主组队 planner/implementer/verifier，calc.py a-b→a+b 修复）；#69 trace A（p0-6/trace-a-transcript.json，真实模型任务全链路） | p0_2b.py / p0_6.py | PASS |
| 失败 gate | #70 11/11 exec gate（p0-7/summary.json）；#69 trace B/B3（commandExecution/fileChange decline，副作用文件不存在） | p0_7.py / p0_6.py | PASS |
| 确认取消 | #69 trace C（p0-6/trace-c-transcript.json + trace-c-rerun.json，sleep 子进程确认存在→interrupt→cleanupSeconds=0） | p0_6_c.py / p0_6.py | PASS |
| 断连对账 | #69 trace D（p0-6/trace-d-precrash.json + trace-d-reconcile.json，SIGKILL 引擎→新连接对账，孤儿感知 terminalProven） | p0_6.py | PASS |

清单经 grok 独立评审（本 PR）；缺失接口明确裁剪范围（§4），不改源码补齐（#69 最小 fork 为 S0 例外，已裁决）。

## 7. 定稿确认

- [x] turn/start permissions/environments 探针（AVAILABLE_REQUIRES_CONFIG / AVAILABLE）
- [x] #65 关闭（grok r7 APPROVE，P0-2e 编排层沙箱实证）
- [x] #67 关闭（grok r6 APPROVE，p0_4c 模型工具往返）
- [x] #69 关闭（grok r3 APPROVE，A/B/B2/B3/C/D allPass + 最小 fork）
- [x] #71 关闭（grok r2 APPROVE，SUFFICIENT_NO_FORK）
- [x] #64/#66/#68/#70 关闭（先前合并）
- [x] 四条可复现 trace 汇总评审（§6）
- [x] 依赖 #64–#71 全部关闭，移除 DRAFT 标记