import { spawn, type ChildProcess } from "node:child_process"
import { createServer, type IncomingMessage, type ServerResponse } from "node:http"
import { writeFileSync, mkdirSync, readFileSync, existsSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

// ---------- types ----------

interface JsonRpcRequest {
  jsonrpc: "2.0"
  id: number
  method: string
  params?: unknown
}

interface JsonRpcResponse {
  jsonrpc: "2.0"
  id: number
  result?: unknown
  error?: { code: number; message: string; data?: unknown }
}

interface JsonRpcNotification {
  jsonrpc: "2.0"
  method: string
  params?: unknown
}

interface ServerRequestFrame {
  jsonrpc: "2.0"
  id: string | number
  method: string
  params?: unknown
}

interface ServiceInfo {
  id: string
  version: string
  url: string
  pid: number
  password: string
}

// ---------- codex app-server bridge ----------

import { existsSync as existsSyncSync } from "node:fs"
import { dirname as pathDirname, join as pathJoin } from "node:path"
import { fileURLToPath } from "node:url"

function resolveCodexCommand(): string {
  // 1. Explicit env var takes highest priority
  if (process.env.CODEX_APP_SERVER_COMMAND) return process.env.CODEX_APP_SERVER_COMMAND

  // 2. Look for codex in the same directory as this adapter binary
  //    (covers staged copies inside OpenCode desktop's userData)
  const here = dirnameOfThisBinary()
  if (here) {
    const localCodex = pathJoin(here, "codex")
    if (existsSyncSync(localCodex)) return localCodex
  }

  // 3. Fallback to PATH lookup
  return "codex"
}

function dirnameOfThisBinary(): string | undefined {
  try {
    // When compiled with bun --compile, __dirname points to the binary's location
    // When running from source, import.meta.url gives the script path
    const url = typeof globalThis.__dirname !== "undefined"
      ? `file://${globalThis.__dirname}`
      : import.meta.url
    return pathDirname(fileURLToPath(url))
  } catch {
    return undefined
  }
}

function resolveWorkspace(): string {
  // The desktop sets CODEX_WORKSPACE to the project directory before spawning
  // the adapter. Without it, the adapter would report its own staging directory.
  return process.env.CODEX_WORKSPACE ?? process.cwd()
}

class CodexBridge {
  private child: ChildProcess | null = null
  private buffer = ""
  private nextId = 1
  private readonly pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: NodeJS.Timeout }
  >()
  private readonly sseClients = new Set<{
    res: ServerResponse
    directory: string | null
  }>()
  private ready = false
  private readonly readyWaiters: Array<() => void> = []
  private readonly serverRequests = new Map<string | number, ServerRequestFrame>()

  async start(): Promise<void> {
    const command = resolveCodexCommand()
    const args = process.env.CODEX_APP_SERVER_ARGS?.split(" ") ?? ["app-server"]

    this.child = spawn(command, args, {
      stdio: ["pipe", "pipe", "inherit"],
      env: { ...process.env },
    })
    if (!this.child.stdout || !this.child.stdin) {
      throw new Error("failed to open codex app-server stdio")
    }
    this.child.stdout.setEncoding("utf8")
    this.child.stdout.on("data", (chunk: string) => this.onData(chunk))
    this.child.on("exit", (code) => {
      const err = new Error(`codex app-server exited with code ${code}`)
      for (const [, p] of this.pending) {
        clearTimeout(p.timer)
        p.reject(err)
      }
      this.pending.clear()
      this.broadcast("server.disposed", {})
    })
    await this.initialize()
  }

  private async initialize(): Promise<void> {
    await this.request("initialize", {
      clientInfo: { name: "codex-opencode-adapter", title: "Codex OpenCode Adapter", version: "0.1.0" },
      capabilities: { experimentalApi: true },
    })
    this.ready = true
    for (const w of this.readyWaiters) w()
    this.readyWaiters.length = 0
  }

  async whenReady(): Promise<void> {
    if (this.ready) return
    await new Promise<void>((resolve) => this.readyWaiters.push(resolve))
  }

  private onData(chunk: string): void {
    this.buffer += chunk
    for (;;) {
      const idx = this.buffer.indexOf("\n")
      if (idx === -1) return
      const line = this.buffer.slice(0, idx).trim()
      this.buffer = this.buffer.slice(idx + 1)
      if (!line) continue
      let frame: JsonRpcResponse | JsonRpcNotification | ServerRequestFrame
      try {
        frame = JSON.parse(line)
      } catch {
        continue
      }
      this.onFrame(frame)
    }
  }

  private onFrame(frame: JsonRpcResponse | JsonRpcNotification | ServerRequestFrame): void {
    if ("id" in frame && "method" in frame) {
      const req = frame as ServerRequestFrame
      this.serverRequests.set(req.id, req)
      this.broadcastServerRequest(req)
      return
    }
    if ("id" in frame) {
      const res = frame as JsonRpcResponse
      const p = this.pending.get(res.id)
      if (!p) return
      this.pending.delete(res.id)
      clearTimeout(p.timer)
      if (res.error) {
        p.reject(new Error(res.error.message))
      } else {
        p.resolve(res.result)
      }
      return
    }
    const n = frame as JsonRpcNotification
    this.onNotification(n.method, n.params)
  }

  private onNotification(method: string, params: unknown): void {
    const p = (params ?? {}) as Record<string, unknown>

    if (method === "serverRequest/resolved") {
      const id = p.id as string | number
      this.serverRequests.delete(id)
      this.broadcast("permission.replied", {
        sessionID: "",
        requestID: String(id),
        reply: "once",
      })
      return
    }

    if (method === "thread/created" || method === "thread/updated") {
      const thread = p.thread as Record<string, unknown> | undefined
      if (thread) {
        this.broadcast(
          method === "thread/created" ? "session.created" : "session.updated",
          { sessionID: thread.id as string, info: mapThreadToSession(thread) },
          typeof p.directory === "string" ? p.directory : undefined,
        )
      }
      return
    }

    if (method.startsWith("item/")) {
      const item = p.item as Record<string, unknown> | undefined
      if (!item) return
      if (method === "item/started" || method === "item/completed") {
        this.broadcast(
          "message.part.updated",
          { part: mapItemToPart(item) },
          typeof p.directory === "string" ? p.directory : undefined,
        )
      }
      return
    }

    if (method === "turn/started") {
      this.broadcast("session.status", { sessionID: p.threadId ?? "", status: { type: "busy" } })
      return
    }
    if (method === "turn/completed") {
      this.broadcast("session.status", { sessionID: p.threadId ?? "", status: { type: "idle" } })
      this.broadcast("session.idle", { sessionID: p.threadId ?? "" })
      return
    }
  }

  private broadcastServerRequest(req: ServerRequestFrame): void {
    const params = (req.params ?? {}) as Record<string, unknown>
    const perm = {
      id: String(req.id),
      sessionID: (params.threadId as string) ?? "",
      permission: req.method,
      patterns: [] as string[],
      metadata: params,
      always: [] as string[],
      tool: undefined,
    }
    this.broadcast("permission.asked", perm, typeof params.directory === "string" ? params.directory : undefined)
  }

  respondToServerRequest(id: string | number, result: unknown): void {
    this.serverRequests.delete(id)
    this.send({ jsonrpc: "2.0", id, result } as JsonRpcResponse & { method?: never })
  }

  request(method: string, params?: unknown): Promise<unknown> {
    const id = this.nextId++
    const frame: JsonRpcRequest = { jsonrpc: "2.0", id, method, params }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`codex app-server request ${method} timed out`))
      }, 60_000)
      this.pending.set(id, { resolve, reject, timer })
      this.send(frame)
    })
  }

  private send(frame: JsonRpcRequest | JsonRpcResponse): void {
    if (!this.child?.stdin) throw new Error("codex app-server not started")
    this.child.stdin.write(JSON.stringify(frame) + "\n")
  }

  addSseClient(client: { res: ServerResponse; directory: string | null }): void {
    this.sseClients.add(client)
  }

  removeSseClient(client: { res: ServerResponse; directory: string | null }): void {
    this.sseClients.delete(client)
  }

  private broadcast(type: string, properties: unknown, directory?: string): void {
    // OpenCode v2 SSE format: plain JSON objects, one per line, no "data:" prefix.
    // The client expects: {"id": ..., "type": "server.connected"|..., "data": {...}}
    const event = {
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      type,
      data: properties ?? {},
      ...(directory ? { directory } : {}),
    }
    const data = JSON.stringify(event) + "\n"
    for (const client of this.sseClients) {
      if (client.directory && directory && client.directory !== directory) continue
      try {
        client.res.write(data)
      } catch {
        this.sseClients.delete(client)
      }
    }
  }

  async stop(): Promise<void> {
    for (const client of this.sseClients) {
      try {
        client.res.end()
      } catch {}
    }
    this.sseClients.clear()
    this.child?.kill("SIGTERM")
    this.child = null
  }
}

// ---------- mapping helpers ----------

function mapThreadToSession(thread: Record<string, unknown>, directory?: string) {
  const now = Date.now()
  const created = typeof thread.createdAt === "number" ? thread.createdAt * 1000 : now
  const updated = typeof thread.updatedAt === "number" ? thread.updatedAt * 1000 : now
  const parentThreadId = (thread.parentThreadId as string | null) ?? null
  return {
    id: thread.id as string,
    slug: thread.id as string,
    projectID: (thread.projectId as string) ?? (thread.cwd as string) ?? resolveWorkspace(),
    directory: directory ?? (thread.cwd as string) ?? resolveWorkspace(),
    title: (thread.name as string) ?? (thread.preview as string) ?? "Untitled",
    version: "0.1.0",
    time: {
      created,
      updated,
      idle: updated,
    },
    cost: 0,
    tokens: {
      input: 0,
      output: 0,
      reasoning: 0,
      cache: { read: 0, write: 0 },
    },
    ...(parentThreadId ? { parentID: parentThreadId } : {}),
  }
}

function mapItemToPart(item: Record<string, unknown>) {
  const now = Date.now()
  return {
    id: (item.id as string) ?? String(now),
    sessionID: (item.threadId as string) ?? "",
    messageID: (item.turnId as string) ?? "",
    type: "text",
    text: typeof item.text === "string" ? item.text : JSON.stringify(item),
    time: { start: now },
  }
}

// ---------- service registration ----------

function getServiceFilePath(): string {
  const stateDir = process.env.XDG_STATE_HOME ?? join(homedir(), ".local", "state")
  return join(stateDir, "opencode", "service.json")
}

function writeServiceFile(info: ServiceInfo): void {
  const file = getServiceFilePath()
  mkdirSync(join(file, ".."), { recursive: true })
  writeFileSync(file, JSON.stringify(info, null, 2), { mode: 0o600 })
}

// ---------- HTTP server ----------

function authorized(req: IncomingMessage, password: string): boolean {
  const header = req.headers.authorization
  if (!header) return false
  const token = header.replace(/^Basic\s+/i, "")
  try {
    const decoded = Buffer.from(token, "base64").toString("utf8")
    return decoded.endsWith(`:${password}`)
  } catch {
    return false
  }
}

function sendJson(res: ServerResponse, status: number, body: unknown, password: string, req: IncomingMessage) {
  if (!authorized(req, password)) {
    res.writeHead(401, { "content-type": "application/json" })
    res.end(JSON.stringify({ error: "unauthorized" }))
    return
  }
  const text = body === undefined ? "" : JSON.stringify(body)
  res.writeHead(status, {
    "content-type": "application/json",
    "access-control-allow-origin": "oc://renderer",
    "access-control-allow-credentials": "true",
  })
  res.end(text)
}

async function readBody(req: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = []
  for await (const chunk of req) chunks.push(chunk as Buffer)
  if (chunks.length === 0) return {}
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"))
  } catch {
    return {}
  }
}

const bridge = new CodexBridge()
let servicePassword = ""

async function handleRequest(req: IncomingMessage, res: ServerResponse, password: string) {
  const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "localhost"}`)
  const path = url.pathname

  if (req.method === "OPTIONS") {
    res.writeHead(204, {
      "access-control-allow-origin": "oc://renderer",
      "access-control-allow-credentials": "true",
      "access-control-allow-methods": "GET,POST,PUT,PATCH,DELETE,OPTIONS",
      "access-control-allow-headers": "authorization,content-type",
    })
    res.end()
    return
  }

  // Service info (required by OpenCode desktop discovery)
  if (req.method === "GET" && path === "/api/info") {
    sendJson(res, 200, {
      version: process.env.OPENCODE_SERVICE_VERSION ?? "0.0.0-dev-20978",
      pid: process.pid,
      urls: [`http://${req.headers.host ?? "localhost"}`],
      paths: { tmp: "/tmp" },
    }, password, req)
    return
  }

  // Health
  if (req.method === "GET" && path === "/global/health") {
    sendJson(res, 200, { healthy: true, version: "0.1.0" }, password, req)
    return
  }

  // Config
  if (req.method === "GET" && path === "/global/config") {
    sendJson(res, 200, {}, password, req)
    return
  }

  // Project
  if (req.method === "GET" && path === "/project") {
    try {
      const result = (await bridge.request("project/list", {})) as { data?: Array<Record<string, unknown>> }
      let projects = (result?.data ?? []).map((p) => {
        // codex app-server returns projects with roots[] containing path.
        const root = (p.roots as Array<Record<string, unknown>>)?.[0]
        const worktree = (root?.path as string) ?? (p.path as string) ?? (p.directory as string) ?? resolveWorkspace()
        return {
          id: (p.id as string) ?? "unknown",
          name: (p.name as string) ?? "unknown",
          canonical: worktree,
          worktree,
          sandboxes: [],
          time: {
            created: typeof p.createdAt === "number" ? p.createdAt * 1000 : Date.now(),
            updated: typeof p.updatedAt === "number" ? p.updatedAt * 1000 : Date.now(),
            active: typeof p.updatedAt === "number" ? p.updatedAt * 1000 : Date.now(),
            initialized: typeof p.createdAt === "number" ? p.createdAt * 1000 : Date.now(),
          },
        }
      })
      // If no projects are registered in the app-server, fall back to the
      // workspace directory so the desktop can still create sessions there.
      if (projects.length === 0) {
        const workspace = process.env.CODEX_WORKSPACE ?? resolveWorkspace()
        projects = [{
          id: workspace,
          name: workspace.split("/").pop() ?? workspace,
          canonical: workspace,
          worktree: workspace,
          sandboxes: [],
          time: { created: Date.now(), updated: Date.now(), initialized: Date.now() },
        }]
      }
      sendJson(res, 200, projects, password, req)
    } catch {
      sendJson(res, 200, [{ id: "codex", worktree: resolveWorkspace(), sandboxes: [], time: { created: Date.now(), updated: Date.now(), initialized: Date.now() } }], password, req)
    }
    return
  }
  if (req.method === "GET" && path === "/project/current") {
    const directory = url.searchParams.get("directory") ?? resolveWorkspace()
    try {
      const result = (await bridge.request("project/list", {})) as { data?: Array<Record<string, unknown>> }
      const match = (result?.data ?? []).find((p) => {
        const pPath = (p.path as string) ?? (p.directory as string) ?? ""
        return pPath === directory || directory.startsWith(pPath + "/")
      })
      sendJson(res, 200, match ? {
        id: (match.id as string) ?? "codex",
        worktree: (match.path as string) ?? directory,
        sandboxes: [],
        time: { created: Date.now(), updated: Date.now(), initialized: Date.now() },
      } : {
        id: "codex",
        worktree: directory,
        sandboxes: [],
        time: { created: Date.now(), updated: Date.now(), initialized: Date.now() },
      }, password, req)
    } catch {
      sendJson(res, 200, { id: "codex", worktree: directory, sandboxes: [], time: { created: Date.now(), updated: Date.now(), initialized: Date.now() } }, password, req)
    }
    return
  }

  // Path
  if (req.method === "GET" && path === "/path") {
    const home = homedir()
    const directory = url.searchParams.get("directory") ?? resolveWorkspace()
    sendJson(res, 200, {
      home,
      state: join(home, ".codex"),
      config: join(home, ".codex"),
      worktree: directory,
      directory,
    }, password, req)
    return
  }

  // Provider
  if (req.method === "GET" && path === "/api/provider") {
    sendJson(res, 200, { location: { directory: url.searchParams.get("directory") ?? resolveWorkspace() }, data: [{ id: "codex", name: "Codex", activation: "enabled", package: "codex" }] }, password, req)
    return
  }

  // Model
  if (req.method === "GET" && path === "/api/model") {
    try {
      const result = (await bridge.request("model/list", {})) as { data?: Array<Record<string, unknown>> }
      const models = (result?.data ?? []).map((m) => ({
        id: m.id as string,
        modelID: m.id as string,
        providerID: "codex",
        name: (m.displayName as string) ?? (m.id as string),
        capabilities: {
          input: ["text"],
          output: ["text"],
          tools: true,
        },
        variants: [],
        time: { released: Date.now() },
        cost: [],
        status: "active",
        enabled: true,
        limit: { context: 128000, output: 4096 },
      }))
      sendJson(res, 200, { location: { directory: url.searchParams.get("directory") ?? resolveWorkspace() }, data: models }, password, req)
    } catch (e) {
      sendJson(res, 200, { location: { directory: url.searchParams.get("directory") ?? resolveWorkspace() }, data: [] }, password, req)
    }
    return
  }
  if (req.method === "GET" && path === "/api/model/default") {
    try {
      const result = (await bridge.request("model/list", {})) as { data?: Array<Record<string, unknown>> }
      const first = (result?.data ?? [])[0]
      sendJson(res, 200, {
        location: { directory: url.searchParams.get("directory") ?? resolveWorkspace() },
        data: first ? {
          id: first.id as string,
          modelID: first.id as string,
          providerID: "codex",
          name: (first.displayName as string) ?? (first.id as string),
          capabilities: {
            input: ["text"],
            output: ["text"],
            tools: true,
          },
          variants: [],
          time: { released: Date.now() },
          cost: [],
          status: "active",
          enabled: true,
          limit: { context: 128000, output: 4096 },
        } : null,
      }, password, req)
    } catch (e) {
      sendJson(res, 200, { location: { directory: url.searchParams.get("directory") ?? resolveWorkspace() }, data: null }, password, req)
    }
    return
  }

  // Agent
  if (req.method === "GET" && path === "/agent") {
    sendJson(res, 200, [{ name: "codex", description: "Codex coding agent", mode: "primary", native: true, permission: [], options: {} }], password, req)
    return
  }

  // Command
  if (req.method === "GET" && path === "/command") {
    sendJson(res, 200, [], password, req)
    return
  }

  // Permission
  if (req.method === "GET" && path === "/permission") {
    sendJson(res, 200, [], password, req)
    return
  }
  const permissionReply = /^\/permission\/([^/]+)\/reply$/.exec(path)
  if (req.method === "POST" && permissionReply) {
    const body = await readBody(req)
    bridge.respondToServerRequest(permissionReply[1], { decision: body.reply ?? "once" })
    sendJson(res, 200, true, password, req)
    return
  }

  // Question
  if (req.method === "GET" && path === "/question") {
    sendJson(res, 200, [], password, req)
    return
  }
  const questionReply = /^\/question\/([^/]+)\/reply$/.exec(path)
  if (req.method === "POST" && questionReply) {
    const body = await readBody(req)
    bridge.respondToServerRequest(questionReply[1], { answers: body.answers ?? [] })
    sendJson(res, 200, true, password, req)
    return
  }

  // Sessions
  if (req.method === "GET" && path === "/session") {
    const directory = url.searchParams.get("directory") ?? resolveWorkspace()
    const result = (await bridge.request("thread/list", {})) as { data?: Array<Record<string, unknown>> }
    const sessions = (result?.data ?? [])
      .filter((thread) => {
        const cwd = thread.cwd as string | undefined
        if (!cwd) return true
        return cwd === directory || cwd.startsWith(directory + "/") || directory.startsWith(cwd + "/")
      })
      .map((thread) => mapThreadToSession(thread, directory))
    sendJson(res, 200, sessions, password, req)
    return
  }
  if (req.method === "POST" && path === "/session") {
    const body = await readBody(req)
    const result = (await bridge.request("thread/start", {
      cwd: body.directory ?? resolveWorkspace(),
    })) as { thread?: Record<string, unknown> }
    const thread = result?.thread ?? { id: String(Date.now()) }
    sendJson(res, 200, mapThreadToSession(thread), password, req)
    return
  }
  if (req.method === "GET" && path === "/session/status") {
    sendJson(res, 200, {}, password, req)
    return
  }
  const sessionGet = /^\/session\/([^/]+)$/.exec(path)
  if (req.method === "GET" && sessionGet) {
    const result = (await bridge.request("thread/read", { threadId: sessionGet[1] })) as {
      thread?: Record<string, unknown>
    }
    sendJson(res, 200, mapThreadToSession(result?.thread ?? { id: sessionGet[1] }), password, req)
    return
  }
  const sessionMessages = /^\/session\/([^/]+)\/message$/.exec(path)
  if (req.method === "GET" && sessionMessages) {
    const result = (await bridge.request("thread/read", { threadId: sessionMessages[1] })) as {
      thread?: { turns?: Array<Record<string, unknown>> }
    }
    const messages: unknown[] = []
    for (const turn of result?.thread?.turns ?? []) {
      for (const item of (turn.items as Array<Record<string, unknown>>) ?? []) {
        messages.push({
          id: item.id as string,
          sessionID: sessionMessages[1],
          role: (item.type as string) === "userMessage" ? "user" : "assistant",
          time: { created: Date.now() },
          agent: "codex",
          model: { providerID: "codex", modelID: "codex" },
        })
      }
    }
    sendJson(res, 200, messages, password, req)
    return
  }
  const sessionPrompt = /^\/session\/([^/]+)\/prompt_async$/.exec(path)
  if (req.method === "POST" && sessionPrompt) {
    const body = await readBody(req)
    const parts = (body.parts as Array<Record<string, unknown>>) ?? []
    const text = parts.filter((p) => p.type === "text").map((p) => p.text).join("\n")
    await bridge.request("turn/start", { threadId: sessionPrompt[1], input: text })
    sendJson(res, 204, undefined, password, req)
    return
  }
  const sessionAbort = /^\/session\/([^/]+)\/abort$/.exec(path)
  if (req.method === "POST" && sessionAbort) {
    await bridge.request("turn/interrupt", { threadId: sessionAbort[1] }).catch(() => undefined)
    sendJson(res, 200, true, password, req)
    return
  }

  // SSE events
  if (req.method === "GET" && (path === "/api/event" || path === "/global/event" || path === "/event")) {
    if (!authorized(req, password)) {
      res.writeHead(401)
      res.end()
      return
    }
    console.log("[adapter] SSE connected:", path)
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      "Connection": "keep-alive",
      "Access-Control-Allow-Origin": "oc://renderer",
      "Access-Control-Allow-Credentials": "true",
    })
    const client = { res, directory: url.searchParams.get("directory") }
    bridge.addSseClient(client)
    res.write(`data: ${JSON.stringify({ id: "0", type: "server.connected", data: {} })}\n\n`)
    const keepalive = setInterval(() => {
      try {
        res.write(`: keepalive\n\n`)
      } catch {
        clearInterval(keepalive)
      }
    }, 15_000)
    req.on("close", () => {
      console.log("[adapter] SSE disconnected:", path)
      clearInterval(keepalive)
      bridge.removeSseClient(client)
    })
    return
  }

  // Fallback
  if (req.method === "GET") {
    sendJson(res, 200, path.includes("/session/") ? {} : [], password, req)
    return
  }
  sendJson(res, 200, true, password, req)
}

// ---------- main ----------

async function main() {
  const port = Number(process.env.PORT ?? 0)
  const hostname = process.env.HOSTNAME ?? "127.0.0.1"

  // When spawned by OpenCode desktop, SIGTERM from the parent may propagate
  // to us. We must survive so the desktop can reconnect after reconnecting.
  // The desktop is responsible for our lifecycle; ignore SIGTERM/SIGINT.
  process.on("SIGTERM", () => {
    // Only shut down when the parent desktop is actually gone.
    // This is a no-op check; we rely on the desktop to kill us explicitly.
  })
  process.on("SIGINT", () => {
    void shutdown(server)
  })

  await bridge.start()

  const server = createServer((req, res) => {
    void handleRequest(req, res, servicePassword).catch((error) => {
      console.error("[adapter] request failed:", error)
      res.writeHead(500, { "content-type": "application/json" })
      res.end(JSON.stringify({ error: String(error) }))
    })
  })

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, hostname, () => resolve())
  })

  const address = server.address()
  if (!address || typeof address === "string") throw new Error("failed to get server address")

  const url = `http://${address.address === "::" ? "127.0.0.1" : address.address}:${address.port}`
  servicePassword = process.env.OPENCODE_SERVICE_PASSWORD ?? crypto.randomUUID()

  const info: ServiceInfo = {
    id: crypto.randomUUID(),
    version: process.env.OPENCODE_SERVICE_VERSION ?? "0.0.0-dev-20978",
    url,
    pid: process.pid,
    password: servicePassword,
  }

  writeServiceFile(info)
  console.log(`[adapter] service registered at ${url}`)
  console.log(`[adapter] service file: ${getServiceFilePath()}`)

  // Keep the process alive until explicitly terminated.
  // The service file tells OpenCode desktop where to connect;
  // exiting early causes the desktop to respawn and reconnect in a loop.
  // A pending promise keeps the Node.js event loop alive indefinitely.
  await new Promise(() => {})
}

async function shutdown(server: ReturnType<typeof createServer>) {
  await bridge.stop()
  server.close(() => process.exit(0))
  setTimeout(() => process.exit(1), 5000)
}

main().catch((error) => {
  console.error("[adapter] fatal:", error)
  process.exit(1)
})
