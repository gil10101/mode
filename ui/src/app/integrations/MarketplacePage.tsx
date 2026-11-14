// The Marketplace page (03 §7C.6; ADR-0060), a card's detail page of the
// Integrations view: the catalog section (a grid of pack cards: name,
// "by author · v1", the description, the count chips led by their kind marks,
// Install; the refresh beside the "fetched" stamp), the share section
// (Import pack… from a file or a drop, Create pack… from the rows of this
// computer), and the index address. Install and Import open the same preview dialog: the
// pack's items as watched rows with one switch each, the note under each
// (new / replaces / kept beside / its folder is not here), the conflict rule,
// a folder field for a session automation whose folder is missing; Install
// writes through the stores and the dialog closes on the outcome line.
import { useEffect, useRef, useState, type DragEvent, type ReactNode } from 'react'
import { AgentAvatar } from '../../kit/avatars'
import { Button } from '../../kit/button'
import { Icon } from '../../kit/icons'
import { useArtwork } from '../../kit/theme'
import { Seg, SegItem, Switch } from '../../kit/controls'
import { SwatchDots, swatchOfList } from '../../kit/swatch'
import { Form, FormField, Section, SourceHead, SourceRow } from '../../kit/views'
import type { AgentSummary } from '../../state/contract-agents'
import type { AutomationRow } from '../../state/contract-automations'
import type { CatalogueEntry, ItemKind, ItemPreview } from '../../state/contract-packs'
import type { Swatch, ThemeRow } from '../../state/contract-themes'
import { agentHue, useAgents } from '../../state/agents-store'
import { useAutomations } from '../../state/automations-store'
import { KIND_LABEL, canInstall, entryMeta, installable, itemNote, itemTag, packLine, usePacks, type PackPreview } from '../../state/packs-store'
import { useSessionsStore } from '../../state/sessions-store'
import { useSetting } from '../../state/settings-store'
import { useThemes } from '../../state/theme-store'
import { CommitInput, Field, Group, Note } from '../settings/form-controls'
import { navigate } from '../router'
import { Dialog, useNow } from '../ui'
import { ago } from './model'

export const IMPORT_EVENT = 'mode:pack-import'
export const CREATE_EVENT = 'mode:pack-create'

const KINDS: readonly ItemKind[] = ['agent', 'automation', 'theme']

export function MarketplacePage({ sub }: { sub?: string }) {
  const now = useNow(5000)
  const catalogue = usePacks((s) => s.catalogue)
  const loading = usePacks((s) => s.loading)
  const loaded = usePacks((s) => s.loaded)
  const preview = usePacks((s) => s.preview)
  const result = usePacks((s) => s.result)
  const error = usePacks((s) => s.error)
  const [creating, setCreating] = useState(false)
  const [dragging, setDragging] = useState(false)
  const file = useRef<HTMLInputElement>(null)
  const [indexUrl] = useSetting<string>('marketplace.index_url')
  const [index, setIndex] = useState(false)
  useEffect(() => {
    if (sub === 'index') setIndex(true)
  }, [sub])
  const closeIndex = () => {
    setIndex(false)
    if (sub === 'index') navigate({ view: 'integrations', rest: 'marketplace' }, { replace: true })
  }

  useEffect(() => {
    if (!usePacks.getState().loaded) void usePacks.getState().load()
    const open = () => file.current?.click()
    const create = () => setCreating(true)
    window.addEventListener(IMPORT_EVENT, open)
    window.addEventListener(CREATE_EVENT, create)
    return () => {
      window.removeEventListener(IMPORT_EVENT, open)
      window.removeEventListener(CREATE_EVENT, create)
    }
  }, [])

  const readFile = async (f: File | undefined | null) => {
    if (!f) return
    const text = await f.text()
    await usePacks.getState().previewText(text, f.name)
  }
  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    setDragging(false)
    const f = Array.from(e.dataTransfer.files).find((x) => /\.json$/i.test(x.name) || x.type === 'application/json')
    void readFile(f)
  }

  const cat = catalogue?.catalogue ?? null
  const packs = cat?.packs ?? []
  const fetched = cat ? ago(cat.fetched_at, now) : null
  const meta = loading ? 'fetching…' : cat ? `${packs.length} ${packs.length === 1 ? 'pack' : 'packs'}${fetched ? ` · fetched ${fetched}` : ''}` : undefined
  const url = catalogue?.url ?? indexUrl

  return (
    <div className={`ig-market${dragging ? ' is-drop' : ''}`} data-testid="marketplace-page" onDragOver={(e) => (e.preventDefault(), setDragging(true))} onDragLeave={() => setDragging(false)} onDrop={onDrop}>
      <Section
        title="Catalog"
        meta={meta}
        testId="marketplace-catalogue"
        actions={
          <Button variant="icon" quiet aria-label="Refresh catalog" title="Refresh" disabled={loading} onClick={() => void usePacks.getState().load(true)}>
            <Icon id="refresh" />
          </Button>
        }
        foot={
          <>
            <Button quiet onClick={() => setIndex(true)}>
              Change index…
            </Button>
            <span className="m-t-meta ig-url" title={url}>
              {cat?.name ? `${cat.name} · ${url}` : url}
            </span>
          </>
        }
      >
        {packs.length > 0 ? (
          <div className="ig-packs" data-testid="pack-cards">
            {packs.map((p) => (
              <PackCard key={p.slug} p={p} />
            ))}
          </div>
        ) : (
          <div className={`m-t-meta${catalogue?.error ? ' is-bad' : ''}`} role="status">
            {catalogue?.error ? catalogue.error : loading || !loaded ? 'Fetching the catalog…' : 'The catalog lists no packs.'}
          </div>
        )}
        {catalogue?.error && packs.length > 0 && (
          <div className="m-t-meta is-bad ig-after" role="status">
            {catalogue.error}
          </div>
        )}
      </Section>

      <Section
        title="Share"
        testId="marketplace-share"
        foot={
          <>
            <Button onClick={() => file.current?.click()}>Import pack…</Button>
            <Button onClick={() => setCreating(true)}>Create pack…</Button>
          </>
        }
      >
        <div className="ig-pad m-t-meta">A pack is one JSON file of agents, automations and theme packs. Export pack on a row writes one to Downloads; drop one here to preview it before anything is installed.</div>
        {error && !preview && (
          <div className="ig-pad m-t-meta is-bad" role="alert">
            {error}
          </div>
        )}
        {result && (
          <div className="ig-pad m-t-meta" role="status" data-testid="pack-result">
            {result.message}
          </div>
        )}
      </Section>

      <input
        ref={file}
        type="file"
        accept=".json,application/json"
        hidden
        aria-label="Pack file"
        data-testid="pack-file"
        onChange={(e) => {
          void readFile(e.target.files?.[0])
          e.target.value = ''
        }}
      />
      {preview && <PreviewDialog p={preview} />}
      {creating && <CreateDialog onClose={() => setCreating(false)} />}
      {index && <IndexDialog onClose={closeIndex} />}
    </div>
  )
}


/* ---- the kind marks (03 §7C.6): the bot wearing its accessory for an agent, the alarm clock for an automation, the palette for a theme ---- */

/** The six swatch colors in the order the index lists them. */
export function swatchColors(s: Swatch | undefined | null): string[] | null {
  return s ? [s.bg, s.surface, s.fg, s.accent, s.ok, s.info] : null
}

export function KindMark({ kind, variant, hueKey, label }: { kind: ItemKind; variant?: string | null; hueKey?: string; label?: string }) {
  const art = useArtwork()
  if (kind === 'agent') return <AgentAvatar variant={variant ?? ''} hue={agentHue(hueKey ?? 'agent')} className="ig-kind-agent" />
  return (
    <span className="ig-kind" role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true} data-testid={`kind-${kind}`}>
      <img src={art.kinds[kind]} alt="" />
    </span>
  )
}

/* ---- the catalog cards ------------------------------------------------------------- */

const WORD: Record<ItemKind, [string, string]> = { agent: ['agent', 'agents'], automation: ['automation', 'automations'], theme: ['theme', 'themes'] }
const count = (kind: ItemKind, n: number) => `${n} ${WORD[kind][n === 1 ? 0 : 1]}`

/** One pack of the index: name, "by author · v1", the description, the count chips led by their kind marks, Install. */
function PackCard({ p }: { p: CatalogueEntry }) {
  const swatch = swatchOfList(p.colors)
  const meta = entryMeta(p)
  return (
    <article className="m-card ig-pack" data-testid={`pack-${p.slug}`} aria-label={p.name}>
      <div className="m-card-title">{p.name}</div>
      {meta && <div className="m-meta">{meta}</div>}
      {p.description && (
        <p className="ig-pack-desc" title={p.description}>
          {p.description}
        </p>
      )}
      <div className="ig-pack-foot">
        <span className="ig-counts" aria-label={p.counts}>
          {p.agents > 0 && (
            <span className="ig-count">
              <KindMark kind="agent" variant={p.avatars[0]} hueKey={p.slug} />
              {count('agent', p.agents)}
            </span>
          )}
          {p.automations > 0 && (
            <span className="ig-count">
              <KindMark kind="automation" />
              {count('automation', p.automations)}
            </span>
          )}
          {p.themes > 0 && (
            <span className="ig-count">
              <KindMark kind="theme" />
              {count('theme', p.themes)}
            </span>
          )}
          {p.themes > 0 && swatch && <SwatchDots swatch={swatch} className="ig-pack-dots" />}
        </span>
        <Button onClick={() => void usePacks.getState().previewUrl(p.url)}>Install</Button>
      </div>
    </article>
  )
}

/* ---- the preview dialog ---------------------------------------------------------------- */

function PreviewDialog({ p }: { p: PackPreview }) {
  const installing = usePacks((s) => s.installing)
  const error = usePacks((s) => s.error)
  const close = () => usePacks.getState().cancel()
  const invalid = !p.ok
  const title = invalid ? 'Not a pack' : p.url ? `Install ${p.name ?? 'pack'}` : `Import ${p.name ?? 'pack'}`
  const hasConflict = p.items.some((i) => i.existing)
  const line = packLine(p)
  return (
    <Dialog title={title} onClose={close} testId="pack-preview">
      {invalid ? (
        <div className="ap-dialog-note is-bad" role="alert">
          {p.message ?? `${p.source} is not a Mode pack.`}
        </div>
      ) : (
        <div className="ig-preview">
          <div className="ig-preview-head">
            {line && <div className="m-t-meta">{line}</div>}
            {p.description && <div className="m-fg-2 ig-scope">{p.description}</div>}
            <div className="m-t-meta ig-url" title={p.source}>
              {p.source}
            </div>
          </div>
          {KINDS.map((kind) => {
            const items = p.items.filter((i) => i.kind === kind)
            if (!items.length) return null
            return (
              <div key={kind} className="ig-items" data-testid={`pack-items-${kind}`}>
                <div className="m-group-label">{KIND_LABEL[kind]}</div>
                <SourceHead labels={['Install']} />
                {items.map((i) => (
                  <ItemRow key={itemTag(i)} i={i} p={p} />
                ))}
              </div>
            )
          })}
          {hasConflict && (
            <div className="m-inline ig-conflict">
              <span className="m-t-meta">Already here:</span>
              <Seg label="When an item is already here">
                <SegItem active={p.conflict === 'keep_both'} onClick={() => usePacks.getState().setConflict('keep_both')}>
                  Keep both
                </SegItem>
                <SegItem active={p.conflict === 'replace'} onClick={() => usePacks.getState().setConflict('replace')}>
                  Replace
                </SegItem>
              </Seg>
            </div>
          )}
          {error && (
            <div className="ap-dialog-note is-bad" role="alert">
              {error}
            </div>
          )}
        </div>
      )}
      <div className="m-card-actions">
        {!invalid && (
          <Button variant="primary" disabled={installing || !canInstall(p)} onClick={() => void usePacks.getState().install()}>
            {installing ? 'Installing…' : 'Install'}
          </Button>
        )}
        <Button onClick={close}>{invalid ? 'Close' : 'Cancel'}</Button>
      </div>
    </Dialog>
  )
}

function ItemRow({ i, p }: { i: ItemPreview; p: PackPreview }) {
  const on = !!p.on[itemTag(i)]
  const ok = installable(i)
  const note = itemNote(i, p.conflict)
  return (
    <>
      <SourceRow
        name={
          <span className="ig-row">
            <KindMark kind={i.kind} variant={i.avatar_variant} hueKey={i.key} />
            <span className="ig-item">
              <span className="m-row-name">{i.name}</span>
              <span className="m-t-meta ig-item-line" title={i.detail}>
                {i.detail}
              </span>
              {i.kind === 'theme' && i.swatches && <SwatchDots swatch={i.swatches.light ?? i.swatches.dark} className="ig-item-dots" />}
              <span className={`m-t-meta${ok ? '' : ' is-bad'}`}>{note}</span>
            </span>
          </span>
        }
        cells={[<Switch key="on" on={on && ok} disabled={!ok} label={`Install ${i.name}`} onChange={(v) => usePacks.getState().toggle(i, v)} />]}
      />
      {i.needs_folder && on && ok && (
        <div className="ig-folder">
          <CommitInput label={`Folder for ${i.name}`} value={p.folders[i.key] ?? ''} placeholder="/path/to/repository" className="ig-wide" onCommit={(v) => usePacks.getState().setFolder(i.key, String(v ?? ''))} />
        </div>
      )}
    </>
  )
}

/* ---- Create pack… --------------------------------------------------------------------- */

function CreateDialog({ onClose }: { onClose: () => void }) {
  const agents = useAgents((s) => s.rows)
  const agentsLoaded = useAgents((s) => s.loaded)
  const automations = useAutomations((s) => s.rows)
  const automationsLoaded = useAutomations((s) => s.loaded)
  const themes = useThemes((s) => s.themes)
  const themesLoaded = useThemes((s) => s.loaded)
  const [name, setName] = useState('')
  const [author, setAuthor] = useState('')
  const [description, setDescription] = useState('')
  const [on, setOn] = useState<Record<string, boolean>>({})
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!agentsLoaded) void useAgents.getState().load()
    if (!automationsLoaded) void useAutomations.getState().load()
    if (!themesLoaded) void useThemes.getState().load()
  }, [agentsLoaded, automationsLoaded, themesLoaded])
  const chosen = (prefix: string) =>
    Object.entries(on)
      .filter(([k, v]) => v && k.startsWith(prefix))
      .map(([k]) => k.slice(prefix.length))
  const count = Object.values(on).filter(Boolean).length
  const userThemes = themes.filter((t) => t.kind !== 'builtin')
  const toggle = (tag: string, v: boolean) => setOn((o) => ({ ...o, [tag]: v }))
  const save = async () => {
    setBusy(true)
    const out = await usePacks.getState().exportPack({
      agents: chosen('agent:'),
      automations: chosen('automation:'),
      themes: chosen('theme:'),
      name: name.trim() || undefined,
      author: author.trim() || undefined,
      description: description.trim() || undefined,
    })
    setBusy(false)
    useSessionsStore.getState().showNotice(out ? `Exported ${out.name} (${out.counts}) to ${out.path}` : `Couldn't create the pack: ${usePacks.getState().error ?? 'unknown error'}`)
    if (out) onClose()
  }
  return (
    <Dialog title="Create pack" onClose={onClose} testId="pack-create">
      <Form label="Pack">
        <FormField label="Name" htmlFor="pack-name">
          <input id="pack-name" className="m-input" value={name} placeholder="Reviewer starter" onChange={(e) => setName(e.target.value)} />
        </FormField>
        <FormField label="Author" htmlFor="pack-author">
          <input id="pack-author" className="m-input" value={author} placeholder="optional" onChange={(e) => setAuthor(e.target.value)} />
        </FormField>
        <FormField label="Description" htmlFor="pack-description">
          <input id="pack-description" className="m-input" value={description} placeholder="optional" onChange={(e) => setDescription(e.target.value)} />
        </FormField>
      </Form>
      <div className="ig-picker ig-create">
        <PickGroup label="Agents" rows={agents.map((a: AgentSummary) => ({ tag: `agent:${a.id}`, name: a.name, line: a.title ?? '', mark: <KindMark kind="agent" variant={a.avatar_variant} hueKey={a.id} /> }))} on={on} toggle={toggle} empty="No agents yet." />
        <PickGroup label="Automations" rows={automations.map((a: AutomationRow) => ({ tag: `automation:${a.id}`, name: a.name, line: `${a.trigger_phrase} · ${a.action_phrase}`, mark: <KindMark kind="automation" /> }))} on={on} toggle={toggle} empty="No automations yet." />
        <PickGroup label="Themes" rows={userThemes.map((t: ThemeRow) => ({ tag: `theme:${t.id}`, name: t.name, line: t.appearance, mark: <KindMark kind="theme" />, dots: <SwatchDots swatch={t.swatches.light ?? t.swatches.dark} className="ig-item-dots" /> }))} on={on} toggle={toggle} empty="No imported or custom packs; the built-ins ship with Mode." />
      </div>
      <div className="m-card-actions">
        <Button variant="primary" disabled={busy || count === 0} onClick={() => void save()}>
          {busy ? 'Writing…' : count ? `Export ${count} ${count === 1 ? 'item' : 'items'}` : 'Export'}
        </Button>
        <Button onClick={onClose}>Cancel</Button>
      </div>
    </Dialog>
  )
}

function PickGroup({ label, rows, on, toggle, empty }: { label: string; rows: { tag: string; name: string; line: string; mark: ReactNode; dots?: ReactNode }[]; on: Record<string, boolean>; toggle: (tag: string, v: boolean) => void; empty: string }) {
  return (
    <div className="ig-items">
      <div className="m-group-label">{label}</div>
      {rows.length ? (
        <>
          <SourceHead labels={['Include']} />
          {rows.map((r) => (
            <SourceRow
              key={r.tag}
              name={
                <span className="ig-row">
                  {r.mark}
                  <span className="ig-item">
                    <span className="m-row-name">{r.name}</span>
                    {r.line && (
                      <span className="m-t-meta ig-item-line" title={r.line}>
                        {r.line}
                      </span>
                    )}
                    {r.dots}
                  </span>
                </span>
              }
              cells={[<Switch key="on" on={!!on[r.tag]} label={`Include ${r.name}`} onChange={(v) => toggle(r.tag, v)} />]}
            />
          ))}
        </>
      ) : (
        <div className="ig-pad m-t-meta">{empty}</div>
      )}
    </div>
  )
}

/* ---- Change index… (`integrations/marketplace/index`) ---------------------------------- */

/** The catalog address (`marketplace.index_url`): an https index; a change fetches again. */
export function IndexDialog({ onClose }: { onClose: () => void }) {
  const [url, setUrl] = useSetting<string>('marketplace.index_url')
  const [problem, setProblem] = useState<string | null>(null)
  return (
    <Dialog title="Catalog index" onClose={onClose} testId="pack-index">
      <Form label="Catalog">
        <Group id="catalogue">catalog</Group>
        <Field label="Index address">
          <CommitInput
            label="Catalog index address"
            value={url ?? ''}
            placeholder="https://…/index.json"
            className="ig-wide"
            testId="pack-index-url"
            onCommit={(v) => {
              const next = String(v ?? '').trim()
              void setUrl(next).then((ok) => {
                setProblem(ok ? null : 'expects an https:// address')
                if (ok) void usePacks.getState().load(true)
              })
            }}
          />
        </Field>
        <Note bad={!!problem}>{problem ?? 'An https address of a catalog index; the packs it lists must be on the same host. Mode fetches it and sends nothing.'}</Note>
      </Form>
      <div className="m-card-actions">
        <Button onClick={onClose}>Close</Button>
      </div>
    </Dialog>
  )
}
