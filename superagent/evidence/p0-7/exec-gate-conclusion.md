# P0-7 / #70 结论：command/exec 门禁执行面

日期：2026-11-17 · 引擎：superagent-engine @ 425a848a9 · 平台：Darwin arm64
复现：`python3 superagent/tests/p0_7.py`（证据：cases.json / transcript.json / summary.json，allPass=true，11 例）
审查历史：首版 grok APPROVE 已合并；一个更早启动、延迟完成的独立审查（grok-review-legacy.json，
CHANGES_REQUESTED）在合并后写入，提出进程树清理/语义拒绝/默认继承/PII 四项 findings。
本 PR 为逐条整改后的重跑证据，PII 已在落盘前 scrub（installationId/serverName/codexHome/authMode/planType → <scrubbed>，
用户名/主机名 → 占位符）。

## 十一态实测（summary.json）

| case | 验证点 | 结果 |
|---|---|---|
| success | 真实退出码 0 + stdout 标记 | PASS |
| failure | 非零退出码原样返回（门禁可判失败） | PASS |
| timeout | 超时 → exitCode 124 | PASS |
| cancel | 叶子进程 sleep 30：terminate{processId} → pgrep 前后对比消失 | PASS |
| cancelTree | **进程树**：父 python 派生 sleep 47 后代；terminate 前 pgrep 父子均在， terminate 后父子均消失（进程组清理实证） | PASS |
| escapeWrite | 越权写 writableRoots 之外 → 沙箱阻止 | PASS |
| networkBlocked | networkAccess=false → 网络请求失败（仅以 stdout 判定，避免 traceback 源码回显假匹配） | PASS |
| invalidPolicyBlocked | 畸形输入（相对路径 writableRoot）→ 反序列化拒绝 -32600，失败关闭，标记文件未落地 | PASS |
| semanticPolicyBlocked | **良构但语义不可表达**：readOnly.access={"type":"restricted"} 是被识别的旧版字段， 由 SandboxPolicy Deserialize 语义拒绝（"readOnly.access is no longer supported"）， 非路径畸形；命令未执行、标记未落地 | PASS |
| defaultPolicyNoHostWrite | 不传 sandboxPolicy/permissionProfile → 回退会话 config.permissions； 工作区外写仍被阻止（默认继承不授予宿主级写） | PASS |
| defaultPolicyInWorkspace | 同默认回退下工作区内命令正常执行（exitCode 0） | PASS |

## 契约要点（含代码引用）

- command/exec 不绑定 thread、**无审批交互**：不产生 requestApproval，权限不继承会话审批策略；
  显式 sandboxPolicy 经 to_core（app-server-protocol/src/protocol/v2/permissions.rs:652）→
  set_legacy_sandbox_policy（core/src/config/mod.rs:526）→ can_set_permission_profile 约束层
  （core/src/config/mod.rs:514-522）。
- 两类拒绝路径区分：① 畸形输入在反序列化层拒绝（AbsolutePathBuf）；
  ② 良构旧版字段（readOnly.access / workspaceWrite.readOnlyAccess = restricted）在
  SandboxPolicy Deserialize 中语义拒绝（app-server-protocol/src/protocol/v2/permissions.rs:618-620、635-637）。
  两者均 fail-closed、不退回宿主 shell。约束层（can_set_permission_profile）的 managed 拒绝
  需要受管策略环境，本 spike 未构造，属部署期约束而非 P0 缺口。
- 缺省回退：既不传 sandboxPolicy 也不传 permissionProfile 时，处理器 else 分支直接采用
  self.config.permissions（app-server/src/request_processors/command_exec_processor.rs:242-262）；
  实测默认 profile 下越权写被阻（defaultPolicyNoHostWrite），即默认继承不放大权限。
- 取消清理路径：core/src/exec.rs:1043-1099（terminate_process_group + 宽限 + kill_process_group）；
  cancelTree 以父子双 pgrep 实证整组清理。

## 裁决

VerificationExecutor 门禁可直接基于 command/exec 退出码构建，无需 fork。
编排器应始终显式传入 SandboxPolicy（不依赖默认回退），并以进程组清理为取消语义。