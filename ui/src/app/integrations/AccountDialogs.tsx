// The account dialogs every connector surface opens (mounted once by the
// shell): Connect / Re-authorize through the OAuth loopback (06 §4.1 as the
// UI sees it: the per-service toggles and the verification warning for
// Google, the manifest steps for Slack, then "Waiting for the browser…" with
// Copy link and Cancel, then the result), the Remove confirmation (03 §2.4,
// §7C.3) and the Slack channel picker (06 §8.4).
import { Button } from '../../kit/button'
import { Switch } from '../../kit/controls'
import { useAccounts, type ConnectFlow } from '../../state/accounts-store'
import { useSetting } from '../../state/settings-store'
import { copyText, Dialog } from '../ui'
import { loadManifest, slackRequestsPerMinute } from './accounts-model'

/** 03 §7C.2 / 06 §6.2, verbatim. */
export const VERIFICATION_WARNING = "Google shows an 'unverified app' screen while your OAuth client is in Testing; tokens expire after 7 days and Mode will ask you to re-authorize."

export function AccountDialogs() {
  const flow = useAccounts((s) => s.flow)
  const removing = useAccounts((s) => s.removing)
  const picker = useAccounts((s) => s.picker)
  return (
    <>
      {flow && (!flow.inline || flow.phase === 'setup') && <ConnectDialog flow={flow} />}
      {removing && <RemoveDialog />}
      {picker && <ChannelPicker />}
    </>
  )
}

function title(f: ConnectFlow): string {
  if (f.mode === 'reauth') return `Re-authorize ${f.accountName ?? 'the account'}`
  return f.provider === 'slack' ? 'Connect a Slack workspace' : 'Connect a Google account'
}

function ConnectDialog({ flow: f }: { flow: ConnectFlow }) {
  const acc = useAccounts.getState()
  const close = () => acc.cancelFlow()
  const google = f.provider === 'google'
  const toggle = (svc: 'gmail' | 'calendar', on: boolean) => acc.patchFlow({ services: on ? Array.from(new Set([...f.services, svc])) : f.services.filter((x) => x !== svc) })
  const canBegin = google ? f.services.length > 0 : f.mode === 'reauth' || f.clientId.trim().length > 0
  return (
    <Dialog title={title(f)} onClose={close} testId="connect-dialog">
      <div className="ig-connect" data-phase={f.phase} data-testid="connect-flow">
        {f.phase === 'setup' && google && (
          <>
            <div className="ig-connect-rows">
              <label className="ig-connect-row">
                <Switch on={f.services.includes('gmail')} label="Gmail" onChange={(on) => toggle('gmail', on)} />
                <span>Gmail</span>
                <span className="m-t-meta">messages, read and archive</span>
              </label>
              <label className="ig-connect-row">
                <Switch on={f.services.includes('calendar')} label="Calendar" onChange={(on) => toggle('calendar', on)} />
                <span>Calendar</span>
                <span className="m-t-meta">read-only</span>
              </label>
              {f.services.includes('gmail') && (
                <label className="ig-connect-row">
                  <Switch on={!f.readOnly} label="Write back" onChange={(on) => acc.patchFlow({ readOnly: !on })} />
                  <span>Write back</span>
                  <span className="m-t-meta">mark read and archive in Gmail too</span>
                </label>
              )}
            </div>
            <p className="ig-connect-note">{VERIFICATION_WARNING}</p>
            <p className="ig-connect-note m-t-meta">Mode opens Google's consent page in your browser and waits on this computer until you come back. Windows may ask once to allow it.</p>
          </>
        )}
        {f.phase === 'setup' && !google && (
          <>
            <ol className="ig-steps">
              <li>
                At api.slack.com/apps choose <b>Create New App → From a manifest</b>
                {', pick the workspace and paste the manifest. '}
                <Button quiet onClick={() => void loadManifest(!f.readOnly).then((text) => copyText(text, 'Manifest copied'))}>
                  Copy manifest
                </Button>
              </li>
              <li>Under OAuth &amp; Permissions turn on <b>Token rotation</b> and <b>PKCE</b>; neither can be undone for the app.</li>
              <li>
                Copy the <b>Client ID</b> from Basic Information:
                <input className="m-input ig-client" aria-label="Client ID" placeholder="1234567890.1234567890" value={f.clientId} onChange={(e) => acc.patchFlow({ clientId: e.target.value })} />
              </li>
              <li>Continue, then choose Allow on Slack's page.</li>
            </ol>
            <label className="ig-connect-row">
              <Switch on={!f.readOnly} label="Mark read in Slack" onChange={(on) => acc.patchFlow({ readOnly: !on })} />
              <span>Mark read in Slack too</span>
              <span className="m-t-meta">adds the write scopes to the manifest</span>
            </label>
            <p className="ig-connect-note m-t-meta">Workspaces where an admin approves new apps may stop the install there.</p>
          </>
        )}
        {(f.phase === 'starting' || f.phase === 'waiting') && (
          <div className="ig-wait" role="status">
            <div>{f.phase === 'starting' ? 'Opening the browser…' : 'Waiting for the browser…'}</div>
            <p className="ig-connect-note m-t-meta">{f.openFailed ? 'The browser could not be opened. Copy the link and open it yourself; Mode is still waiting.' : 'Finish in your browser; this window updates when you come back.'}</p>
            {f.authUrl && (
              <div className="m-inline">
                <code className="ig-url" title={f.authUrl}>
                  {f.authUrl}
                </code>
                <Button quiet onClick={() => copyText(f.authUrl!, 'Link copied')}>
                  Copy link
                </Button>
              </div>
            )}
          </div>
        )}
        {f.phase === 'done' && (
          <div className="ig-wait" role="status" data-testid="connect-done">
            {`Connected ${f.account?.display_name ?? ''}`}
            {f.account && f.provider === 'google' && f.account.services.length < f.services.length && (
              <p className="ig-connect-note m-t-meta">{f.account.services.includes('calendar') ? 'Calendar only — re-authorize to add Gmail.' : 'Gmail only — re-authorize to add Calendar.'}</p>
            )}
          </div>
        )}
        {f.phase === 'failed' && (
          <div className="ig-wait is-bad" role="alert" data-testid="connect-error">
            {f.error ?? 'Could not finish connecting — try again.'}
          </div>
        )}
        <div className="m-card-actions">
          {f.phase === 'done' ? (
            <>
              {f.provider === 'slack' && f.account && (
                <Button
                  onClick={() => {
                    const id = f.account!.id
                    acc.cancelFlow()
                    void acc.openPicker(id)
                  }}
                >
                  Choose channels
                </Button>
              )}
              <Button variant="primary" onClick={close}>
                Done
              </Button>
            </>
          ) : (
            <>
              <Button onClick={close}>Cancel</Button>
              {(f.phase === 'setup' || f.phase === 'failed') && (
                <Button variant="primary" disabled={!canBegin} onClick={() => void acc.begin()}>
                  {f.phase === 'failed' ? 'Try again' : 'Continue in browser'}
                </Button>
              )}
            </>
          )}
        </div>
      </div>
    </Dialog>
  )
}

function RemoveDialog() {
  const a = useAccounts((s) => s.removing)!
  const acc = useAccounts.getState()
  return (
    <Dialog title={`Remove ${a.display_name}?`} onClose={() => acc.cancelRemove()} testId="remove-dialog">
      <p className="ig-connect-note">Messages and events fetched through this account will be removed. Mode revokes its access and deletes the stored key; contacts stay.</p>
      <div className="m-card-actions">
        <Button onClick={() => acc.cancelRemove()}>Cancel</Button>
        <Button variant="primary" onClick={() => void acc.confirmRemove()}>
          Remove
        </Button>
      </div>
    </Dialog>
  )
}

function ChannelPicker() {
  const p = useAccounts((s) => s.picker)!
  const account = useAccounts((s) => s.accounts.find((a) => a.id === p.accountId))
  const allSources = useAccounts((s) => s.sources)
  const sources = allSources.filter((x) => x.account_id === p.accountId)
  const [interval] = useSetting<number>('polling.slack_seconds')
  const [budget] = useSetting<number>('polling.slack_budget_per_minute')
  const acc = useAccounts.getState()
  const q = p.query.trim().toLowerCase().replace(/^#/, '')
  const list = (p.channels ?? []).filter((c) => !q || c.name.toLowerCase().replace(/^#/, '').includes(q))
  const alwaysOn = sources.filter((s) => s.always_on)
  const conversations = alwaysOn.length + p.picked.length
  const perMin = slackRequestsPerMinute(conversations, interval || 30)
  const over = budget > 0 && perMin > budget
  const toggle = (id: string, on: boolean) => acc.patchPicker({ picked: on ? [...p.picked.filter((x) => x !== id), id] : p.picked.filter((x) => x !== id) })
  return (
    <Dialog title={`Watched channels · ${account?.display_name ?? 'Slack'}`} onClose={() => acc.closePicker()} testId="channel-picker">
      <input className="m-input" aria-label="Search channels" placeholder="Search channels…" value={p.query} onChange={(e) => acc.patchPicker({ query: e.target.value })} />
      <div className="ig-picker" role="list" aria-label="Channels">
        {p.channels === null && <div className="m-t-meta ig-pad">Loading channels…</div>}
        {list.map((c) => {
          const on = p.picked.includes(c.id)
          return (
            <label key={c.id} className="m-task ig-pick" role="listitem" data-channel={c.id}>
              <Switch on={on} label={`Watch ${c.name}`} onChange={(v) => toggle(c.id, v)} />
              <span className="m-task-title">{c.name}</span>
              {c.is_private && <span className="m-t-meta">private</span>}
              <span className="m-t-meta">{`${c.num_members} members`}</span>
            </label>
          )
        })}
        {p.channels !== null && !list.length && <div className="m-t-meta ig-pad">{`No channel matches “${p.query}”.`}</div>}
      </div>
      <div className="ig-connect-note m-t-meta">{`DMs, group DMs and mentions are always watched (${alwaysOn.length}).`}</div>
      <div className={`ig-connect-note${over ? ' is-bad' : ''}`} data-testid="picker-budget">
        {`${p.picked.length} ${p.picked.length === 1 ? 'channel' : 'channels'} · ≈ ${perMin} requests/min at your interval${over ? ` · above the budget of ${budget}; some conversations will be polled less often` : ''}`}
      </div>
      {p.error && (
        <div className="ig-connect-note is-bad" role="alert">
          {p.error}
        </div>
      )}
      <div className="m-card-actions">
        <Button onClick={() => acc.closePicker()}>Cancel</Button>
        <Button variant="primary" disabled={p.saving || p.channels === null} onClick={() => void acc.savePicker()}>
          Save
        </Button>
      </div>
    </Dialog>
  )
}
