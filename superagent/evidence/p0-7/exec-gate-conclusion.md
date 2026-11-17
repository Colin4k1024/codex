# P0-7 / #70 结论：command/exec 门禁执行面

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
复现：`python3 superagent/tests/p0_7.py`（证据：cases.json / transcript.json / summary.json，allPass=true）

## 七态实测（summary.json）

| case | 验证点 | 结果 |
|---|---|---|
| success | 真实退出码 0 + stdout 标记 | PASS |
| failure | 非零退出码原样返回（门禁可判失败） | PASS |
| timeout | 超时 → exitCode 124 | PASS |
| cancel | command/exec/terminate{processId} → 子进程树清理（pgrep 前后对比） | PASS |
| escapeWrite | 越权写 writableRoots 之外 → 沙箱阻止 | PASS |
| networkBlocked | networkAccess=false → 网络请求失败（仅以 stdout 判定，避免 traceback 源码回显假匹配） | PASS |
| invalidPolicyBlocked | 相对路径 writableRoot → invalid_request 失败关闭，**不退回宿主 shell**，标记文件未落地 | PASS |

## 契约要点

- command/exec 不绑定 thread，权限不自动继承会话审批；沙箱策略必须显式传入
  （SandboxPolicy workspaceWrite{writableRoots:[绝对路径],networkAccess}）。
- 策略转换 to_core：app-server-protocol/src/protocol/v2/permissions.rs:652；
  旧版沙箱映射 core/src/config/mod.rs:526（set_legacy_sandbox_policy）。
- 无法表达的策略 → 结构化错误（fail-closed），满足"不退回宿主 shell"。
- 取消清理路径：core/src/exec.rs:1043-1099（terminate_process_group + 宽限 + kill_process_group）。

## 裁决

VerificationExecutor 门禁可直接基于 command/exec 退出码构建，无需 fork。