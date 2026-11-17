## 背景
Agent Protocol 缺口：注册/发现/跨机通信。现有基座：message-board 远程后端、agent-identity（Ed25519/JWT）、agent-message-board-client。

## 任务
- [ ] agent 能力注册/目录/心跳服务
- [ ] 跨主机消息路由（board 远程后端 + 会话级凭据）
- [ ] 身份：agent-identity JWT/JWKS 验证集成
- [ ] agent 跨机放置策略（最小：手动指定环境）

## 验收
两台主机的 agent 互相发现并完成一次交接；身份伪造被拒。

依赖 #95、#66。

