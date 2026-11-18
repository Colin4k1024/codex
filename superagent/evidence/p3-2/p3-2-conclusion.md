# P3-2 (#88) 混合检索 MCP server — 结论

Refs #88

## 交付
`superagent/crates/retrieval-mcp`（standalone crate，path 依赖 `superagent-memory-service` #87 的 FTS5 混合索引）。`RetrievalServer` 把索引包装为 MCP `hybrid_search` 工具：
- **caller-bound 身份**：`CallerContext{caller_id,role,session_id}` 由 host 在 `new()` 固定；`check_identity` 拒绝任何参数声明的 `callerId/role/sessionId` 与绑定值不符（`IdentitySpoof`），身份不由参数声明。
- **有界结果 + 引用来源**：`limit` 受 `InjectionBudget.max_results`(默认 8) 钳制；每条 hit 引用 `kind/sourceId/snippet/score`。
- **低信任注入**：结果 `trust:"low"`、`injectedAs:"data"`、`developerInstructions:false`；参数请求 developer 提权被忽略；token 预算 `max_tokens`(默认 900) 逐条累加、超限截断。
- **跨会话检索 trace**：`source_id` 以 `session:` 前缀编码会话；检索命中其他会话来源时 `crossSession:true`，并向 `trace()` 写入 `{event:"retrieval",caller,role,sessionId,query,hitCount,sources,crossSession,injectedTokens}`。
- 模型自由/进程自由：仅调用 memory-service 的 SQLite FTS5（trigram CJK），无模型/网络/子进程；embeddings 向量分支保留为 `ReservedVectorIndex`（EmbeddingsUnsupported），不夸大。

## 验收映射（issue #88）
| 验收 | 测试（tests/retrieval.rs，6 integration） | 证据 |
|---|---|---|
| 跨会话检索 trace | `cross_session_retrieval_trace`：caller B 检索命中 A:rollout-0，crossSession=true，trace[0] sessionId=B、sources 含 A: | test-output.log |
| 越权身份被拒 | `caller_bound_identity_rejects_spoof`：spoof callerId/role/sessionId → IdentitySpoof；匹配身份放行；developerInstructions:true 参数被忽略 | test-output.log |
| 注入预算合规 | `injection_budget_compliance`（max_results=2 → len≤2；max_tokens=0 → len 0/injectedTokens 0）+ `bounded_results_cite_source`（limit=1000 → len≤8，每条引用 kind/sourceId/snippet）+ `low_trust_no_developer_elevation`（trust low / injectedAs data / developerInstructions false） | test-output.log |
| 工具/查询边界 | `unknown_tool_and_empty_query_rejected`：evil/tool → UnknownTool；空/空白 query → EmptyQuery | test-output.log |

## 门禁（toolchain 1.99.1，与 CI 一致）
- `cargo +1.99.1 fmt --check`：exit 0（fmt-clippy.log）
- `cargo +1.99.1 clippy --all-targets -- -D warnings`：exit 0，error-lines=0（fmt-clippy.log）
- `cargo +1.99.1 test`：0 unit + 6 integration（retrieval）passed / 0 failed；5 次连续全套运行均 6 passed（test-output.log）

## 边界
- 本 crate 为模型自由验收：身份绑定/越权拒绝/有界引用/低信任注入/跨会话 trace 均由真实 SQLite 索引 + 确定性断言证明，不依赖模型 turn。
- 向量检索为可选后验（#87 已保留接口），本 issue 不执行模型/向量引擎，无 embeddings 夸大。
- 关闭门禁：grok 对最新 SHA 独立审查 APPROVE + 上述验收全部 PASS。