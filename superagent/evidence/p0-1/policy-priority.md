# 管理员策略（requirements）优先级与专属 CODEX_HOME 结论

日期：2026-11-17。Issue #64（P0-1）验收项「策略优先级有书面结论」。
基线：Colin4k1024/codex @ 425a848a9（fork of openai/codex）。以下行号均相对 `codex-rs/`。

## 结论

1. **产品专属 CODEX_HOME 不能绕开管理员策略。** 四类策略来源全部取自操作系统/远端，
   均不从 CODEX_HOME 读取；CODEX_HOME 只用于解析层内相对路径的 base_dir。
2. **优先级（低 → 高）：system → cloud bundle → legacy managed_config → macOS MDM。**
   高层覆盖低层的标量/列表值；exec rules 取最严格结果。
3. 本机实测 `configRequirements/read` 返回 `requirements: null`（无管理员策略部署），
   与 doctor 输出一致；API 与配置加载走同一 composed 状态，专属 HOME 下同样生效。

## 证据（代码）

| 来源 | 位置 | 说明 |
|---|---|---|
| system（Unix） | `config/src/loader/mod.rs:774-775` | 固定 `/etc/codex/requirements.toml` |
| system（Windows） | `config/src/loader/mod.rs:848-901` | HKLM `SOFTWARE\Policies\OpenAI\Codex` RequirementsPath，否则 `%ProgramData%\OpenAI\Codex\requirements.toml` |
| cloud bundle | `cloud-config/src/bundle_loader.rs:52-77`；`config/src/loader/managed_requirements.rs:40-48` | 远端 config-server / ChatGPT backend 拉取，注入为 bundle_requirements |
| legacy managed | `config/src/loader/layer_io.rs:22,222-227` | 固定 `/etc/codex/managed_config.toml`；Unix 路径显式忽略 codex_home（`let _ = codex_home;`） |
| macOS MDM | `config/src/loader/macos.rs:27-34,150-185` | domain `com.openai.codex`，key `requirements_toml_base64`；仅 forced 值生效 |
| 优先级顺序 | `config/src/loader/mod.rs:110-120`；`config/src/loader/managed_requirements.rs:110-117` | 注释「ascending precedence order」；按 system→cloud→legacy→MDM 依次 extend |
| 合并语义 | `config/src/requirements_layers/stack.rs:1-17` | 低层提供默认，高层覆盖；`auto_review.required_on_models` 高优先并集 |
| MDM > legacy | `config/src/loader/mod.rs:939-941` | 「MDM has higher precedence than the legacy managed_config.toml file」 |
| CODEX_HOME 无关 | `config/src/loader/managed_requirements.rs:42,86`；`layer_io.rs:66,86` | codex_home 仅作 base_dir；Windows 下 CODEX_HOME/managed_config.toml 已废弃并告警 |
| 测试覆盖开关 | `config/src/state.rs:42-53,71,97`；`config/src/loader/mod.rs:783-790` | LoaderOverrides.system_requirements_path 仅测试代码使用，无生产 CLI 入口 |
| API 同源 | `app-server/src/request_processors/config_processor.rs:152-168`；`app-server/src/config_manager.rs:364-373` | configRequirements/read 读取同一 load_managed_requirements_state |

## 证据（本机实测）

- `evidence/p0-1/config-requirements-transcript.json`：专属 HOME 下
  `configRequirements/read → {"supportsIndependentSpeedModes": true, "requirements": null}`。
- `evidence/p0-1/doctor.txt`：auth/paths/reachability 检查均指向专属 HOME；0 fail。
- 未修改 `/etc/codex`、MDM 或任何系统策略文件（goal 约束：不扩大权限、不改范围外配置）。
  优先级结论由代码路径 + 层序注释 + 合并语义测试（`requirements_layers/stack_tests.rs`）支撑；
  注入真实策略文件的端到端验证属 P1 配置集成范围，不在本 spike 修改系统目录。

## 对产品的影响

- 专属 CODEX_HOME 是隔离用户配置/会话的手段，不是权限提升手段；设计 §6 假设成立。
- 若客户机器部署了 MDM/系统策略，产品引擎同样受约束；产品 UI 需展示
  configRequirements/read 结果（P1 设置诊断页）。