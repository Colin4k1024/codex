const APP_NAME = 'Codex Desktop'
const APP_ID = 'com.codex.desktop'

type Platform = 'darwin' | 'win32' | 'linux'

function getPlatformPackageExtension(platform: Platform): string {
  switch (platform) {
    case 'darwin':
      return 'dmg'
    case 'win32':
      return 'msi'
    case 'linux':
      return 'AppImage'
    default:
      throw new Error(`Unsupported platform: ${platform}`)
  }
}

function getPackageName(version: string, platform: Platform, arch: string): string {
  const ext = getPlatformPackageExtension(platform)
  return `${APP_NAME}-${version}-${platform}-${arch}.${ext}`
}

function getSidecarName(platform: Platform): string {
  return platform === 'win32' ? 'codex-app-server.exe' : 'codex-app-server'
}

function getSidecarPath(platform: Platform): string {
  const path = require('node:path') as typeof import('node:path')
  const resourcesPath = process.resourcesPath || path.join(__dirname, '..')
  return path.join(resourcesPath, 'sidecar', getSidecarName(platform))
}

function validateVersionBinding(appVersion: string, protocolVersion: string): boolean {
  if (!appVersion || !protocolVersion) {
    throw new Error('App version and protocol version must be defined')
  }
  const [appMajor] = appVersion.split('.')
  const [protocolMajor] = protocolVersion.split('.')
  if (appMajor !== protocolMajor) {
    throw new Error(
      `Incompatible versions: app ${appVersion} requires protocol major ${appMajor}.x, got ${protocolVersion}`,
    )
  }
  return true
}

function validatePackageIntegrity(packagePath: string, expectedSha256?: string): boolean {
  const fs = require('node:fs') as typeof import('node:fs')
  if (!fs.existsSync(packagePath)) {
    throw new Error(`Package not found: ${packagePath}`)
  }
  if (expectedSha256) {
    const crypto = require('node:crypto') as typeof import('node:crypto')
    const actualSha256 = crypto
      .createHash('sha256')
      .update(fs.readFileSync(packagePath))
      .digest('hex')
    if (actualSha256 !== expectedSha256) {
      throw new Error(
        `Package integrity check failed: expected sha256 ${expectedSha256}, got ${actualSha256}`,
      )
    }
  }
  return true
}

function checkDiskSpace(requiredBytes: number, targetPath: string): boolean {
  const fs = require('node:fs') as typeof import('node:fs')
  try {
    const stats = fs.statfsSync(targetPath)
    const available = stats.bavail * stats.bsize
    if (available < requiredBytes) {
      throw new Error(
        `Insufficient disk space: need ${requiredBytes} bytes, have ${available} bytes`,
      )
    }
    return true
  } catch (error: unknown) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return true
    }
    throw error
  }
}

interface UpdateManifestOptions {
  version: string
  platform: Platform
  arch: string
  sha256: string
  url: string
  protocolVersion: string
}

function createUpdateManifest({
  version,
  platform,
  arch,
  sha256,
  url,
  protocolVersion,
}: UpdateManifestOptions) {
  return {
    version,
    platform,
    arch,
    sha256,
    url,
    protocolVersion,
    createdAt: new Date().toISOString(),
    appId: APP_ID,
  }
}

export {
  APP_NAME,
  APP_ID,
  getPackageName,
  getSidecarPath,
  validateVersionBinding,
  validatePackageIntegrity,
  checkDiskSpace,
  createUpdateManifest,
}
