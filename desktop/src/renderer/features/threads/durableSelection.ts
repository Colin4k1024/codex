const DURABLE_THREAD_ID_STORAGE_KEY = 'codexDesktop.selectedThreadId'

export function readDurableThreadId(): string | null {
  try {
    const value = window.localStorage.getItem(DURABLE_THREAD_ID_STORAGE_KEY)
    return value !== null && value.length > 0 ? value : null
  } catch {
    // Storage can be unavailable in locked-down Electron profiles.
    return null
  }
}

export function persistDurableThreadId(threadId: string): void {
  if (threadId.length === 0) {
    return
  }
  try {
    window.localStorage.setItem(DURABLE_THREAD_ID_STORAGE_KEY, threadId)
  } catch {
    // Durable selection is a best-effort convenience; recovery still works from thread/list.
  }
}
