## 背景
桌面端复活：按 HARNESS_TAURI_DESKTOP_DESIGN 与 HARNESS_CLAUDECODEUI_MIGRATION，复用网关 API（不再直连引擎）。

## 任务
- [ ] Tauri 2 壳 + ClaudeCodeUI 派生前端（388134c7 基线，AGPL LICENSE/NOTICE/修改标识合规）
- [ ] 品牌配置：displayName/appIcon/wordmark/accentColor；深浅主题
- [ ] 聊天/任务/审批/报告面板经网关 API；连接断开显示"执行状态待确认"
- [ ] macOS 签名/公证；Linux/Windows 各自验收后声明支持

## 验收
桌面完成一次团队任务与审批；许可合规清单完整；干净机器可安装。

依赖 #90。参考 HARNESS_TAURI_DESKTOP_DESIGN.md。

