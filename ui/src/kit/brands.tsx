// The marks of external apps and services (10 §7.4.16, ADR-0053): one
// `<symbol id="b-…">` per brand from assets/brands (generated into
// brands.gen.ts), drawn in currentColor so near-black marks follow the theme
// and the others wear their brand color from the stylesheet. `<BrandDefs/>`
// is rendered once per document next to `<IconDefs/>`; `<BrandMark id>`
// references a symbol. The helpers map Mode's own names (backends, terminals,
// MCP servers, conference links) to a mark, or to none when a service has no
// mark of its own (a generic icon is drawn then: never another brand's logo).
import { cx } from './cx'
import { BRAND_IDS, BRANDS, type BrandId } from './brands.gen'

export { BRAND_IDS, BRANDS, type BrandId }

/**
 * The symbol sheet. Hidden by size, not `display: none`: a mark fetched with
 * its gradients (Zoom, Teams, PowerShell, Azure) references paint servers
 * inside the sheet, and Chromium drops those from a `display: none` subtree,
 * which would leave the mark blank where the gradient was.
 */
export function BrandDefs() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" style={{ position: 'absolute' }} aria-hidden="true" focusable="false">
      {BRAND_IDS.map((id) => (
        <symbol key={id} id={`b-${id}`} viewBox={BRANDS[id].viewBox} dangerouslySetInnerHTML={{ __html: BRANDS[id].body }} />
      ))}
    </svg>
  )
}

export interface BrandMarkProps {
  id: BrandId
  /** 18 px (a tile), 16 px (a row) or 12 px (inline before a name). */
  size?: 18 | 16 | 12
  /** The accessible name; the brand's label by default, empty hides it from readers. */
  label?: string
  className?: string
}

export function BrandMark({ id, size = 18, label, className }: BrandMarkProps) {
  const name = label ?? BRANDS[id].label
  return (
    <svg className={cx('m-brand-mark', `is-${id}`, size === 16 && 'is-16', size === 12 && 'is-12', className)} role={name ? 'img' : undefined} aria-label={name || undefined} aria-hidden={name ? undefined : true}>
      <use href={`#b-${id}`} />
    </svg>
  )
}

/** The AI backends (06 §10): `usage_samples.source` / `ai.backend` values. */
export function brandForBackend(backend: string): BrandId | null {
  switch (backend) {
    case 'claude_api':
      return 'claude'
    case 'ollama':
      return 'ollama'
    case 'lm_studio':
      return 'lmstudio'
    default:
      return null
  }
}

/** Hosts whose mark is their own (06 §10.1A); a provider elsewhere is marked by its kind. */
const PROVIDER_HOSTS: readonly (readonly [string, BrandId])[] = [
  ['openrouter.ai', 'openrouter'],
  ['groq.com', 'groq'],
  ['openai.com', 'openai'],
  ['anthropic.com', 'claude'],
  ['mistral.ai', 'mistral'],
  ['deepseek.com', 'deepseek'],
  ['perplexity.ai', 'perplexity'],
  ['googleapis.com', 'googlecloud'],
  ['amazonaws.com', 'bedrock'],
  ['x.ai', 'grok'],
]

/**
 * An AI provider's mark (06 §10.1A): the service its address names, else the
 * kind's own (a local server, a cloud); `null` for an endpoint Mode cannot
 * name (a gateway at some host), which draws a generic icon.
 */
export function brandForProvider(p: { kind: string; url?: string | null; builtin?: boolean }): BrandId | null {
  const host = (p.url ?? '').replace(/^https?:\/\//, '').split('/')[0]?.split(':')[0]?.toLowerCase() ?? ''
  for (const [h, id] of PROVIDER_HOSTS) if (host === h || host.endsWith(`.${h}`)) return id
  switch (p.kind) {
    case 'ollama':
      return 'ollama'
    case 'lm_studio':
      return 'lmstudio'
    case 'bedrock':
      return 'bedrock'
    case 'vertex':
      return 'googlecloud'
    case 'foundry':
      return 'foundry'
    case 'claude_api':
      return p.builtin || !host ? 'claude' : null
    default:
      return null
  }
}

/** A terminal app's display name (ADR-0009) to its mark; terminals without one share the terminal window. */
export function brandForTerminal(name: string | null | undefined): BrandId | null {
  if (!name) return null
  const n = name.trim().toLowerCase()
  if (n.includes('iterm')) return 'iterm2'
  if (n === 'warp') return 'warp'
  if (n === 'ghostty') return 'ghostty'
  if (n === 'alacritty') return 'alacritty'
  if (n === 'wezterm') return 'wezterm'
  if (n === 'kitty') return 'kitty'
  if (n.includes('gnome')) return 'gnometerminal'
  if (n === 'powershell') return 'powershell'
  if (n === 'wsl') return 'linux'
  if (n.includes('terminal') || n === 'konsole' || n === 'xterm') return 'terminal'
  return 'terminal'
}

/** A session connector (03 §3.9B): an MCP server's service by name, else the MCP mark; rules, plugins and the relay have none. */
export function brandForConnector(kind: string, name: string): BrandId | null {
  if (kind !== 'mcp_server') return null
  const n = name.trim().toLowerCase()
  if (n.includes('github')) return 'github'
  if (n.includes('linear')) return 'linear'
  if (n.includes('notion')) return 'notion'
  if (n.includes('slack')) return 'slack'
  if (n.includes('gmail') || n.includes('google-mail')) return 'gmail'
  if (n.includes('calendar')) return 'googlecalendar'
  return 'modelcontextprotocol'
}

/** A calendar event's conference provider (06 §6): Meet, Zoom, Teams; `other` has no mark. */
export function brandForConference(conference: string | null | undefined): BrandId | null {
  switch (conference) {
    case 'meet':
      return 'googlemeet'
    case 'zoom':
      return 'zoom'
    case 'teams':
      return 'teams'
    default:
      return null
  }
}
