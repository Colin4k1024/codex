# SuperAgent 产品工作区

混合路线落地：本目录是独立产品工作区（独立 cargo workspace，**不属于 codex-rs workspace**），
承载 SuperAgent 元智能体的能力包、编排器、网关与证据。引擎 = fork 构建的重命名二进制
`superagent-engine`（本机端点安全按进程名 SIGKILL codex，改名已验证可运行）。

设计：仓库根 `SUPERAGENT_LANDING_DESIGN.md`；GitHub 跟踪：Epic #63，issues #64–#102。

## 布局
- `pack/` 能力包（roles/skills/workflows/templates + manifest）— P1
- `crates/` 产品 Rust crates（adapter/contracts/orchestrator/storage/cli/gateway/memory-service）— P2+
- `packaging/` 引擎 sidecar 与安装清单
- `evidence/` 每 issue 验收证据（`p0-1/`…）与 acceptance  criteria 快照
- `tests/` spike 客户端、验证脚本、fixtures

## 关键约定
- 产品专属 CODEX_HOME：`~/Library/Application Support/SuperAgent/codex-home`（不触碰 ~/.codex）
- 认证：显式复用模式（复制 auth.json，goal 决策记录在案）
- 每 issue 闭环：分支 → 证据/代码 → PR → grok 独立审查（tests/grok-review.sh）APPROVE → 合并 → 关闭
- goal state：`~/.qoder-cn/projects/-Users-fanjia-Desktop-code-codex/superagent-goal-state.json`