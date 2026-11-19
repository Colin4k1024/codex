import { desktopClient, jsonResponse } from "../services/desktop-client";

// Product modification (AGPL Section 7 — see desktop/SOURCE.md): the data layer goes
// through the typed DesktopClient (Rust commands), NOT the upstream Express backend.
// U06 removed the Web fetch/JWT fallback: there is no localhost service in the desktop
// product. Boot-critical endpoints use typed commands; unmigrated endpoints use a
// generic product request (desktop_request) that returns graceful empty payloads. The
// '/api/...' strings below are product-request endpoint identifiers, not network URLs.

const parseBody = (options) => {
  if (!options || options.body == null) return null;
  if (typeof options.body === 'string') {
    try { return JSON.parse(options.body); } catch { return options.body; }
  }
  return null; // FormData etc. not supported over the desktop mock boundary
};

// Authenticated product request — always over Tauri IPC (no fetch, no Bearer JWT).
export const authenticatedFetch = (url, options = {}) =>
  desktopClient
    .request(options.method || 'GET', url, parseBody(options))
    .then((data) => jsonResponse(data));

const desktopJson = (promise) => promise.then((data) => jsonResponse(data));

// API endpoints
export const api = {
  // Auth endpoints (provisioned by the Rust DesktopClient)
  auth: {
    status: () => desktopJson(desktopClient.authStatus()),
    login: (username, password) => desktopJson(desktopClient.login(username, password)),
    register: (username, password) => desktopJson(desktopClient.register(username, password)),
    user: () => desktopJson(desktopClient.authUser()),
    logout: () => desktopClient.logout().then(() => jsonResponse({})),
  },

  // Protected endpoints
  projects: () => desktopJson(desktopClient.listProjects()),
  sessions: (projectName, limit = 5, offset = 0) =>
    desktopJson(desktopClient.listSessions(projectName, limit, offset)),
  // Unified endpoint — all providers through one URL
  unifiedSessionMessages: (sessionId, provider = 'claude', { projectName = '', projectPath = '', limit = null, offset = 0 } = {}) =>
    desktopJson(desktopClient.getMessages(sessionId, { provider, projectName, projectPath, limit, offset })),
  renameProject: (projectName, displayName) =>
    authenticatedFetch(`/api/projects/${projectName}/rename`, {
      method: 'PUT',
      body: JSON.stringify({ displayName }),
    }),
  deleteSession: (projectName, sessionId) =>
    authenticatedFetch(`/api/projects/${projectName}/sessions/${sessionId}`, {
      method: 'DELETE',
    }),
  renameSession: (sessionId, summary, provider) =>
    authenticatedFetch(`/api/sessions/${sessionId}/rename`, {
      method: 'PUT',
      body: JSON.stringify({ summary, provider }),
    }),
  deleteCodexSession: (sessionId) =>
    authenticatedFetch(`/api/codex/sessions/${sessionId}`, {
      method: 'DELETE',
    }),
  deleteGeminiSession: (sessionId) =>
    authenticatedFetch(`/api/gemini/sessions/${sessionId}`, {
      method: 'DELETE',
    }),
  deleteProject: (projectName, force = false) =>
    authenticatedFetch(`/api/projects/${projectName}${force ? '?force=true' : ''}`, {
      method: 'DELETE',
    }),
  searchConversationsUrl: (query, limit = 50) => {
    const token = localStorage.getItem('auth-token');
    const params = new URLSearchParams({ q: query, limit: String(limit) });
    if (token) params.set('token', token);
    return `/api/search/conversations?${params.toString()}`;
  },
  createProject: (path) =>
    authenticatedFetch('/api/projects/create', {
      method: 'POST',
      body: JSON.stringify({ path }),
    }),
  createWorkspace: (workspaceData) =>
    authenticatedFetch('/api/projects/create-workspace', {
      method: 'POST',
      body: JSON.stringify(workspaceData),
    }),
  readFile: (projectName, filePath) =>
    desktopJson(desktopClient.readFile(projectName, filePath)),
  /** @param {number | null} [expectedMtimeMs] */
  saveFile: (projectName, filePath, content, expectedMtimeMs = null) =>
    desktopJson(desktopClient.saveFile(projectName, filePath, content, expectedMtimeMs)),
  getFiles: (projectName, options = {}) =>
    desktopJson(desktopClient.listFiles(projectName)),

  // File operations
  createFile: (projectName, { path, type, name }) =>
    authenticatedFetch(`/api/projects/${projectName}/files/create`, {
      method: 'POST',
      body: JSON.stringify({ path, type, name }),
    }),

  renameFile: (projectName, { oldPath, newName }) =>
    authenticatedFetch(`/api/projects/${projectName}/files/rename`, {
      method: 'PUT',
      body: JSON.stringify({ oldPath, newName }),
    }),

  deleteFile: (projectName, { path, type }) =>
    authenticatedFetch(`/api/projects/${projectName}/files`, {
      method: 'DELETE',
      body: JSON.stringify({ path, type }),
    }),

  uploadFiles: (projectName, formData) =>
    authenticatedFetch(`/api/projects/${projectName}/files/upload`, {
      method: 'POST',
      body: formData,
      headers: {}, // Let browser set Content-Type for FormData
    }),

  transcribe: (formData) =>
    authenticatedFetch('/api/transcribe', {
      method: 'POST',
      body: formData,
      headers: {}, // Let browser set Content-Type for FormData
    }),

  // TaskMaster endpoints
  taskmaster: {
    init: (projectName) =>
      authenticatedFetch(`/api/taskmaster/init/${projectName}`, {
        method: 'POST',
      }),
    addTask: (projectName, { prompt, title, description, priority, dependencies }) =>
      authenticatedFetch(`/api/taskmaster/add-task/${projectName}`, {
        method: 'POST',
        body: JSON.stringify({ prompt, title, description, priority, dependencies }),
      }),
    parsePRD: (projectName, { fileName, numTasks, append }) =>
      authenticatedFetch(`/api/taskmaster/parse-prd/${projectName}`, {
        method: 'POST',
        body: JSON.stringify({ fileName, numTasks, append }),
      }),
    getTemplates: () =>
      authenticatedFetch('/api/taskmaster/prd-templates'),
    applyTemplate: (projectName, { templateId, fileName, customizations }) =>
      authenticatedFetch(`/api/taskmaster/apply-template/${projectName}`, {
        method: 'POST',
        body: JSON.stringify({ templateId, fileName, customizations }),
      }),
    updateTask: (projectName, taskId, updates) =>
      authenticatedFetch(`/api/taskmaster/update-task/${projectName}/${taskId}`, {
        method: 'PUT',
        body: JSON.stringify(updates),
      }),
  },

  // Browse filesystem for project suggestions
  browseFilesystem: (dirPath = null) => {
    const params = new URLSearchParams();
    if (dirPath) params.append('path', dirPath);

    return authenticatedFetch(`/api/browse-filesystem?${params}`);
  },

  createFolder: (folderPath) =>
    authenticatedFetch('/api/create-folder', {
      method: 'POST',
      body: JSON.stringify({ path: folderPath }),
    }),

  // User endpoints
  user: {
    gitConfig: () => desktopJson(desktopClient.gitConfig()),
    updateGitConfig: (gitName, gitEmail) =>
      desktopClient.updateGitConfig(gitName, gitEmail).then(() => jsonResponse({})),
    onboardingStatus: () => desktopJson(desktopClient.onboardingStatus()),
    completeOnboarding: () =>
      desktopClient.completeOnboarding().then(() => jsonResponse({})),
  },

  // Generic GET method for any endpoint
  get: (endpoint) => authenticatedFetch(`/api${endpoint}`),

  // Generic POST method for any endpoint
  post: (endpoint, body) => authenticatedFetch(`/api${endpoint}`, {
    method: 'POST',
    ...(body instanceof FormData ? { body } : { body: JSON.stringify(body) }),
  }),

  // Generic PUT method for any endpoint
  put: (endpoint, body) => authenticatedFetch(`/api${endpoint}`, {
    method: 'PUT',
    body: JSON.stringify(body),
  }),

  // Generic DELETE method for any endpoint
  delete: (endpoint, options = {}) => authenticatedFetch(`/api${endpoint}`, {
    method: 'DELETE',
    ...options,
  }),
};