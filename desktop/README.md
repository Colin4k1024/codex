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
```

`test:electron` launches the built shell and closes it immediately through the `--smoke` flag.

## Security model

- Electron main is the only layer with Node and child-process authority.
- Renderer code cannot import Node APIs or Electron directly.
- Renderer code accesses Electron through the typed preload bridge exposed as `window.codexDesktop`.
- Context isolation, sandboxing, and web security remain enabled.
- The preload surface is intentionally minimal and versioned with the app-server contract.
