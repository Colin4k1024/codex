# P2-9 / #86 — worktree 并行隔离与合并重验 · 结论

判定：**PROVEN**（真实 git worktree，模型无关；唯一子进程是 git + 调用方 gate）。

## 交付物
- 新 crate `superagent/crates/worktree`（`superagent-worktree`），CI 经
  `find superagent/crates -name Cargo.toml` 自动发现并纳入 fmt/clippy/test 门禁。
- `src/lib.rs`：`WorktreePool`（受控并发）、`commit_paths`（显式路径提交）、
  `merge`（显式合并 + 冲突阻塞）、`run_gate`（合并后真实退出码重验）。
- `tests/isolation.rs`：6 个真实 git 集成测试。
- `test-output.log`：门禁与并行稳定性原始输出。

## 验收 → 测试映射

| # | 验收（#86） | 证据 |
|---|---|---|
| A | 每个写任务独立 git worktree，限制并发数 | `isolation_separate_worktrees_primary_untouched`（task-a/task-b 各自只见自身文件，主树两者皆无、`status==""`）；`concurrency_cap_enforced`（max=2：a/b 成功，c→`ConcurrencyLimit(2)`；release a 后 c 成功） |
| B | 依赖产物经显式合并传递 | `dependency_artifact_propagates_via_explicit_merge`（task-a 产 artifact.txt，合并前 integration 无此文件，`merge`→`Merged`，`acquire_from` integration 分支的 task-b 见到 artifact） |
| B | 合并冲突阻塞下游 | `merge_conflict_blocks_downstream_target_unchanged`（shared.txt 双方异改：merge a→Merged，merge b→`Conflict{files 含 shared.txt}`，integration 保持 "A version" 且 `status==""`，下游只见 "A version"） |
| C | 合并后重新验证相关 gate（真实退出码） | `post_merge_gate_reverify_real_exit_code`（gate `python3 -B -c` 校验 calc.py=='ok'：好产物合并→`run_gate` exit0/passed；坏产物合并→exit1/!passed） |
| D | 不自动 git add -A/commit/reset | `commit_paths` 仅 `git add -- <显式 paths>`（空 paths 报错），从不 `-A`；pool 全程无自动 commit/reset；`no_auto_git_ops_protects_user_uncommitted_content` 断言主树 HEAD 不变、修改未暂存（`" M README.md"`，非 `"M  README.md"`） |
| E | 并行不吞用户未提交内容 | `no_auto_git_ops_protects_user_uncommitted_content`（主树 README 已改 + user-notes.txt 未跟踪；pool acquire/commit/merge/release 后 HEAD 不变、`" M README.md"` + `"?? user-notes.txt"` 俱在、notes 内容完好、task.txt 未泄漏进主树） |

## 门禁（toolchain 1.99.1，见 test-output.log）
- `cargo fmt --check`：exit 0
- `cargo clippy --all-targets -- -D warnings`：exit 0，error-lines=0
- `cargo test`：6 passed; 0 failed
- 并行稳定性：5 次连续全套件均 `6 passed; 0 failed`

## 诚实边界
- 模型无关：不触模型/网络；唯一子进程为 git 与调用方提供的 gate argv。
- 串行 MVP（#85）已关闭；本 crate 是其受控并行可选扩展，未改动 orchestrator/executor/core。
- 冲突处理为「阻塞下游 + 目标树不变 + `merge --abort` 复原」，不做自动冲突消解（符合验收：冲突阻塞下游）。
- 依赖 #85 已 CLOSED。