## 背景
MVP（P0–P2）退出验收：team-delivery 工作流真实闭环。

## 任务
- [ ] fixture：intake（scope.json）→ plan（plan.json schema 验证）→ implement（change-summary.json）→ verify（真实退出码）→ review（review.json 无未解决阻塞项）→ report（report.md+json）
- [ ] 真实缺陷修复且测试通过；失败测试触发最多两次修复
- [ ] 不可修复 fixture 停在 failed/blocked；缺验收命令进入 waiting_input
- [ ] 首版串行单工作区；记录初始 HEAD 与工作区状态；不自动 git add/commit/reset
- [ ] 端到端 trace + 报告归档；MVP 退出评审

## 验收
不以"完成了"判成功；全部节点终态有证据；报告明确支持平台与限制。

依赖 #83、#84、#77。参考 T18；设计 §4。

