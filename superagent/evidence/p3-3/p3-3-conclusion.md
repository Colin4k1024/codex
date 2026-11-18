# P3-3 RSI 闭环 v1（#89）结论

## 交付
`superagent/crates/rsi`（superagent-rsi）：受控自进化闭环
候选→固定 fixture 评估→人工审批→digest 固定晋升→回退。**model-free /
process-free / network-free**：依赖仅 serde、serde_json 与
`superagent-marketplace`（path ../marketplace，复用 #98 sha256 digest + HMAC 签名 +
安全解包 + run 绑定 + 回退）；fixture 评估为确定性 lookup，无 Command/网络/模型。

## 验收对照
- **候选经验采集（来源可追溯）**：`collect(RefinedMemory{source_session,source_id},
  effects)` 将 memories 精炼产出存入候选库，`Candidate` 携带 source_session/source_id；
  审计 `collect` 事件链接 candidateId→source。测试 `full_loop_...` 断言来源字段。
- **固定 fixture 评估（同模型同 fixture，完成率/通过率影响）**：`evaluate` 对
  `Fixture{cases:[{key,prior,expected}]}` 确定性计分：baseline=prior==expected，
  candidate=AnswerRule 覆盖后==expected；`EvalReport{fixture_id,model,total,
  baseline_pass,candidate_pass,delta,per_case}`。测试 `fixed_fixture_eval_reproducible_traceable`
  断言同模型同 fixture 同规则两次评估报告**逐字节相同**、per_case 逐例可追溯、
  baseline_pass=1（含已通过的 q3，非空泛 baseline）、无关候选 delta=0。
- **人工审批晋升（digest 固定，run 绑定旧版本）**：`approve(approver,Decision)` 仅在
  Evaluated 后生效；`promote` 要求 Approved（否则 NotApproved），构造 PackManifest
  （experience.json= benign AnswerRule）经 marketplace sign→install/upgrade，digest
  固定；`bind_run` 钉住当前 digest，升级后 `resolve_run_binding` 仍为旧 digest。
  测试 `full_loop_...` + `run_binding_pinned_across_promote` 断言 v1→v2 升级后 run
  绑定保持 d1、current=d2。
- **版本回退演练**：`rollback` 经 marketplace.history 恢复上一版本 digest。测试
  `full_loop_...` 断言回退后 version=1.0.0、digest==p1.digest、current==p1.digest。
- **恶意 artifact 提权测试**：`Effect::Elevate{scope}` / `Effect::DeveloperInstructions`
  为特权效果，`evaluate` 与 `promote` 均扫描拒绝（PrivilegeEscalation），候选置
  Rejected，不能审批/晋升，不进入提示词。测试 `malicious_artifact_privilege_escalation_rejected`
  断言两类提权均被拒、evil-pack 未安装、prompt 无 danger-full-access/注入文本。
- **未经审批的经验不进入提示词**：`prompt_context()` 仅纳入 **Promoted** 候选的 benign
  AnswerRule；Pending/Evaluated/Approved-未晋升/Rejected 全部排除，特权效果永不出现。
  测试 `unapproved_experience_never_in_prompt` 逐状态断言 prompt 为空直至 promote。
- **不自动晋升**：测试 `no_auto_promote_and_approval_gate` 断言 Pending promote→
  NotApproved、未评估 approve→NotEvaluated、重复 promote→AlreadyPromoted、未知候选/
  空 effects 守卫。
- **一次完整演练 + 评估报告归档**：`full_loop_candidate_eval_promote_rollback` 走通
  collect→evaluate(delta=2)→approve→promote(v1)→bind_run→promote(v2)→rollback(v1)，
  审计链含 collect/evaluate/approve/promote/rollback；EvalReport 存于 Candidate.eval
  且 evaluate 审计事件记录 baseline/candidate/delta（报告归档）。

## 边界（诚实声明）
- fixture 评估是**确定性 lookup**（prior/expected 录制 + AnswerRule 覆盖），代表
  "同模型同 fixture" 的可复现完成率/通过率度量，不调用真实模型；model 字段记录固定
  模型标识以保证同模型口径。这与 #85/#77 已验收的 baseline FAIL→PASS 确定性夹具一致。
- 晋升/回退/run 绑定**真实复用** #98 marketplace（非重写）：digest=sha256(manifest)、
  HMAC 签名校验、安全解包、history 回退、bind_run 钉 digest 均由 marketplace 提供。
- 候选来源 RefinedMemory 复用 #87/#88 memories 精炼产出的**可追溯来源契约**
  （source_session/source_id），本 crate 不重复实现 SQLite 精炼，仅消费其产出结构。

## 门禁（raw 转录，toolchain 1.99.1）
- `cargo +1.99.1 fmt --check` exit 0；`clippy --all-targets -- -D warnings` exit 0，0 error。
- `cargo +1.99.1 test`：0 unit + 6 integration passed，test-exit=0；连续 5 次稳定运行均 6 passed。
- 证据：`fmt-clippy.log`、`test-output.log`（原始 2>&1 透传 + echo 退出码）。

## 测试清单（tests/rsi.rs，6）
1. full_loop_candidate_eval_promote_rollback
2. unapproved_experience_never_in_prompt
3. no_auto_promote_and_approval_gate
4. malicious_artifact_privilege_escalation_rejected
5. fixed_fixture_eval_reproducible_traceable
6. run_binding_pinned_across_promote