# Codex Desktop

Experimental Electron shell for the Codex app-server client.

## Development

```bash
pnpm --filter codex-desktop... install
pnpm --dir desktop dev
```

The dev script starts Vite, compiles the Electron main/preload bundle, and launches Electron against the Vite dev server.

## Checks

```bash
pnpm --dir desktop lint
pnpm --dir desktop typecheck
pnpm --dir desktop test
pnpm --dir desktop build
pnpm --dir desktop test:electron
pnpm --dir desktop test:visual
```

`test:electron` launches the built shell and closes it immediately through the `--smoke` flag.

## Security model

- Electron main is the only layer with Node and child-process authority.
- Renderer code cannot import Node APIs or Electron directly.
- Renderer code accesses Electron through the typed preload bridge exposed as `window.codexDesktop`.
- Context isolation, sandboxing, and web security remain enabled.
- The preload surface is intentionally minimal and versioned with the app-server contract.

## App-server protocol package

`desktop/packages/app-server-protocol` regenerates its TypeScript contract from `codex-rs/app-server-protocol/schema/typescript` and the JSON-RPC envelope JSON schema. Generated files are copied into the package during build and are never edited by hand. The checked-in `protocol-revision.json` pins the current fixture hash; CI fails closed when the Rust schema drift changes that hash.

## Design system

The renderer uses Tailwind CSS with semantic color tokens for light and dark themes. Reusable UI primitives live under `desktop/src/renderer/components/ui` and cover buttons, cards, inputs, badges, status indicators, and navigation items. The shell provides a responsive header, sidebar, main region, and footer with keyboard-accessible landmarks.

## Local Docker CI runner

`desktop-ci` targets the `local-docker` GitHub Actions runner label while hosted runner billing is unavailable. Register a runner against this repository with the `local-docker` label. Do **not** expose the host Docker socket to the runner container: this workflow checks out and executes pull-request code, so a Docker socket would grant that code root-equivalent host access. The runner is Linux-only; #38 remains the release prerequisite for restoring the full cross-platform compatibility matrix.

## Thread workspace rollback flag

Append `?disableThreads=1` to the renderer URL to hide the thread workspace and fall back to the empty shell. This is the rollback path for the conversation feature; protocol and process-manager capabilities remain available.
