# Codex Desktop Packaging

This document describes the packaging, signing, and update safety design for Codex Desktop.

## Package Formats

| Platform | Format      | Notes                               |
| -------- | ----------- | ----------------------------------- |
| macOS    | `.dmg`      | Signed and notarized                |
| Windows  | `.msi`      | Signed with Authenticode            |
| Linux    | `.AppImage` | Self-contained, no install required |

## Sidecar Strategy

The app-server binary is bundled as a sidecar inside the app package:

- macOS: `Contents/Resources/sidecar/codex-app-server`
- Windows: `resources/sidecar/codex-app-server.exe`
- Linux: `resources/sidecar/codex-app-server`

## Version Binding

The app and app-server protocol versions must match on the major version number. The app validates this binding at startup and refuses to start if versions are incompatible.

## Update Safety

1. **Integrity check**: Every update package includes a SHA-256 manifest. The installer verifies the package hash before installation.
2. **Signature verification**: Packages are signed with platform-appropriate certificates. Unsigned or tampered packages are rejected.
3. **Protocol compatibility**: The update manifest includes the protocol version. The app refuses to install updates with incompatible protocol versions.
4. **Rollback**: The previous version is preserved until the new version passes a smoke test. If the new version fails to start, the app automatically rolls back to the previous version.
5. **Disk space check**: The installer verifies sufficient disk space before downloading an update.

## CI Artifacts

CI builds produce unsigned artifacts for testing. Release builds are produced separately with signing credentials. No CI job publishes to distribution channels.

## Data Safety

- Packages do not contain user data or keys.
- Install/uninstall does not modify `CODEX_HOME` or user data directories.
- Updates preserve user settings and session data.
