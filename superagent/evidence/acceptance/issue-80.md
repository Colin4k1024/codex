## 背景
独立状态库，不与引擎 rollout 混 schema。继承 HARNESS_STANDALONE §8。

## 任务
- [ ] SQLite 迁移：runs/nodes/attempts/events/artifacts
- [ ] run 固定 workflow digest、bundle digest、输入摘要、仓库身份、起始 HEAD、配置摘要
- [ ] attempt 记录 thread/turn/process 关联；intent 先事务落盘，外部 ID 拿到后写绑定
- [ ] run 排他租约；事件自增序号供 UI 重放；日志落文件，库只存有界摘要/索引/引用

## 验收
并发租约竞争被拒绝；事务一致性测试；发送/落盘断点可注入（供 {{P2-7}}）。

依赖 #78。参考 T15。

