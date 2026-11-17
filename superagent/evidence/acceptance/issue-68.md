## 背景
能力包（Swarm Skill）经现有 skills 加载路径注入。需验证 extraRoots 的加载与持久化，决定是否触发 fork 最小变更 {{F-2}}。

## 任务
- [ ] 示例技能目录经 skills/extraRoots/set 加载；目录合并与提示注入可见
- [ ] $ 提及与隐式触发验证
- [ ] 重启后 extraRoots 是否持久；不持久则验证配置路径（SkillsConfig）
- [ ] max_context_tokens 目录上限行为

## 验收
技能可见可触发；持久化书面结论（触发 {{F-2}} 裁决）。

## 代码
skills/；ext/skills/；config/src/skills_config.rs。依赖 #64。

