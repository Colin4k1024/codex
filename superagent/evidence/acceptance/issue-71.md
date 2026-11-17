## 背景
**关键裁决实验**：MAv2 子代理控制目前只有模型工具，没有公开 agent/* RPC。需确认外部编排器能否仅凭 thread 事件/items 充分观测子代理状态，决定是否触发 fork 变更 {{F-1}}。

## 任务
- [ ] 外部客户端经 thread 事件/items（SubAgentActivityItem 等）观测子代理生命周期（spawn/活动/终态）
- [ ] SubagentStart/SubagentStop hooks 事件作为补充观测面
- [ ] 评估是否需要 list/read/interrupt/resume 控制面；agent-graph-store 拓扑是否可经公开接口读取
- [ ] 输出裁决：充分（不改 fork）或缺口清单（触发 {{F-1}}），附事件样本

## 验收
书面裁决 + 事件样本；若缺口，列出最小 RPC 面建议。

## 代码
core/src/agent/registry.rs；agent-graph-store/；protocol items。依赖 #65、#69。

