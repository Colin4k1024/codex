# F-2 / #102 裁决记录：skills extraRoots 持久化 — 选定产品侧规避，不实施 fork（#102 保持 open，关闭门禁=#76）

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
依据：#68（P0-5）已合并证据（PR #106，grok APPROVE），本记录为 #102 的 fork 裁决交付。
**本 PR 不关闭 #102**：核心验收"能力包技能重启后仍生效"尚未端到端证明（见 §4/§6）。

## 1. 触发条件评估

#102 触发条件："#68 裁决：skills/extraRoots/set 重启丢失且无配置键可承载"。

**缺口成立**（superagent/evidence/p0-5/，均在本 HEAD 可查）：
- persistence-conclusion.json + skills-session2.json：新进程 `skills/list` 不含上一进程
  经 `skills/extraRoots/set` 注入的 spike-greet → NOT_PERSISTENT。
- skills-persistence-conclusion.md「持久化书面结论」：`skills/extraRoots/set` 只更新进程内
  运行时 roots + watcher（codex-rs/app-server/src/request_processors/catalog_processor.rs:632-648；
  codex-rs/ext/skills/src/host_service.rs:178-187 RwLock）；`SkillsConfig`
  （codex-rs/config/src/skills_config.rs:41-56）无 extra roots 字段，config.toml 无键可承载。

## 2. 规避可行性（issue 明示优先路径）

#102 形态条款："优先产品侧规避（安装器每次启动重设 roots）；确认规避不可行才改 fork"。

**规避可行且已实证**：skills-session1.json 即"全新引擎进程 → initialize →
skills/extraRoots/set → skills/list 立即列出技能 → 模型 turn 实际触发"，
与安装器/启动器每次会话启动的注入路径完全同构。$ 提及与隐式触发均在注入后当场生效
（mention-implicit.json：mentionTriggered=true、implicitTriggered=true）。
不存在"规避不可行"的情形，因此**不满足改 fork 的前置条件**。

## 3. 裁决

- **不实施 fork 变更**：SkillsConfig / app-server / schema 均不动，上游合并性零影响。
- **持久化义务转移至产品侧**：#76（P1-4 安装器 + superagent 启动器）必须在每次引擎会话
  启动时调用 `skills/extraRoots/set` 注入已安装能力包 roots；该义务已写入 #76 验收链路
  （doctor 含 pack 校验；干净 macOS 一条命令安装后技能可用）。
- **#102 不随本 PR 关闭**：本记录仅交付 fork 裁决（规避路径选定 + 机制可行性证明）。
  #102 保持 open，状态=blocked-on-#76，直至 §6 关闭门禁满足。
- **重开 fork 条件**：若未来产品要求引擎原生持久化（例如第三方客户端不经安装器直接拉起引擎
  且要求技能常驻），或 #76 实证规避路径不可行，按 #102 原形态（最小配置键 + schema 重导出 + 测试）实施。

## 4. 验收逐条对照

| #102 验收 | 状态 | 证据 |
|---|---|---|
| 能力包技能重启后仍生效 | **NOT_PROVEN（本 PR 不声称达成）**。已证的只是注入机制：新进程 set→list 立即可用（skills-session1.json），这降低所选规避路径的风险，但不等于"重启后仍生效"。端到端生效依赖 #76 安装器每次启动重注入，#76 尚未实现 | p0-5/skills-session1.json（机制）；persistence-conclusion.json（裸重启 NOT_PERSISTENT） |
| schema 更新 | 不适用：未实施 fork（形态条款仅在改 fork 时要求 schema 重导出） | 本记录 §3；git diff 零引擎改动 |
| 上游合并友好 | 满足：零引擎源码改动，仅新增产品侧证据文件 | 本 PR diff |

## 5. 诚实性声明

- "重启后仍生效"端到端演示**尚未发生**（#76 未实现）。机制级证明（新进程注入即生效）
  与义务转移（#76）**不计入该验收的达成**；本记录明确将其标记为 NOT_PROVEN。
- 本记录不修改任何引擎代码；全部引用证据来自已合并、已独立审查的 #68 PR。

## 6. #102 关闭门禁（显式）

#102 仅当以下全部满足才可关闭：
1. #76 交付真实 trace：干净环境一条命令安装能力包 → 启动引擎（技能生效）→ **重启引擎**
   → 安装器/启动器自动重注入 → 技能仍生效（skills/list + 模型 turn 实际触发）；
2. 该 trace 作为证据提交入库并经 grok 独立审查 APPROVE；
3. 若规避路径被实证不可行，则回到 #102 原形态实施 fork 并按其验收关闭。
在此之前 #102 保持 open（blocked-on-#76），不得以"机制已证/义务已转移"为由关闭。

## 7. 审查整改记录

r1 grok CHANGES_REQUESTED（1 blocker）：F1 "重启后仍生效" NOT_PROVEN，机制级证明+义务转移
不得计入验收。整改：标题/§3/§4/§5 改为 NOT_PROVEN 明示；新增 §6 显式关闭门禁；
PR 移除 "Closes #102"；#102 保持 open 并记录 blocked-on-#76。