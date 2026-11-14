// Integrations (03 §7C): the card grid (Claude Code, Gmail, Slack, Google
// Calendar, AI backends, then "Available later"), selection into the detail
// panel, and a card's detail page through the drill-in rule
// (`integrations/<slug>`, breadcrumb-free; `integrations/claude-code/install`
// opens the hook install diff preview). Keys (03 §10.9): arrows move between
// cards, Enter opens, C runs the primary action, D opens Diagnostics, Esc
// goes up one level.
import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { Brand } from '../../kit/avatars'
import { IntegrationHead, IntegrationStatus, Section } from '../../kit/views'
import { BrandMark } from '../../kit/brands'
import { Button, MenuButton } from '../../kit/button'
import { Icon } from '../../kit/icons'
import { useArtwork } from '../../kit/theme'
import { DetailPanel, MainBody, MainHead } from '../../kit/layout'
import { Switch } from '../../kit/controls'
import { invoke } from '../../tauri'
import type { AiBackendsStatus } from '../../state/contract-settings'
import type { ClaudeCodeCard, ConnectorsStatus } from '../../state/contract-connectors'
import { useSetting, errorText } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { openMenu, useNow, type MenuEntry } from '../ui'
import { ClaudeCodePage } from './ClaudeCodePage'
import { HookDialog } from './HookPreview'
import { useClaudeHealth, useHooks } from './hooks-store'
import { aiCard, claudeCodeCard, CORE_CARD, marketplaceCard, withCoreStatus, type CardLine, type CardStatus } from './model'
import { MarketplacePage } from './MarketplacePage'
import { usePacks } from '../../state/packs-store'
import { AiPage } from '../settings/AiPage'
import { useAi } from '../settings/ai'
// package 10: live Gmail / Slack / Google Calendar cards and their account flows
import { useAccounts, type ConnectorCardKind } from '../../state/accounts-store'
import { liveConnectorCard, type LiveCard } from './accounts-model'
import { AccountsPage } from './AccountsPage'
import './integrations.css'

export type CardSlug = 'claude-code' | 'gmail' | 'slack' | 'google-calendar' | 'ai-backends' | 'marketplace'

const SLUGS: readonly CardSlug[] = ['claude-code', 'gmail', 'slack', 'google-calendar', 'ai-backends', 'marketplace']

const NAMES: Record<CardSlug, string> = { 'claude-code': 'Claude Code', gmail: 'Gmail', slack: 'Slack', 'google-calendar': 'Google Calendar', 'ai-backends': 'AI backends', marketplace: 'Marketplace' }

/** 03 §7C.4: the later cards and their one-line reasons. */
const LATER: readonly { name: string; reason: string; icon: ReactNode }[] = [
  { name: 'GitHub', reason: 'PR and issue triggers for Automations, later', icon: <BrandMark id="github" label="GitHub" /> },
  { name: 'Outlook', reason: 'Microsoft Graph connector, later', icon: <BrandMark id="outlook" label="Outlook" /> },
  { name: 'IMAP', reason: 'any mailbox without an API, later', icon: <Icon id="mail" /> },
  { name: 'Apple Mail', reason: 'macOS only, later', icon: <BrandMark id="applemail" label="Apple Mail" /> },
  { name: 'Apple Calendar', reason: 'macOS only, later', icon: <BrandMark id="applecalendar" label="Apple Calendar" /> },
]

const DOCS_URL = 'https://code.claude.com/docs'

export function IntegrationsView({ rest }: { rest?: string }) {
  const [slug, sub] = (rest ?? '').split('/') as [string | undefined, string | undefined]
  const page = slug && (SLUGS as readonly string[]).includes(slug) ? (slug as CardSlug) : null
  const flow = useHooks((s) => s.flow)
  const closeFlow = useHooks((s) => s.close)

  useEffect(() => {
    void useHooks.getState().refresh()
    void useHooks.getState().loadDiagnostics()
    void useAccounts.getState().load()
  }, [])

  // `integrations/claude-code/install` opens the preview (03 §2.5).
  useEffect(() => {
    if (page === 'claude-code' && sub === 'install' && !useHooks.getState().flow) void useHooks.getState().preview('install')
  }, [page, sub])

  // Esc goes up one level: detail page → grid (03 §2.4).
  useEffect(() => {
    if (!page) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || useHooks.getState().flow) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      navigate({ view: 'integrations' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [page])

  const closeDialog = () => {
    closeFlow()
    if (page === 'claude-code' && sub === 'install') navigate({ view: 'integrations', rest: 'claude-code' }, { replace: true })
  }

  return (
    <section className="ap-view ig-view" aria-label="Integrations" data-testid="integrations-view">
      {page ? <CardPage slug={page} sub={sub} /> : <Grid />}
      {flow && <HookDialog onClose={closeDialog} />}
    </section>
  )
}

/* ---- the grid ------------------------------------------------------------------- */

interface CardSpec {
  slug: CardSlug
  icon: ReactNode
  status?: CardStatus
  lines: CardLine[]
  action?: { label: string; primary?: boolean; disabled?: boolean; hint?: string; run: () => void }
  extra?: ReactNode
  later?: string | null
  menu: MenuEntry[]
}

function useCards(): { cards: CardSpec[]; ai: AiBackendsStatus | null } {
  const now = useNow(5000)
  const status = useHooks((s) => s.status)
  const diag = useHooks((s) => s.diag)
  // The AI backends card: `ai_backends_status` (local lines), `ai_health` (the word), `ai_backends_list` (the key).
  const ai = useAi((s) => s.status)
  const aiHealth = useAi((s) => s.health)
  const aiRows = useAi((s) => s.rows)
  const [connectors, setConnectors] = useState<ConnectorsStatus | null>(null)
  const [ccCore, setCcCore] = useState<ClaudeCodeCard | null>(null)
  const [localOnly] = useSetting<boolean>('ai.local_only')
  const [backend] = useSetting<string>('ai.backend')
  const health = useClaudeHealth()
  const art = useArtwork()
  const binaryFound = diag ? diag.claude.found : health?.claude_code !== 'missing'
  const loadAi = useCallback(() => void useAi.getState().refresh(), [])
  const accounts = useAccounts((s) => s.accounts)
  useEffect(() => {
    loadAi()
  }, [loadAi, backend])
  // `connectors_status` follows the accounts (a connect, a pause, a removal).
  useEffect(() => {
    invoke<ConnectorsStatus>('connectors_status').then(
      (c) => setConnectors(c && typeof c === 'object' && 'gmail' in c ? c : null),
      () => setConnectors(null),
    )
  }, [accounts])
  useEffect(() => {
    invoke<ClaudeCodeCard>('claude_code_card', { binary_found: binaryFound }).then(setCcCore, () => setCcCore(null))
  }, [binaryFound, status])

  const sources = useAccounts((s) => s.sources)
  const offline = useSessionsStore((s) => !!s.health?.offline)
  // The Marketplace card: `catalogue_get`, fetched once per run (03 §7C.6).
  const packsResult = usePacks((s) => s.catalogue)
  const packsLoaded = usePacks((s) => s.loaded)
  useEffect(() => {
    if (!usePacks.getState().loaded && !usePacks.getState().loading) void usePacks.getState().load()
  }, [])
  const mk = marketplaceCard(packsResult, now, packsLoaded)
  const cc = claudeCodeCard(status, diag, now, health, ccCore)
  const aic = aiCard(ai ? { ...ai, backend: (backend as AiBackendsStatus['backend']) ?? ai.backend } : null, { health: aiHealth, rows: aiRows })
  const diagnostics = (): MenuEntry => ({ id: 'diagnostics', label: 'Diagnostics', run: () => navigate({ view: 'settings', rest: 'advanced/diagnostics' }) })
  const ccRun = () => {
    if (cc.action.kind === 'docs') void invoke('plugin:opener|open_url', { url: DOCS_URL }).catch(() => {})
    else void useHooks.getState().preview('install')
  }
  const cards: CardSpec[] = [
    {
      slug: 'claude-code',
      icon: <Brand brand="claudecode" label="Claude Code" />,
      status: cc.status,
      lines: cc.lines,
      action: { label: cc.action.label, run: ccRun },
      menu: [
        { id: 'settings', label: 'Settings', run: () => navigate({ view: 'integrations', rest: 'claude-code' }) },
        diagnostics(),
        { id: 'remove', label: 'Uninstall hooks', danger: true, dimmed: cc.status.state === 'idle', run: () => void useHooks.getState().preview('uninstall') },
      ],
    },
    ...(['gmail', 'slack', 'google-calendar'] as const).map((kind): CardSpec => {
      const c = liveConnectorCard(kind, accounts, sources, { offline: offline || connectors?.offline === true, now })
      return {
        slug: kind,
        icon: <Brand brand={kind === 'google-calendar' ? 'googlecalendar' : kind} label={NAMES[kind]} />,
        // The core's word (worst health of the service's accounts) over the accounts' own.
        status: withCoreStatus(c.status, connectors?.[CORE_CARD[kind]]),
        lines: c.lines,
        later: null,
        action: { ...c.action, run: () => runLive(kind, c) },
        menu: [
          { id: 'settings', label: 'Settings', run: () => navigate({ view: 'integrations', rest: kind }) },
          diagnostics(),
          ...c.accounts.map((a): MenuEntry => ({ id: `remove-${a.id}`, label: c.accounts.length > 1 ? `Remove ${a.display_name}…` : 'Remove…', danger: true, run: () => useAccounts.getState().askRemove(a.id) })),
        ],
      }
    }),
    {
      slug: 'ai-backends',
      icon: <Brand brand="claude" label={NAMES['ai-backends']} />,
      status: aic.status,
      lines: aic.lines,
      action: {
        label: aic.action.label,
        run: () => {
          if (aic.action.kind === 'key') navigate({ view: 'integrations', rest: 'ai-backends' })
          else
            void invoke<{ ok: boolean; latency_ms: number | null; error?: string | null }>('ai_test_call').then(
              (r) => useSessionsStore.getState().showNotice(r.ok ? `Answered in ${r.latency_ms ?? '?'} ms` : `Could not reach the backend (${r.error ?? 'no answer'}).`),
              (e) => useSessionsStore.getState().showNotice(`Could not reach the backend (${errorText(e)}).`),
            )
        },
      },
      extra: (
        <span className="m-inline ig-switch">
          <Switch on={localOnly === true} label="Local-only" onChange={(on) => void useAi.getState().set({ local_only: on })} />
          <span className="m-t-dense m-fg-2">Local-only</span>
        </span>
      ),
      menu: [
        { id: 'settings', label: 'Settings', run: () => navigate({ view: 'settings', rest: 'ai' }) },
        diagnostics(),
        {
          id: 'remove',
          label: 'Remove key',
          danger: true,
          dimmed: !(aiRows?.find((r) => r.backend === 'claude_api')?.key_stored ?? ai?.claude_api.key_stored),
          run: () => void useAi.getState().setKey(null),
        },
      ],
    },
    {
      slug: 'marketplace',
      icon: (
        <span className="m-brand">
          <img src={art.kinds.marketplace} alt="" />
        </span>
      ),
      status: mk.status,
      lines: mk.lines,
      action: {
        label: mk.action.label,
        run: () => {
          if (mk.action.kind === 'retry') void usePacks.getState().load(true)
          else navigate({ view: 'integrations', rest: 'marketplace' })
        },
      },
      menu: [
        { id: 'settings', label: 'Settings', run: () => navigate({ view: 'integrations', rest: 'marketplace' }) },
        diagnostics(),
        { id: 'refresh', label: 'Refresh catalog', run: () => void usePacks.getState().load(true) },
        { id: 'index', label: 'Change index…', run: () => navigate({ view: 'integrations', rest: 'marketplace/index' }) },
      ],
    },
  ]
  return { cards, ai }
}

/** The live connector cards' primary actions (03 §7C.2): the OAuth loopback, Reconnect, Retry, the pickers. */
function runLive(kind: ConnectorCardKind, c: LiveCard) {
  const acc = useAccounts.getState()
  switch (c.action.kind) {
    case 'connect':
    case 'add':
      acc.startConnect(kind, kind === 'google-calendar' ? { services: ['gmail', 'calendar'] } : undefined)
      break
    case 'reconnect':
      if (c.action.accountId) void acc.startReauth(c.action.accountId)
      break
    case 'reauth-calendar':
      if (c.action.accountId) void acc.startReauth(c.action.accountId, { services: ['gmail', 'calendar'] })
      break
    case 'retry':
      if (c.action.accountId) void acc.retry(c.action.accountId)
      break
    case 'channels':
      if (c.action.accountId) void acc.openPicker(c.action.accountId)
      break
    case 'calendars':
      navigate({ view: 'integrations', rest: kind })
      break
  }
}

function Line({ line }: { line: CardLine }) {
  if (typeof line === 'string') return <>{line}</>
  return (
    <>
      {line.mark && <BrandMark id={line.mark} size={12} label="" />}
      {line.before}
      <code>{line.code}</code>
      {line.after}
    </>
  )
}

function Grid() {
  const { cards } = useCards()
  const [selected, setSelected] = useState<CardSlug | null>(null)
  const gridRef = useRef<HTMLDivElement>(null)
  const sel = cards.find((c) => c.slug === selected) ?? null

  const onKey = (e: React.KeyboardEvent, c: CardSpec) => {
    const t = e.target as HTMLElement
    if (t.tagName === 'BUTTON' && e.key === 'Enter') return
    if (e.key === 'Enter') {
      e.preventDefault()
      navigate({ view: 'integrations', rest: c.slug })
    } else if ((e.key === 'c' || e.key === 'C') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      if (c.action && !c.action.disabled && !c.later) {
        e.preventDefault()
        c.action.run()
      }
    } else if ((e.key === 'd' || e.key === 'D') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault()
      navigate({ view: 'settings', rest: 'advanced/diagnostics' })
    } else if (e.key.startsWith('Arrow')) {
      const all = Array.from(gridRef.current?.querySelectorAll<HTMLElement>('.ig-card') ?? [])
      const i = all.indexOf(e.currentTarget as HTMLElement)
      const cols = Math.max(1, Math.round((gridRef.current?.clientWidth ?? 1) / ((e.currentTarget as HTMLElement).offsetWidth || 1)))
      const step = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowDown' ? cols : -cols
      const next = all[i + step]
      if (next) {
        e.preventDefault()
        next.focus()
      }
    } else if (e.key === 'Escape' && selected) {
      e.preventDefault()
      setSelected(null)
    }
  }

  return (
    <>
      <MainHead title="Integrations" titleTestId="view-title">
        <MenuButton
          label="Integrations options"
          onClick={(e) =>
            openMenu(
              e.currentTarget,
              [
                { id: 'refresh', label: 'Refresh', run: () => void Promise.all([useHooks.getState().refresh(), useHooks.getState().loadDiagnostics()]) },
                { id: 'diagnostics', label: 'Diagnostics', run: () => navigate({ view: 'settings', rest: 'advanced/diagnostics' }) },
              ],
              'Integrations options',
            )
          }
        />
      </MainHead>
      <div className="ig-split">
        <MainBody className="ig-body" testId="integrations-body">
          <div className="m-integrations m-stagger" ref={gridRef}>
            {cards.map((c, i) => (
              <Card key={c.slug} c={c} i={i} selected={selected === c.slug} onSelect={() => setSelected(c.slug)} onKey={(e) => onKey(e, c)} />
            ))}
          </div>
          <div className="m-group-label">Available later</div>
          <div className="m-integrations is-later m-stagger">
            {LATER.map((l, i) => (
              <div key={l.name} className="m-integration is-later" aria-label={`${l.name}, available later`} style={{ '--i': cards.length + i } as CSSProperties}>
                <span className="m-brand">{l.icon}</span>
                <IntegrationHead name={l.name} />
                <div className="m-integration-body">
                  <div className="m-meta">{l.reason}</div>
                </div>
              </div>
            ))}
          </div>
        </MainBody>
        {sel && (
          <DetailPanel label={`${NAMES[sel.slug]} summary`} className="ig-panel">
            <div className="ig-panel-head">
              {sel.icon}
              <span className="m-card-title">{NAMES[sel.slug]}</span>
              <MenuButton
                label={`${NAMES[sel.slug]} options`}
                onClick={(e) =>
                  openMenu(e.currentTarget, [...sel.menu, { kind: 'sep', id: 'sep-close' }, { id: 'close', label: 'Close panel', run: () => setSelected(null) }], `${NAMES[sel.slug]} options`)
                }
              />
            </div>
            {sel.status && <IntegrationStatus status={sel.status} className="ig-panel-status" />}
            {sel.lines.length > 0 && (
              <div className="ig-panel-lines">
                {sel.lines.map((l, i) => (
                  <div key={i} className="m-meta">
                    <Line line={l} />
                  </div>
                ))}
              </div>
            )}
            <div className="m-card-actions ig-panel-actions">
              {sel.action && !sel.later && (
                <Button variant="primary" disabled={sel.action.disabled} onClick={sel.action.run}>
                  {sel.action.label}
                </Button>
              )}
              <Button quiet onClick={() => navigate({ view: 'integrations', rest: sel.slug })}>
                Open
              </Button>
            </div>
          </DetailPanel>
        )}
      </div>
    </>
  )
}

/** One `.m-integration` (the kit's IntegrationCard markup, selectable and keyboard-driven: kit candidate). */
function Card({ c, i, selected, onSelect, onKey }: { c: CardSpec; i: number; selected: boolean; onSelect: () => void; onKey: (e: React.KeyboardEvent) => void }) {
  return (
    <div
      style={{ '--i': i } as CSSProperties}
      className={`m-integration ig-card${c.later ? ' is-later' : ''}${selected ? ' ig-selected' : ''}`}
      tabIndex={0}
      role="group"
      aria-label={NAMES[c.slug]}
      data-testid={`card-${c.slug}`}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest('button')) return
        onSelect()
      }}
      onDoubleClick={() => navigate({ view: 'integrations', rest: c.slug })}
      onKeyDown={onKey}
    >
      {c.icon}
      <IntegrationHead name={NAMES[c.slug]} status={c.later ? undefined : c.status} />
      {!c.later && (
        <MenuButton
          label={`${NAMES[c.slug]} options`}
          onClick={(e) => {
            e.stopPropagation()
            openMenu(e.currentTarget, c.menu, `${NAMES[c.slug]} options`)
          }}
        />
      )}
      <div className="m-integration-body">
        {c.lines.map((l, i) => (
          <div key={i} className="m-meta">
            <Line line={l} />
          </div>
        ))}
        {!c.later && (c.action || c.extra) && (
          <div className="m-card-actions">
            {c.action && (
              <Button variant={c.action.primary ? 'primary' : 'default'} disabled={c.action.disabled} title={c.action.hint} onClick={c.action.run}>
                {c.action.label}
              </Button>
            )}
            {c.extra}
          </div>
        )}
      </div>
    </div>
  )
}

/* ---- a card's detail page (03 §7C.3) -------------------------------------------------- */

function CardPage({ slug, sub }: { slug: CardSlug; sub?: string }) {
  const { cards } = useCards()
  const c = cards.find((x) => x.slug === slug)!
  return (
    <>
      <MainHead title={NAMES[slug]} titleTestId="view-title">
        <Button quiet aria-label="Back to Integrations" onClick={() => navigate({ view: 'integrations' })}>
          ‹ Integrations
        </Button>
      </MainHead>
      <MainBody className="ig-body ig-page" testId="integration-page">
        <div className="ig-page-head" data-testid="integration-page-head">
          {c.icon}
          <IntegrationHead name={NAMES[slug]} status={c.status} />
        </div>
        {slug === 'claude-code' ? (
          <ClaudeCodePage />
        ) : slug === 'ai-backends' ? (
          <Section title="Backends and models" className="ig-ai">
            <AiPage />
          </Section>
        ) : slug === 'marketplace' ? (
          <MarketplacePage sub={sub} />
        ) : (
          <AccountsPage kind={slug} />
        )}
      </MainBody>
    </>
  )
}
