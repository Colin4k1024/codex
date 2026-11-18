# P4-3 渠道接入（#92）结论

## 交付
`superagent/crates/channels`（superagent-channels）：Lark/Slack/webhook 入站适配 →
归一化任务 → 出站审批卡 → 渠道回复的确定性闭环。**model-free / process-free /
network-free**：依赖仅 serde、serde_json 与 `superagent-iam`（path ../iam，复用 #91
身份映射）；无 Command / 网络 / 模型调用。

## 验收对照（一个渠道端到端：消息→任务→审批→回复）
- **(A) 入站适配 → 网关 → 元智能体会话**：`parse_webhook` / `parse_lark`
  (`im.message.receive_v1`，content 为 JSON 字符串需二次解析) / `parse_slack`
  (Events API `message`，thread_ts 优先于 ts) 将各渠道载荷归一为 `InboundMessage`。
  `ChannelRouter::ingest` 经 IAM `map_channel_identity` 绑定 tenant/role/user，生成
  `Task{session_id=tenant:thread}`。测试 `webhook_end_to_end_message_task_approval_reply`
  走通 webhook 全链路并断言审计链 `ingest→approval_requested→approval_decided`；
  `lark_and_slack_inbound_normalization` 断言两渠道归一形状与畸形载荷拒绝。
- **(B) 出站审批卡不藏真实命令**：`ApprovalRequest{action(原始命令), cwd,
  permission_scope, reason}`，`ApprovalCard::render` 逐字输出 action/cwd/
  permissionScope/reason/choices=[allow,deny]。测试 `approval_card_shows_raw_command`
  以真实危险命令 `curl ... -d @secrets.env` 断言 render 与序列化字符串均**逐字包含**
  原始命令（未脱敏/未隐藏）。
- **(C) 渠道身份 → IAM 映射**：`channel_identity_iam_mapping` 断言 slack U123→
  u1(member,t1)，未映射身份 `UnknownIdentity` 拒绝（不静默放行），且映射身份可经
  IAM `issue_token`/`authenticate` 往返。
- **(D) 断开显示"执行状态待确认"而非"已取消"**：`mark_disconnected` 将 Running 任务
  置 `PendingConfirmation`（label="执行状态待确认"），**绝不**置 Cancelled。测试
  `disconnect_pending_confirmation_not_cancelled` 断言 label=="执行状态待确认" 且
  !="已取消"，审计 status 字段一致。
- **审批守卫**：`approval_deny_and_guards` 断言 Deny→已拒绝回复+任务 Denied、重复决策
  `AlreadyDecided`、未知卡/任务 `UnknownCard`/`UnknownTask`、非 Running 任务
  `NotRunning` 不能再请求审批。

## 边界（诚实声明）
- 本 crate 是渠道适配/审批/身份映射的**确定性协议层**，不启动真实引擎进程；端到端
  测试以归一化任务+审批卡+回复证明协议闭环。真实引擎会话执行由 #90 网关引擎池承载
  （gateway crate 以 caller-supplied argv 启动 per-tenant 引擎），本 crate 不重复造
  进程管理，亦不声称已驱动真实 Lark/Slack 服务端 webhook 投递。
- 审批卡 choices 固定 [allow,deny]，决策幂等（一卡一决策）。

## 门禁（raw 转录，toolchain 1.99.1）
- `cargo +1.99.1 fmt --check` exit 0；`clippy --all-targets -- -D warnings` exit 0，0 error。
- `cargo +1.99.1 test`：0 unit + 6 integration passed，test-exit=0；连续 5 次稳定运行均 6 passed。
- 证据：`fmt-clippy.log`、`test-output.log`（原始 2>&1 透传 + echo 退出码）。

## 测试清单（tests/channels.rs，6）
1. webhook_end_to_end_message_task_approval_reply
2. lark_and_slack_inbound_normalization
3. channel_identity_iam_mapping
4. approval_card_shows_raw_command
5. disconnect_pending_confirmation_not_cancelled
6. approval_deny_and_guards