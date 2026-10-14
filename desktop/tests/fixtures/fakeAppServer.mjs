// Fake `codex app-server --listen stdio://` for desktop integration tests.
// Behavior is selected with FAKE_MODE:
//   echo       - respond to every request with { result: params } (default)
//   malformed  - write one malformed frame, then behave like echo
//   unknown    - respond -32601 method not found
//   overloaded - respond -32001 overloaded
//   shutdown   - respond -32600 with data.reason=serverShuttingDown
//   crash      - exit(1) immediately after startup
//   noResponse - ignore initialize so the client times out
//   badInit    - respond to initialize with an invalid result
//   notify     - after initialized, emit notifyCount ordered notifications
//   huge       - write one 9 MiB frame without newline
//   closeStdout - destroy stdout after initialized and stay alive
//   closeStdin  - destroy stdin after initialized and stay alive
import { setTimeout as delay } from 'node:timers/promises'
import process from 'node:process'

const mode = process.env.FAKE_MODE ?? 'echo'
if (mode === 'crash') {
  process.exit(1)
}
const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`)
const respond = (id, result) => send({ id, result })
const respondError = (id, code, message, data) => send({ id, error: { code, message, data } })

const threadId = 'integration-thread-1'
const thread = {
  id: threadId,
  sessionId: 'integration-session-1',
  forkedFromId: null,
  parentThreadId: null,
  preview: 'Integration thread',
  ephemeral: false,
  section: null,
  sectionEnteredAt: null,
  projectId: null,
  historyMode: 'paginated',
  modelProvider: 'openai',
  model: 'gpt-test',
  reasoningEffort: null,
  createdAt: 1,
  updatedAt: 2,
  recencyAt: 2,
  status: { type: 'idle' },
  path: null,
  cwd: '/tmp',
  cliVersion: 'test',
  originator: null,
  source: 'appServer',
  threadSource: null,
  agentNickname: null,
  agentRole: null,
  gitInfo: null,
  name: 'Integration thread',
  turns: [],
}
const threadEnvelope = (turns = []) => ({
  thread: { ...thread, turns },
  model: 'gpt-test',
  modelProvider: 'openai',
  serviceTier: null,
  disabledPluginIds: [],
  cwd: '/tmp',
  instructionSources: [],
  approvalPolicy: 'untrusted',
  approvalsReviewer: 'none',
  sandbox: {},
  reasoningEffort: null,
})
const resumeEnvelope = (turns = []) => ({
  ...threadEnvelope(turns),
  collaborationMode: null,
  turnsBackwardsCursor: null,
  itemsBackwardsCursor: null,
})
const userItem = {
  type: 'userMessage',
  id: 'integration-user-1',
  clientId: null,
  content: [{ type: 'text', text: 'Hello from integration', text_elements: [] }],
}
const assistantItem = { type: 'agentMessage', id: 'integration-assistant-1', text: '' }
const makeTurn = (status, items = []) => ({
  id: 'integration-turn-1',
  rootTurnId: 'integration-turn-1',
  items,
  itemsView: 'full',
  status,
  error: null,
  startedAt: 1,
  completedAt: status === 'inProgress' ? null : 2,
  durationMs: status === 'inProgress' ? null : 1,
})

function emitTurnEvents() {
  send({
    method: 'item/started',
    params: { threadId, turnId: 'integration-turn-1', item: assistantItem },
  })
  send({
    method: 'item/agentMessage/delta',
    params: {
      threadId,
      turnId: 'integration-turn-1',
      itemId: 'integration-assistant-1',
      delta: 'Model reply',
    },
  })
  send({
    method: 'turn/completed',
    params: {
      threadId,
      turn: makeTurn('completed', [userItem, { ...assistantItem, text: 'Model reply' }]),
    },
  })
}

async function handleRequest(message) {
  if (mode === 'noResponse') {
    return
  }

  if (mode === 'threads') {
    if (message.method === 'thread/list') {
      respond(message.id, { data: [thread], nextCursor: null, backwardsCursor: null })
      return
    }
    if (message.method === 'thread/start' || message.method === 'thread/resume') {
      respond(message.id, resumeEnvelope([]))
      return
    }
    if (message.method === 'thread/read') {
      respond(message.id, {
        thread: {
          ...thread,
          turns: [makeTurn('completed', [userItem, { ...assistantItem, text: 'Durable reply' }])],
        },
      })
      return
    }
    if (message.method === 'turn/start') {
      respond(message.id, { turn: makeTurn('inProgress', [userItem]) })
      setTimeout(emitTurnEvents, 25)
      return
    }
  }

  if (message.method === 'initialize') {
    if (mode === 'badInit') {
      respond(message.id, { unexpected: true })
      return
    }
    respond(message.id, {
      userAgent: 'fake-app-server',
      codexHome: '/fake/codex-home',
      platformFamily: 'unix',
      platformOs: 'macos',
    })
    return
  }

  switch (mode) {
    case 'unknown':
      respondError(message.id, -32601, `method not found: ${message.method}`)
      return
    case 'overloaded':
      respondError(message.id, -32001, 'Server overloaded; retry later.')
      return
    case 'shutdown':
      respondError(message.id, -32600, 'Server is draining; retry after reconnecting', {
        reason: 'serverShuttingDown',
      })
      return
    case 'crash':
      process.exit(1)
      return
    case 'slow':
      await delay(200)
      respond(message.id, message.params ?? null)
      return
    default:
      respond(message.id, message.params ?? null)
  }
}

let buffer = ''
process.stdin.setEncoding('utf8')
process.stdin.on('data', (chunk) => {
  buffer += chunk
  for (;;) {
    const index = buffer.indexOf('\n')
    if (index === -1) {
      return
    }
    const line = buffer.slice(0, index)
    buffer = buffer.slice(index + 1)
    if (line.trim().length === 0) {
      continue
    }
    let message
    try {
      message = JSON.parse(line)
    } catch {
      continue
    }
    if (typeof message.method === 'string' && message.id === undefined) {
      if (message.method === 'initialized') {
        if (mode === 'malformed') {
          process.stdout.write('this is not json\n')
        }
        if (mode === 'huge') {
          process.stdout.write(`${'x'.repeat(9 * 1024 * 1024)}\n`)
        }
        if (mode === 'notify') {
          const count = Number(process.env.FAKE_NOTIFY_COUNT ?? '3')
          for (let i = 0; i < count; i += 1) {
            send({ method: 'test/ordered', params: { sequence: i } })
          }
        }
        if (mode === 'closeStdout') {
          process.stdout.end()
          setInterval(() => {}, 1 << 30)
        }
        if (mode === 'closeStdin') {
          process.stdin.destroy()
          setInterval(() => {}, 1 << 30)
        }
      }
      continue
    }
    void handleRequest(message)
  }
})
