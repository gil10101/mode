// Filters (03 §4.4): level (the three UI levels and "not yet scored"; no tier
// words, 10 §6.2), contact, topic (free text over messages_fts), channel (the
// account_sources picker), read state and date. Applied filters become
// removable chips in the toolbar; ⋮ → Save filter stores the set under a
// name in `inbox.saved_filters`.
import { useState } from 'react'
import { Button } from '../../kit/button'
import { Seg, SegItem, Select } from '../../kit/controls'
import { SettingGroup, SettingRow } from '../../kit/settings'
import type { InboxFilter, SourceRow, Tier } from '../../state/contract-connectors'
import { useAccounts } from '../../state/accounts-store'
import { knownSenders, useInbox, type SavedFilter } from '../../state/inbox-store'
import { useSetting } from '../../state/settings-store'
import { Dialog } from '../ui'

export const LEVEL_FILTERS: readonly { id: string; label: string; tiers: Tier[] }[] = [
  { id: 'urgent', label: 'urgent', tiers: ['vip', 'direct', 'ai-urgent'] },
  { id: 'normal', label: 'normal', tiers: ['ai-normal'] },
  { id: 'low', label: 'low', tiers: ['low'] },
  { id: 'unscored', label: 'not yet scored', tiers: ['unscored'] },
]

export function levelIds(tiers: readonly Tier[] | undefined): string[] {
  if (!tiers?.length) return []
  return LEVEL_FILTERS.filter((l) => l.tiers.every((t) => tiers.includes(t))).map((l) => l.id)
}

export function sourceLabel(s: SourceRow, accountName: string | undefined): string {
  const name = s.kind === 'slack_dm' ? `DM · ${s.name}` : s.kind === 'slack_mpim' ? `group DM · ${s.name}` : s.kind === 'slack_mentions' ? 'mentions' : s.name
  return accountName ? `${name} · ${accountName}` : name
}

/** The toolbar chips for the active filters, each removable (03 §2.4 chips). */
export function filterChips(f: InboxFilter, names: { contact: (id: string) => string; source: (id: string) => string }): { key: string; label: string; remove: () => InboxFilter }[] {
  const out: { key: string; label: string; remove: () => InboxFilter }[] = []
  const levels = levelIds(f.tiers)
  for (const id of levels) {
    const l = LEVEL_FILTERS.find((x) => x.id === id)!
    out.push({ key: `level-${id}`, label: `level: ${l.label}`, remove: () => ({ ...f, tiers: (f.tiers ?? []).filter((t) => !l.tiers.includes(t)) }) })
  }
  if (f.contact_id) out.push({ key: 'contact', label: `from: ${names.contact(f.contact_id)}`, remove: () => ({ ...f, contact_id: null }) })
  if (f.query) out.push({ key: 'topic', label: `topic: ${f.query}`, remove: () => ({ ...f, query: null }) })
  if (f.source_id) out.push({ key: 'source', label: `in: ${names.source(f.source_id)}`, remove: () => ({ ...f, source_id: null }) })
  if (f.unread_only) out.push({ key: 'unread', label: 'unread', remove: () => ({ ...f, unread_only: false }) })
  if (f.date) out.push({ key: 'date', label: f.date === 'today' ? 'today' : f.date === 'week' ? 'this week' : 'date range', remove: () => ({ ...f, date: null }) })
  if (f.archived) out.push({ key: 'archived', label: 'with archived', remove: () => ({ ...f, archived: false }) })
  return out
}

export function FiltersDialog({ onClose }: { onClose: () => void }) {
  const filter = useInbox((s) => s.filter)
  const rows = useInbox((s) => s.rows)
  const sources = useAccounts((s) => s.sources)
  const accounts = useAccounts((s) => s.accounts)
  const [levels, setLevels] = useState<string[]>(levelIds(filter.tiers))
  const [contact, setContact] = useState(filter.contact_id ?? '')
  const [topic, setTopic] = useState(filter.query ?? '')
  const [source, setSource] = useState(filter.source_id ?? '')
  const [unread, setUnread] = useState(!!filter.unread_only)
  const [date, setDate] = useState<'any' | 'today' | 'week'>(filter.date === 'today' || filter.date === 'week' ? filter.date : 'any')
  const senders = knownSenders(rows)
  const watched = sources.filter((s) => s.enabled && s.kind !== 'calendar')

  const apply = () => {
    const next: InboxFilter = {
      tiers: LEVEL_FILTERS.filter((l) => levels.includes(l.id)).flatMap((l) => l.tiers),
      contact_id: contact || null,
      query: topic.trim() || null,
      source_id: source || null,
      unread_only: unread,
      date: date === 'any' ? null : date,
      archived: filter.archived,
    }
    useInbox.getState().setFilter(next)
    onClose()
  }

  return (
    <Dialog title="Filter messages" onClose={onClose} testId="inbox-filters">
      <div className="ib-filters">
        <SettingGroup compact>
          <SettingRow title="Level" stack>
            <div className="aw-chips" role="group" aria-label="Level">
              {LEVEL_FILTERS.map((l) => (
                <span
                  key={l.id}
                  className={`m-chip ib-chip${levels.includes(l.id) ? ' is-on' : ''}`}
                  role="checkbox"
                  aria-checked={levels.includes(l.id)}
                  tabIndex={0}
                  onClick={() => setLevels((v) => (v.includes(l.id) ? v.filter((x) => x !== l.id) : [...v, l.id]))}
                  onKeyDown={(e) => {
                    if (e.key === ' ' || e.key === 'Enter') {
                      e.preventDefault()
                      setLevels((v) => (v.includes(l.id) ? v.filter((x) => x !== l.id) : [...v, l.id]))
                    }
                  }}
                >
                  {l.label}
                </span>
              ))}
            </div>
          </SettingRow>
          <SettingRow title="From" htmlFor="ib-f-contact">
            <Select
              id="ib-f-contact"
              label="From"
              value={contact}
              options={[['', 'anyone'] as const, ...senders.map((s) => [s.contact_id ?? s.address ?? s.name, `${s.name}${s.address ? ` · ${s.address}` : ''}`] as const)]}
              onChange={setContact}
            />
          </SettingRow>
          <SettingRow title="Topic" htmlFor="ib-f-topic">
            <input id="ib-f-topic" className="m-input" placeholder="words in the subject or text" value={topic} onChange={(e) => setTopic(e.target.value)} />
          </SettingRow>
          <SettingRow title="Channel" htmlFor="ib-f-source">
            <Select
              id="ib-f-source"
              label="Channel"
              value={source}
              options={[['', 'every channel'] as const, ...watched.map((s) => [s.id, sourceLabel(s, accounts.find((a) => a.id === s.account_id)?.display_name)] as const)]}
              onChange={setSource}
            />
          </SettingRow>
          <SettingRow title="Read state">
            <Seg label="Read state">
              <SegItem active={!unread} onClick={() => setUnread(false)}>
                All
              </SegItem>
              <SegItem active={unread} onClick={() => setUnread(true)}>
                Unread
              </SegItem>
            </Seg>
          </SettingRow>
          <SettingRow title="Date">
            <Seg label="Date">
              <SegItem active={date === 'any'} onClick={() => setDate('any')}>
                Any time
              </SegItem>
              <SegItem active={date === 'today'} onClick={() => setDate('today')}>
                Today
              </SegItem>
              <SegItem active={date === 'week'} onClick={() => setDate('week')}>
                This week
              </SegItem>
            </Seg>
          </SettingRow>
        </SettingGroup>
        <div className="m-card-actions">
          <Button
            quiet
            onClick={() => {
              useInbox.getState().clearFilters()
              onClose()
            }}
          >
            Clear filters
          </Button>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={apply}>
            Apply
          </Button>
        </div>
      </div>
    </Dialog>
  )
}

export function SaveFilterDialog({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState('')
  const [saved, setSaved] = useSetting<SavedFilter[]>('inbox.saved_filters')
  const filter = useInbox((s) => s.filter)
  const account = useInbox((s) => s.account)
  const save = () => {
    const n = name.trim()
    if (!n) return
    const list = (Array.isArray(saved) ? saved : []).filter((x) => x.name !== n)
    void setSaved([...list, { name: n, filter, account_id: account }]).then((ok) => ok && onClose())
  }
  return (
    <Dialog title="Save filter" onClose={onClose} testId="save-filter">
      <input
        className="m-input"
        aria-label="Filter name"
        placeholder="Name, e.g. Release threads"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') save()
        }}
      />
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" disabled={!name.trim()} onClick={save}>
          Save
        </Button>
      </div>
    </Dialog>
  )
}
