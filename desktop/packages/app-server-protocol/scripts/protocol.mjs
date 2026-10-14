import { createHash } from 'node:crypto'
import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join, relative, resolve, sep } from 'node:path'

export const packageDir = resolve(fileURLToPath(new URL('..', import.meta.url)))
export const repositoryRoot = resolve(packageDir, '../../..')
export const generatedSourceDir = join(
  repositoryRoot,
  'codex-rs/app-server-protocol/schema/typescript',
)
export const jsonRpcSchemaFile = join(
  repositoryRoot,
  'codex-rs/app-server-protocol/schema/json/JSONRPCMessage.json',
)
export const protocolRevisionFile = join(packageDir, 'protocol-revision.json')

function walk(dir) {
  const entries = readdirSync(dir, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const fullPath = join(dir, entry.name)
    if (entry.isDirectory()) {
      files.push(...walk(fullPath))
    } else if (entry.isFile() && entry.name.endsWith('.ts')) {
      files.push(fullPath)
    }
  }
  return files
}

export function listGeneratedFiles() {
  return walk(generatedSourceDir)
    .map((file) => relative(generatedSourceDir, file))
    .sort()
}

export function hashGeneratedFiles() {
  const hash = createHash('sha256')
  const files = listGeneratedFiles()
  const jsonRpcSchema = readFileSync(jsonRpcSchemaFile).toString().replace(/\r\n/g, '\n')
  hash.update('json/JSONRPCMessage.json')
  hash.update('\0')
  hash.update(jsonRpcSchema)
  hash.update('\0')
  for (const relativePath of files) {
    const normalizedPath = relativePath.split(sep).join('/')
    hash.update(normalizedPath)
    hash.update('\0')
    hash.update(
      readFileSync(join(generatedSourceDir, relativePath)).toString().replace(/\r\n/g, '\n'),
    )
    hash.update('\0')
  }
  return {
    fileCount: files.length + 1,
    revision: hash.digest('hex'),
  }
}

export function assertProtocolRevision() {
  const actual = hashGeneratedFiles()
  const expected = JSON.parse(readFileSync(protocolRevisionFile, 'utf8'))
  if (actual.revision !== expected.revision || actual.fileCount !== expected.fileCount) {
    throw new Error(
      `App-server protocol fixtures changed. Expected revision ${expected.revision} with ${expected.fileCount} files, got ${actual.revision} with ${actual.fileCount} files. Run pnpm --dir desktop/packages/app-server-protocol run update-revision.`,
    )
  }
  return actual
}
