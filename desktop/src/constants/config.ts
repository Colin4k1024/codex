/**
 * Environment Flag: Is Platform
 * Indicates if the app is running in Platform mode (hosted) or OSS mode (self-hosted)
 */
export const IS_PLATFORM = import.meta.env.VITE_IS_PLATFORM === 'true';

/**
 * Desktop flag (product modification, AGPL Section 7 — see desktop/SOURCE.md).
 * True when running inside the YourAgent Tauri shell. Selects the self-contained
 * DesktopClient data path (Tauri commands) instead of the upstream Express/WS backend.
 */
export const IS_DESKTOP =
  import.meta.env.VITE_DESKTOP === 'true' ||
  (typeof window !== 'undefined' && '__TAURI__' in window);