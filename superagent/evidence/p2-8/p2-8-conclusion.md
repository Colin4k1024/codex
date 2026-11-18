# P2-8 / #85 — MVP 退出验收结论（team-delivery fixture 端到端）

## 判定：PROVEN（model-free，真实 transport + 真实退出码）

产品编排器（#81）+ 真实执行器（#82 AgentExecutor/VerificationExecutor）+ CLI
run/status/report（#83）+ 恢复核心（#84）在真实 stdio JSON-RPC（fake_engine
子进程）与真实 `command/exec` 子进程退出码上，跑通完整 team-delivery 工作流。
无模型、无网络。模型驱动的真模型自主组队由 #77（真实 TUI 团队交付）独立证明；
本套件证明产品编排器的团队交付 DAG 机制 + 命名 artifact 链 + 真实门禁证据。

## 验收逐条 → 证据（crates/cli/tests/mvp_exit.rs）

| 验收要求 | 测试 / 证据 | 结果 |
|---|---|---|
| fixture 链 intake(scope.json)→plan(plan.json schema 验证)→implement(change-summary.json)→verify(真实退出码)→review(review.json 无未解决阻塞项)→report(report.md+json) | `mvp_team_delivery_real_defect_fix_succeeds`：6 节点 DAG，每节点真实命令产出命名 artifact；门禁 `scope-valid`/`plan-schema`/`change-summary-valid`/`tests-pass`/`review-clear`/`report-archive` 全部以真实退出码证明 | PASS |
| plan.json schema 验证 | `plan-schema` 门禁：steps 为非空 list 且每项含 id+acceptanceCommand（真实 python 校验 exit 0） | PASS |
| review.json 无未解决阻塞项 | `review-clear` 门禁：blocking==0 且所有 severity==blocking 的 finding 均 resolved | PASS |
| report 明确支持平台与限制 | `report-archive` 门禁 + 断言 report.json 含非空 platforms/limits/verified；report.md 落盘 | PASS |
| 真实缺陷修复且测试通过 | seed_fixture calc.py `return a - b`（baseline `run_unittest` 必须 FAIL）→ implement 改 `a + b` → verify 真实 unittest exit 0；独立 `run_unittest` 复核 PASS；断言 calc 含 a+b 不含 a-b | PASS |
| 失败测试触发最多两次修复 | `mvp_failing_test_bounded_retry_then_failed`：buggy fixture，verify 门禁 unittest 真失败，Policy.max_attempts=3 → attempts∈[2,3] 后 Failed（非静默成功），fixture 仍 buggy | PASS |
| 不可修复 fixture 停在 failed/blocked | 同上：终态 Failed，status_json state=="failed" | PASS |
| 缺验收命令进入 waiting_input | `mvp_missing_acceptance_command_waiting_input`：Approval-only 门禁 → 节点 WaitingInput，非交互 run 返回 CliError::Blocked，exit==BLOCKED | PASS |
| 首版串行单工作区 | execute_run 串行驱动 Orchestrator（单 workspace=tempdir）；6 节点按 DAG 顺序串行执行 | PASS |
| 记录初始 HEAD 与工作区状态；不自动 git add/commit/reset | `mvp_records_initial_head_and_no_auto_git_ops`：git init/commit fixture，run 后 HEAD 不变、无新 commit、calc.py 改动留工作区未暂存（无 "M  calc.py"）、identity.startHead==head_before | PASS |
| 端到端 trace + 报告归档 | 每测试断言 durable `run_report` 事件 + evidence 目录非空（真实门禁证据落盘）；status_json 读 durable state | PASS |
| 不以"完成了"判成功；全部节点终态有证据 | 成功仅由 proven_gates 决定（orchestrator lib.rs:403-453）；每节点断言 Succeeded + 对应 proven gate；失败/阻塞/等待均有真实退出码或状态证据 | PASS |

## 门禁（toolchain 1.99.1，见 test-output.log）
- `cargo fmt --check`：exit 0
- `cargo clippy --all-targets -- -D warnings`：exit 0，0 error
- `cargo test`（全 cli crate）：11 cli + 4 mvp_exit 全 PASS
- 并行稳定性：mvp_exit 全套件连续 6 次（默认并行测试线程）全 4 PASS

## 诚实性 / 修复记录
- **flake 根因已定位并修复**：成功测试曾在并行下偶发 FAIL。诊断为 Python 字节码
  缓存竞态——baseline `run_unittest` 写入 `__pycache__/calc.pyc`（a-b），patch 以
  **相同字节大小**重写 calc.py（a+b），并行时序下 mtime 落在同一秒，importlib 复用
  陈旧 pyc → verify 真实 exit 1。修复：所有 fixture python 调用加 `-B` +
  `sys.dont_write_bytecode=True`，patch 同时 `shutil.rmtree(__pycache__)`。修复后
  6× 并行全稳定。此修复不削弱任何验收（baseline 仍真 FAIL、verify 仍真退出码）。
- 无模型/网络断言：fake_engine `echo` scenario，`command/exec` 跑真实子进程。
- 无凭据：测试仅用 tempdir + 内存 store，不触及 ~/.codex / auth.json。
- 平台：macOS arm64（darwin 25.6.0），python3 3.14。限制：model-free fixture，
  串行单工作区；真模型自主角色委派见 #77。

## 依赖
#83（CLI）✓ closed、#84（recovery）✓ closed、#77（TUI 团队）✓ closed。
关闭以 grok 对最新 SHA APPROVE 为准。