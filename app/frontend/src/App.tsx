import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import './App.css'

/**
 * MCP server endpoint — matches the contract locked by Worker #1
 * (see `app/src-tauri/src/mcp/server.rs`). The macOS Swift build binds
 * to the same port + path, so Claude / Cursor / Codex clients configured
 * against macOS keep working when pointed at the Windows port.
 *
 * DEV: We fetch through the Vite dev-server proxy (`/mcp-api` →
 * `http://127.0.0.1:19789`, configured in `vite.config.ts`). The MCP server
 * is locked to CLI-client use only and intentionally does NOT send CORS
 * headers — fetching it cross-origin from a browser tab on 127.0.0.1:1420
 * would be blocked by the browser, leaving the status card stuck on
 * "Failed" even when the server is healthy. The proxy performs the fetch
 * same-origin from Vite's dev server, sidestepping CORS without weakening
 * the MCP server's security posture.
 *
 * PROD: The Vite dev-server proxy is not available in a bundled Tauri
 * build. The Tauri webview origin in production is `tauri://localhost`
 * (macOS) / `https://tauri.localhost` (Windows) / `http://tauri.localhost`
 * (Linux) — a cross-origin fetch to `http://127.0.0.1:19789` from those
 * origins would be blocked by the browser (same CORS wall as dev, since
 * the MCP server contract does NOT send CORS headers). Instead of a
 * browser `fetch()`, production uses a Tauri Rust command
 * (`probe_oauth_protected_resource`) that performs the HTTP GET from the
 * Rust process (same process as the MCP server). This bypasses the
 * browser's same-origin policy entirely — there is no cross-origin fetch.
 * See `app/src-tauri/src/commands/mod.rs` for the command implementation.
 */
const MCP_OAUTH_RESOURCE_URL = '/mcp-api/.well-known/oauth-protected-resource'

/** Expected shape of the RFC 9728 `oauth-protected-resource` response. */
interface OAuthProtectedResource {
  resource: string
}

/**
 * Shape returned by the Tauri `probe_oauth_protected_resource` command.
 * Must match `ProbeResult` in `app/src-tauri/src/commands/mod.rs`.
 */
interface TauriProbeResult {
  status: number | null
  body: string | null
  error: string | null
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
      if (import.meta.env.DEV) {
        // ── DEV: use Vite dev-server proxy (same-origin, no CORS) ──
        const res = await fetch(MCP_OAUTH_RESOURCE_URL, {
          signal: controller.signal,
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
      } else {
        // ── PROD: use Tauri Rust command (same-process, no CORS) ──
        // The invoke() call routes through Tauri's IPC bridge to the Rust
        // `probe_oauth_protected_resource` command, which performs the
        // HTTP GET from the Rust process. This avoids the browser's
        // same-origin policy entirely — there is no browser fetch.
        const result = await invoke<TauriProbeResult>('probe_oauth_protected_resource')

        if (result.error) {
          throw new Error(result.error)
        }
        if (result.status && result.status >= 400) {
          throw new Error(`HTTP ${result.status}`)
        }

        const text = result.body ?? ''
        let parsed: OAuthProtectedResource | null = null
        try {
          parsed = JSON.parse(text) as OAuthProtectedResource
        } catch {
          // RFC 9728 says the body MUST be JSON, but we degrade gracefully.
        }
        const resource = parsed?.resource ?? 'unknown'
        setStatus({
          state: 'connected',
          summary: `Connected — ${resource}`,
          detail: text,
          lastCheckedAt: new Date().toISOString(),
        })
      }
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

  // Initial probe on mount. Auto-retry while not connected is handled by
  // the separate effect below (so that this one only runs once and does
  // not own an interval).
  useEffect(() => {
    probe()
    return () => {
      abortRef.current?.abort()
    }
  }, [probe])

  // Auto-retry while not connected. Runs its own interval that re-probes
  // every RETRY_INTERVAL_MS until the state becomes 'connected', at which
  // point this effect's cleanup clears the interval.
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
              href={import.meta.env.DEV ? MCP_OAUTH_RESOURCE_URL : 'http://127.0.0.1:19789/.well-known/oauth-protected-resource'}
              target="_blank"
              rel="noreferrer"
              style={{ color: '#8ab4f8', textDecoration: 'none' }}
            >
              {import.meta.env.DEV
                ? MCP_OAUTH_RESOURCE_URL
                : 'http://127.0.0.1:19789/.well-known/oauth-protected-resource'}
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
