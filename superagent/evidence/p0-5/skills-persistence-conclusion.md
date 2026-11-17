# P0-5 / #68 结论：skills extraRoots 加载、触发与持久化（F-2 裁决）

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64

## 实测结果
1. **加载可见**：`skills/extraRoots/set {extraRoots:[…/p0-5/skills]}` 后 `skills/list`
   立即列出 spike-greet（skills-session1.json）。
2. **$ 提及触发**：turn 输入 `$spike-greet` → 模型精确回复 `SUPERAGENT_SPIKE_OK`
   （mention-implicit.json mentionTriggered=true）。解析路径：skills/src/mentions.rs:41,84-100
   → ext/skills/src/selection.rs:21-66。
3. **隐式触发**：同线程追问"spike passphrase greeting"（无 $ 提及）→ 仍回复精确口令
   （implicitTriggered=true）。隐式路径为提示驱动（catalog_prompt.rs:8）+
   detect_implicit_skill_invocation（ext/skills/src/invocation.rs:41-70）。
4. **重启持久化**：新进程 `skills/list` 不再含 spike-greet
   （skills-session2.json）→ **NOT_PERSISTENT**（persistence-conclusion.json）。
5. **目录上限**：config `[skills] max_context_tokens = 1` 时，skills/list 仍列出技能，
   但模型 turn 报告无可用技能（context-cap.json capExcludesSkillFromModel=true）。
   执行点：ext/skills/src/render.rs:169-195（配置值上限 10000 token，默认上下文 2%），
   超限技能从模型可见目录剔除并告警（render.rs:140-156）。

## 持久化书面结论
- `skills/extraRoots/set` 只更新进程内运行时 roots + watcher，发 skills/changed
  （app-server catalog_processor.rs:632-648；ext/skills/src/host_service.rs:178-187 RwLock）。
- `SkillsConfig`（config/src/skills_config.rs:41-56）**没有 extra roots 字段**，
  config.toml 无任何键可持久化 extraRoots。
- 因此产品侧"重启后能力包仍在"无法经现有配置路径达成。

## F-2 裁决：缺口成立，但按 #102 形态优先产品侧规避 → 不改 fork
#102 明确"优先产品侧规避（安装器每次启动重设 roots）；确认规避不可行才改 fork"。
规避可行性已实证：skills-session1.json 即"新进程 → extraRoots/set → 技能立即可用"，
与安装器启动路径完全同构。因此：
- **不实施 fork 变更**；SkillsConfig 不动，上游合并性不受影响。
- 产品安装器/启动器（P1-4，#76）必须在每次引擎会话启动时调用 skills/extraRoots/set
  注入已安装能力包 roots，作为持久化的产品侧承载。
- #102 以"规避可行且已验证、fork 不需要"关闭；若未来要求引擎原生持久化，再按 #102 形态重开。