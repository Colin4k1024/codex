import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import { hashGeneratedFiles, protocolRevisionFile } from './protocol.mjs'

const revision = hashGeneratedFiles()
mkdirSync(dirname(protocolRevisionFile), { recursive: true })
writeFileSync(protocolRevisionFile, JSON.stringify(revision, null, 2) + '\n')
console.log(
  `Updated app-server protocol revision to ${revision.revision} (${revision.fileCount} files)`,
)
