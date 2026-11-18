# P5-5 开发工作台（#99）结论

## 交付
`superagent/crates/workbench`（superagent-workbench）：以 P2 workflow schema 为
数据模型的低代码画布 + 最小租户 Application Space 资源视图。**model-free /
process-free**：lib 依赖仅 serde、serde_json、`superagent-contracts`（path ../contracts，
#78）；dev-deps `superagent-orchestrator`（#81）、`superagent-storage`（#80）用于
编排器执行证明（注入确定性 executor，无真实模型/进程）。

## 验收对照（画布生成的 workflow 通过 schema 校验并可被编排器执行）
- **画布编辑器（节点/依赖/gate/产物可视化）**：`Canvas{nodes:Vec<CanvasNode>,
  gates,artifacts,variables}`，`CanvasNode` 携带 `Position` 布局；`edges()` 输出
  依赖边 (from,to) 供图可视化；gates/artifacts 暴露。测试 `canvas_visualization_metadata`
  断言边、gate、artifact、布局保留。
- **画布 → workflow JSON 生成，经 #78 schema 校验**：`to_workflow()` 剥离布局生成
  `contracts::Workflow`（schema_version=SCHEMA_VERSION）；`to_workflow_json()` 导出；
  `validate(ctx)` = `contracts::dry_run`。测试 `canvas_generates_valid_workflow` 断言
  dry_run 空、导出不含 position、JSON 往返还原 Workflow；`canvas_json_schema_roundtrip`
  断言未知节点 type / 未知顶层字段经 deny_unknown_fields 拒绝（绝不成为成功 stub）；
  `invalid_canvas_rejected` 断言环/未知依赖/未知 gate 被 dry_run 捕获且 orchestrator.plan 失败。
- **可被编排器执行**：测试 `canvas_workflow_executes_via_orchestrator` 将画布生成的
  workflow 交 `Orchestrator::new(wf,ctx,Policy::default())`，`plan()` 拓扑序 a 先，
  `run(FnExec 完成+proven gate, Store::open_in_memory, run, spec, 1000)` →
  rep.state=Succeeded，a/b/c 全 Succeeded 且 proven_gates 含 g（证据支撑成功）。
- **Application Space / Resource Management 最小版（租户资源视图）**：`AppSpace`
  按租户登记 workflow/artifacts/runs，`view(tenant)` 隔离。测试 `app_space_tenant_resource_view`
  断言 t1/t2 资源互不可见、未知租户 None、tenants 列表、登记幂等。

## 边界（诚实声明）
- 本 crate 是画布**数据模型 + 生成 + 校验 + 资源视图**的产品侧实现；可视化渲染（前端
  画布 UI）不在本 crate 范围，布局元数据 (Position/edges) 为前端渲染提供数据契约。
- "可被编排器执行" 以 #81 orchestrator + 注入确定性 executor + #80 内存 Store 真实
  run 至 Succeeded 证明（与 #81 自身验收测试同一夹具口径），不声称已驱动真实引擎团队任务。
- schema 校验复用 #78 contracts::dry_run（语义）+ serde deny_unknown_fields（结构），
  非重写校验器。

## 门禁（raw 转录，toolchain 1.99.1）
- `cargo +1.99.1 fmt --check` exit 0；`clippy --all-targets -- -D warnings` exit 0，0 error。
- `cargo +1.99.1 test`：0 unit + 6 integration passed，test-exit=0；连续 5 次稳定运行均 6 passed。
- 证据：`fmt-clippy.log`、`test-output.log`（原始 2>&1 透传 + echo 退出码）。

## 测试清单（tests/workbench.rs，6）
1. canvas_generates_valid_workflow
2. canvas_workflow_executes_via_orchestrator
3. invalid_canvas_rejected
4. canvas_json_schema_roundtrip
5. app_space_tenant_resource_view
6. canvas_visualization_metadata