## 背景
编排器经 app-server stdio JSON-RPC 驱动引擎。继承 HARNESS_STANDALONE §4 接入契约。

## 任务
- [ ] spawn superagent-engine app-server --listen stdio://（参数数组，stdout 专用 JSONL，stderr 单独收集）
- [ ] initialize/initialized → thread/start → turn/start → 持续事件泵 → turn/completed
- [ ] 命令/文件审批、用户输入、MCP elicitation 的 typed handler；未知 server request 返回协议错误并阻塞，不自动批准
- [ ] 断连/超时行为；等待审批时仍消费其他事件
- [ ] 资源上限初值：单帧 16MiB、日志单 run 100MiB；请求超时按类型区分

## 验收
真实模型任务、一次拒绝审批、确认取消、断连对账四条 trace（与 {{P0-9}} 共享）。

## 参考
app-server/README.md；HARNESS_STANDALONE T03/T04。依赖 #64。

