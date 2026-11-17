## 背景
Swarm Flow 的数据契约：确定性工作流定义，未知节点类型必须编译失败，不能转成成功 stub。

## 任务
- [ ] workflow/node/gate/artifact JSON schema（contracts crate）
- [ ] 节点类型区分 capability（团队命令引用）与 verification（可执行程序）；loop 必须带 max_attempts/wall_timeout/显式 gate
- [ ] 产品 Rust 类型单独导出 TS schema；引擎 DTO 隔离在 adapter，不贯穿业务模块
- [ ] dry-run 校验：变量、DAG、工具/模型可用性、门禁定义

## 验收
schema 校验测试（含未知类型失败、DAG 循环检测）；dry-run 不启动模型/命令。

依赖 #72。参考 HARNESS_STANDALONE T13；设计 §2/§3。

