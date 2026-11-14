// Settings → Appearance, the themes gallery (03 §8.5; 10 §4.4–§4.5, §5):
// one card per pack (System, Paper, Gruvbox Dark, Gruvbox Light, Cursor Dark,
// Nord, Catppuccin Mocha, then imported packs) with a preview strip and six
// swatches drawn from the pack's own colors, the active pack selected by
// background; `theme_set_active` on click; Import pack… (a file or a dropped
// `.json`) validates and previews before anything is saved, then Import /
// Replace / Keep both; Duplicate (`theme_duplicate`, any pack, from the
// row's ⋮ for the active one), Export and Remove for imported packs.
//
// Package 11 builds this section for the Appearance page to mount (the page
// is the reconciliation pass's): `<ThemeSection />` in place of the v0.1
// one-card gallery, and `themeMenuEntries()` for the row's ⋮. It reuses the
// page's `sg-*` gallery classes (settings.css) and sets each card's swatch
// tokens as custom properties (dynamic values).
import { useEffect, useRef, useState, type DragEvent } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { cx } from '../../kit/cx'
import type { ThemeRow } from '../../state/contract-themes'
import { packMeta, sortPacks, useThemes } from '../../state/theme-store'
import { useSessionsStore } from '../../state/sessions-store'
import { openMenu, type MenuEntry } from '../ui'
import { SwatchDots, ThemeSwatch } from '../../kit/swatch'

const IMPORT_EVENT = 'mode:theme-import'

async function exportPack(t: ThemeRow) {
  const path = await useThemes.getState().exportPack(t.id)
  useSessionsStore.getState().showNotice(path ? `Exported ${t.name} to ${path}` : `Couldn't export ${t.name}`)
}

/** 10 §4.5 Duplicate: a `custom` copy (built-ins too) that can be edited and removed. */
async function duplicatePack(t: ThemeRow) {
  const row = await useThemes.getState().duplicate(t.id)
  useSessionsStore.getState().showNotice(row ? `Duplicated ${t.name} as ${row.name}` : `Couldn't duplicate ${t.name}`)
}

async function removePack(t: ThemeRow) {
  const ok = await useThemes.getState().remove(t.id)
  useSessionsStore.getState().showNotice(ok ? `Removed ${t.name}` : `Couldn't remove ${t.name}`)
}

/** The Appearance row's ⋮: Import pack…, Duplicate pack, Export pack, Remove pack (built-ins cannot be removed). */
export function themeMenuEntries(): MenuEntry[] {
  const active = useThemes.getState().themes.find((t) => t.active)
  const own = !!active && active.kind !== 'builtin'
  return [
    { id: 'import', label: 'Import pack…', run: () => window.dispatchEvent(new Event(IMPORT_EVENT)) },
    { id: 'duplicate', label: 'Duplicate pack', dimmed: !active, run: () => active && void duplicatePack(active) },
    { id: 'export', label: 'Export pack', dimmed: !active, run: () => active && void exportPack(active) },
    { id: 'remove', label: 'Remove pack', danger: own, dimmed: !own, reason: own ? undefined : 'Built-in packs cannot be removed', run: () => active && own && void removePack(active) },
  ]
}

function ThemeCard({ t }: { t: ThemeRow }) {
  const [hover, setHover] = useState(false)
  const primary = t.swatches.light ?? t.swatches.dark
  const shown = hover && t.swatches.light && t.swatches.dark ? t.swatches.dark : primary
  const pick = () => void useThemes.getState().setActive(t.id)
  return (
    <div
      className={cx('m-card', t.active && 'is-selected')}
      role="radio"
      aria-checked={t.active}
      aria-label={`${t.name}, ${packMeta(t)}`}
      tabIndex={0}
      onClick={pick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          pick()
        }
      }}
      data-testid={`theme-${t.slug}`}
    >
      <ThemeSwatch swatch={shown} />
      <div className="m-card-title sg-card-title">{t.name}</div>
      <div className="m-meta">{packMeta(t)}</div>
      <div className="sg-dots">
        <SwatchDots swatch={shown} />
        {t.kind !== 'builtin' && (
          <MenuButton
            label={`${t.name} options`}
            onClick={(e) => {
              e.stopPropagation()
              openMenu(
                e.currentTarget,
                [
                  { id: 'duplicate', label: 'Duplicate', run: () => void duplicatePack(t) },
                  { id: 'export', label: 'Export JSON', run: () => void exportPack(t) },
                  { id: 'delete', label: 'Delete', danger: true, run: () => void removePack(t) },
                ],
                t.name,
              )
            }}
          />
        )}
      </div>
    </div>
  )
}

export function ThemeSection() {
  const themes = useThemes((s) => s.themes)
  const loaded = useThemes((s) => s.loaded)
  const resolved = useThemes((s) => s.resolved)
  const preview = useThemes((s) => s.preview)
  const error = useThemes((s) => s.error)
  const file = useRef<HTMLInputElement>(null)
  const [dragging, setDragging] = useState(false)

  useEffect(() => {
    void useThemes.getState().load()
    const open = () => file.current?.click()
    window.addEventListener(IMPORT_EVENT, open)
    return () => window.removeEventListener(IMPORT_EVENT, open)
  }, [])

  const readFile = async (f: File | undefined | null) => {
    if (!f) return
    const text = await f.text()
    await useThemes.getState().previewImport(text, f.name)
  }
  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    setDragging(false)
    const f = Array.from(e.dataTransfer.files).find((x) => /\.json$/i.test(x.name) || x.type === 'application/json')
    void readFile(f)
  }

  const packs = sortPacks(themes)
  const imported = packs.filter((t) => t.kind !== 'builtin')
  const errors = preview?.issues.filter((i) => i.severity === 'error') ?? []
  const warnings = preview?.issues.filter((i) => i.severity === 'warning') ?? []

  return (
    <div className="sg-theme-section" data-testid="theme-section">
      <div
        className={cx('sg-themes', dragging && 'is-drop')}
        id="settings-themes"
        data-setting="theme.active"
        role="radiogroup"
        aria-label="Theme"
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
      >
        {packs.map((t) => (
          <ThemeCard key={t.id} t={t} />
        ))}
        {preview && preview.ok && (
          <div className="m-card" aria-label={`Preview of ${preview.name}`} data-testid="theme-preview">
            <ThemeSwatch swatch={preview.swatches?.light ?? preview.swatches?.dark} />
            <div className="m-card-title sg-card-title">{preview.name}</div>
            <div className="m-meta">preview · not saved</div>
            <div className="sg-dots">
              <SwatchDots swatch={preview.swatches?.light ?? preview.swatches?.dark} />
            </div>
          </div>
        )}
      </div>
      {resolved?.notices?.map((n) => (
        <div key={n} className="m-t-meta sg-after">
          {n}
        </div>
      ))}
      {resolved?.accent_adjusted && <div className="m-t-meta sg-after">accent adjusted for contrast</div>}
      {preview && (
        <div className="sg-after" role="status" data-testid="theme-import">
          {errors.length > 0 ? (
            <div className="m-fg-2">
              {`Could not import ${preview.file}: ${errors.length} ${errors.length === 1 ? 'error' : 'errors'}`}
            </div>
          ) : warnings.length > 0 ? (
            <div className="m-fg-2">
              {`${preview.name ?? ''} has ${warnings.length} contrast ${warnings.length === 1 ? 'warning' : 'warnings'}; text may be hard to read.`}
            </div>
          ) : (
            <div className="m-fg-2">{`${preview.name ?? ''} is ready to import.`}</div>
          )}
          {preview.issues
            .filter((i) => i.severity !== 'fill')
            .map((i) => (
              <div key={`${i.path}:${i.message}`} className="m-t-meta">
                <code>{i.path}</code>
                {` ${i.message}`}
              </div>
            ))}
          <div className="m-card-actions">
            {errors.length === 0 && !preview.conflict && (
              <Button variant="primary" onClick={() => void useThemes.getState().confirmImport()}>
                {warnings.length ? 'Import anyway' : 'Import'}
              </Button>
            )}
            {errors.length === 0 && preview.conflict && (
              <>
                <Button variant="primary" onClick={() => void useThemes.getState().confirmImport('replace')}>
                  Replace
                </Button>
                <Button onClick={() => void useThemes.getState().confirmImport('keep_both')}>Keep both</Button>
              </>
            )}
            <Button onClick={() => useThemes.getState().cancelImport()}>Cancel</Button>
          </div>
        </div>
      )}
      {error && !preview && (
        <div className="m-t-meta sg-after" role="alert">
          {error}
        </div>
      )}
      {loaded && !imported.length && !preview && (
        <div className="m-t-meta sg-after">
          {'Import a pack to see it here. Packs are JSON files with tokens only. '}
          <Button quiet onClick={() => file.current?.click()}>
            Import pack…
          </Button>
        </div>
      )}
      <input
        ref={file}
        type="file"
        accept=".json,application/json"
        hidden
        aria-label="Theme pack file"
        onChange={(e) => {
          void readFile(e.target.files?.[0])
          e.target.value = ''
        }}
      />
    </div>
  )
}
