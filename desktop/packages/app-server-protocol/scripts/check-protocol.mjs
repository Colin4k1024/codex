import { assertProtocolRevision } from './protocol.mjs'

const { revision, fileCount } = assertProtocolRevision()
console.log(`app-server protocol revision ${revision} (${fileCount} files)`)
