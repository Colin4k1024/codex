# workflows/ — 工作流定义结构规范（P1-1 定义，schema 由 P2-1 #78 交付）

确定性工作流定义（JSON），数据契约以 #78 contracts schema 为准：

- 节点类型区分 capability（团队命令引用）与 verification（可执行程序）。
- loop 节点必须带 max_attempts / wall_timeout / 显式 gate。
- 未知节点类型必须编译失败，不得转成成功 stub。
- dry-run 校验变量/DAG/工具与模型可用性/门禁定义，不启动模型或命令。

本目录在 #78 合并前只承载结构规范；首个 fixture 工作流随 #77/#85 交付。