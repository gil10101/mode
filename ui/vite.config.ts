import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'
import type { Plugin } from 'vite'

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url))

/**
 * Dev server only (08 §12), and only with `MODE_DEV_SAY=1`: `POST /__mode/say?token=…&text=…` relays a typed transcript
 * to the overlay webview over the HMR channel, which calls the dev build's `dictation_say`; QA drives "Hey Mode" in the
 * real app without a microphone. The token is random per server start and printed once; a cross-site caller (any
 * `Sec-Fetch-Site` but `same-origin` / `none`, or an `Origin` naming another host) is refused, so a web page open in a
 * browser cannot push text into the app. `vite build` never applies it; the client side lives behind `import.meta.hot`.
 */
function devSay(): Plugin {
  return {
    name: 'mode-dev-say',
    apply: () => process.env.MODE_DEV_SAY === '1',
    configureServer(server) {
      const token = crypto.randomUUID()
      const port = server.config.server.port ?? 5173
      server.config.logger.info(`[mode] dictation_say relay: curl -X POST "http://localhost:${port}/__mode/say?token=${token}&text=…"`)
      server.middlewares.use('/__mode/say', (req, res) => {
        const refuse = (why: string) => {
          res.statusCode = 403
          res.setHeader('content-type', 'text/plain')
          res.end(`${why}\n`)
        }
        const site = req.headers['sec-fetch-site']
        if (site !== undefined && site !== 'same-origin' && site !== 'none') return refuse('cross-site')
        const origin = req.headers.origin
        if (origin) {
          let host = ''
          try {
            host = new URL(origin).host
          } catch {
            return refuse('bad origin')
          }
          if (host !== req.headers.host) return refuse('cross-origin')
        }
        const url = new URL(req.url ?? '', 'http://localhost')
        if (req.method !== 'POST' || url.searchParams.get('token') !== token) return refuse('forbidden')
        const text = url.searchParams.get('text') ?? ''
        server.ws.send({ type: 'custom', event: 'mode:say', data: { text } })
        res.setHeader('content-type', 'text/plain')
        res.end(text ? 'sent\n' : 'no text\n')
      })
    },
  }
}

// Tauri serves `frontendDist` (../../ui/dist) and, in dev, `devUrl`
// (http://localhost:5173). Two entries, one bundle (08 §3.1).
export default defineConfig({
  plugins: [react(), devSay()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    // Tauri's watcher owns crates/; Vite must not reload on Rust changes.
    watch: { ignored: ['**/crates/**', '**/target/**'] },
    // The onboarding Welcome step imports the landing page's mascot (site/bot.js), outside this root.
    fs: { allow: [here('..')] },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: ['es2022', 'chrome105', 'safari13'],
    minify: process.env.TAURI_ENV_DEBUG ? false : 'oxc',
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: {
        overlay: here('./overlay.html'),
        app: here('./app.html'),
        // The component gallery (src/dev) is a dev and test page; Tauri builds
        // (TAURI_ENV_PLATFORM set by the CLI) leave it out of the bundle.
        ...(process.env.TAURI_ENV_PLATFORM ? {} : { gallery: here('./gallery.html') }),
        // The Notes view on the browser mock's sample vault (package 7 dev and e2e page).
        ...(process.env.TAURI_ENV_PLATFORM ? {} : { notes: here('./notes.html') }),
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    // The suites assume the machine's zone is UTC, as CI's is (the automation phrases say "· UTC"
    // only when a schedule's zone differs from the device's); pin it so a developer's Mac agrees.
    env: { TZ: 'UTC' },
    setupFiles: ['src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    css: false,
  },
})
