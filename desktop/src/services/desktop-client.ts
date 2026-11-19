// Typed DesktopClient: the product data boundary for the self-contained desktop
// build. Backed by Tauri commands (Rust), NOT by the upstream Express/WebSocket
// backend. U02 established the typed facade with fixture data; U03 wires the real
// Codex app-server adapter (engine.rs/bridge.rs). Commands fall back to fixtures
// when no engine is configured so the shell still boots. Engine notifications and
// server requests arrive over 'desktop-event' and are mapped here to the
// NormalizedMessage `kind` shapes the legacy chat handlers already understand.
// Call sites keep their existing shapes: command results are adapted to the fetch
// Response contract the legacy hooks expect.
//
// Modification mark (AGPL Section 7): new product file, not part of upstream
// ClaudeCodeUI. See desktop/SOURCE.md.

type TauriGlobal = {
  core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
  event: {
    listen: (
      event: string,
      handler: (e: { payload: unknown }) => void,
    ) => Promise<() => void>;
  };
};

const tauri = (): TauriGlobal | undefined =>
  (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;

export const isDesktopRuntime = (): boolean =>
  typeof window !== 'undefined' && '__TAURI__' in window;

// Browser fixture fallback (U02 mock): when the desktop build is previewed in a
// plain browser (no Tauri IPC), serve the same fixture shapes as the Rust mock so
// the workspace can be developed/verified without the native shell. Inside Tauri,
// __TAURI__ is present and real commands are used. This is NOT the Express backend.
const fixtureSession = () => ({
  id: 'session-welcome',
  title: 'Welcome to YourAgent',
  summary: 'Self-contained desktop session (U02 mock).',
  name: 'Welcome',
  createdAt: '2026-11-18T00:00:00Z',
  created_at: '2026-11-18T00:00:00Z',
  updated_at: '2026-11-18T00:00:00Z',
  lastActivity: '2026-11-18T00:00:00Z',
  messageCount: 0,
  __provider: 'codex',
  __projectName: 'superagent',
});
const fixtureProject = () => ({
  name: 'superagent',
  displayName: 'SuperAgent',
  fullPath: '/Users/fanjia/Desktop/code/codex',
  path: '/Users/fanjia/Desktop/code/codex',
  sessions: [fixtureSession()],
  codexSessions: [fixtureSession()],
  cursorSessions: [],
  geminiSessions: [],
  sessionMeta: { total: 1, hasMore: false },
  taskmaster: { hasTaskmaster: false },
});
const mockInvoke = (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
  const user = { username: (args?.username as string) || 'demo' };
  switch (cmd) {
    case 'bootstrap':
      return Promise.resolve({
        schemaVersion: 1, product: 'YourAgent', version: '0.1.0',
        user, token: 'desktop-mock-token', hasCompletedOnboarding: true,
        workspaces: [fixtureProject()], account: { connected: true }, pendingApprovals: 0,
      });
    case 'auth_status':
      return Promise.resolve({ needsSetup: false, isAuthenticated: true });
    case 'login':
    case 'register':
      return Promise.resolve({ token: 'desktop-mock-token', user });
    case 'auth_user':
      return Promise.resolve({ user });
    case 'onboarding_status':
      return Promise.resolve({ hasCompletedOnboarding: true });
    case 'git_config':
      return Promise.resolve({ gitName: 'YourAgent', gitEmail: 'agent@youragent.local' });
    case 'list_projects':
      return Promise.resolve([fixtureProject()]);
    case 'list_sessions':
      return Promise.resolve({ sessions: [fixtureSession()], total: 1, hasMore: false });
    case 'get_messages':
      return Promise.resolve([]);
    case 'send_input':
      return Promise.resolve({ accepted: true });
    case 'engine_connect':
      return Promise.resolve({ connected: false, reason: 'engine unavailable in browser preview' });
    case 'engine_status':
    case 'engine_disconnect':
      return Promise.resolve({ connected: false });
    case 'cancel_turn':
    case 'respond_to_request':
      return Promise.resolve({ ok: false, error: 'engine not connected' });
    case 'desktop_request': {
      const endpoint = (args?.endpoint as string) || '';
      const listLike = ['files', 'sessions', 'projects', 'templates', 'tasks', 'branches', 'changes'];
      return Promise.resolve(listLike.some((k) => endpoint.includes(k)) ? [] : {});
    }
    default:
      return Promise.resolve({});
  }
};

const invoke = <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
  const t = tauri();
  if (t) return t.core.invoke(cmd, args) as Promise<T>;
  return mockInvoke(cmd, args) as Promise<T>;
};

// Adapt a command result to the fetch Response contract used by legacy call sites.
export const jsonResponse = (data: unknown, status = 200): Response =>
  new Response(JSON.stringify(data ?? null), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });

export interface DesktopBootstrap {
  schemaVersion: number;
  product: string;
  version: string;
  user: { username: string };
  token: string;
  hasCompletedOnboarding: boolean;
}

// ─── U03 engine event mapping ────────────────────────────────────────────────
// The Rust adapter forwards app-server traffic verbatim as
// {kind:"notification",method,params} and {kind:"server_request",requestId,method,params}.
// Map those to the NormalizedMessage `kind` shapes useChatRealtimeHandlers expects.
// Tool cards are emitted on item/completed only (the store appends without dedup,
// so emitting started+completed would duplicate). agentMessage deltas stream and
// the completed snapshot finalizes the stream rather than appending a second copy.

const tsFrom = (ms?: number): string =>
  typeof ms === 'number' ? new Date(ms).toISOString() : new Date().toISOString();

const textOfInput = (content: unknown): string => {
  if (!Array.isArray(content)) return typeof content === 'string' ? content : '';
  return content
    .map((c) => (c && typeof c === 'object' && 'text' in c ? String((c as { text?: unknown }).text ?? '') : ''))
    .join('');
};

// Mirror bridge.rs item_to_message for completed items (live event path).
const mapCompletedItem = (item: Record<string, any>, threadId: string, tsMs?: number): Record<string, any> | null => {
  const base = {
    id: String(item.id ?? `${item.type}-${tsMs ?? Date.now()}`),
    sessionId: threadId,
    timestamp: tsFrom(tsMs),
    provider: 'codex',
  };
  switch (item.type) {
    case 'agentMessage':
      // Finalize the buffered stream; do not append a duplicate text message.
      return { ...base, kind: 'stream_end' };
    case 'userMessage':
      return { ...base, kind: 'text', role: 'user', content: textOfInput(item.content) };
    case 'reasoning': {
      const summary = Array.isArray(item.summary) ? item.summary.map((s: any) => s?.text ?? '').join('\n') : '';
      return { ...base, kind: 'thinking', text: summary };
    }
    case 'plan':
      return { ...base, kind: 'text', role: 'assistant', content: String(item.text ?? '') };
    case 'commandExecution':
      return {
        ...base, kind: 'tool_use', toolName: 'commandExecution', toolId: base.id,
        toolInput: { command: item.command, cwd: item.cwd },
        toolResult: { content: String(item.aggregatedOutput ?? ''), isError: item.exitCode !== 0 },
        exitCode: item.exitCode,
      };
    case 'fileChange':
    case 'mcpToolCall':
    case 'functionCallOutput':
      return { ...base, kind: 'tool_use', toolName: String(item.type), toolId: base.id, toolInput: item };
    default:
      return null;
  }
};

const approvalToolName = (method: string): string => {
  const seg = method.split('/')[1];
  return seg || method;
};

export const mapEngineEvent = (payload: unknown): Record<string, any> | null => {
  const p = payload as Record<string, any> | null;
  if (!p || typeof p !== 'object') return null;

  if (p.kind === 'server_request') {
    const method = String(p.method ?? '');
    const params = (p.params ?? {}) as Record<string, any>;
    if (method.endsWith('requestApproval')) {
      return {
        id: `req-${p.requestId}`,
        kind: 'permission_request',
        sessionId: params.threadId ?? null,
        provider: 'codex',
        timestamp: tsFrom(),
        requestId: p.requestId,
        toolName: approvalToolName(method),
        input: params,
        context: params,
      };
    }
    // Other server requests (elicitation/userInput/auth refresh) are not mapped
    // to chat kinds yet; never auto-answer them.
    return null;
  }

  if (p.kind === 'notification') {
    const method = String(p.method ?? '');
    const params = (p.params ?? {}) as Record<string, any>;
    const threadId = String(params.threadId ?? params.thread?.id ?? '');
    const base = { sessionId: threadId, provider: 'codex', timestamp: tsFrom(params.completedAtMs ?? params.startedAtMs) };
    switch (method) {
      case 'thread/started':
        return { ...base, kind: 'session_created', id: `thread-${threadId}`, sessionId: threadId, newSessionId: threadId };
      case 'item/agentMessage/delta':
        return { ...base, kind: 'stream_delta', id: String(params.itemId ?? 'stream'), content: String(params.delta ?? '') };
      case 'item/reasoning/textDelta':
      case 'item/reasoning/summaryTextDelta':
        return { ...base, kind: 'thinking', id: String(params.itemId ?? 'reasoning'), text: String(params.delta ?? '') };
      case 'item/completed':
        return params.item ? mapCompletedItem(params.item, threadId, params.completedAtMs ?? params.startedAtMs) : null;
      case 'turn/completed':
        return { ...base, kind: 'complete', id: `turn-${params.turn?.id ?? threadId}`, exitCode: 0 };
      case 'thread/status/changed':
        return { ...base, kind: 'status', id: `status-${threadId}`, status: params.status };
      default:
        return null;
    }
  }

  // Non-engine payloads pass through unchanged.
  return p;
};

export const desktopClient = {
  bootstrap: () => invoke<DesktopBootstrap>('bootstrap'),
  authStatus: () => invoke<{ needsSetup: boolean; isAuthenticated: boolean }>('auth_status'),
  login: (username: string, password: string) =>
    invoke<{ token: string; user: { username: string } }>('login', { username, password }),
  register: (username: string, password: string) =>
    invoke<{ token: string; user: { username: string } }>('register', { username, password }),
  authUser: () => invoke<{ user: { username: string } }>('auth_user'),
  logout: () => invoke<void>('logout'),
  onboardingStatus: () => invoke<{ hasCompletedOnboarding: boolean }>('onboarding_status'),
  completeOnboarding: () => invoke<void>('complete_onboarding'),
  gitConfig: () => invoke<{ gitName: string; gitEmail: string }>('git_config'),
  updateGitConfig: (gitName: string, gitEmail: string) =>
    invoke<void>('update_git_config', { gitName, gitEmail }),
  listProjects: () => invoke<unknown[]>('list_projects'),
  listSessions: (projectName: string, limit: number, offset: number) =>
    invoke<unknown>('list_sessions', { projectName, limit, offset }),
  getMessages: (sessionId: string, opts: Record<string, unknown>) =>
    invoke<unknown>('get_messages', { sessionId, ...opts }),
  sendInput: (message: unknown) => invoke<void>('send_input', { message }),
  // U03 engine lifecycle + explicit approval/cancel actions.
  engineConnect: () => invoke<{ connected: boolean; info?: unknown; reason?: string }>('engine_connect'),
  engineStatus: () => invoke<{ connected: boolean; info?: unknown }>('engine_status'),
  engineDisconnect: () => invoke<{ connected: boolean }>('engine_disconnect'),
  cancelTurn: (threadId: string, turnId: string) =>
    invoke<{ ok: boolean; error?: string }>('cancel_turn', { threadId, turnId }),
  respondToRequest: (requestId: unknown, result: unknown) =>
    invoke<{ ok: boolean; error?: string }>('respond_to_request', { requestId, result }),
  // Generic product request for endpoints not yet migrated to typed commands.
  // U02 mock: Rust returns an empty/graceful payload so unmigrated features
  // degrade rather than crash. Replaced by typed commands + real adapter in U03+.
  request: (method: string, endpoint: string, body?: unknown) =>
    invoke<unknown>('desktop_request', { method, endpoint, body: body ?? null }),
  subscribe: (handler: (payload: unknown) => void): Promise<() => void> => {
    const t = tauri();
    if (t) {
      return t.event.listen('desktop-event', (e) => {
        const mapped = mapEngineEvent(e.payload);
        if (mapped) handler(mapped);
      });
    }
    return Promise.resolve(() => {});
  },
};