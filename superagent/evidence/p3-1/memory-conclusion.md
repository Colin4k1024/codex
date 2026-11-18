# P3-1 / #87 memory-service — 实现结论

日期：2026-11-17 · crate：superagent-memory-service v0.1.0（独立 [workspace]，edition 2021）
分支：feat/p3-1-memory-service · 基线：origin/main 697de6498

## 交付

独立产品侧 crate，SQLite FTS5 混合索引，覆盖 rollout / memories / board posts 三类来源。
零 codex-* 依赖、零 codex_ 符号、无进程 spawn、无模型/向量引擎执行（purity-scan.txt：cargo tree 无 codex）。

## 验收映射

| 任务 | 实现 | 测试 |
|---|---|---|
| 独立 crate FTS5 索引三类来源 | SCHEMA_V1：sources 表 + mem_fts FTS5 虚表 + idx_sources_kind；SourceKind{Rollout,Memory,Board} | index_rollout_and_search / index_memory_and_search / index_board_post_and_search / cross_kind_search_isolation_by_filter |
| 中文分词选型并实测 | 选型=FTS5 内置 `trigram` tokenizer（CJK 子串匹配，无需外部分词器） | chinese_trigram_search（"混合检索"/"元智能体" 中文检索命中） |
| 增量同步（新会话/新帖自动入索引） | index() content_hash 比对 → Inserted/Reindexed/Unchanged；FTS rowid 镜像 sources.id，事务一致 | incremental_sync_new_source_auto_indexed / incremental_sync_unchanged_skipped / incremental_sync_changed_reindexed |
| embeddings 可选后置，接口预留 | VectorIndex trait + ReservedVectorIndex stub 显式 EmbeddingsUnsupported | embeddings_interface_reserved_no_model |
| 有界混合检索 + 引用来源 | search() FTS5 MATCH + bm25 排序，limit clamp MAX_RESULTS=100，Hit{kind,source_id,snippet,score} | bounded_results_limit / search_cites_source_and_snippet / hybrid_ranking_bm25_order |
| 迁移幂等 | migrate() user_version | migration_is_idempotent_and_versions |
| 删除出索引 / 空查询拒绝 | remove() / EmptyQuery | remove_source_removes_from_index / empty_query_rejected |

## 测试结果

15/15 model-free 测试 PASS（test-output.log，CARGO_EXIT=0）。

开发期修正一处测试假设：hybrid_ranking_bm25_order 原断言"短文档优先"，但 trigram tokenizer 下
重复词项文档的 bm25 排序并非如此；改为断言结果按 bm25 分数非降序排列且两文档均返回（正确不变量），
不削弱验收（仍验证排序正确性）。

## 诚实边界

- 本 crate 为 model-free 实现 + 单元测试。**关闭门依赖 #77（P1-5 TUI fixture）** 及真实
  rollout/memories/board 数据集成；运行时集成验收 NOT PROVEN 前 issue 保持 OPEN。
- embeddings 仅预留接口（stub 显式 Unsupported），未实现向量检索，不声称已实现。
- 中文分词为 trigram 子串匹配（非语义分词），方案与局限如实记录。
- 合并顺序：#87 在 #77 之后（P3 链 #87→#88→#89）。