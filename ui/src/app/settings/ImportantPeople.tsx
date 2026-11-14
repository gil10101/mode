// Settings → Urgency → the important people list (03 §8.10, 07 §5): contacts with
// `vip = 1`, their identities and the note that becomes the why line ("Important person:
// Sam Okafor (your manager)"); Add important person from the senders the Inbox knows or
// by typing an email address / Slack handle, with an optional note; Remove
// turns `vip` off (the contact stays). Merge / split of identities waits for
// contact commands.
import { useEffect, useState } from 'react'
import { Button } from '../../kit/button'
import type { CoreVipRow, VipRow } from '../../state/contract-connectors'
import { knownSenders, normalizeVip, useInbox } from '../../state/inbox-store'
import { useSessionsStore } from '../../state/sessions-store'
import { invoke } from '../../tauri'
import { errorText } from '../../state/settings-store'
import { Combo } from '../../kit/controls'
import { Block, Field, Note } from './controls'

export function ImportantPeople() {
  const [vips, setVips] = useState<VipRow[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [who, setWho] = useState('')
  const [note, setNote] = useState('')
  const [adding, setAdding] = useState(false)
  const rows = useInbox((s) => s.rows)
  const senders = knownSenders(rows).filter((s) => s.contact_id && !vips?.some((v) => v.id === s.contact_id))

  const load = () =>
    invoke<(VipRow | CoreVipRow)[]>('vips_list').then(
      (r) => {
        setVips((r ?? []).map(normalizeVip))
        setError(null)
      },
      (e) => {
        setVips([])
        setError(errorText(e))
      },
    )
  useEffect(() => {
    void load()
  }, [])

  const add = async () => {
    const text = who.trim()
    if (!text) return
    const sender = senders.find((s) => s.name.toLowerCase() === text.toLowerCase() || s.address?.toLowerCase() === text.toLowerCase())
    setAdding(true)
    try {
      const target = sender?.contact_id ? { contact_id: sender.contact_id } : text.startsWith('@') ? { slack_user: text.slice(1) } : { email: text }
      await invoke('vip_add', { ...target, vip_note: note.trim() || null })
      await load()
      setWho('')
      setNote('')
      setError(null)
      void useInbox.getState().load()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setAdding(false)
    }
  }

  const remove = async (v: VipRow) => {
    setVips((list) => (list ?? []).filter((x) => x.id !== v.id))
    try {
      await invoke('vip_remove', { contact_id: v.id })
      useSessionsStore.getState().showNotice(`${v.display_name} is no longer an important person`)
      void useInbox.getState().load()
    } catch (e) {
      setVips((list) => [...(list ?? []), v])
      setError(errorText(e))
    }
  }

  return (
    <>
      <Block testId="vip-list">
        {vips === null ? (
          <div className="m-set-desc">Loading…</div>
        ) : vips.length === 0 ? (
          <div className="m-set-desc">No important people yet. Their messages always count as urgent.</div>
        ) : (
          <div className="m-list sg-list" role="list" aria-label="Important people">
            {vips.map((v) => (
              <div key={v.id} className="sg-grow" role="listitem" data-vip={v.id}>
                <span className="sg-grow-name">{`${v.display_name}${v.vip_note ? ` (${v.vip_note})` : ''}`}</span>
                <span className="m-t-meta">{v.identities.join(', ')}</span>
                <Button quiet aria-label={`Remove ${v.display_name} from important people`} onClick={() => void remove(v)}>
                  ×
                </Button>
              </div>
            ))}
          </div>
        )}
      </Block>
      <Field title="Add important person" desc="Urgent at once, without scoring, on every account where Mode knows them; the note becomes the why line." stack>
        <div className="m-inline sg-line" onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLElement).tagName === 'INPUT' && void add()}>
          <Combo label="Add important person" placeholder="name, email or @handle" value={who} suggestions={senders.map((s) => s.name)} onChange={setWho} />
          <input className="m-input" aria-label="Note" placeholder="note, e.g. your manager" value={note} onChange={(e) => setNote(e.target.value)} />
          <Button disabled={!who.trim() || adding} onClick={() => void add()}>
            Add important person
          </Button>
        </div>
      </Field>
      {error && <Note bad>{error}</Note>}
    </>
  )
}
