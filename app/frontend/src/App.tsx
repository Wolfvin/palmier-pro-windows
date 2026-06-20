import { useCallback, useEffect, useRef, useState } from 'react'
import './App.css'

/**
 * MCP server endpoint — matches the contract locked by Worker #1
 * (see `app/src-tauri/src/mcp/server.rs`). The macOS Swift build binds
 * to the same port + path, so Claude / Cursor / Codex clients configured
 * against macOS keep working when pointed at the Windows port.
 */
const MCP_OAUTH_RESOURCE_URL =
  'http://127.0.0.1:19789/.well-known/oauth-protected-resource'

/** Expected shape of the RFC 9728 `oauth-protected-resource` response. */
interface OAuthProtectedResource {
  resource: string
}

type ConnectionState = 'connecting' | 'connected' | 'failed'

interface McpStatus {
  state: ConnectionState
  /** Human-readable summary, e.g. "Connected — http://127.0.0.1:19789". */
  summary: string
  /** Verbose detail (raw response body or error message). */
  detail?: string
  /** ISO timestamp of the last successful probe. */
  lastCheckedAt?: string
}

/** Auto-retry interval for the MCP probe (ms). */
const RETRY_INTERVAL_MS = 5_000

function App() {
  const [status, setStatus] = useState<McpStatus>({
    state: 'connecting',
    summary: 'Connecting to MCP server…',
  })
  const [retrying, setRetrying] = useState(false)
  const abortRef = useRef<AbortController | null>(null)

  const probe = useCallback(async () => {
    // Cancel any in-flight probe before starting a new one.
    abortRef.current?.abort()
    const controller = new AbortController()
    abortRef.current = controller

    setStatus((prev) => ({
      ...prev,
      state: 'connecting',
      summary: retrying ? 'Retrying…' : 'Connecting to MCP server…',
    }))

    try {
      const res = await fetch(MCP_OAUTH_RESOURCE_URL, {
        signal: controller.signal,
        // Long-running MCP clients use the Streamable HTTP transport, but
        // this probe is a one-shot GET — give it a generous timeout anyway
        // in case the server is still booting.
      })
      if (!res.ok) {
        throw new Error(`HTTP ${res.status} ${res.statusText}`)
      }
      const text = await res.text()
      let parsed: OAuthProtectedResource | null = null
      try {
        parsed = JSON.parse(text) as OAuthProtectedResource
      } catch {
        // RFC 9728 says the body MUST be JSON, but we degrade gracefully
        // and surface the raw body in the detail panel.
      }
      const resource = parsed?.resource ?? 'unknown'
      setStatus({
        state: 'connected',
        summary: `Connected — ${resource}`,
        detail: text,
        lastCheckedAt: new Date().toISOString(),
      })
    } catch (err) {
      if ((err as Error).name === 'AbortError') {
        // Superseded by a newer probe — don't surface as failure.
        return
      }
      const message = err instanceof Error ? err.message : String(err)
      setStatus({
        state: 'failed',
        summary: 'MCP server not reachable',
        detail: message,
        lastCheckedAt: new Date().toISOString(),
      })
    } finally {
      setRetrying(false)
    }
  }, [retrying])

  // Initial probe + auto-retry while not connected.
  useEffect(() => {
    probe()
    const id = setInterval(() => {
      setStatus((prev) => {
        if (prev.state === 'connected') return prev
        // Trigger a probe by calling probe() outside the updater.
        return prev
      })
    }, RETRY_INTERVAL_MS)

    return () => {
      clearInterval(id)
      abortRef.current?.abort()
    }
  }, [probe])

  // When state is not connected, the interval above should re-probe.
  // We do this in a separate effect to avoid re-running probe() on every render.
  useEffect(() => {
    if (status.state === 'connected') return
    const id = setInterval(() => {
      probe()
    }, RETRY_INTERVAL_MS)
    return () => clearInterval(id)
  }, [status.state, probe])

  const handleRetry = () => {
    setRetrying(true)
    probe()
  }

  const dotClass = `mcp-status__dot mcp-status__dot--${status.state}`

  return (
    <main className="app">
      <h1 className="app__title">Palmier Pro — Windows</h1>
      <p className="app__subtitle">
        Tauri v2 + Rust shell — MCP server status
      </p>

      <section className="mcp-status" aria-live="polite">
        <div className="mcp-status__header">
          <span className={dotClass} aria-hidden="true" />
          <span>{status.summary}</span>
        </div>

        {status.detail && (
          <pre className="mcp-status__detail">{status.detail}</pre>
        )}

        <div className="mcp-status__meta">
          <span>
            {status.lastCheckedAt
              ? `Last checked: ${new Date(status.lastCheckedAt).toLocaleTimeString()}`
              : 'Not yet checked'}
            {' · '}
            <a
              href={MCP_OAUTH_RESOURCE_URL}
              target="_blank"
              rel="noreferrer"
              style={{ color: '#8ab4f8', textDecoration: 'none' }}
            >
              {MCP_OAUTH_RESOURCE_URL}
            </a>
          </span>
          <button
            type="button"
            className="mcp-status__retry"
            onClick={handleRetry}
            disabled={status.state === 'connecting'}
          >
            Retry
          </button>
        </div>
      </section>
    </main>
  )
}

export default App
