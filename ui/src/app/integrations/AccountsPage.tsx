// The Gmail, Slack and Google Calendar card pages (03 §7C.3): the accounts
// table (status with its detail, last sync, ⋮ with Re-authorize, Pause /
// Resume, Retry, Remove), Add account through the OAuth loopback, then the
// card's settings: watched labels (Gmail), watched conversations with the
// Important-channel toggle and the channel picker (Slack, 06 §8.4, 07 §5), per-calendar
// toggles (Calendar), the poll intervals, the OAuth client (`oauth_client_set`)
// and the verification warning (Gmail), the manifest (`slack_manifest`: copy
// or save as a file; Slack).
import { useEffect, useState } from 'react'
import { Button } from '../../kit/button'
import { Switch } from '../../kit/controls'
import { List } from '../../kit/lists'
import { ConnectorRow, Form, Section, SourceHead, SourceRow as WatchRow } from '../../kit/views'
import type { AccountRow, CoreSecretStatus, SourceRow } from '../../state/contract-connectors'
import { errorText, useAccounts, type ConnectorCardKind } from '../../state/accounts-store'
import { useSessionsStore } from '../../state/sessions-store'
import { invoke } from '../../tauri'
import { Field, Group, Note, SettingNumber } from '../settings/form-controls'
import { copyText, openMenu, useNow, type MenuEntry } from '../ui'
import { VERIFICATION_WARNING } from './AccountDialogs'
import { accountState, loadManifest, saveText, slackManifest } from './accounts-model'
import { ago } from './model'

const SERVICE: Record<ConnectorCardKind, 'gmail' | 'slack' | 'calendar'> = { gmail: 'gmail', slack: 'slack', 'google-calendar': 'calendar' }

export function accountMenu(a: AccountRow): MenuEntry[] {
  const acc = useAccounts.getState()
  const out: MenuEntry[] = []
  if (a.status === 'needs_reauth' || a.status === 'revoked') out.push({ id: 'reauth', label: 'Re-authorize', run: () => void acc.startReauth(a.id) })
  if (a.status === 'error') out.push({ id: 'retry', label: 'Retry', run: () => void acc.retry(a.id) })
  if (a.status === 'paused') out.push({ id: 'resume', label: 'Resume', run: () => void acc.resume(a.id) })
  if (a.status === 'active') out.push({ id: 'pause', label: 'Pause', run: () => void acc.pause(a.id) })
  if (a.status === 'active' || a.status === 'paused') out.push({ id: 'reauth', label: 'Re-authorize', run: () => void acc.startReauth(a.id) })
  out.push({ kind: 'sep', id: 'sep' }, { id: 'remove', label: 'Remove…', danger: true, run: () => acc.askRemove(a.id) })
  return out
}

function accountDetail(a: AccountRow, now: number): string {
  const who = a.provider === 'slack' ? [a.team_name, a.handle].filter(Boolean).join(' · ') : (a.email ?? '')
  const services = a.services.join(', ')
  const sync = a.last_sync_at ? `synced ${ago(a.last_sync_at, now)}` : 'not synced yet'
  return [who, services, sync].filter(Boolean).join(' · ')
}

export function AccountsPage({ kind }: { kind: ConnectorCardKind }) {
  const now = useNow(5000)
  const accounts = useAccounts((s) => s.accounts)
  const sources = useAccounts((s) => s.sources)
  const offline = useSessionsStore((s) => !!s.health?.offline)
  const mine = accounts.filter((a) => a.services.includes(SERVICE[kind]))
  const acc = useAccounts.getState()
  const add = () => acc.startConnect(kind, kind === 'google-calendar' ? { services: ['gmail', 'calendar'] } : undefined)

  return (
    <div className="ig-accounts-page" data-testid={`accounts-${kind}`}>
      <Section
        title={kind === 'slack' ? 'Workspaces' : 'Accounts'}
        meta={mine.length ? `${mine.length} ${kind === 'slack' ? (mine.length === 1 ? 'workspace' : 'workspaces') : mine.length === 1 ? 'account' : 'accounts'}` : undefined}
        list={mine.length > 0}
        foot={
          <Button disabled={offline} title={offline ? 'needs a network connection' : undefined} onClick={add}>
            {kind === 'slack' ? 'Add workspace' : 'Add account'}
          </Button>
        }
      >
      {mine.length ? (
        <List className="ig-accounts">
          {mine.map((a) => {
            const st = accountState(a)
            return (
              <ConnectorRow
                key={a.id}
                state={st.state}
                stateLabel={st.label}
                name={a.display_name}
                detail={accountDetail(a, now)}
                meta={a.status_detail ?? a.notices?.[0] ?? (a.can_write && !a.can_write.mark_read ? 'read-only' : undefined)}
                onMore={() => {
                  const el = document.querySelector(`.ig-account-${a.id} .m-btn-icon`)
                  if (el) openMenu(el, accountMenu(a), `${a.display_name} options`)
                }}
                className={`ig-account-${a.id}`}
              />
            )
          })}
        </List>
      ) : (
        <Note>No account yet.</Note>
      )}
      </Section>
      {kind === 'gmail' && <GmailSettings sources={sources.filter((s) => (s.kind === 'gmail_inbox' || s.kind === 'gmail_label') && mine.some((a) => a.id === s.account_id))} accounts={mine} />}
      {kind === 'slack' && <SlackSettings sources={sources.filter((s) => s.kind.startsWith('slack') && mine.some((a) => a.id === s.account_id))} accounts={mine} />}
      {kind === 'google-calendar' && <CalendarSettings sources={sources.filter((s) => s.kind === 'calendar' && mine.some((a) => a.id === s.account_id))} accounts={mine} />}
    </div>
  )
}

/** The watched rows of a section (03 §7C.3): name · a switch per column under a labeled head. */
function SourceRows({ rows, accounts, vip, column = 'Watch' }: { rows: SourceRow[]; accounts: AccountRow[]; vip?: boolean; column?: string }) {
  const acc = useAccounts.getState()
  const name = (id: string) => accounts.find((a) => a.id === id)?.display_name
  if (!rows.length) return <Note>Nothing watched yet.</Note>
  return (
    <div className="ig-sources">
      <SourceHead labels={vip ? ['Important', column] : [column]} />
      {rows.map((s) => (
        <div key={s.id} data-source={s.id}>
          <WatchRow
            name={`${s.name}${accounts.length > 1 ? ` · ${name(s.account_id)}` : ''}`}
            note={s.always_on ? 'always watched' : undefined}
            cells={[
              ...(vip ? [<Switch key="vip" on={s.vip} label={`${s.name} important`} onChange={(on) => void acc.setSource(s.id, { vip: on })} />] : []),
              <Switch key="on" on={s.enabled} disabled={s.always_on} label={`Watch ${s.name}`} onChange={(on) => void acc.setSource(s.id, { enabled: on })} />,
            ]}
          />
        </div>
      ))}
    </div>
  )
}

function GmailSettings({ sources, accounts }: { sources: SourceRow[]; accounts: AccountRow[] }) {
  return (
    <>
      <Section title="Watched labels" list>
        <SourceRows rows={sources} accounts={accounts} vip />
      </Section>
      <Section title="Settings">
        <Form label="Gmail settings">
          <Group id="gmail">polling</Group>
          <SettingNumber k="polling.gmail_seconds" label="Poll every" unit="seconds" />
          <OAuthClient />
          <Note>{VERIFICATION_WARNING}</Note>
        </Form>
      </Section>
    </>
  )
}

/**
 * The Google OAuth client of 06 §6.1 (path A: the user's own project), stored in the keyring through
 * `oauth_client_set` (`{name, stored}` back). Whether one is stored: `secret_status` when the core answers
 * for `oauth_client:google`, else known once a Google account is connected (none connects without it).
 */
export function oauthClientKnown(probe: boolean | null, accounts: readonly Pick<AccountRow, 'provider'>[]): boolean | null {
  if (probe !== null) return probe
  return accounts.some((a) => a.provider === 'google') ? true : null
}

function OAuthClient() {
  const [probe, setProbe] = useState<boolean | null>(null)
  const accounts = useAccounts((s) => s.accounts)
  const stored = oauthClientKnown(probe, accounts)
  const [id, setId] = useState('')
  const [secret, setSecret] = useState('')
  const [project, setProject] = useState('')
  useEffect(() => {
    invoke<CoreSecretStatus>('secret_status', { name: 'oauth_client:google' }).then(
      (r) => setProbe(typeof r?.stored === 'boolean' ? r.stored : null),
      () => setProbe(null),
    )
  }, [])
  const save = () =>
    invoke<CoreSecretStatus>('oauth_client_set', { provider: 'google', client_id: id.trim(), client_secret: secret.trim(), ...(project.trim() ? { project_id: project.trim() } : {}) }).then(
      (r) => {
        setProbe(r?.stored ?? true)
        setProject('')
        setId('')
        setSecret('')
        useSessionsStore.getState().showNotice('OAuth client saved')
      },
      (e) => useSessionsStore.getState().showNotice(`Couldn't save the OAuth client: ${errorText(e)}`),
    )
  return (
    <Field label="OAuth client">
      <div className="m-inline">
        <input className="m-input sg-text" aria-label="Client ID" placeholder={stored ? 'stored · paste to replace' : 'Client ID'} value={id} onChange={(e) => setId(e.target.value)} />
        <input className="m-input sg-text" aria-label="Client secret" type="password" placeholder="Client secret" value={secret} onChange={(e) => setSecret(e.target.value)} />
        <input className="m-input sg-text" aria-label="Project ID" placeholder="Project ID (optional)" value={project} onChange={(e) => setProject(e.target.value)} />
        <Button disabled={!id.trim() || !secret.trim()} onClick={() => void save()}>
          Save
        </Button>
        <span className="m-t-meta">{stored ? 'stored in the keyring' : 'from your own Google Cloud project'}</span>
      </div>
    </Field>
  )
}

function SlackSettings({ sources, accounts }: { sources: SourceRow[]; accounts: AccountRow[] }) {
  const acc = useAccounts.getState()
  const [showManifest, setShowManifest] = useState(false)
  const [manifest, setManifest] = useState<string | null>(null)
  useEffect(() => {
    void loadManifest(true).then(setManifest)
  }, [])
  const text = manifest ?? slackManifest(true)
  const conversations = sources.filter((s) => s.kind !== 'slack_channel' || s.enabled)
  return (
    <>
      <Section
        title="Watched conversations"
        list
        foot={accounts.map((a) => (
          <Button key={a.id} onClick={() => void acc.openPicker(a.id)}>
            {accounts.length > 1 ? `Choose channels · ${a.display_name}` : 'Choose channels'}
          </Button>
        ))}
      >
        <SourceRows rows={conversations} accounts={accounts} vip />
      </Section>
      <Section title="Settings">
      <Form label="Slack settings">
        <Group id="slack">polling</Group>
        <SettingNumber k="polling.slack_seconds" label="Poll every" unit="seconds" />
        <SettingNumber k="polling.slack_budget_per_minute" label="Budget" unit="requests per minute" />
        <Field label="Manifest">
          <div className="m-inline">
            <Button onClick={() => copyText(text, 'Manifest copied')}>Copy manifest</Button>
            <Button onClick={() => useSessionsStore.getState().showNotice(saveText('mode-slack-manifest.yaml', text) ? 'Manifest saved to your downloads' : "Couldn't save the manifest; copy it instead")}>Save manifest…</Button>
            <Button quiet onClick={() => setShowManifest(!showManifest)}>
              {showManifest ? 'Hide' : 'Show'}
            </Button>
          </div>
        </Field>
        {showManifest && (
          <pre className="ig-manifest" data-testid="slack-manifest">
            {text}
          </pre>
        )}
        <Note>Workspaces where an admin approves new apps may block the install.</Note>
      </Form>
      </Section>
    </>
  )
}

function CalendarSettings({ sources, accounts }: { sources: SourceRow[]; accounts: AccountRow[] }) {
  return (
    <>
      <Section title="Calendars" list>
        <SourceRows rows={sources} accounts={accounts} column="Shown" />
      </Section>
      <Section title="Settings">
        <Form label="Calendar settings">
          <Group id="calendar">polling</Group>
          <SettingNumber k="polling.calendar_seconds" label="Poll every" unit="seconds" />
          <Note>Mode reads your calendar and never writes to it.</Note>
        </Form>
      </Section>
    </>
  )
}
