# Frontend source attribution and modification record

The YourAgent desktop frontend is derived from the ClaudeCodeUI source tree, not
rewritten from screenshots. This file records the source baseline, what was
imported, and the modifications made, as required by the upstream license
(AGPL-3.0-or-later with Section 7 additional terms — see `LICENSE` and `NOTICE`).

## Source baseline

- Project: ClaudeCodeUI (`@cloudcli-ai/cloudcli` 1.28.0)
- Upstream: https://github.com/siteboon/claudecodeui
- Copyright: Siteboon AI B.V. and contributors
- Local reference checkout: `/Users/fanjia/Desktop/code/claudecodeui`
- Locked commit: `388134c7a5f1a2741931d47c4a1f89aae31d4292`
- License: AGPL-3.0-or-later, including Section 7 additional terms (attribution,
  modification marking, no endorsement, no trademark grant). `LICENSE` and `NOTICE`
  are reproduced verbatim in this directory.

## What was imported (source derivation)

- `src/`, `shared/`, `plugins/`, `public/`, `index.html` — upstream frontend source
  copied verbatim at the locked commit.
- `vite.config.js`, `tailwind.config.js`, `postcss.config.js`, `tsconfig.json` —
  upstream build configuration.
- `package.json` — REWRITTEN as a frontend-only closure (`youragent-desktop-frontend`).
  Server dependencies are excluded: express, ws, node-pty, better-sqlite3, sqlite3,
  sqlite, bcrypt, jsonwebtoken, multer, web-push, cors, chokidar, cross-spawn,
  node-fetch, @anthropic-ai/claude-agent-sdk, @openai/codex-sdk, and the
  postinstall/prepare scripts. Pure-JS libraries the frontend imports are retained.
- `dist/` — production bundle produced by `vite build` on the derived source.
- `LICENSE`, `NOTICE` — verbatim upstream license and attribution notices.

The upstream Express/JWT/WebSocket backend (`server/`) is NOT imported. The
self-contained desktop build talks to product Rust commands via the DesktopClient.

## Modifications made for the desktop product

New product files (not part of upstream):
- `src/services/desktop-client.ts` — typed DesktopClient over Tauri commands
  (`window.__TAURI__.core.invoke`) + event subscription; adapts command results to
  the fetch Response contract the legacy hooks expect. Includes a browser fixture
  fallback (same shapes as the Rust mock) for previewing the desktop build without
  the native shell. U03 adds engine lifecycle methods (engineConnect/engineStatus/
  engineDisconnect/cancelTurn/respondToRequest) and `mapEngineEvent`, which maps
  app-server notifications/server requests to the NormalizedMessage `kind` shapes
  the legacy chat handlers understand. Server approval requests are surfaced as
  typed `permission_request` events and are NEVER auto-answered. U04 adds typed
  file/Git methods (listFiles/readFile/saveFile/gitStatus/gitDiff/gitBranches/
  gitRemoteStatus/gitCommits/gitCommitDiff/gitFileWithDiff) with matching browser
  fixtures. U05 adds typed Harness methods (harnessListTasks/harnessCreateTask/
  harnessGetTask/harnessReadArtifact/harnessExportReport) + exported view interfaces
  (HarnessTaskSummary/HarnessTaskDetail/HarnessNodeView/HarnessGateView/
  HarnessArtifactView/HarnessEventView) mirroring the Rust serde shapes, with browser
  fixtures.
- `src-tauri/src/engine.rs` — U03 stdio JSON-RPC client for `codex app-server`.
  Spawns the engine with an ISOLATED `CODEX_HOME` (default `$HOME/.youragent/codex-home`,
  override `YOURAGENT_CODEX_HOME`; never the user's `~/.codex`). Performs
  initialize/initialized handshake, routes responses to per-request channels and
  forwards notifications/server requests to the WebView over `desktop-event`.
  Engine path comes from `YOURAGENT_ENGINE_PATH`; absent it, commands fall back to
  fixtures. Server requests are only resolved via an explicit user decision.
- `src-tauri/src/bridge.rs` — U03 pure mapping functions: protocol `Thread` →
  frontend ProjectSession; `ThreadItem` → NormalizedMessage history (user/agent
  text, reasoning, plan, commandExecution tool cards with exit code, fileChange/
  mcpToolCall). Unknown item types fall back to safe text rather than dropping data.
- `src-tauri/src/workspace.rs` — U04 file-tree / editor / read-only Git backend.
  Resolves a project name to a workspace root (`YOURAGENT_WORKSPACE_ROOT`, else the
  `superagent` fixture root) and confines every relative path: rejects absolute
  paths, `..` traversal and symlink escapes (canonicalize + prefix check), never
  follows symlinks when walking. Bounded tree (depth/entry caps, noise dirs skipped),
  2 MiB read/save size limit. Saves are atomic (temp + rename) and version-checked
  against the loaded `mtimeMs` — a stale save returns `{conflict:true}` instead of
  silently overwriting. Git is read-only: fixed `git` subcommands with structured
  args (no shell), `GIT_EXTERNAL_DIFF`/`diff.external` disabled so no external helper
  runs; status/diff/branches/remote-status/commits/commit-diff/file-with-diff.
- `src-tauri/src/harness.rs` — U05 Harness business-task backend. Reuses the
  SuperAgent `superagent-orchestrator` / `superagent-contracts` / `superagent-storage`
  crates (path deps) rather than reinventing a state machine: a deterministic DAG run
  whose node success is proven ONLY by gate evidence (real process exit codes for
  verification gates, artifact evidence for capability nodes). Persists run identity,
  events and artifacts to an isolated SQLite store (`YOURAGENT_HARNESS_DB`, else
  `$HOME/.youragent/harness.db`; never user data). A `FixtureExecutor` drives the
  `team-delivery` workflow in three modes (success / verify_fails / needs_input) and
  runs real `true`/`false` programs so gates carry genuine exit codes. Exposes typed
  views (task summary/detail, node, gate, artifact, event) + markdown report export.
  Detailed node/gate snapshots are kept in memory; restart-recovery of gate evidence
  from the store is a documented follow-up. Live engine-driven business execution
  (wiring the P2-5/P2-6 executor to the engine adapter) is the next step.
- `src/components/harness-tasks/**` (view/HarnessTasksPanel, hooks/useHarnessTasks,
  index) — U05 new product Tasks tab. Renders the task list, per-node state badges,
  gate evidence (proven / exit_code / evidence_ref), artifacts (read by backend id)
  and the markdown report. State comes verbatim from Rust typed commands; frontend
  TaskMaster identifiers never drive node success.
- `src-tauri/src/lib.rs` — product commands: bootstrap/auth_status/login/register/
  auth_user/logout/onboarding_status/complete_onboarding/git_config/update_git_config/
  list_projects/list_sessions/get_messages/send_input/desktop_request plus U03
  engine_connect/engine_status/engine_disconnect/respond_to_request/cancel_turn.
  list_sessions/get_messages/send_input are engine-first (thread/list,
  thread/items/list, thread/start + turn/start, turn/interrupt) with fixture
  fallback when no engine is configured. U04 adds list_files/read_file/save_file
  and git_status/git_diff/git_branches/git_remote_status/git_commits/
  git_commit_diff/git_file_with_diff, all delegating to workspace.rs. U05 adds
  harness_list_tasks/harness_create_task/harness_get_task/harness_read_artifact/
  harness_export_report, delegating to harness.rs (HarnessState managed at boot).
  `desktop_request` remains a graceful-
  degradation stub for unmigrated endpoints (NOT a fake Express API). Approval
  decisions default to `decline` (never auto-accept).
- `run-dev.sh`, `.env` (VITE_DESKTOP=true), this `SOURCE.md`.
- `src-tauri/Cargo.toml` — added `chrono` (timestamp conversion in bridge.rs) and
  U05 path deps `superagent-orchestrator` / `superagent-contracts` / `superagent-storage`.

Modified upstream files (marked here per Section 7):
- `src/constants/config.ts` — added `IS_DESKTOP` (Tauri runtime / VITE_DESKTOP).
- `src/utils/api.js` — in desktop mode the data layer routes through the
  DesktopClient (typed commands for the boot path; generic product request for the
  rest). The upstream OSS fetch path is retained for the browser build. U04:
  `getFiles`/`readFile`/`saveFile` route to typed file commands; `saveFile` carries
  an optional `expectedMtimeMs` for conflict detection.
- `src/components/auth/context/AuthContext.tsx` — added an `IS_DESKTOP` branch that
  provisions the session via `desktopClient.bootstrap()` (auto-authentication).
- `src/contexts/WebSocketContext.tsx` — in desktop mode, ordered product events over
  Tauri IPC replace the WebSocket; `sendMessage` goes through `desktopClient.sendInput`.
  U03: on boot it subscribes to `desktop-event` then calls `engineConnect()` so the
  real adapter is wired before any event arrives; falls back gracefully when no
  engine is configured.
- `src/components/code-editor/hooks/useCodeEditorDocument.ts` — U04: tracks the
  loaded `mtimeMs`, passes it on save, and surfaces a `{conflict:true}` response as
  a save-conflict state (no silent overwrite) with a Reload action; exposes `reload`.
- `src/components/code-editor/view/CodeEditor.tsx` — U04: save-error banner shows a
  Reload button when a save conflict is detected.
- `src/components/git-panel/hooks/useGitPanelController.ts` — U04: the seven Git read
  calls route to typed DesktopClient methods in desktop mode; a central guard rejects
  any non-GET Git request with a read-only error (write ops deferred).
- `src/components/git-panel/hooks/useRevertLocalCommit.ts` — U04: no-op in desktop mode.
- `src/components/git-panel/view/**` (GitPanel, GitPanelHeader, BranchesView,
  ChangesView, CommitComposer, FileChangeList, FileChangeItem, NewBranchModal) — U04:
  a `readOnly` prop (set from `IS_DESKTOP`) disables all Git mutation buttons
  (switch/create/delete branch, fetch/pull/push/publish, revert, discard/delete file,
  commit/initial-commit, generate message); read-only Refresh stays enabled.
- `src/components/main-content/view/MainContent.tsx` — U05: in desktop mode the Tasks
  tab renders the Rust-managed `HarnessTasksPanel` (shown regardless of TaskMaster
  installation); the web build keeps `TaskMasterPanel` behind its existing gate.
- `src-tauri/tauri.conf.json` — Tauri 2 host config; `frontendDist` = derived `dist/`;
  `devUrl` = product Vite dev server; CSP widened for the real frontend; YourAgent
  window branding; Developer ID signing identity for macOS.

Upstream branding ("CloudCLI UI" / "Claude Code UI") is retained in the imported
source pending the product rebrand; attribution is preserved here and in NOTICE.

## Migration status (HARNESS_CLAUDECODEUI_MIGRATION.md)

- U02 (mock DesktopClient, self-contained boot): DONE — the derived frontend boots
  into the real workspace (sidebar/project, provider selection, chat composer,
  settings, Chat/Shell/Files/Git tabs) with NO Express/localhost backend, verified
  via DOM inspection. Data is fixture mock.
- U03 (real Rust Codex app-server adapter): DONE (wired + verified at the adapter
  boundary) — engine.rs/bridge.rs replace the fixtures for sessions/history/send/
  cancel/approval. Verified: real initialize + thread/list handshake against the
  0.158.0 binary with an isolated CODEX_HOME (asserted NOT `~/.codex`); 19 Rust unit
  tests (mapping + handshake + mock-server end-to-end) and frontend `mapEngineEvent`
  assertions pass; clippy/fmt clean; frontend typecheck clean. Live turn execution,
  streaming reconciliation and approval round-trips require an authenticated engine
  account and explicit human approval, so they are gated on user auth (the isolated
  home is signed-out) and are NOT auto-run. Known follow-up: the legacy single-slot
  `latestMessage` can drop events under batch; an ordered subscription/reducer is the
  next refinement (design permits borrowing NormalizedMessage shapes meanwhile).
- U04 (file tree / editor / read-only Git): DONE (wired + verified at the command
  boundary) — workspace.rs backs the Files tree, editor read/save and a read-only Git
  panel. Verified: 27 Rust unit tests pass (path confinement rejects absolute/`..`/
  symlink escape; read/save round-trip with mtime conflict detection; size limits;
  porcelain status categorization; commit-hash validation blocks injection); clippy/
  fmt clean; frontend typecheck clean. Git write operations are disabled both at the
  data layer (central non-GET guard) and in the UI (`readOnly` buttons). Editor saves
  are version-checked and prompt Reload on conflict rather than overwriting. Live
  in-app verification against a real workspace root is recorded below.
  Verified live: an `#[ignore]` real-workspace smoke test run against the actual codex
  repo (`YOURAGENT_WORKSPACE_ROOT`) confirms the frontend data contracts (bare-array
  file tree, flat status arrays, branch/remote/commit shapes, HEAD commit diff) and
  path-boundary rejection on the real root; the rebuilt dist is embedded in the
  Developer ID-signed `YourAgent.app` (signature `--verify --deep --strict` valid),
  which boots alive with an isolated CODEX_HOME + real workspace root and no engine
  (fixture fallback) and shuts down cleanly.
- U05 (Harness Tasks tab, node status, gate evidence, reports): DONE (wired +
  verified at the command boundary) — harness.rs reuses the SuperAgent orchestrator/
  contracts/storage crates; the Tasks tab is the Rust-managed Harness business view in
  desktop mode (no TaskMaster execution dependency). Verified: 33 Rust unit tests pass
  (6 new harness tests: success run proves every gate; failing verify blocks downstream
  and fails the run with finite retries; missing acceptance waits for input; artifact
  read by backend id + report export + traversal rejection; list/get refresh events;
  fixture workflow plans without validation errors); clippy/fmt clean; frontend
  typecheck clean. Node success is proven only by gate evidence (real `true`/`false`
  exit codes / artifacts), never by chat text or TaskMaster identifiers. The rebuilt
  dist is embedded in the Developer ID-signed `YourAgent.app` (signature
  `--verify --deep --strict` valid, Team KRV5T3384C), which boots alive with an isolated
  CODEX_HOME + isolated harness DB (created at boot) + real workspace root and no engine,
  then shuts down cleanly. In-app click-through of the WebView could not be automated in
  this environment (browser automation cannot reach the Tauri WebView); the data
  contracts are covered by the Rust tests. Follow-ups: restart-recovery of detailed gate
  evidence from the store, and live engine-driven business execution (P2-5/P2-6 executor
  wired to the engine adapter).
- U06 (full Web-dependency removal + packaging): pending.

## License obligations carried forward

- Keep `LICENSE` and `NOTICE` with any distribution.
- Preserve reasonable, visible attribution to ClaudeCodeUI / Siteboon AI B.V.
- Mark modified versions clearly (this file is the modification record).
- Do not imply upstream endorsement; no trademark rights are granted.
- The AGPL copyleft scope for the combined work must be resolved before any
  closed-source distribution; splitting into Tauri/Rust processes does not by
  itself remove the obligation.