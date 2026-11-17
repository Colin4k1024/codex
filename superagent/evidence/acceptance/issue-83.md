## 背景
产品 CLI 入口，复用编排器服务接口。

## 任务
- [ ] superagent run team-delivery --task "..." / --task-file requirement.md / --dry-run
- [ ] superagent status <run-id> --json；resume；cancel；report
- [ ] cancel：停止调度新节点 → turn/interrupt 或 command/exec/terminate → 持续消费终态 → 有界关闭引擎 → 确认清理后标记 cancelled
- [ ] report 区分已验证/未验证项、已有变更与本次变更；记录版本/用量/证据
- [ ] 无交互模式遇所需授权返回 blocked 和稳定退出码

## 验收
dry-run 不启动模型/命令；取消有清理证据；report 内容完整。

依赖 #81、#82。参考设计 §4 P2；HARNESS_INTEGRATION §6 命令语义。

