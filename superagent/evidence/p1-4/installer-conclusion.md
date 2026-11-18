# P1-4 (#76) 安装器 + 启动器 + doctor — 结论与验收映射

日期：2026-11-17 · 分支 feat/p1-4-installer（堆叠于 feat/p1-1-pack-manifest / PR #113）
状态：**实现完成；18/18 单元测试 PASS；运行时全链路 PASS；干净 macOS 冷启动 PROVEN（确定性 pin）**（install → restart → skills 仍生效 → doctor 全绿 → TUI 渲染 → 用户配置零改动；冷启动 install+doctor ALL GREEN，pin_consistent=true）。
合并顺序：#72 → #73（PR #113）→ #76。并行开发符合 goal 规则。

## 1. 交付物

| 交付 | 路径 |
|---|---|
| 安装器（原子解包/路径穿越拒绝/~/.codex 守卫） | superagent/tools/install_pack.py |
| 启动器（专属 CODEX_HOME + sidecar，默认 --no-daemon） | superagent/bin/superagent |
| doctor（版本/摘要/认证/pack/平台，凭据零读取） | superagent/tools/doctor.py |
| 一条命令安装 | superagent/install.sh |
| 单元/故障注入测试（17） | superagent/tests/test_installer.py |
| 运行时验收脚本（可移植，ROOT 自 BASH_SOURCE，proxy opt-in） | superagent/tests/p1-4-run.sh |
| 冷启动复现脚本（pinned commit 构建引擎 + fresh HOME + doctor 全绿） | superagent/tests/p1-4-coldstart.sh |
| TUI pty 探针（终端能力应答） | superagent/tests/p1_4_tui.py |
| 用户配置快照工具 | superagent/tests/p1_4_snapshot.py |
| fixture pack（真实 SKILL.md，digest 2d35a5a1…） | superagent/tests/p1-4/pack/ |

## 2. 验收映射

| 验收项 | 证据 |
|---|---|
| 干净 macOS 一条命令安装 | **冷启动已证明（确定性 pin）**（coldstart-conclusion.json `install_plus_doctor_green=true, pin_consistent=true`）：脚本内置 pinned `SOURCE_COMMIT=92240f13e`（COLDSTART_SOURCE_COMMIT 可覆盖），隔离 worktree checkout 到该 commit 并 `cargo build` 引擎（sha256 a33d8f19…）→ fresh HOME（无任何既有产品状态）→ `install.sh`（install_pack+doctor）exit=0 → doctor 8 项 ALL GREEN exit=0。**BUILD_START header == worktree_commit == pinned_source_commit == 92240f13e**（同一 commit 对应同一 engine_sha256），build log 由提交版脚本自身产出。隔离性另以 fake user home 测试等价覆盖。引擎二进制按既有决策不入库，缺失时 doctor FAIL 并给出构建指引；auth 为文档化用户登录前置（`superagent login`），冷启动以 provision auth.json 模拟登录后状态（doctor 只 stat 不 read）。 |
| 安装器原子解包、拒绝路径穿越 | staging 全量校验后**整包 all-or-nothing** 交换：保留所有 backup 直到每项成功，任一项失败则回滚所有已交换项。**全新安装（无既有目标，backup=None）中途失败亦整包回滚**：移除已交换的新目标并清理其空父目录，恢复到安装前的空状态（test_mid_swap_failure_rolls_back_fresh_install 断言 tree_state=={} 且 os.listdir(home)==[]；test_mid_swap_failure_rolls_back_whole_pack 断言已有安装整包恢复原状）。tar 绝对路径/`..`/symlink/hardlink/非常规成员 fail-closed；故障注入测试断言目标零写入。tar 解包在 Python≥3.12 用 `filter="data"`，旧版本回退到已预校验的 plain extractall（test_tar_extract_fallback_pre_3_12 / test_tar_traversal_rejected_pre_3_12）。 |
| superagent 进入元智能体 TUI | bin/superagent 以专属 CODEX_HOME 拉起 superagent-engine；tui-probe.log：`tui_rendered=True error_screen=False bytes=10956`；tui-screen.log 含完整渲染帧（OpenAI Codex / Ask Codex to do anything / GPT-6-Astra default / /init /status /permissions）。sidecar 无 packaged daemon → 启动器默认 --no-daemon（SUPERAGENT_DAEMON=1 覆盖）。 |
| doctor 全绿 | doctor.log：8 项全 OK → ALL GREEN, exit 0（platform/engine-version/engine-digest/auth/pack/installed-pack/codex-home-writable/user-config-isolation）。 |
| doctor 输出排除凭据 | auth.json 只 stat 不 read；test_doctor_all_green_and_credential_free 注入 SECRET_TOKEN 断言文本+JSON 输出零泄漏。 |
| 用户现有配置零改动 | ~/.codex 配置面（顶层文件+skills/agents/prompts，89 文件）sha256 前后一致：user-codex-before/after.config.json，user-codex-config.diff 为空，user-config-unchanged=true。bulk 元数据变动（tmp/arg0-macos 锁文件）归因于用户并行运行的 ChatGPT.app（Codex desktop）；本产品引擎 arg0 目录在产品 home（tmp/arg0-macos 独立存在）。 |
| 不修改 ~/.codex/config.toml/AGENTS.md | is_safe_install_target 拒绝 ~/.codex 及子目录为目标；test_user_config_untouched 哈希断言。 |

## 3. #102 closure gate（f2-decision.md §6）

restart-persistence-conclusion.json：
```
skill_visible_after_install_session1 = true
skill_visible_after_restart_session2 = true
restart_persistence_via_install = PERSISTENT
user_config_unchanged = true
issue_102_closure_gate = SATISFIED
```
机制：pack skills 安装至产品 CODEX_HOME/skills/（引擎默认发现根：codex-rs/exec-server/src/discoverV2/capability_locations.rs:73 `("codex-home-skills", codex_home.join("skills"))`；引擎测试 codex-rs/core/src/session/tests.rs:5759 佐证）。每会话从磁盘加载，**无需 extraRoots 重注入**。session1/session2 为两个全新引擎进程的真实 skills/list RPC transcript（未设置任何 extraRoots）。
→ #102 的"能力包技能重启后仍生效"验收由本 PR 端到端证明；#102 可在本 PR 合并后按 gate 关闭（需 grok 对本证据 APPROVE）。

## 3.5 干净 macOS 冷启动证明（r1 blocker + r2 major 整改）

`bash superagent/tests/p1-4-coldstart.sh`（可复现；`COLDSTART_SOURCE_COMMIT` 覆盖 pin，`COLDSTART_WT` 复用 worktree，`COLDSTART_FORCE_BUILD=1` 强制重建）：

1. **引擎从确定性 pinned SOURCE_COMMIT 构建**：脚本内置常量 `SOURCE_COMMIT=92240f13ea22f283037baffb9baa5f6bdf8023d7`（非运行时 `git rev-parse HEAD`）。隔离 worktree checkout 到该 commit，`cargo build --release -p codex-cli --bin codex`（BUILD_RC=0），copy→packaging/superagent-engine。engine_sha256=`a33d8f19ae91a83790fa7c36b503d0438da841584855c2ce7af66caaf1e89f3a`，`codex-cli 0.0.0`。**一致性守卫**断言 `BUILD_START source_commit == worktree_commit == SOURCE_COMMIT`（coldstart-build.log 由提交版脚本自身写出 BUILD_START/BUILD_RC/ENGINE_SHA256/COLDSTART_BUILD_DONE）。首次 r1 为全新 worktree 从头构建（26m49s，无预置二进制）；r2 在 codex-rs 逐字节相同的 pin 上以 cargo 指纹校验增量重建（Finished 2.97s），engine_sha256 不变——同一源码同一二进制。
2. **fresh HOME**：mktemp 全新 home，无任何既有产品状态；产品 CODEX_HOME=`$FRESH/Library/Application Support/SuperAgent/codex-home`。auth.json 从真实产品 home provision（模拟用户登录后；doctor 只 stat 不 read）。
3. **一条命令安装**：`HOME=$FRESH bash install.sh --pack tests/p1-4/pack --codex-home $PRODUCT_HOME` → exit=0，`OK: installed p1-4-fixture v0.1.0 digest=2d35a5a1…`，内嵌 doctor 8 项 ALL GREEN（coldstart-install.log）。install.sh 取自 pin commit（含整包回滚修复）。
4. **doctor 显式 JSON**：exit=0，`all_ok=true`（coldstart-doctor.json）。
5. **结论**（coldstart-conclusion.json）：`pinned_source_commit=worktree_commit=build_start_source_commit=92240f13e, pin_consistent=true, engine_built_from_source=true, fresh_home_no_preexisting_product_state=true, one_command_install_exit=0, doctor_exit=0, doctor_all_green=true, install_plus_doctor_green=true`。

诚实边界：本机隔离 HOME 模拟（非物理全新机器/VM）；r2 构建为 cargo 指纹校验增量重建（target 缓存自 r1 从头构建），非每次从头——源码相同故二进制相同，从头构建能力已由 r1 的 26m49s 全量日志证明；auth 为文档化用户登录前置（`superagent login`），以 provision 模拟；arg0 helper 目录在首次真实会话才创建，冷启动仅 install+doctor（`--version`）故 coldstart-arg0-isolation.json `arg0_dir_under_product_home=false` 为预期，隔离由整段 HOME 重定向结构性保证。

## 4. 诚实性声明

1. TUI 品牌为引擎原生 "OpenAI Codex"（重命名仅进程名）；元智能体人格指令与 UI 品牌验收归 **#74（PR #116）**，本 PR 仅证明启动器以专属 CODEX_HOME 拉起可渲染 TUI。
2. fixture 技能为演示技能；真实 team-delivery 技能属 #75。机制证明不依赖技能内容。
3. doctor 分别报告源 pack 与 installed-pack；run-record 版本绑定的执行接入属 #77/#81。
4. **全新机器冷启动已演示**（p1-4-coldstart.sh，可复现）：pinned commit 构建引擎 + fresh HOME + install.sh + doctor ALL GREEN（coldstart-conclusion.json）。仍属本机隔离 HOME 模拟而非物理全新机器/VM；auth 以 provision 模拟登录后状态（真实首跑需用户 `superagent login`）。coldstart-arg0-isolation.json 的 `arg0_dir_under_product_home=false` 是预期：冷启动仅跑 install+doctor（引擎 `--version`），不启动会话，故 arg0 helper 目录尚未创建；隔离由整段运行重定向 HOME 结构性保证（产品 CODEX_HOME 在 fresh HOME 下，真实 ~/.codex 不被触及）。arg0 目录在首次真实会话/TUI 运行时创建（见 §B.5 产品 home 独立 arg0 取证）。
5. bulk 元数据变动非本产品写入（归因 ChatGPT.app，产品 home 独立 arg0 目录佐证）；验收判据为配置面哈希零变化。
6. TUI 探针需应答终端能力查询（kitty/DECRQM/OSC 10/11）才能渲染——已内置应答器；纯 dumb pty 会停在查询阶段（首次探针误报已修复：现断言 MARKERS 且拒绝 daemon 错误屏）。

## 5. 复现

```bash
python3 superagent/tests/test_installer.py   # 18 tests OK
bash superagent/tests/p1-4-run.sh            # 运行时全链路（model-free，可移植）
bash superagent/tests/p1-4-coldstart.sh      # 冷启动：pinned commit 构建引擎 + fresh HOME + doctor 全绿
```