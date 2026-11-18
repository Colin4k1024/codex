# P4-2 (#91) IAM/RBAC：用户/角色/策略与资源级授权 — 结论

Refs #91

## 交付
`superagent/crates/iam`（standalone crate，serde/serde_json/hmac/sha2/base64）。网关自有 IAM 表，与引擎执行层 sandbox/permissions 两层不互相替代：
- **用户/角色/策略表 + JWT 认证**：`IamServer{users,bindings,policies}`；HS256 JWT（hmac-sha256 + base64url）`issue_token`/`authenticate`（验签 + exp 过期检查；篡改/错密钥/过期/ malformed 分别拒绝）。默认策略表 admin（全 kind×action）/member（Thread/Run/Artifact Read+Write、Tenant Read）/viewer（全 kind Read）。
- **资源级授权**：`authorize(claims, ResourceRef{kind:Tenant|Thread|Run|Artifact, tenant, id}, action)`；**硬租户边界**——claims.tenant ≠ resource.tenant 一律 CrossTenant 拒绝（即使 admin）；租户内按角色策略 (kind,action)  grant，否则 Forbidden；未知角色拒绝。每次授权写审计 `{event:"authz",sub,role,tenant,resourceKind,resourceTenant,resourceId,action,decision,reason}`。
- **渠道身份映射**：`bind_channel(channel,external_id,user_id)`（用户须存在）+ `map_channel_identity` → IAM User；未映射 → UnknownIdentity；映射身份可签发 token 回认。
- **凭据卫生**：审计记录不携带 token/secret；`redact(text,secrets)` 掩码 + `scan_for_secrets(text,secrets)` 泄漏扫描；`redacted_audit` 输出脱敏诊断。

## 验收映射（issue #91）
| 验收 | 测试（tests/iam.rs，6 integration） | 证据 |
|---|---|---|
| 越权访问矩阵测试全过 | `authz_matrix_full_pass`：3 角色 × 4 资源 kind × 4 action × {同租户,跨租户} = **96 组合**逐一断言 authorize 结果与期望矩阵一致（checked=96，audit=96）；`resource_level_tenant_isolation`：admin 跨租户 4 kind 全拒（硬边界），同租户 admin 放行；`role_policy_and_unknown_role`：未知角色/viewer Write/member Delete 拒绝、admin Delete 放行 | test-output.log |
| 凭据泄漏扫描通过 | `credential_redaction_scan`：raw audit 不含 secret/token（scan 空）；redacted_audit scan 空；redact 掩码生效；**阳性对照**——真实泄漏字符串被扫描器检出（len=1），证明扫描器非恒空 | test-output.log |
| JWT 认证（任务项） | `jwt_auth_roundtrip_and_reject`：签发→验签 claims 正确；过期 Expired、错密钥 InvalidSignature、篡改 payload InvalidSignature、malformed MalformedToken、未知用户 UnknownUser | test-output.log |
| 渠道身份映射（任务项） | `channel_identity_mapping`：slack U123→u1(member,t1)；未映射 UnknownIdentity；绑定未知用户 UnknownUser；映射身份签发 token 回认 sub=u1 | test-output.log |

## 门禁（toolchain 1.99.1，与 CI 一致；原始 transcript）
- `cargo +1.99.1 fmt --check`：exit 0（fmt-clippy.log，raw 2>&1）
- `cargo +1.99.1 clippy --all-targets -- -D warnings`：exit 0，error-lines=0（fmt-clippy.log，raw 2>&1）
- `cargo +1.99.1 test`：0 unit + 6 integration（iam）passed / 0 failed；5 次连续全套运行均 6 passed（test-output.log，raw 2>&1 + test-exit）

## 边界
- 模型自由/进程自由/网络自由：纯 hmac/sha2 密码学 + 策略计算；JWT 为 HS256 对称签名（网关自有密钥），不依赖外部 IdP。
- 引擎侧 sandbox/permissions 仍是执行层约束（P0-7 #70），本 IAM 层不替代之；两层叠加。
- 测试用密钥为显式 fixture 常量（非真实凭据），审计/证据不含真实 secret。
- 关闭门禁：grok 对最新 SHA 独立审查 APPROVE + 上述验收全部 PASS。