# P5-1 remote environment integration (#95)

## Implemented

- `EnvironmentConfig` on the adapter/executor: environment id, exec-server URL, bearer token, cwd, workspace roots.
- `environment/add` with `authBearerToken` (token redacted from run logs) and `environment/info` capability snapshot.
- `thread/start.environments` for the agent turn; no environment config preserves the existing wire bytes.
- Registration/info failure is fatal: no silent local fallback, no thread/turn.
- Live evidence: authenticated exec-server (capability token), remote model turn wrote `REMOTE-P51` in `/tmp/p5-1-work`, remote marker `REMOTE-ENV-51`, and a workspace-outside write was denied by the exec-server sandbox.

## Not proven / follow-ups

- Full exec-server `capability_discovery` (FS/process/sandbox matrix) is not exposed through app-server `environment/info`; this PR records shell/cwd only.
- The exec-server must be provisioned with `sandbox_mode=workspace-write` and writable roots for the permission boundary; the adapter only registers URL/token.
- VerificationExecutor/command gates do not register an environment in this PR (agent path only).
- No real Reddit/Mastodon/Gmail; this is local loopback exec-server.
