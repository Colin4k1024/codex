## 背景
外层确定性编排核心：成功由门禁证据判定，禁止自然语言"完成了"判成功。

## 任务
- [ ] 串行 DAG 依赖检查与调度
- [ ] 状态机：pending→ready→running→verifying→succeeded；waiting_input/blocked/failed/uncertain/cancelling
- [ ] 有限重试（验证失败最多两次修复，每次重跑相关 gate）、预算、wall_timeout
- [ ] 只有可重试失败生成下一次 attempt；非幂等节点（发布/迁移）不盲目重试
- [ ] 依赖失效传播：输入/bundle/产物变化使下游失效

## 验收
DAG 循环/失败传播测试；失败 gate 阻止 succeeded；预算耗尽 blocked/failed。

依赖 #78、#80。参考 T16；设计 §2。

