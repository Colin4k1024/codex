import { useEffect, useState } from 'react'
import { formatDesktopInfo } from '../shared/desktopInfo'
import type { CodexDesktopBridge, DesktopInfo } from '../shared/types'

declare global {
  interface Window {
    codexDesktop: CodexDesktopBridge
  }
}

export function App() {
  const [info, setInfo] = useState<DesktopInfo | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    window.codexDesktop
      .getDesktopInfo()
      .then((value) => {
        if (!cancelled) {
          setInfo(value)
        }
      })
      .catch((err: Error) => {
        if (!cancelled) {
          setError(err.message)
        }
      })
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <main className="shell">
      <header className="shell__header">
        <h1>Codex Desktop</h1>
        <p>Electron scaffold connected through a typed preload bridge.</p>
      </header>
      <section className="shell__content">
        {info ? (
          <p className="shell__status">{formatDesktopInfo(info)}</p>
        ) : error ? (
          <p className="shell__error">Unable to read desktop info: {error}</p>
        ) : (
          <p className="shell__loading">Loading desktop info…</p>
        )}
      </section>
    </main>
  )
}
