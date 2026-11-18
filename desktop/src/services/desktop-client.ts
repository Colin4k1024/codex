// Typed DesktopClient: the product data boundary for the self-contained desktop
// build. Backed by Tauri commands (Rust), NOT by the upstream Express/WebSocket
// backend. This is the U02 mock facade from HARNESS_CLAUDECODEUI_MIGRATION.md;
// the Rust commands currently return fixture data and are replaced by the real
// Codex app-server adapter in U03. Call sites keep their existing shapes: command
// results are adapted to the fetch Response contract the legacy hooks expect.
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
  // Generic product request for endpoints not yet migrated to typed commands.
  // U02 mock: Rust returns an empty/graceful payload so unmigrated features
  // degrade rather than crash. Replaced by typed commands + real adapter in U03+.
  request: (method: string, endpoint: string, body?: unknown) =>
    invoke<unknown>('desktop_request', { method, endpoint, body: body ?? null }),
  subscribe: (handler: (payload: unknown) => void): Promise<() => void> => {
    const t = tauri();
    if (t) return t.event.listen('desktop-event', (e) => handler(e.payload));
    return Promise.resolve(() => {});
  },
};