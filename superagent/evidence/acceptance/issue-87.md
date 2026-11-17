## 背景
现有 memories 检索仅文件子串匹配（ext/memories local/search.rs），无 FTS/向量。补 Hybrid Retrieval 缺口。

## 任务
- [ ] memory-service（独立 crate）：SQLite FTS5 索引 rollout/memories/board posts
- [ ] 中文分词方案选型（FTS5 trigram 或外部分词）并实测
- [ ] 增量同步（新会话/新帖自动入索引）
- [ ] embeddings 向量检索作为可选后置，接口预留

## 验收
检索命中率/延迟基线报告；中文查询可用；索引不阻塞引擎会话。

依赖 #77。参考设计 §4 P3。

