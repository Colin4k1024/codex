import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join, sep } from 'node:path'
import { compileFromFile } from 'json-schema-to-typescript'
import {
  assertProtocolRevision,
  generatedSourceDir,
  jsonRpcSchemaFile,
  listGeneratedFiles,
  packageDir,
} from './protocol.mjs'

const generatedDir = join(packageDir, 'src/generated')
rmSync(generatedDir, { recursive: true, force: true })
mkdirSync(generatedDir, { recursive: true })
cpSync(generatedSourceDir, generatedDir, { recursive: true })
const jsonRpcTypes = await compileFromFile(jsonRpcSchemaFile)
writeFileSync(join(generatedDir, 'jsonrpc.ts'), jsonRpcTypes)

const files = [...listGeneratedFiles(), 'jsonrpc.ts']
const exports = []
const seenNames = new Set()
for (const relativePath of files) {
  const source = join(generatedDir, relativePath)
  const contents = readFileSync(source, 'utf8')
  const isV2 = relativePath.startsWith('v2' + sep)
  const names = [...contents.matchAll(/export\s+(?:type|interface)\s+([A-Za-z0-9_$]+)/g)]
    .map((match) => (isV2 ? `${match[1]} as V2${match[1]}` : match[1]))
    .filter((name) => {
      const baseName = name.split(' as ')[0]
      if (seenNames.has(baseName)) {
        return false
      }
      seenNames.add(baseName)
      return true
    })
  if (names.length === 0) {
    continue
  }
  const importPath = './' + relativePath.replace(/\.ts$/, '').split(sep).join('/')
  exports.push(`export type { ${names.join(', ')} } from '${importPath}'`)
}

const indexPath = join(generatedDir, 'index.ts')
writeFileSync(indexPath, exports.join('\n') + '\n')

const revision = assertProtocolRevision()
const revisionPath = join(generatedDir, 'protocolRevision.ts')
writeFileSync(
  revisionPath,
  `export const PROTOCOL_REVISION = '${revision.revision}'\nexport const PROTOCOL_FILE_COUNT = ${revision.fileCount}\n`,
)
