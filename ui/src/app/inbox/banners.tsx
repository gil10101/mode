// The connector status banner of 03 §4.9, shared by the Inbox and the
// Calendar (03 §6.8 "Account error: the connector banner of §4.9"): one line
// per account in trouble, with its fix (Re-authorize, Retry · Open
// Diagnostics, Resume, Reconnect · Remove), and the offline line.
import { Button } from '../../kit/button'
import { connectorBanners, useAccounts } from '../../state/accounts-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'

export function ConnectorBanners({ services }: { services: readonly ('gmail' | 'calendar' | 'slack')[] }) {
  const accounts = useAccounts((s) => s.accounts)
  const offline = useSessionsStore((s) => !!s.health?.offline)
  const banners = connectorBanners(accounts, offline, services)
  if (!banners.length) return null
  const acc = useAccounts.getState()
  return (
    <>
      {banners.map((b) => (
        <div key={b.key} className="ap-banner" role="status" data-testid={`banner-${b.kind}`} data-account={b.accountId ?? undefined}>
          <span>{b.text}</span>
          {b.kind === 'needs_reauth' && <Button onClick={() => void acc.startReauth(b.accountId!)}>Re-authorize</Button>}
          {b.kind === 'error' && (
            <>
              <Button onClick={() => void acc.retry(b.accountId!)}>Retry</Button>
              <Button quiet onClick={() => navigate({ view: 'settings', rest: 'advanced/diagnostics' })}>
                Open Diagnostics
              </Button>
            </>
          )}
          {b.kind === 'paused' && <Button onClick={() => void acc.resume(b.accountId!)}>Resume</Button>}
          {b.kind === 'revoked' && (
            <>
              <Button onClick={() => void acc.startReauth(b.accountId!)}>Reconnect</Button>
              <Button quiet onClick={() => acc.askRemove(b.accountId!)}>
                Remove
              </Button>
            </>
          )}
        </div>
      ))}
    </>
  )
}
