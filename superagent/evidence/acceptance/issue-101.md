## 触发条件
#71 裁决：外部客户端仅凭 thread 事件/items 不足以观测/控制子代理。**未触发前不实施。**

## 形态（触发后）
- [ ] app-server v2 新 method family：agent/list、agent/read（只读优先），必要时 agent/interrupt、agent/resume
- [ ] 包装现有 AgentRegistry/ThreadManager，不往 codex-core 加代码
- [ ] 单 PR ≤800 行；just write-app-server-schema；协议集成测试
- [ ] 保持上游可合并性（新文件为主，避免改既有表面）

## 验收
编排器经公开 RPC 观测/控制子代理；schema/TS 导出更新；测试通过。

依赖 #71。参考设计 §6。

