# P5-2 / #96 结论 — agent 注册/发现服务与跨机通信

Refs #96

## 交付物
- `superagent/crates/agent-registry/`（standalone crate，独立 `[workspace]`）
  - `src/lib.rs`：注册/目录/心跳服务、跨主机 board 路由 + 交接、Ed25519/JWKS 身份验证、手动放置策略。
  - `tests/agent_registry.rs`：7 个集成测试。
- `superagent/evidence/p5-2/`：fmt-clippy.log、test-output.log（原始 2>&1 transcript）、本结论。

## 验收 → 测试映射（全部 PASS）
验收原文：**两台主机的 agent 互相发现并完成一次交接；身份伪造被拒。** 依赖 #95、#66（均已关闭）。

| 验收/任务 | 命名测试 | 关键断言 |
| --- | --- | --- |
| 两台主机互相发现 + 完成一次交接 | `two_hosts_mutual_discovery_and_handoff` | host-1 discover→agent-b@host-2，host-2 discover→agent-a@host-1（互相）；A→B Handoff（payload task-42，from_host host-1 / to_host host-2）→ B recv → B→A HandoffAck → A recv；audit agent_registered×2 + board_routed×2 + handoff_received×2 |
| 身份伪造被拒 | `identity_forgery_rejected` | 错误签名密钥→InvalidSignature；篡改签名段→InvalidSignature；过期→Expired；签名合法但 host claim 与放置不符→Forged；用他人 token  drain 收件箱→Forged；伪造尝试无任何投递 |
| 身份不可变（重注册替换拒绝） | `reregistration_identity_replacement_rejected` | 用不同密钥重注册存活 agent-a→Forged（任何变更前）；JWKS key 不变（真实 token 仍验证、攻击者密钥仍 InvalidSignature）；会话凭据不变（sess-a 路由、sess-evil→SessionDenied）；无伪造投递 |
| 能力注册/目录/心跳服务 | `heartbeat_liveness_and_stale_exclusion` | 超过心跳 TTL 无刷新→catalog/discover 为空；新心跳恢复 last_seen 与发现；subject 不符→拒绝 |
| 跨主机消息路由（board + 会话级凭据） | `cross_host_routing_session_credentials` | 错误会话凭据→SessionDenied（即便身份 token 合法）；正确凭据路由；未知 peer→UnknownAgent |
| agent 跨机放置策略（最小：手动指定环境） | `manual_placement_policy` | 注册时指定 host_id；placement(agent-a)==host-1 / placement(agent-b)==host-2；未知→UnknownAgent |
| 身份：JWT/JWKS 验证集成 | `jwks_verification_and_unknown_kid` | JWKS 每 agent 一把 OKP/Ed25519 公钥；未知 kid→UnknownKid； malformed→MalformedToken；合法 agent 仍验证通过 |

## 机制说明
- **真实 Ed25519/JWKS 密码学**：每个 agent 持有自己的 `SigningKey`（`AgentIdentity::from_seed` 确定性种子，fixture 友好，无 OsRng）；注册处仅见公钥并发布到 JWKS（OKP/Ed25519，x=base64url(pubkey)）。身份 token 为 EdDSA JWT（header.kid=agent_id，claims iss/aud/sub/host/iat/exp），`verify` 校验签名（ against JWKS 公钥）+ 过期 + iss/aud + subject/host 绑定。伪造（错误密钥/篡改/过期/host 不符/冒名 drain）全部被拒。
- **注册/目录/心跳**：`register` 验证身份后记录 capabilities/endpoint/host 放置 + 会话凭据；`heartbeat` 刷新 last_seen；`catalog`/`discover` 仅返回 TTL 内存活 agent，`discover(host)` 返回**其它主机**的存活 agent（跨主机互相发现）。
- **跨主机路由 + 交接**：fixture 内存 board 按收件人 inbox 路由 `Envelope`；`send_handoff`/`ack_handoff` 要求发送者合法身份 token + 匹配会话级凭据 + 双方存活；`recv` 要求收件人身份 token。一次 Handoff→Ack 闭环即"完成一次交接"。
- **已发布身份不可变**：`register` 对已存在 agent_id 要求呈现**相同公钥**并对**现有 JWKS key** 验证；不同密钥即身份替换伪造，在任何 key/session 变更前返回 Forged。仅全新 agent_id 插入 key（验证失败回滚）。失败重注册不吊销存活身份。

## 诚实边界（不夸大）
- **fixture board 传输 + 两个逻辑主机**：host-1/host-2 为同一进程内的逻辑主机标识，board 为内存传输；**非两台物理主机、非真实网络投递**。机制（互相发现 + 交接 + 伪造拒绝）以真实 Ed25519 密码学确定性证明，与 #95 fixture exec-server 后端同构。
- **Ed25519/JWKS 方案兼容 codex-agent-identity，但为 standalone 实现**：issue 提及的基座 `codex-agent-identity`/`agent-message-board-*` 均为 codex-rs **workspace-tied**（edition/lints/deps 继承 workspace，依赖 codex-http-client/codex-protocol/真实网络/OsRng），无法作为 standalone superagent crate 的 path dep。本 crate 以独立 ed25519-dalek + base64 实现兼容的 Ed25519/JWKS 验证（沿用 superagent-iam 手写 JWT 的既有模式），**非字面集成 workspace-tied 基座**。
- model-free / process-free / network-free：依赖仅 serde、serde_json、ed25519-dalek、base64；无 Command/网络/模型。
- 测试签名密钥为确定性 fixture 种子（[1;32]/[2;32] 等），非真实凭据；代码/证据无真实 secret。
- 范围限于 `superagent/crates/agent-registry` + `superagent/evidence/p5-2`，无 codex-rs core 改动。

## 门禁证据（原始 transcript 已提交）
- `fmt-clippy.log`：`cargo +1.99.1 fmt --check` exit 0；`clippy --all-targets -- -D warnings` exit 0（error 行 0）。
- `test-output.log`：0 unit + 7 integration passed，test-exit 0；5 次连续稳定性运行全部 7-passed（run1..5-exit=0）。

## grok96 整改（CHANGES_REQUESTED → 修复）
- **major（身份替换伪造路径）**：原 `register` 先插入新公钥再 verify，允许用不同密钥重注册存活 agent_id 静默替换身份/会话凭据。已修为身份不可变：重注册须相同公钥 + 对现有 JWKS key 验证，不同密钥在任何变更前 Forged；新增 `reregistration_identity_replacement_rejected` 断言 key/session 不变 + 无伪造投递。
- **minor（heartbeat sub-binding 断言）**：原"mismatched subject"实为未注册 kid→UnknownKid，注释误称 Forged。已更正注释，并新增真正 sub-binding 断言：用已注册 agent-a 的 token 对 agent-b 心跳→Forged。