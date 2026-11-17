## 背景
Model Routing 缺口：现仅 spawn 级模型覆盖（agents.default_subagent_model）。补任务类→档位路由。

## 任务
- [ ] 路由服务：任务类→模型档位映射（消费 model-provider 配置）
- [ ] 记录实际选择与用量；成本/延迟基线对比
- [ ] 与 spawn 级 override 兼容；路由不扩大用户/管理员限制

## 验收
路由策略可配置可审计；同 fixture 成本/延迟对比报告。

依赖 #90。代码：model-provider/；models-manager/。

