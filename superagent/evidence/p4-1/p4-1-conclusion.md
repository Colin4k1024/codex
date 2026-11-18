# P4-1 / #90 — 网关统一入口 + 租户引擎进程池 · 结论

判定：**PROVEN**（真实引擎子进程 + 真实 TCP socket；模型无关，无网络外呼）。

## 交付物
- 新 crate `superagent/crates/gateway`（`superagent-gateway`），CI 经
  `find superagent/crates -name Cargo.toml` 自动发现并纳入 fmt/clippy/test 门禁。
- `src/lib.rs`：
  - `GatewayPool`：每租户独立 `CODEX_HOME`（`base/<tenant>`）+ 引擎子进程；
    受控并发（`ConcurrencyLimit`）；`acquire`/`is_alive`/`restart`/`release`/`release_all`。
  - `restart`：kill+wait 回收崩溃引擎 → 重新 spawn → 审计 crash+restart（新 pid）。
  - `release`：kill+wait 回收（无孤儿/泄漏）。
  - `forward`：把 app-server JSON-RPC 经 stdio 转发到指定租户引擎，跳过流式
    notification 直到匹配 id 的响应（会话/turn/审批/事件转发）。
  - `AuditLog`：append-only JSONL 生命周期事件（ts/tenant/event/pid/codexHome）。
  - `RunManifest`：productVersion/engineDigest/platform/protocolSummary。
  - `serve_connection`/`handle_connection`：**真实 TCP 统一入口**，按 `tenantId`
    路由 app-server JSON-RPC 到对应租户引擎并回写响应。
- `tests/gateway.rs`：6 个真实子进程/TCP 集成测试。
- `test-output.log`：门禁与并行稳定性原始输出。

## 验收 → 测试映射

| 验收（#90） | 证据 |
|---|---|
| 双租户隔离测试 | `dual_tenant_isolation`：两租户引擎 initialize/test-env 各报自身 `CODEX_HOME`（互异）；A home 写入 auth.json 标记，B home 不可见；pid 各异 |
| 进程池崩溃恢复测试 | `pool_crash_recovery`：kill -9 杀死活引擎 → `is_alive` 转 false → `restart` 新 pid（≠旧）存活且 `test/ping` pong=true；审计含 crash+restart |
| 进程池泄漏测试 | `pool_release_no_leak`：acquire 3 租户 → `release_all` → active_count=0；对每个 pid `kill -0` 失败（ESRCH，已回收无孤儿/僵尸）；`is_alive` 返回 UnknownTenant |
| 审计可追溯 | `audit_and_manifest_traceable`：audit.jsonl 含 spawn/crash/restart/release 事件 + tenant + codexHome；manifest.json 四字段非空 |
| 并发上限（任务：进程池化） | `concurrency_cap_enforced`：max=2，第三个 acquire→ConcurrencyLimit(2)；release 一个后可再 acquire |
| 统一入口包装 app-server 协议（任务） | `unified_entry_tcp_routes_tenants`：真实 TCP socket，客户端经统一入口发 tenant-a/tenant-b 的 initialize，响应各带对应隔离 `CODEX_HOME`（证明协议转发 + 租户路由 + 隔离） |

## 门禁（toolchain 1.99.1，见 test-output.log）
- `cargo fmt --check`：exit 0
- `cargo clippy --all-targets -- -D warnings`：exit 0，error-lines=0
- `cargo test`：6 passed; 0 failed
- 并行稳定性：3 次连续全套件均 `6 passed; 0 failed`

## 诚实边界
- 模型无关：唯一子进程是调用方提供的引擎 argv（测试用 fake_engine.py，真实 stdio
  JSON-RPC）与 `kill`；无模型、无网络外呼。TCP 入口仅绑定 127.0.0.1 本地回环。
- 统一入口已证明「真实 socket 上的 app-server 协议转发 + 按租户路由 + 隔离」。
  HTTP/WS 是该路由核心之上的传输绑定（frame codec）；本模型无关套件证明协议转发
  与租户隔离/恢复/泄漏/审计，未单独演练 HTTP/WS 帧编解码（传输适配层）。
- 租户隔离证明覆盖配置/认证（CODEX_HOME 目录隔离 + auth 标记互不可见）与会话
  （每租户独立引擎进程/独立 thread 状态）；未声称跨租户网络 ACL（连接级路由）。
- 依赖 #85 已 CLOSED。未改动 orchestrator/executor/core。