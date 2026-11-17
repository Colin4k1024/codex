## 背景
混合路线 MVP 的前置任务。本机端点安全会 SIGKILL 名为 codex 的进程（exit 137），已验证改名后可正常运行；产品引擎统一命名 superagent-engine。

## 任务
- [ ] fork 构建 release 二进制，重命名为 superagent-engine，放入产品目录（记录版本+摘要+平台）
- [ ] 创建产品专属 CODEX_HOME（如 ~/Library/Application Support/SuperAgent/codex-home），不触碰 ~/.codex
- [ ] 专属 HOME 下完成登录，验证账号状态读取
- [ ] 验证管理员策略（requirements/cloud config）在专属 HOME 下的优先级与生效路径
- [ ] 运行引擎 generate-json-schema，归档为 wire 契约基线

## 验收
改名引擎稳定运行（无 137）；专属 HOME 登录与正常会话可用；策略优先级有书面结论；schema 文件归档。

## 参考
设计 §3/§7；HARNESS_STANDALONE_TASKS T02/T06/T07。

