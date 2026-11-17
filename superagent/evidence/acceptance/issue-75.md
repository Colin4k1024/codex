## 背景
Swarm Skill = 团队交付流程技能，经现有 skills 路径注入，遵守上下文预算硬约束。

## 任务
- [ ] team-delivery 工作流技能（intake/plan/implement/verify/review/report 流程与产物契约）
- [ ] 委派策略与交接契约：节点间传 artifact 引用+摘要 ≤800 tokens
- [ ] 验收标准模板（门禁命令来源、schema）
- [ ] 预算合规：单片段 ≤900、每轮新增 ≤2000 tokens；超 1k 片段按仓库 P0 规则人工复核
- [ ] 长资料走文件读取/MCP，不灌提示词

## 验收
技能可触发；注入片段全数满足预算（实测）；低信任产物不经 developer 级注入提权。

依赖 #68、#73。参考设计 §2；AGENTS.md 上下文规则。

