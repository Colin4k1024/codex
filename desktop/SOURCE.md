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

## What was imported

- `dist/` — the production frontend bundle produced by running `vite build` on the
  upstream source at the locked commit (React 18 / Vite / Tailwind / CodeMirror /
  xterm / i18next / KaTeX). Regenerate with:
  `cd <claudecodeui> && npm install --ignore-scripts && node_modules/.bin/vite build`.
- `LICENSE`, `NOTICE` — verbatim upstream license and attribution notices.

The upstream Express/JWT/WebSocket backend (`server/`) is NOT imported into the
product build. In dev mode it runs from the reference checkout (see `run-dev.sh`);
the production migration replaces it with a typed DesktopClient over Tauri commands
(`HARNESS_CLAUDECODEUI_MIGRATION.md`, steps U02–U06).

## Modifications made for the desktop wrap

- `src-tauri/tauri.conf.json`: Tauri 2 host config; `frontendDist` points at the
  imported `dist/`; `devUrl` points at the upstream Vite dev server for dev mode;
  CSP widened to load the real frontend (fonts/images/workers/backend connect);
  window branding set to YourAgent; Developer ID signing identity for macOS.
- `dist/index.html` and bundled assets are the upstream build output, unmodified.
  Upstream branding ("CloudCLI UI" / "Claude Code UI") is retained in the imported
  bundle pending the product rebrand; attribution is preserved here and in NOTICE.
- No upstream source file was edited in the reference checkout; the wrap consumes
  its build output and runs its backend unmodified in dev mode.

## License obligations carried forward

- Keep `LICENSE` and `NOTICE` with any distribution.
- Preserve reasonable, visible attribution to ClaudeCodeUI / Siteboon AI B.V.
- Mark modified versions clearly (this file is the modification record).
- Do not imply upstream endorsement; no trademark rights are granted.
- The AGPL copyleft scope for the combined work must be resolved before any
  closed-source distribution; splitting into Tauri/Rust processes does not by
  itself remove the obligation.