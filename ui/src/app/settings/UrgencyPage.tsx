// Settings → Urgency (03 §8.10; a v0.2 page) on the connectors
// core's commands: the important people list (`vips_*`), the keyword rules
// (`keyword_rules_list` seeds the three premade ones, disabled;
// `keyword_rules_set` validates and assigns ids), Re-score
// (`urgency_rescore`, the last 24 h), the scorer's counters
// (`urgency_diagnostics`, counts only), the scoring settings and the monthly
// AI token budget with the tokens used so far from `ai_health` (tokens only,
// never a currency: ADR-0032).
import { useCallback, useEffect, useState } from 'react'
import { Button } from '../../kit/button'
import { Select, Switch } from '../../kit/controls'
import { SettingError } from '../../kit/settings'
import { invoke } from '../../tauri'
import { cmdErrorText } from '../../state/contract'
import type { KeywordRule, UrgencyDiagnostics } from '../../state/contract-connectors'
import { useSetting, useSettingsStore } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { tokens } from '../sessions/format'
import { useAi } from './ai'
import { Block, Field, Group, Note, SettingNumber, SettingSwitch } from './controls'
// package 10: the important people list (07 §5)
import { ImportantPeople } from './ImportantPeople'

export type { KeywordRule }

export function UrgencyPage() {
  return (
    <>
      <Group title="Important people" id="vips">
        <ImportantPeople />
      </Group>

      <Group title="Keyword rules" id="keywords">
        <KeywordRules />
      </Group>

      <Group title="Scoring" id="scoring">
        <ModelLine />
        <SettingNumber k="ai.scoring_chars" title="Characters sent" desc="Of each message, for scoring." />
        <SettingSwitch k="urgency.direct_email_requires_known_contact" title="Direct email" desc="Counts as direct only from a known contact." />
        <SettingNumber k="urgency.pill_message_max_age_hours" title="Pill shows" desc="Messages younger than this." unit="hours" />
        <Rescore />
        <BudgetRows />
      </Group>
    </>
  )
}

/** The scoring model is set on Settings → AI (one place writes it, through `ai_backend_set`). */
function ModelLine() {
  const [model] = useSetting<string>('ai.model_scoring')
  const [backend] = useSetting<string>('ai.backend')
  const [localModel] = useSetting<string | null>('ai.local_model_scoring')
  const shown = backend === 'claude_api' ? model : localModel
  return (
    <Field title="Scoring model" desc={shown || 'Not chosen.'}>
      <Button quiet onClick={() => navigate({ view: 'settings', rest: 'ai/models' })}>
        Change on AI
      </Button>
    </Field>
  )
}

/** The monthly AI token budget and its bar (also on Settings → AI): used from `ai_health`. */
export function BudgetRows() {
  const [budget] = useSetting<number>('ai.budget_tokens_month')
  const health = useAi((s) => s.health)
  const status = useAi((s) => s.status)
  const loaded = useAi((s) => s.loaded)
  useEffect(() => {
    if (!useAi.getState().loaded) void useAi.getState().refresh()
  }, [])
  const used = health?.tokens_used_month ?? status?.tokens_used_month ?? null
  const pct = used !== null && budget > 0 ? Math.min(100, Math.round((used / budget) * 100)) : 0
  return (
    <>
      <SettingNumber k="ai.budget_tokens_month" title="Monthly budget" desc="When reached, unscored messages become normal with the why line “not scored (budget)”." unit="tokens" />
      <Field title="Used this month" desc="A daily pace of a tenth of the budget is derived from it.">
        <div className="m-usage sg-usage" data-testid="ai-budget">
          {used === null ? (loaded ? 'not available' : 'reading…') : `${tokens(used)} of ${tokens(budget)} tokens`}
          <div className="m-usage-bar">
            <i className={pct >= 80 ? 'sg-warn' : undefined} style={{ '--pct': `${pct}%` } as React.CSSProperties} />
          </div>
        </div>
      </Field>
    </>
  )
}

/** Re-score the last 24 h (07 §3.4.8) and the scorer's counters. */
function Rescore() {
  const [diag, setDiag] = useState<UrgencyDiagnostics | null>(null)
  const [busy, setBusy] = useState(false)
  const load = useCallback(() => {
    invoke<UrgencyDiagnostics>('urgency_diagnostics').then(setDiag, () => setDiag(null))
  }, [])
  useEffect(load, [load])
  const run = async () => {
    setBusy(true)
    try {
      const n = await invoke<number>('urgency_rescore')
      useSessionsStore.getState().showNotice(`Re-scoring ${n} ${n === 1 ? 'message' : 'messages'} from the last 24 h`)
      load()
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't re-score: ${cmdErrorText(e)}`)
    } finally {
      setBusy(false)
    }
  }
  return (
    <Field
      title="Re-score"
      desc={diag ? <span data-testid="urgency-counters">{`${diag.counters.scored} scored · ${diag.queue_depth} waiting · ${diag.counters.injection_flags} flagged`}</span> : 'Messages of the last 24 h, scored again.'}
    >
      <Button disabled={busy} onClick={() => void run()}>
        Re-score the last 24 h
      </Button>
    </Field>
  )
}

const FIELDS: readonly (readonly [KeywordRule['field'], string])[] = [
  ['subject', 'subject'],
  ['snippet', 'snippet'],
  ['sender', 'sender'],
  ['channel', 'channel'],
  ['any', 'any field'],
]

const MATCHES: readonly (readonly [KeywordRule['match'], string])[] = [
  ['contains', 'contains'],
  ['word', 'whole word'],
  ['regex', 'pattern'],
  ['deadline', 'deadline'],
]

const EFFECTS: readonly (readonly [KeywordRule['effect'], string])[] = [
  ['raise', 'raise to urgent'],
  ['lower', 'lower to low'],
]

/** A new rule from the typed pattern: a pattern when it has pattern characters, else contains. */
export function newRule(pattern: string, position: number): KeywordRule {
  const p = pattern.trim()
  const regex = /[|()[\]\\^$.*+?]/.test(p)
  return { id: '', name: p.slice(0, 40), enabled: true, field: 'subject', match: regex ? 'regex' : 'contains', pattern: p, effect: 'raise', horizon_days: 3, position }
}

function KeywordRules() {
  const [rules, setRules] = useState<KeywordRule[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pattern, setPattern] = useState('')
  useEffect(() => {
    invoke<KeywordRule[]>('keyword_rules_list').then(
      (r) => setRules(Array.isArray(r) ? r : []),
      (e) => {
        setRules([])
        setError(cmdErrorText(e))
      },
    )
  }, [])
  const save = async (next: KeywordRule[]): Promise<boolean> => {
    const before = rules
    setRules(next)
    try {
      const stored = await invoke<KeywordRule[]>('keyword_rules_set', { rules: next })
      setRules(Array.isArray(stored) ? stored : next)
      useSettingsStore.getState().patchValues({ 'urgency.keyword_rules': Array.isArray(stored) ? stored : next })
      setError(null)
      return true
    } catch (e) {
      setRules(before)
      setError(cmdErrorText(e))
      return false
    }
  }
  const list = rules ?? []
  const patch = (i: number, p: Partial<KeywordRule>) => void save(list.map((r, j) => (j === i ? { ...r, ...p } : r)))
  return (
    <>
      {rules === null && <Note>Loading…</Note>}
      {list.length > 0 && (
        <Block setting="urgency.keyword_rules">
          <div className="m-list sg-list" role="list" aria-label="Keyword rules">
            {list.map((r, i) => (
              <div key={r.id || i} className="sg-grow sg-keyword" role="listitem" data-rule={r.id}>
                <Switch on={r.enabled} label={`${r.name} on`} onChange={(on) => patch(i, { enabled: on })} />
                <span className="sg-grow-name">{r.pattern && r.pattern !== r.name ? `${r.name} · ${r.pattern}` : r.name}</span>
                <Select label="Field" value={r.field} options={FIELDS} onChange={(field) => patch(i, { field })} />
                <Select label="Match" value={r.match} options={MATCHES} onChange={(match) => patch(i, { match })} />
                <Select label="Effect" value={r.effect} options={EFFECTS} onChange={(effect) => patch(i, { effect })} />
                <Button quiet aria-label={`Remove ${r.name}`} onClick={() => void save(list.filter((_, j) => j !== i))}>
                  ×
                </Button>
              </div>
            ))}
          </div>
        </Block>
      )}
      <Field title="Add rule" desc="Checked before AI scoring; Re-score applies them to the last 24 h." error={error ? <SettingError testId="error-urgency.keyword_rules">{error}</SettingError> : undefined}>
        <input className="m-input" aria-label="New keyword rule" placeholder="invoice|outage" value={pattern} onChange={(e) => setPattern(e.target.value)} />
        <Button
          disabled={!pattern.trim() || rules === null}
          onClick={() => {
            void save([...list, newRule(pattern, list.length)]).then((ok) => {
              if (ok) setPattern('')
            })
          }}
        >
          Add rule
        </Button>
      </Field>
    </>
  )
}
