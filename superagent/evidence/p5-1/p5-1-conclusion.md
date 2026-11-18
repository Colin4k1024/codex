# P5-1 / #95 远程工具执行：exec-server / environment 集成 — 验收结论

## 交付
- `superagent/crates/remote-exec/`：远程工具执行集成（model-free，纯 hmac/sha2 加密 + 策略计算 + 内存 fixture exec-server 后端）。
  - 依赖 `superagent-orchestrator`（path `../orchestrator`，#81）+ `superagent-contracts`（#78）；dev-dep `superagent-storage`（#80）。
  - `RemoteExecServer`（fixture exec-server 后端）：
    - `register_environment(env_id, capabilities, sandbox)` → 环境注册 + bootstrap（`bootstrapped=true`，审计 `environment_bootstrap`）。
    - `issue_token` / `connect(token, now)` → 认证连接（hmac-sha256 签名 + 过期 + bootstrap 校验；坏签名→Auth，过期→Expired，未知环境→UnknownEnvironment）。
    - `capability_discovery(conn)` → 远程环境 FS/Process/Sandbox 能力清单（接入编排器 dispatch 决策）。
    - `exec_tool(conn, req)` → 远程派发，**权限边界**：所需 capability 必须在环境清单内（否则 CapabilityDenied），环境 sandbox 必须 ≥ 工具 min_sandbox（否则 SandboxDenied）；拒绝显式，无 host-shell 回退。
  - `RemoteExecExecutor` 实现 orchestrator `NodeExecutor`：标准 `Orchestrator::run` 驱动节点经 capability_discovery → 权限边界 → 远程派发 → 带 evidence_ref 的 gate outcome。capability/sandbox 拒绝 → `ExecKind::Blocked`（编排器置 Blocked，**不重试、不回退 host-shell**，#82 语义）。Capability 节点产出 artifact evidence（无 NL 成功）。
  - agent 本体本地运行（跨机放置归 #96）。

## 验收对照（tests/remote_exec.rs，0 unit + 6 integration）
| 验收要求 | 测试 | 断言 |
|---|---|---|
| 一个任务在远程环境完成工具执行 | `task_completes_tool_execution_in_remote_environment` | Orchestrator::run → Succeeded，节点 proven_gates=["g"]；审计 environment_bootstrap + remote_exec(env-1,build) |
| 能力发现接入编排器 | `capability_discovery_feeds_dispatch_and_denies_missing` | discovery 返回环境清单；缺 Process 能力 → 节点 Blocked，attempts=1（不重试） |
| 权限边界测试 | `permission_boundary_sandbox_denied_no_fallback` | ReadOnly 环境 vs WorkspaceWrite 工具 → Blocked，attempts=1，reason 含 sandbox；exec_tool 直接返回 SandboxDenied |
| 认证连接 | `authenticated_connection_rejects_bad_and_expired_tokens` | 坏/篡改 token→Auth；过期→Expired；未知环境 issue_token→UnknownEnvironment |
| 环境注册/bootstrap | `environment_bootstrap_and_unknown_environment` | bootstrapped=true + sandbox + 审计；未知环境 connection→capability_discovery/exec_tool UnknownEnvironment |
| 能力节点证据 | `capability_node_remote_artifact_evidence` | Capability 节点经远程 artifact evidence→Succeeded；未绑定工具节点→Failed（无静默成功） |

## 门禁（toolchain 1.99.1，raw 2>&1）
- `cargo +1.99.1 fmt --check` exit 0；`clippy --all-targets -- -D warnings` exit 0（无 error）。
- `cargo +1.99.1 test`：0 unit + 6 integration passed，test-exit=0；5 次连续稳定性运行全 6-passed。
- 见 `fmt-clippy.log` / `test-output.log`。

## 诚实边界（NOT PROVEN / 范围声明）
- **fixture exec-server 后端，非真实跨机 exec-server 进程**：`RemoteExecServer` 为内存环境注册表 + 确定性派发，替代真实 `codex-rs/exec-server` 进程。集成路径（认证→bootstrap→能力发现→权限边界→远程派发→evidence→编排器状态）**真实且确定**，经真实 #81 orchestrator 驱动；但未联系活的跨机 exec-server。与 #94 fixture 后端已接受模式一致。任务说明「做集成而非重建」：本 crate 交付编排器侧集成契约（NodeExecutor + capability_discovery + 权限边界），exec-server 进程侧实体（codex-rs/exec-server，workspace 绑定）的真实跨机起进程实测属后续运行时工作。
- **thread/start.environments（experimental）未实测**：按任务说明「不可用则裁剪范围」，本首版裁剪 experimental environments 实测，聚焦远程工具执行集成契约 + 能力发现 + 权限边界（可由 model-free 代码证据裁决）。
- **agent 本体本地运行**：跨机放置归 #96（需第二台主机，HUMAN_BLOCKED）。本 crate 不执行真实进程/模型/网络。
- 未做：真实 exec-server 二进制起进程 + noise/websocket 传输 + 跨机环境 bootstrap 实测。这些需 codex-rs workspace 重型构建 + 引擎运行时，非本 model-free 集成契约范围。