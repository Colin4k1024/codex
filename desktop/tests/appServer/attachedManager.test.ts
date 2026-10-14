import { createServer, type Server as HttpServer } from 'node:http'
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WebSocketServer, type WebSocket } from 'ws'
import { AttachedAppServerManager } from '../../src/main/appServer/attachedManager'
import type { AppServerNotificationEvent, AppServerStatus } from '../../src/shared/appServer'

interface DaemonOptions {
  appServerVersion?: string
}

interface ClientFrame {
  id?: string | number
  method?: string
  params?: unknown
  result?: unknown
}

class FakeDaemonServer {
  readonly clients = new Set<WebSocket>()
  readonly sentFrames: Array<Record<string, unknown>> = []
  readonly receivedFrames: ClientFrame[] = []
  readonly socketPath: string
  private readonly appServerVersion: string
  private httpServer: HttpServer | null = null
  private webSocketServer: WebSocketServer | null = null

  constructor(socketPath: string, options: DaemonOptions = {}) {
    this.socketPath = socketPath
    this.appServerVersion = options.appServerVersion ?? '0.72.0'
  }

  async start(): Promise<void> {
    this.httpServer = createServer()
    this.webSocketServer = new WebSocketServer({ server: this.httpServer, path: '/' })
    this.webSocketServer.on('connection', (websocket) => {
      this.clients.add(websocket)
      websocket.on('message', (data) => {
        const frame = JSON.parse(data.toString()) as ClientFrame
        this.receivedFrames.push(frame)
        this.handleFrame(websocket, frame)
      })
      websocket.on('close', () => {
        this.clients.delete(websocket)
      })
    })

    await new Promise<void>((resolve, reject) => {
      this.httpServer?.once('error', reject)
      this.httpServer?.listen(this.socketPath, resolve)
    })
  }

  async close(): Promise<void> {
    for (const client of this.clients) {
      client.terminate()
    }
    this.clients.clear()
    const webSocketServer = this.webSocketServer
    const httpServer = this.httpServer
    this.webSocketServer = null
    this.httpServer = null
    await new Promise<void>((resolve) => {
      webSocketServer?.close(() => resolve())
      if (!webSocketServer) {
        resolve()
      }
    })
    await new Promise<void>((resolve, reject) => {
      httpServer?.close((error) => {
        if (error) {
          reject(error)
          return
        }
        resolve()
      })
      httpServer?.closeAllConnections()
      if (!httpServer) {
        resolve()
      }
    })
    await rm(this.socketPath, { force: true })
  }

  broadcast(method: string, params: unknown): void {
    for (const client of this.clients) {
      this.sendTo(client, { method, params })
    }
  }

  request(client: WebSocket, id: string | number, method: string, params: unknown): void {
    this.sendTo(client, { id, method, params })
  }

  sendTo(client: WebSocket, frame: Record<string, unknown>): void {
    const serialized = JSON.stringify(frame)
    this.sentFrames.push(frame)
    client.send(serialized)
  }

  private handleFrame(client: WebSocket, frame: ClientFrame): void {
    if (frame.method === 'initialize') {
      this.sendTo(client, {
        id: frame.id,
        result: {
          userAgent: `codex_cli_rs/${this.appServerVersion} (test)`,
          codexHome: '/tmp/codex-home',
          platformFamily: 'unix',
          platformOs: 'darwin',
        },
      })
      return
    }

    if (frame.method === 'initialized') {
      return
    }

    if (frame.method === 'thread/list') {
      this.sendTo(client, {
        id: frame.id,
        result: { data: [], nextCursor: null, backwardsCursor: null },
      })
      return
    }

    if (frame.id !== undefined && frame.method !== undefined) {
      this.sendTo(client, { id: frame.id, result: null })
    }
  }
}

const managers: AttachedAppServerManager[] = []
const servers: FakeDaemonServer[] = []
let tempDirectory: string

beforeEach(async () => {
  tempDirectory = await mkdtemp(path.join(tmpdir(), 'codex-desktop-attached-'))
})

afterEach(async () => {
  for (const manager of managers.splice(0)) {
    manager.dispose()
  }
  for (const server of servers.splice(0)) {
    await server.close()
  }
  await rm(tempDirectory, { force: true, recursive: true })
})

function socketPath(name: string): string {
  return path.join(tempDirectory, `${name}.sock`)
}

function createManager(
  server: FakeDaemonServer,
  options: {
    appServerVersion?: string
    reconnect?: { maxAttempts?: number; delayMs?: number; backoffFactor?: number }
  } = {},
): AttachedAppServerManager {
  const manager = new AttachedAppServerManager({
    socketPath: server.socketPath,
    clientInfo: { name: 'codex-desktop', title: 'Codex Desktop', version: '0.1.0' },
    limits: { requestTimeoutMs: 1_000 },
    reconnect: { maxAttempts: 5, delayMs: 20, backoffFactor: 1, ...options.reconnect },
  })
  managers.push(manager)
  return manager
}

async function createStartedServer(
  name: string,
  options: DaemonOptions = {},
): Promise<FakeDaemonServer> {
  const server = new FakeDaemonServer(socketPath(name), options)
  await server.start()
  servers.push(server)
  return server
}

function waitForStatus(
  manager: AttachedAppServerManager,
  state: AppServerStatus['state'],
  timeoutMs = 5_000,
): Promise<AppServerStatus> {
  return new Promise((resolve, reject) => {
    if (manager.status.state === state) {
      resolve(manager.status)
      return
    }
    const timer = setTimeout(() => {
      manager.onStatus(() => undefined)
      reject(new Error(`timed out waiting for ${state}; state was ${manager.status.state}`))
    }, timeoutMs)
    const unsubscribe = manager.onStatus((status) => {
      if (status.state === state) {
        clearTimeout(timer)
        unsubscribe()
        resolve(status)
      }
    })
  })
}

describe('AttachedAppServerManager', () => {
  it('attaches through initialize without creating a child-process owner', async () => {
    const server = await createStartedServer('attach')
    const manager = createManager(server)

    const status = await manager.start()

    expect(status).toMatchObject({ state: 'Ready', pid: null, pendingRequests: 0 })
    expect(manager.appServerVersion).toBe('0.72.0')
    expect(server.clients.size).toBe(1)
    await vi.waitFor(() => {
      expect(server.receivedFrames.map((frame) => frame.method)).toEqual([
        'initialize',
        'initialized',
      ])
    })

    const secondStart = await manager.start()
    expect(secondStart.state).toBe('Ready')
    expect(server.clients.size).toBe(1)
  })

  it('round-trips business RPC and forwards daemon notifications', async () => {
    const server = await createStartedServer('rpc')
    const manager = createManager(server)
    const events: AppServerNotificationEvent[] = []
    manager.onNotification((event) => {
      events.push(event)
    })
    await manager.start()

    await expect(manager.request('thread/list', { cursor: null, limit: 20 })).resolves.toEqual({
      data: [],
      nextCursor: null,
      backwardsCursor: null,
    })
    manager.notify('thread/started', { threadId: 'external-thread' })
    server.broadcast('turn/started', {
      threadId: 'external-thread',
      turnId: 'external-turn',
    })

    await vi.waitFor(() => {
      expect(events).toContainEqual({
        method: 'turn/started',
        params: { threadId: 'external-thread', turnId: 'external-turn' },
      })
    })
    expect(server.receivedFrames).toContainEqual({
      method: 'thread/started',
      params: { threadId: 'external-thread' },
    })
  })

  it('fails closed when the daemon socket is missing', async () => {
    const manager = new AttachedAppServerManager({
      socketPath: socketPath('missing'),
      clientInfo: { name: 'codex-desktop', title: 'Codex Desktop', version: '0.1.0' },
      reconnect: { maxAttempts: 1, delayMs: 10 },
    })
    managers.push(manager)

    const status = await manager.start()

    expect(status.state).toBe('Disconnected')
    expect(status.lastError).toMatchObject({
      kind: 'connectionClosed',
      message: expect.stringContaining('Daemon socket does not exist'),
      retryable: false,
    })
  })

  it('rejects a non-socket daemon path', async () => {
    const daemonPath = path.join(tempDirectory, 'not-a-socket')
    await writeFile(daemonPath, 'not a socket')
    const manager = new AttachedAppServerManager({
      socketPath: daemonPath,
      clientInfo: { name: 'codex-desktop', title: 'Codex Desktop', version: '0.1.0' },
    })
    managers.push(manager)

    const status = await manager.start()

    expect(status.state).toBe('Disconnected')
    expect(status.lastError).toMatchObject({
      kind: 'invalidRequest',
      message: expect.stringContaining('not a Unix socket'),
    })
    expect((await stat(daemonPath)).isFile()).toBe(true)
  })

  it('rejects a daemon with an incompatible major version and stays disconnected', async () => {
    const server = await createStartedServer('bad-version', { appServerVersion: '1.72.0' })
    const manager = createManager(server)

    const status = await manager.start()

    expect(status.state).toBe('Disconnected')
    expect(status.lastError).toMatchObject({
      kind: 'invalidRequest',
      message: expect.stringContaining('major version 1.72.0 is incompatible'),
    })
    expect(manager.appServerVersion).toBeNull()
    await vi.waitFor(() => {
      expect(server.clients.size).toBe(0)
    })
  })

  it('reattaches after daemon restart and only accepts events from the new connection', async () => {
    const server = await createStartedServer('restart')
    const manager = createManager(server)
    const events: string[] = []
    manager.onNotification((event) => {
      events.push(event.method as string)
    })
    await manager.start()
    server.broadcast('turn/started', { threadId: 'thread-before-restart' })
    await vi.waitFor(() => {
      expect(events).toEqual(['turn/started'])
    })

    await server.close()
    const reconnecting = await waitForStatus(manager, 'Reconnecting')
    expect(reconnecting.reconnectAttempt).toBe(1)

    await server.start()
    const ready = await waitForStatus(manager, 'Ready')
    expect(ready.reconnectAttempt).toBeNull()
    expect(server.clients.size).toBe(1)

    server.broadcast('thread/started', { threadId: 'thread-after-restart' })
    await vi.waitFor(() => {
      expect(events).toEqual(['turn/started', 'thread/started'])
    })
    await expect(manager.request('thread/list')).resolves.toMatchObject({ data: [] })
  })

  it('stops reconnecting after the bounded attempt count', async () => {
    const server = await createStartedServer('exhausted')
    const manager = createManager(server, {
      reconnect: { maxAttempts: 1, delayMs: 10 },
    })
    await manager.start()

    await server.close()
    await waitForStatus(manager, 'Reconnecting')
    await rm(server.socketPath, { force: true })
    await waitForStatus(manager, 'Disconnected')
    await vi.waitFor(() => {
      expect(manager.status.lastError).toMatchObject({
        kind: 'connectionClosed',
        message: expect.stringContaining('daemon reattach failed after 1 attempts'),
      })
    })
  })

  it('supports multi-client co-presence and late durable-state reconciliation', async () => {
    const server = await createStartedServer('multi-client')
    const first = createManager(server)
    const second = createManager(server)
    const firstEvents: AppServerNotificationEvent[] = []
    const secondEvents: AppServerNotificationEvent[] = []
    first.onNotification((event) => {
      firstEvents.push(event)
    })
    second.onNotification((event) => {
      secondEvents.push(event)
    })
    await first.start()
    await second.start()

    expect(server.clients.size).toBe(2)
    server.broadcast('thread/started', { threadId: 'shared-thread' })
    await vi.waitFor(() => {
      expect(firstEvents).toContainEqual({
        method: 'thread/started',
        params: { threadId: 'shared-thread' },
      })
      expect(secondEvents).toContainEqual({
        method: 'thread/started',
        params: { threadId: 'shared-thread' },
      })
    })
  })

  it('keeps approval ownership on the connection that received the server request', async () => {
    const server = await createStartedServer('approvals')
    const owner = createManager(server)
    const observer = createManager(server)
    const ownerEvents: AppServerNotificationEvent[] = []
    const observerEvents: AppServerNotificationEvent[] = []
    owner.onNotification((event) => {
      ownerEvents.push(event)
    })
    observer.onNotification((event) => {
      observerEvents.push(event)
    })
    await owner.start()
    await observer.start()

    const ownerSocket = [...server.clients][0]
    server.request(ownerSocket, 'approval-1', 'item/commandExecution/requestApproval', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'item-1',
      command: 'echo owned',
    })

    await vi.waitFor(() => {
      expect(ownerEvents).toContainEqual({
        id: 'approval-1',
        method: 'item/commandExecution/requestApproval',
        params: expect.objectContaining({ command: 'echo owned' }),
      })
    })
    expect(observerEvents).toEqual([])

    await expect(
      observer.respondToServerRequest('approval-1', { decision: 'accept' }),
    ).rejects.toMatchObject({ kind: 'invalidRequest' })
    await owner.respondToServerRequest('approval-1', { decision: 'accept' })

    await vi.waitFor(() => {
      expect(server.receivedFrames).toContainEqual({
        id: 'approval-1',
        result: { decision: 'accept' },
      })
    })
    server.broadcast('serverRequest/resolved', { requestId: 'approval-1' })
    await vi.waitFor(() => {
      expect(ownerEvents).toContainEqual({
        method: 'serverRequest/resolved',
        params: { requestId: 'approval-1' },
      })
      expect(observerEvents).toContainEqual({
        method: 'serverRequest/resolved',
        params: { requestId: 'approval-1' },
      })
    })
  })

  it('rejects lifecycle and business calls outside Ready', async () => {
    const manager = new AttachedAppServerManager({
      socketPath: socketPath('not-ready'),
      clientInfo: { name: 'codex-desktop', title: 'Codex Desktop', version: '0.1.0' },
    })
    managers.push(manager)

    await expect(manager.request('thread/list')).rejects.toMatchObject({ kind: 'notReady' })
    expect(() => manager.notify('thread/started')).toThrowError(/notifications require Ready state/)
    await expect(manager.respondToServerRequest('approval-1', {})).rejects.toMatchObject({
      kind: 'notReady',
    })
  })

  it('closes the attached connection on stop', async () => {
    const server = await createStartedServer('stop')
    const manager = createManager(server)
    await manager.start()
    await manager.stop()

    expect(manager.status.state).toBe('Disconnected')
    await vi.waitFor(() => {
      expect(server.clients.size).toBe(0)
    })
  })
})
