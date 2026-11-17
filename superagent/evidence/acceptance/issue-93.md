## 背景
Cron 执行器放网关，不进 fork。plugin ScheduledTaskSummary（hourly/daily/weekdays/weekly）仅作声明元数据参考。

## 任务
- [ ] tokio 定时调度器 → thread/turn 触发元智能体任务
- [ ] 调度定义 CRUD + 租户隔离；misfire 策略（补跑/跳过）
- [ ] 防重复触发（租约/幂等键）；触发审计

## 验收
定时任务按时触发且可审计；并发/重启不重复触发。

依赖 #90。

