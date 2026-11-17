## 背景
元智能体本体 = 开启 MAv2 的 leader 会话；角色团队经 [agents.*] + agents/*.toml 承载。

## 任务
- [ ] 元智能体人格指令：intake→委派→验证→报告；常驻介绍 ≤400 tokens（tokenizer 实测）
- [ ] planner/implementer/verifier/reviewer/reporter 五个角色文件：description/config_file/nickname_candidates
- [ ] 每角色独立 config 层：模型、沙箱、developer_instructions、交付契约
- [ ] leader 按人格自主 spawn 五角色验证

## 验收
TUI leader 按人格组队；角色隔离生效；预算实测达标。

依赖 #65、#73。参考设计 §2。

