# P5-4 (#98) agent 能力包市场（marketplace 集成）— 结论

Refs #98

## 交付
`superagent/crates/marketplace`（standalone crate，serde/serde_json/hmac/sha2）。agent 能力包成为市场一等公民：
- **市场条目类型（manifest+digest+签名校验）**：`PackManifest{name,version,source,publisher_key_id,licenses,files}`；`digest`=sha256(canonical manifest JSON) hex；`signature`=HMAC-SHA256(publisher_secret, digest)。`sign_pack`/`verify_pack`：重算 digest 比对（DigestMismatch）+ hmac verify_slice 验签（BadSignature）+ 未知 publisher key（UnknownKey）。
- **一键安装/升级/回退**：`install`/`upgrade`/`rollback`；`installed` 当前版本 + `history` 旧版本栈；upgrade 需已安装（NotInstalled），rollback 弹出历史恢复（NothingToRollback）。
- **run 绑定旧 digest 不受升级影响**：`bind_run(run_id,name)` 钉住当前 installed digest；`resolve_run_binding` 升级后仍返回旧 digest。
- **curated 与第三方源区分 + 许可清单随包分发**：`Source{Curated,ThirdParty}` 记录于 installed + 审计；`licenses` 随包安装/审计分发。
- **安全解压（恶意包拒绝）**：`extract` 全有或全无——先校验每个条目（Symlink kind → SymlinkRejected；绝对路径/`..` 分量/空路径 → PathTraversal），再仅写 regular 文件。路径穿越/越权 symlink 在任何写入前被拒。

## 验收映射（issue #98）
| 验收 | 测试（tests/marketplace.rs，6 integration，真实 FS） | 证据 |
|---|---|---|
| 第三方包安装/升级/回退演练 | `third_party_install_upgrade_rollback`：install v1（skill.md 落盘）→upgrade v2（digest 变化、current=v2）→rollback v1（current=v1 digest 还原）；审计含 install/upgrade/rollback | test-output.log |
| run 绑定旧 digest 不受升级影响 | `run_binding_pinned_across_upgrade`：bind_run→d1；upgrade v2 后 resolve_run_binding 仍=d1（≠d2），current=d2 | test-output.log |
| 恶意包（路径穿越）被拒 | `malicious_path_traversal_rejected`：`../evil.txt` 条目 → PathTraversal；root 及 root.parent 均无 evil.txt（无逃逸写入）；pack 未安装 | test-output.log |
| 恶意包（越权 symlink）被拒 | `malicious_symlink_rejected`：Symlink→`../../etc/passwd` 条目 → SymlinkRejected；link 未落盘；pack 未安装 | test-output.log |
| manifest+digest+签名校验（任务项） | `signature_and_digest_verification`：合法验签+安装；篡改 manifest→DigestMismatch（install 亦拒）；翻转签名字节→BadSignature；未知 key→UnknownKey | test-output.log |
| curated/第三方区分 + 许可随包（任务项） | `curated_vs_thirdparty_and_licenses`：Curated/ThirdParty source 分别记录；licenses=[Apache-2.0] 随包；2 条 install 审计均含 licenses | test-output.log |

## 门禁（toolchain 1.99.1，与 CI 一致；原始 transcript）
- `cargo +1.99.1 fmt --check`：exit 0（fmt-clippy.log，raw 2>&1）
- `cargo +1.99.1 clippy --all-targets -- -D warnings`：exit 0，error-lines=0（fmt-clippy.log，raw 2>&1）
- `cargo +1.99.1 test`：0 unit + 6 integration（marketplace）passed / 0 failed；5 次连续全套运行均 6 passed（test-output.log，raw 2>&1 + test-exit）

## 边界
- 模型自由/进程自由/网络自由：纯 hmac/sha2 密码学 + 文件系统策略；签名为 HMAC-SHA256 对称（publisher key 由市场注册），不依赖外部 PKI。
- 复用 #73 pack manifest+digest 概念（sha256 canonical digest）；与 core-plugins marketplace 的 add/remove/upgrade/curated/share 语义对齐，本 crate 为产品侧模型自由实现。
- 测试 publisher secret 为显式 fixture 常量（非真实凭据）；临时目录测试后清理。
- 关闭门禁：grok 对最新 SHA 独立审查 APPROVE + 上述验收全部 PASS。