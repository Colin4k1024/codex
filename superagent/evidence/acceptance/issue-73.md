## 背景
能力包是元智能体的"人格+团队+流程"载体，零引擎源码改动。吸收 HARNESS_INTEGRATION §5 的 bundle/digest 原则。

## 任务
- [ ] 独立产品仓库 pack/ 目录：roles/ skills/ workflows/ templates/
- [ ] manifest：schema_version/version/digest/来源许可（LICENSES）
- [ ] 确定性 digest 校验（CI 重导出比对）
- [ ] 版本绑定规则：run 固定 bundle digest，升级不静默换规则

## 验收
干净环境 manifest 校验通过；digest 可复现；许可清单完整。

依赖 #72。参考设计 §3。

