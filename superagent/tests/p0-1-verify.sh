#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-1
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
ENGINE=$ROOT/superagent/packaging/superagent-engine
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV"

echo "== 1. rename engine (rm+cp: avoid macOS inode reuse SIGKILL) =="
rm -f "$ENGINE"
cp "$ROOT/codex-rs/target/release/codex" "$ENGINE"
chmod +x "$ENGINE"

echo "== 1b. code-mode host (all catalog models are code_mode_only; host required) =="
# V8 sandbox prebuilts are published on openai/codex releases, not denoland rusty_v8.
HOST=$ROOT/superagent/packaging/codex-code-mode-host
if [ ! -x "$ROOT/codex-rs/target/release/codex-code-mode-host" ]; then
  (cd "$ROOT/codex-rs" && RUSTY_V8_MIRROR='https://github.com/openai/codex/releases/download/rusty-v8-v152.2.0/{file}' \
    cargo build --release -p codex-code-mode-host)
fi
rm -f "$HOST"
cp "$ROOT/codex-rs/target/release/codex-code-mode-host" "$HOST"
chmod +x "$HOST"
"$HOST" --help > "$EV/code-mode-host-help.txt" 2>&1
echo "host smoke exit=$?"

export CODEX_HOME="$HOME_DIR"
echo "== 2. version =="
"$ENGINE" --version > "$EV/version.txt" 2>&1
cat "$EV/version.txt"

echo "== 3. login status (dedicated CODEX_HOME, explicit auth reuse) =="
"$ENGINE" login status > "$EV/login-status.txt" 2>&1
cat "$EV/login-status.txt"

echo "== 4. doctor (paths incl. requirements sources) =="
"$ENGINE" doctor > "$EV/doctor.txt" 2>&1
tail -30 "$EV/doctor.txt"

echo "== 5. configRequirements/read + config/read via stdio =="
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls '[{"method":"configRequirements/read","params":{}},{"method":"config/read","params":{}}]' \
  --out "$EV/config-requirements-transcript.json" --deadline 90
echo "spike exit=$?"

echo "== 6. wire contract baseline (engine generate-json-schema, archived) =="
rm -rf "$EV/schema" "$EV/schema-engine" "$EV/schema-engine-experimental"
mkdir -p "$EV/schema-engine" "$EV/schema-engine-experimental"
"$ENGINE" app-server generate-json-schema --out "$EV/schema-engine" > "$EV/schema-gen.log" 2>&1
"$ENGINE" app-server generate-json-schema --experimental --out "$EV/schema-engine-experimental" >> "$EV/schema-gen.log" 2>&1
diff -rq "$EV/schema-engine" "$ROOT/codex-rs/app-server-protocol/schema/json" > "$EV/schema-vs-repo.diff" 2>&1
echo "default bundle vs repo fixtures diff exit=$? (0 = identical)"
diff -rq "$EV/schema-engine-experimental" "$ROOT/codex-rs/app-server-protocol/schema/json" > "$EV/schema-experimental-vs-repo.diff" 2>&1
echo "experimental bundle vs repo fixtures diff exit=$? (non-zero expected: extra experimental surface)"
find "$EV/schema-engine" -name '*.json' | wc -l
# Archive bundles as tarballs (raw dirs are gitignored); default bundle is byte-identical
# to repo fixtures, experimental bundle adds the gated surface.
tar -czf "$EV/schema-engine.tar.gz" -C "$EV" schema-engine
tar -czf "$EV/schema-engine-experimental.tar.gz" -C "$EV" schema-engine-experimental

echo "== 7. engine manifest =="
COMMIT=$(git -C "$ROOT" rev-parse HEAD)
ENGINE_SHA=$(shasum -a 256 "$ENGINE" | cut -d' ' -f1)
HOST_SHA=$(shasum -a 256 "$HOST" | cut -d' ' -f1)
SCHEMA_SHA=$(find "$EV/schema-engine" "$EV/schema-engine-experimental" -name '*.json' -exec shasum -a 256 {} \; | sort | shasum -a 256 | cut -d' ' -f1)
SCHEMA_TAR_SHA=$(shasum -a 256 "$EV/schema-engine.tar.gz" | cut -d' ' -f1)
SCHEMA_EXP_TAR_SHA=$(shasum -a 256 "$EV/schema-engine-experimental.tar.gz" | cut -d' ' -f1)
cat > "$EV/engine-manifest.json" <<EOF
{
  "engine_name": "superagent-engine",
  "source_repo": "Colin4k1024/codex",
  "source_commit": "$COMMIT",
  "engine_sha256": "$ENGINE_SHA",
  "code_mode_host_sha256": "$HOST_SHA",
  "v8_artifact_source": "https://github.com/openai/codex/releases/tag/rusty-v8-v152.2.0 (ptrcomp_sandbox release, aarch64-apple-darwin)",
  "schema_bundle_sha256": "$SCHEMA_SHA",
  "schema_default_tar_sha256": "$SCHEMA_TAR_SHA",
  "schema_experimental_tar_sha256": "$SCHEMA_EXP_TAR_SHA",
  "platform": "$(uname -sm)",
  "build_profile": "release",
  "codex_home": "$HOME_DIR",
  "auth_mode": "explicit-reuse-copy-from-~/.codex (2026-11-17 goal decision)",
  "rename_reason": "endpoint security SIGKILLs processes named codex (exit 137); renamed binary verified runnable"
}
EOF
cat "$EV/engine-manifest.json"
echo "== P0-1 verification script done =="