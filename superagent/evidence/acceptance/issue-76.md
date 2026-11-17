## 背景
产品入口：安装器 + superagent 启动器，引擎重命名 sidecar，专属 CODEX_HOME。

## 任务
- [ ] 安装器：pack 写入产品 CODEX_HOME 的 skills/agents 位置（原子解包、拒绝路径穿越）
- [ ] superagent 启动器：以专属 CODEX_HOME 拉起 superagent-engine TUI
- [ ] superagent doctor：引擎版本/摘要、认证、pack 校验、平台状态；输出排除凭据
- [ ] 不修改用户 ~/.codex、config.toml、AGENTS.md

## 验收
干净 macOS 一条命令安装；superagent 进入元智能体 TUI；doctor 全绿；用户现有配置零改动。

依赖 #64、#73。参考设计 §3。

