import { describe, expect, it } from 'vitest'
import {
  APP_ID,
  createUpdateManifest,
  getPackageName,
  getSidecarPath,
  validateVersionBinding,
} from '../scripts/packaging'

describe('packaging', () => {
  it('generates platform-specific package names', () => {
    expect(getPackageName('0.1.0', 'darwin', 'arm64')).toBe('Codex Desktop-0.1.0-darwin-arm64.dmg')
    expect(getPackageName('0.1.0', 'win32', 'x64')).toBe('Codex Desktop-0.1.0-win32-x64.msi')
    expect(getPackageName('0.1.0', 'linux', 'x64')).toBe('Codex Desktop-0.1.0-linux-x64.AppImage')
  })

  it('generates sidecar paths', () => {
    expect(getSidecarPath('darwin')).toContain('codex-app-server')
    expect(getSidecarPath('win32')).toContain('codex-app-server.exe')
  })

  it('validates version binding', () => {
    expect(validateVersionBinding('0.1.0', '0.1.0')).toBe(true)
    expect(validateVersionBinding('0.1.0', '0.2.0')).toBe(true)
    expect(() => validateVersionBinding('1.0.0', '0.1.0')).toThrow('Incompatible versions')
  })

  it('creates update manifests', () => {
    const manifest = createUpdateManifest({
      version: '0.1.0',
      platform: 'darwin',
      arch: 'arm64',
      sha256: 'abc123',
      url: 'https://example.com/update.dmg',
      protocolVersion: '0.1.0',
    })
    expect(manifest.appId).toBe(APP_ID)
    expect(manifest.version).toBe('0.1.0')
    expect(manifest.protocolVersion).toBe('0.1.0')
  })

  it('rejects invalid version binding', () => {
    expect(() => validateVersionBinding('', '0.1.0')).toThrow('must be defined')
    expect(() => validateVersionBinding('0.1.0', '')).toThrow('must be defined')
  })
})
