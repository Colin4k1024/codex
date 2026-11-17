## 背景
adapter 是编排器与引擎的唯一协议边界，继承 HARNESS_STANDALONE §4 接入契约。

## 任务
- [ ] stdio 子进程管理：参数数组 spawn、stdout JSONL/stderr 分离限大小、退出清理
- [ ] request 关联、持续 event pump（response/notification/server request 三路）、typed 审批 handler
- [ ] 资源上限：单帧 16MiB、日志单 run 100MiB、产物单文件 20MiB（trace 校准）；超限显式报错不伪造
- [ ] 超时按类型区分：连接/查询有限超时；模型任务用运行截止时间；审批 waiting_input；command/exec 等程序退出
- [ ] UI/上层每 30–100ms 合并文字增量；终态与审批立即处理；过载不悄悄丢终态

## 验收
慢消费者、超长行、大日志、未知事件、未知请求、引擎崩溃测试通过；不静默丢终态。

依赖 #69。参考 T03/T04/T09。

