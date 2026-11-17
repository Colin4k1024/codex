## 背景
VerificationExecutor 的门禁必须拿到真实退出码。command/exec 不绑定 thread，权限不自动继承会话审批。

## 任务
- [ ] command/exec 成功/失败/超时/取消四态
- [ ] 越权写入与网络限制被沙箱阻止
- [ ] 显式传入沙箱策略；无法表达当前策略时返回 blocked，不退回宿主 shell
- [ ] 取消后确认子进程树清理

## 验收
门禁程序退出码可靠；越权被阻；取消有清理证据。

## 参考
app-server-protocol/src/protocol/v2/command_exec.rs；HARNESS_STANDALONE T05。依赖 #64。

