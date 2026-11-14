// Settings (03 §8; 10 §7.4.19): the title, then the page list (the filter
// field on top, eleven pages in three groups) and the page column (the page
// title, groups of cards and rows). Every control reads and
// writes one registry key through the settings store (changes apply at once;
// no Save button). `settings/<page>` selects a page; `settings/<page>/<section>`
// scrolls to a section (`advanced/logs`) or, for `general/name`, focuses the
// profile name (the profile menu's Edit profile). `settings/alerts/new` is a
// full page instead (03 §8.11 "New rule", the drill-in rule of 03 §2.4): the
// custom alert rule editor under its own `‹` + breadcrumb head.
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react'
import { Button } from '../../kit/button'
import { MainBody, MainHead } from '../../kit/layout'
import { SettingGroup, SettingsNav, SettingsPage } from '../../kit/settings'
import { useSettingsStore } from '../../state/settings-store'
import { navigate } from '../router'
import { AdvancedPage } from './AdvancedPage'
import { AiPage } from './AiPage'
import { AlertRuleEditor } from './AlertRuleEditor'
import { AlertsPage } from './AlertsPage'
import { AppearancePage } from './AppearancePage'
import { ChatPage } from './ChatPage'
import { DictationPage } from './DictationPage'
import { GeneralPage } from './GeneralPage'
import { NotesPage } from './NotesPage'
import { OverlayPage } from './OverlayPage'
import { NAV_GROUPS, PAGES, parseSettingsRest, searchSettings, type IndexEntry, type PageId } from './pages'
import { PrivacyPage } from './PrivacyPage'
import { UrgencyPage } from './UrgencyPage'
import './settings.css'

const PAGE_BODY: Record<PageId, (p: { section?: string }) => ReactNode> = {
  general: (p) => <GeneralPage {...p} />,
  appearance: () => <AppearancePage />,
  overlay: () => <OverlayPage />,
  dictation: () => <DictationPage />,
  chat: () => <ChatPage />,
  alerts: () => <AlertsPage />,
  notes: () => <NotesPage />,
  urgency: () => <UrgencyPage />,
  ai: () => <AiPage />,
  privacy: () => <PrivacyPage />,
  advanced: () => <AdvancedPage />,
}

/** Pages whose content is wider than the 680-px column (the themes gallery, the logs). */
const WIDE: ReadonlySet<PageId> = new Set(['appearance', 'advanced'])

/** Under this width of the view the page list shows icons only (10 §7.4.19). */
const NAV_COLLAPSE_PX = 720

export function SettingsView({ rest }: { rest?: string }) {
  const { page, section } = parseSettingsRest(rest)
  if (page === 'alerts' && section === 'new') return <AlertRuleEditor />
  return <SettingsPages page={page} section={section} />
}

/** True while `el` is narrower than `px` (a ResizeObserver; the kit's `.m-snav.is-collapsed`). */
function useNarrow(el: RefObject<HTMLElement | null>, px: number): boolean {
  const [narrow, setNarrow] = useState(false)
  useLayoutEffect(() => {
    const node = el.current
    if (!node) return
    const measure = () => setNarrow(node.getBoundingClientRect().width < px)
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(measure)
    ro.observe(node)
    return () => ro.disconnect()
  }, [el, px])
  return narrow
}

function SettingsPages({ page, section }: { page: PageId; section?: string }) {
  const loaded = useSettingsStore((s) => s.loaded)
  const loadError = useSettingsStore((s) => s.loadError)
  const [filter, setFilter] = useState('')
  const [searching, setSearching] = useState(false)
  const filterRef = useRef<HTMLInputElement>(null)
  const viewRef = useRef<HTMLElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const hits = useMemo(() => searchSettings(filter), [filter])
  const narrow = useNarrow(viewRef, NAV_COLLAPSE_PX)
  // A narrow view keeps the list to icons, except while the filter is in use.
  const collapsed = narrow && !searching && !filter

  useEffect(() => {
    if (!useSettingsStore.getState().loaded) void useSettingsStore.getState().load()
  }, [])

  // ⌘/Ctrl+F filters pages and controls (03 §10.7); the field lives in the page list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'f') {
        e.preventDefault()
        setSearching(true)
        requestAnimationFrame(() => {
          filterRef.current?.focus()
          filterRef.current?.select()
        })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Scroll to the section (or focus the field) a route names.
  useEffect(() => {
    if (!section || !loaded || filter) return
    const t = window.setTimeout(() => {
      const target = document.getElementById(`settings-${section}`) ?? bodyRef.current?.querySelector<HTMLElement>(`[data-setting="${section}"]`)
      target?.scrollIntoView?.({ block: 'start' })
      if (page === 'general' && section === 'name') document.querySelector<HTMLInputElement>('[data-testid="profile-name"]')?.focus()
    }, 0)
    return () => window.clearTimeout(t)
  }, [page, section, loaded, filter])

  const open = (x: IndexEntry) => {
    setFilter('')
    navigate({ view: 'settings', rest: x.section ? `${x.page}/${x.section}` : x.page })
  }
  const label = PAGES.find((p) => p.id === page)!.label

  const search = (
    <input
      ref={filterRef}
      className="m-input"
      placeholder="Filter settings…"
      aria-label="Filter settings"
      value={filter}
      onChange={(e) => setFilter(e.target.value)}
      onFocus={() => setSearching(true)}
      onBlur={() => setSearching(false)}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation()
          if (filter) setFilter('')
          else e.currentTarget.blur()
        } else if (e.key === 'Enter' && hits[0]) {
          e.preventDefault()
          open(hits[0])
        } else if (e.key === 'ArrowDown' && hits.length) {
          e.preventDefault()
          bodyRef.current?.querySelector<HTMLElement>('.sg-hit')?.focus()
        }
      }}
    />
  )

  return (
    <section ref={viewRef} className="ap-view sg-view" aria-label="Settings" data-testid="settings-view">
      <MainHead title="Settings" titleTestId="view-title" />
      <div className="sg-split">
        <SettingsNav label="Settings pages" active={page} onSelect={(id) => navigate({ view: 'settings', rest: id })} search={search} groups={NAV_GROUPS} collapsed={collapsed} />
        <MainBody className="sg-body" testId="settings-body">
          <div ref={bodyRef} className="sg-column" data-page={filter ? 'search' : page}>
            {filter ? (
              <SettingsPage title="Search">
                <Hits hits={hits} query={filter} onOpen={open} />
              </SettingsPage>
            ) : (
              <SettingsPage key={page} title={label} wide={WIDE.has(page)}>
                {loadError && (
                  <div className="ap-banner" role="status">
                    <span>Settings could not be read; showing the defaults.</span>
                    <Button quiet onClick={() => void useSettingsStore.getState().load()}>
                      Retry
                    </Button>
                  </div>
                )}
                {loaded ? PAGE_BODY[page]({ section }) : <Skeleton />}
              </SettingsPage>
            )}
          </div>
        </MainBody>
      </div>
    </section>
  )
}

function Hits({ hits, query, onOpen }: { hits: IndexEntry[]; query: string; onOpen: (x: IndexEntry) => void }) {
  if (!hits.length) return <div className="ap-empty">{`No setting matches “${query}”.`}</div>
  return (
    <SettingGroup title={hits.length === 1 ? '1 match' : `${hits.length} matches`}>
      <div
        className="sg-hits"
        role="listbox"
        aria-label="Matching settings"
        onKeyDown={(e) => {
          if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
          const rows = Array.from((e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('.sg-hit'))
          const i = rows.indexOf(document.activeElement as HTMLElement)
          e.preventDefault()
          rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus()
        }}
      >
        {hits.map((x) => (
          <button key={`${x.page}:${x.label}`} type="button" className="m-set-row sg-hit" role="option" aria-selected={false} onClick={() => onOpen(x)}>
            <span className="m-set-text">
              <span className="m-set-title">{x.label}</span>
            </span>
            <span className="m-set-ctl m-set-unit">{PAGES.find((p) => p.id === x.page)!.label}</span>
          </button>
        ))}
      </div>
    </SettingGroup>
  )
}

/** Skeleton rows after 150 ms so a fast load never flashes (03 §9). */
function Skeleton() {
  const [show, setShow] = useState(false)
  useEffect(() => {
    const t = window.setTimeout(() => setShow(true), 150)
    return () => window.clearTimeout(t)
  }, [])
  if (!show) return null
  return (
    <div className="sg-skeleton" aria-busy="true" aria-label="Loading settings">
      <i />
      <i />
      <i />
    </div>
  )
}
