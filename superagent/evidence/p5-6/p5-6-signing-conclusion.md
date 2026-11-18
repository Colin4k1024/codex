# P5-6 / #100 里程碑 — macOS Developer ID 签名/分发路径证明

Refs #100（**不声称关闭**：完整验收仍待 ClaudeCodeUI 迁移 + 公证 + 干净机器安装）

## 本轮交付（用户授权：采用本地 Developer ID 构建测试）
- `desktop/`：Tauri 2 原生桌面宿主工程（standalone，不引用 codex-core）。
  - `src-tauri/`：Rust 宿主（`youragent-desktop`），`bootstrap` 产品命令（设计 §5 产品状态入口），`core:default` capabilities（仅主窗口核心 IPC，无任意 shell/远程页面权限）。
  - `dist/index.html`：最小静态前端，经 `window.__TAURI__.core.invoke("bootstrap")` 调用产品命令（withGlobalTauri，避开 npm 前端构建）。
  - `src-tauri/tauri.conf.json`：bundle.macOS.signingIdentity = 本地 Developer ID，hardenedRuntime=true。
  - 图标：占位品牌资产（ImageGen 生成 1024×1024 源 + `tauri icon` 生成 icns/ico/png）。
- `superagent/evidence/p5-6/`：build.log、signing-verify.log、launch-test.log（原始 transcript）、本结论。

## 构建 + 签名 + 测试结果（全部实测）
- **构建**：`cargo-tauri build`（tauri-cli 2.12.5，release）`Finished release profile in 1m45s`；产出 `YourAgent.app`（5.01 MiB）+ `YourAgent_0.1.0_aarch64.dmg`（3.55 MiB）。
- **签名**：构建过程自动以 `Developer ID Application: Haier Electric Appliance International Co., Ltd. (KRV5T3384C)` 对 .app 内二进制、.app 包、.dmg 逐一签名（build.log "Signing with identity ..."）。
- **验签**（signing-verify.log）：
  - `codesign --verify --deep --strict --verbose=2 YourAgent.app` → `valid on disk` + `satisfies its Designated Requirement`，exit 0。
  - `codesign -dvvv` → Authority 链 `Developer ID Application (KRV5T3384C)` → `Developer ID Certification Authority` → `Apple Root CA`；`flags=0x10000(runtime)`（**hardened runtime 启用**）；`TeamIdentifier=KRV5T3384C`；安全时间戳 `Timestamp=Nov 18 2026`；Identifier=com.youragent.desktop。
  - `codesign --verify --strict DMG` → valid on disk + satisfies DR，exit 0。
- **Gatekeeper**：`spctl -a -vvv -t exec` → `accepted`，`source=Developer ID`，exit 0，**但 `override=security disabled`**——本机 Gatekeeper 评估被禁用所致，**非公证证明**。
- **启动测试**（launch-test.log）：直接运行签名后二进制，进程存活 5s（WebView 宿主启动，无崩溃，无 stderr），观察后终止。

## 诚实边界（不夸大，#100 仍未关闭）
- **签名 ≠ 公证**：本轮完成 Developer ID **代码签名**（含 hardened runtime + 安全时间戳）。**公证（notarization）未做**——build.log 明确 `Warn skipping app notarization, no APPLE_ID & APPLE_PASSWORD & APPLE_TEAM_ID or APPLE_API_KEY ... found`；本机缺 Apple ID app-specific-password / App Store Connect API key。`spctl accepted` 来自本机 `security disabled` 覆盖，**干净机器（Gatekeeper 启用）在未公证时会拒绝**。
- **干净机器可安装未验证**：本机 `hostname`=fanjiadeMacBook-Pro.local（单主机），#100 "干净机器可安装" 需第二台主机实测，未做。
- **ClaudeCodeUI 派生前端未迁移**：本轮为最小静态前端 + bootstrap 命令的**签名路径证明**；设计 U02–U06（派生 App/Sidebar/MainContent/Chat、真实 DesktopClient、文件/Git 适配、Harness Tasks tab、移除 Web JWT/Express/WS）尚未实现。ClaudeCodeUI 基线在本地 `/Users/fanjia/Desktop/code/claudecodeui`（HEAD 388134c7，AGPL-3.0-or-later + Section 7 附加条款），完整迁移与许可合规清单（LICENSE/NOTICE/修改标识/来源署名）属后续。
- **引擎 sidecar 未捆绑**：本轮未捆绑原版 Codex 引擎（设计 §6）；网关 API 真实接入与一轮真实团队任务+审批（#100 验收 "桌面完成一次团队任务与审批"）属后续。
- 占位品牌（YourAgent / com.youragent.desktop）非注册品牌（设计 §3 占位资产约定）。
- 范围限于 `desktop/` + `superagent/evidence/p5-6/`，无 codex-rs core 改动。

## #100 剩余门禁（需人工/生产授权或后续构建）
1. ClaudeCodeUI 派生前端迁移 U02–U06 + AGPL 许可合规清单。
2. 网关 API 真实接入 + 桌面完成一次团队任务与审批。
3. macOS 公证（需 Apple ID 凭据）+ 干净机器安装验收（需第二主机）。
4. Linux/Windows 各自验收后声明支持。