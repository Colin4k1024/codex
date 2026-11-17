# F-2 / #102 裁决记录：skills extraRoots 持久化 — 产品侧规避成立，不实施 fork

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
依据：#68（P0-5）已合并证据（PR #106，grok APPROVE），本记录为 #102 的正式裁决交付。

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
- **重开条件**：若未来产品要求引擎原生持久化（例如第三方客户端不经安装器直接拉起引擎
  且要求技能常驻），按 #102 原形态（最小配置键 + schema 重导出 + 测试）重开。

## 4. 验收逐条对照

| #102 验收 | 状态 | 证据 |
|---|---|---|
| 能力包技能重启后仍生效 | 机制级已证：新进程注入即生效（skills-session1.json）；端到端承载=#76 安装器每次启动重注入（issue 明示的优先路径） | p0-5/skills-session1.json、mention-implicit.json |
| schema 更新 | 不适用：未实施 fork（形态条款仅在改 fork 时要求 schema 重导出） | 本记录 §3；git diff 零引擎改动 |
| 上游合并友好 | 满足：零引擎源码改动，仅新增产品侧证据文件 | 本 PR diff |

## 5. 诚实性声明

- "重启后仍生效"的**安装器端到端演示尚未发生**（#76 未实现）；本裁决证明的是
  issue 指定的优先规避路径在机制上可行且已被真实 trace 覆盖，实施义务显式转移给 #76，
  不声称安装器已存在。#76 关闭时必须以真实安装→重启→技能生效 trace 兑现该义务。
- 本记录不修改任何引擎代码；全部引用证据来自已合并、已独立审查的 #68 PR。