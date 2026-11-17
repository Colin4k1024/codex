## 背景
智能市场中心：agent 能力包成为 marketplace 一等公民。复用 core-plugins marketplace（add/remove/upgrade、curated、share）。

## 任务
- [ ] agent pack 市场条目类型（manifest+digest+签名校验）
- [ ] 一键安装/升级/回退；run 绑定旧 digest 不受升级影响
- [ ] curated 源与第三方源区分；许可清单随包分发

## 验收
第三方包安装/升级/回退演练；恶意包（路径穿越/越权 symlink）被拒。

依赖 #73、#90。代码：core-plugins/src/marketplace/；plugin/src/manifest.rs。

