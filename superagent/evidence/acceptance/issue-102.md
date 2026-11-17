## 触发条件
#68 裁决：skills/extraRoots/set 重启丢失且无配置键可承载。**未触发前不实施。**

## 形态（触发后）
- [ ] 最小配置键或 API 修正，使 extra roots 持久化
- [ ] just write-config-schema / write-app-server-schema；测试
- [ ] 优先产品侧规避（安装器每次启动重设 roots）；确认规避不可行才改 fork

## 验收
能力包技能重启后仍生效；schema 更新；上游合并友好。

依赖 #68。参考设计 §6。

