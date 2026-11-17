## 背景
两类执行器契约，继承 HARNESS_STANDALONE §7。模型一轮完成仅代表执行结束，节点成功要求 gate 全过。

## 任务
- [ ] AgentExecutor：runId/nodeId/attemptId/workspace/角色/产物引用/预算/权限配置 → 终态+threadId+turnId+用量+artifact 引用
- [ ] VerificationExecutor：program/argv/cwd/timeout/输出上限/权限配置 → 真实退出码+日志位置+验收报告（经 command/exec）
- [ ] 无法表达当前沙箱策略时返回 blocked，不退回宿主 shell/externalSandbox
- [ ] 节点修复沿用该节点会话；节点间传 artifact 引用+有限摘要

## 验收
真实模型节点修改 fixture；gate 拿真实退出码；不用模型自报退出码。

依赖 #79、#70。参考 T17。

