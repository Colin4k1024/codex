# P2-1 (#78) contracts — 结论与验收映射

日期：2026-11-17 · 分支 feat/p2-1-contracts（基于 origin/main 697de6498）
状态：**实现完成，21/21 测试 PASS（r1 APPROVE 后 minor 整改）**；合并顺序遵守依赖（#72 关闭后合并）。

## 1. 交付物

| 交付 | 路径 |
|---|---|
| contracts crate（独立 workspace，不进 codex-rs） | superagent/crates/contracts/ |
| Rust 契约类型 + dry-run 校验 | superagent/crates/contracts/src/lib.rs |
| JSON Schema 导出（schemars） | superagent/crates/contracts/schema/json/{Workflow,Node,Gate,ArtifactSpec}.json |
| TS 绑定导出（ts-rs，cargo test 时生成） | superagent/crates/contracts/bindings/ts/*.ts（8 个类型） |
| schema 导出二进制 | superagent/crates/contracts/src/bin/export_schemas.rs |
| 验收测试（21 个） | superagent/crates/contracts/tests/dry_run.rs |
| 合法 fixture | superagent/crates/contracts/tests/fixtures/valid-workflow.json |

## 2. 验收映射

| 验收项 | 证据 |
|---|---|
| schema 校验测试：未知类型失败 | tests/dry_run.rs `unknown_node_type_fails_deserialization`（type=magic_stub → "unknown variant" 反序列化错误）、`unknown_gate_kind_fails_deserialization`（kind=vibes 失败）、`unknown_field_rejected`（Workflow 顶层多余字段失败）。内部 tagged enum 保证未知节点类型**不可能**被反序列化成 stub。 |
| DAG 循环检测 | `dag_cycle_detected`（plan→…→fix-loop→plan 环 → "DAG cycle detected"）、`self_dependency_detected`。实现：Kahn 拓扑排序（src/lib.rs dry_run）。 |
| loop 必须带 max_attempts/wall_timeout/显式 gate | 缺字段 → 反序列化失败（`loop_missing_required_fields_fails_deserialization`）；零值 → dry-run 报错（`loop_zero_bounds_fail_dry_run`）；未知 gate → 报错（`loop_unknown_gate_fails`）。 |
| capability=团队命令引用 / verification=可执行程序 | NodeKind::Capability{command_ref∈command_catalog}、NodeKind::Verification{argv[0]∈programs, gates 非空}；`unavailable_command_ref_fails`、`unavailable_program_fails`、`verification_requires_gates`。 |
| dry-run 校验变量/DAG/工具/模型可用性/门禁定义 | `undeclared_variable_reference_fails`、`required_variable_without_value_fails`、`variable_type_mismatch_fails`、`unavailable_model_fails`、`unknown_artifact_and_schema_ref_fail`、`duplicate_node_ids_fail`、gate 三类（command/artifact_schema/approval）逐类校验。 |
| dry-run 不启动模型/命令 | dry_run 是纯函数：入参仅 &Workflow + &DryRunContext（目录由调用方注入），返回 Vec<ValidationError>。结构守卫测试 `dry_run_is_pure_no_execution_surface` 断言源码无 std::process / Command::new / codex_ 引用；独立扫描 evidence/p2-1/purity-scan.txt（0 匹配）。 |
| TS 单独导出、引擎 DTO 隔离 | ts-rs derive 导出 8 个产品类型为 bindings/ts/*.ts；crate 零 codex-* 依赖（Cargo.toml 仅 schemars/serde/serde_json/ts-rs），purity 扫描确认无 codex_ 符号。引擎 DTO 适配属后续 adapter 层（#77 编排器），不在本 crate。 |

## 3. 测试运行

`cargo test`（superagent/crates/contracts/）：**21 passed; 0 failed**（集成）+ 8 passed（ts-rs 生成的导出测试）。完整输出 evidence/p2-1/test-output.log。
`cargo run --bin export-schemas -- schema/json`：4 个 schema 写出，evidence/p2-1/schema-export.log。

## 4. 诚实性声明 / 已知限制

1. **serde 限制**：Node/Gate 含 `#[serde(flatten)]` 的内部 tagged enum，serde 不支持 flatten 与 deny_unknown_fields 组合（会误报 unknown field `type`）。因此 Node/Gate 对象**内部的未知字段被忽略**；未知**类型**（tag 值）仍显式失败，Workflow 顶层未知字段仍失败。已在 src/lib.rs 模块文档注明。
2. ts-rs 对 deny_unknown_fields 属性发出 parse warning（仅 TS 导出忽略该属性，不影响行为）。
3. TS 导出目录为 bindings/ts/（ts-rs 11 默认 bindings 前缀 + export_to="ts/"）。
4. 依赖 #72（P0-9 接口决策）：本 PR 并行开发（用户规则允许），**合并在 #72 关闭之后**。dry-run 的目录注入设计不依赖 #72 的具体接口结论；若 #72 改变 command catalog 形态，仅需调整 DryRunContext 字段。
5. 本 crate 无运行时执行面；workflow 的实际执行属 #77（P1-5 编排器）。

## 5. 复现命令

```bash
cd superagent/crates/contracts
cargo test                                   # 21+8 tests, TS bindings regenerated
cargo run --bin export-schemas -- schema/json  # JSON schemas
grep -rn "std::process\|Command::new\|codex_" src/  # expect no matches
```

## 6. 审查记录（grok 独立审查）

- r1（head 6e65cf90e）：**APPROVE**，无阻断项；4 个 minor（grok-review.json）。
- minor 整改（r2 提交）：
  1. **已修复**：loop_missing_required_fields_fails_deserialization 原用字符串删除，删 gate 字段会留下尾逗号导致 JSON 语法错误而非缺字段错误。改为 serde_json::Value 逐字段 remove + from_value，断言错误信息为 `missing field \`<name>\``。
  2. **已修复**：Node.json loop 分支 max_attempts/wall_timeout_secs 的 schema minimum 原为 0.0；加 `#[schemars(range(min = 1))]`，schema 层即拒绝 0 值；新增测试 node_schema_loop_bounds_at_schema_level 断言 minimum=1.0。
  3. **保留并披露**（§4.1）：Node/Gate 顶层 additionalProperties:false 受 serde flatten+tagged enum 限制无法生成；未知 type/kind 仍由 oneOf enum 在 schema 层失败。
  4. **保留并披露**：purity 守卫为静态源码断言 + 独立 grep；dry_run 纯函数签名（仅数据入参出参、无运行时句柄）是结构性保证，行为性沙箱证明属执行层（#77/#81）职责。
- 整改后测试：**21 passed; 0 failed**（集成）+ 8 passed（ts-rs 导出）。test-output.log 已更新。