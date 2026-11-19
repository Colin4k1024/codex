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
  typed `permission_request` events and are NEVER auto-answered.
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
- `src-tauri/src/lib.rs` — product commands: bootstrap/auth_status/login/register/
  auth_user/logout/onboarding_status/complete_onboarding/git_config/update_git_config/
  list_projects/list_sessions/get_messages/send_input/desktop_request plus U03
  engine_connect/engine_status/engine_disconnect/respond_to_request/cancel_turn.
  list_sessions/get_messages/send_input are engine-first (thread/list,
  thread/items/list, thread/start + turn/start, turn/interrupt) with fixture
  fallback when no engine is configured. `desktop_request` remains a graceful-
  degradation stub for unmigrated endpoints (NOT a fake Express API). Approval
  decisions default to `decline` (never auto-accept).
- `run-dev.sh`, `.env` (VITE_DESKTOP=true), this `SOURCE.md`.
- `src-tauri/Cargo.toml` — added `chrono` (timestamp conversion in bridge.rs).

Modified upstream files (marked here per Section 7):
- `src/constants/config.ts` — added `IS_DESKTOP` (Tauri runtime / VITE_DESKTOP).
- `src/utils/api.js` — in desktop mode the data layer routes through the
  DesktopClient (typed commands for the boot path; generic product request for the
  rest). The upstream OSS fetch path is retained for the browser build.
- `src/components/auth/context/AuthContext.tsx` — added an `IS_DESKTOP` branch that
  provisions the session via `desktopClient.bootstrap()` (auto-authentication).
- `src/contexts/WebSocketContext.tsx` — in desktop mode, ordered product events over
  Tauri IPC replace the WebSocket; `sendMessage` goes through `desktopClient.sendInput`.
  U03: on boot it subscribes to `desktop-event` then calls `engineConnect()` so the
  real adapter is wired before any event arrives; falls back gracefully when no
  engine is configured.
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
- U04–U06 (file/editor/read-only Git, Harness Tasks tab, full Web-dependency removal
  + packaging): pending.

## License obligations carried forward

- Keep `LICENSE` and `NOTICE` with any distribution.
- Preserve reasonable, visible attribution to ClaudeCodeUI / Siteboon AI B.V.
- Mark modified versions clearly (this file is the modification record).
- Do not imply upstream endorsement; no trademark rights are granted.
- The AGPL copyleft scope for the combined work must be resolved before any
  closed-source distribution; splitting into Tauri/Rust processes does not by
  itself remove the obligation.