# P4-5 / #94 业务 MCP 授权/幂等/审计 + 应用接入 — 验收结论

## 交付
- `superagent/crates/business-mcp/`：自有业务 MCP server（model-free，纯策略计算 + IAM 加密 + 内存 fixture 业务后端）。
  - 依赖 `superagent-iam`（path `../iam`，复用 #91）：身份 JWT authenticate、资源级 authorize（硬租户边界）、渠道身份映射、凭据卫生 redact/scan_for_secrets。
  - `BusinessMcpServer::call_tool(token, now, ToolCall)` 为网关 API 入口，**所有信任决策服务端执行**：
    1. 身份：`iam.authenticate`（签名 + 过期）。
    2. 应用租户边界：`claims.tenant != app.tenant` → `CrossTenant`（每应用一个租户）。
    3. 参数：`validate_params` 按 `ToolDef.required_params` 校验存在性 + 类型（String/Number/Bool）。
    4. 权限：`iam.authorize(claims, ResourceRef{Artifact, tenant, app:record}, Read/Write)`。
    5. 幂等：写操作必须携带非空 `idempotency_key`；`(app, key)` 命中缓存 → 返回缓存结果 `replayed=true`，**不再执行副作用**（防重复执行）。
  - 只读优先：`ToolKind::Read` 直接查询 fixture 后端；`ToolKind::Write` 走授权 + 审计 + 幂等。
  - 应用接入：`register_app(app_id, tenant, channel, external_id, user_id, role)` 创建 IAM 用户 + 绑定渠道身份；xiaohai/HCode/HR 各占独立租户/渠道。
  - 审计：`register_app / call_ok / call_denied / write_executed / write_replayed` 全链路；`audit()` 不含 token/secret（测试用 `scan_for_secrets` 验证）。

## 验收对照（tests/business_mcp.rs，0 unit + 7 integration）
| 验收要求 | 测试 | 断言 |
|---|---|---|
| 一个真实业务只读查询端到端 + 至少一个应用接入演示 | `readonly_query_end_to_end_app_onboarding` | xiaohai 渠道身份→IAM 用户/租户；token→只读查询返回 fixture 记录；审计 call_ok |
| 服务端校验身份/参数/权限 | `server_side_identity_param_permission_validation` | 坏 token→MalformedToken；缺参/错类型→InvalidParams；未知 tool/app→UnknownTool/UnknownApp |
| 写操作幂等 + 防重复执行 | `write_idempotency_anti_replay` | 同 key 二次调用 replayed=true、write_count 不增、结果一致；异 key 新增副作用 |
| 写操作幂等键强制 | `write_requires_idempotency_key` | 无 key→MissingIdempotencyKey，write_count=0 |
| 写操作授权 + 审计 | `write_authorization_and_audit_hygiene` | viewer 写→Forbidden 且 write_count=0；member 写→ok + write_executed；审计无 secret/token 泄漏 |
| 每应用一个租户/渠道身份 | `per_app_tenant_channel_identity_isolation` | 三应用各自渠道→各自租户；xiaohai token 调 HCode→CrossTenant；HCode 自有 token 正常读 |
| MCP 工具审批不替代业务系统授权（边界） | `mcp_tool_approval_does_not_replace_business_authorization` | viewer + tool_approved=true 写仍 Forbidden；member + tool_approved=false 写仍允许 |

## 门禁（toolchain 1.99.1，raw 2>&1）
- `cargo +1.99.1 fmt --check` exit 0；`clippy --all-targets -- -D warnings` exit 0（无 error）。
- `cargo +1.99.1 test`：0 unit + 7 integration passed，test-exit=0；5 次连续稳定性运行全 7-passed。
- 见 `fmt-clippy.log` / `test-output.log`。

## 诚实边界（NOT PROVEN / 范围声明）
- **fixture 业务后端，非真实外部业务系统**：`Backend` 为内存 `records` + `writes` ledger，替代真实 xiaohai/HCode/HR 业务系统。"真实业务只读查询端到端"指**网关 API 路径真实且确定**（身份→租户边界→参数→权限→幂等→后端→审计），但后端是 fixture，未联系任何线上外部系统。与 #88/#91 已接受的 fixture 边界模式一致。
- **MCP 工具审批 ≠ 业务授权**：引擎沙箱的 `tool_approved` 仅记录于审计，**绝不绕过** `iam.authorize`；两层互不替代（代码 + 测试 + 本文件三重背书）。
- 未做：真实外部业务系统凭据托管/网络调用、生产网关池执行（归 #90）。本 crate 不执行进程/模型/网络。