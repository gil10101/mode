// Settings → Privacy (03 §8.15): what leaves the device per connector and AI
// feature, the local-only switch, the retention controls (every
// `retention.*` key), prompt previews, "no telemetry, no account", and
// Delete all Mode data behind a typed confirmation.
import { errorText } from '../../state/settings-store'
import { useState } from 'react'
import { Button } from '../../kit/button'
import { invoke } from '../../tauri'
import { useSessionsStore } from '../../state/sessions-store'
import { Dialog } from '../ui'
import { LocalOnlySwitch } from './AiPage'
import { Field, Group, Note, SettingNumber, SettingSwitch } from './controls'

export function PrivacyPage() {
  const [deleting, setDeleting] = useState(false)
  return (
    <>
      <Group title="What leaves this device" id="leaves">
        <Note>Claude Code sessions: nothing; Mode reads hook events and transcripts on this machine.</Note>
        <Note>Notes: nothing; they stay plain Markdown files in your folder.</Note>
        <Note>Gmail, Calendar and Slack (from v0.2): only the requests that fetch and mark your own messages, to those services.</Note>
        <Note>AI features (from v0.2): the characters set for scoring, summaries and Chat, to the backend you choose; none with Local-only on.</Note>
        <LocalOnlySwitch desc="No remote AI calls." />
        <Note>Marketplace: the catalog index and the packs you open are fetched from the address on the Marketplace page (GitHub by default); nothing is sent.</Note>
        <Note>No telemetry, no account: Mode sends nothing about you anywhere.</Note>
      </Group>

      <Group title="Retention" id="retention">
        <SettingNumber k="retention.sessions_days" title="Sessions" desc="Finished sessions older than this are removed." unit="days" />
        <SettingNumber k="retention.session_events_days" title="Session events" desc="The hook events behind each session." unit="days" />
        <SettingNumber k="retention.usage_days" title="Usage" desc="Token counts behind Stats." unit="days" />
        <SettingNumber k="retention.messages_days" title="Messages" desc="Inbox messages and their scores." unit="days" />
        <SettingNumber k="retention.message_body_days" title="Message bodies" desc="The full text; the subject and sender stay longer." unit="days" />
        <SettingSwitch k="sessions.store_prompt_previews" title="Prompt previews" desc="Store the first line of each prompt." />
        <Note>Chat conversations are kept as long as the Chat page says.</Note>
      </Group>

      <Group title="Delete" id="delete">
        <Field title="Delete all Mode data" desc="Data and keys go; the notes folder, the logs and the hooks stay.">
          <Button onClick={() => setDeleting(true)}>Delete everything…</Button>
        </Field>
      </Group>
      {deleting && <DeleteAll onClose={() => setDeleting(false)} />}
    </>
  )
}

function DeleteAll({ onClose }: { onClose: () => void }) {
  const [typed, setTyped] = useState('')
  const [error, setError] = useState<string | null>(null)
  return (
    <Dialog title="Delete all Mode data" onClose={onClose} testId="delete-all">
      <div className="ap-dialog-note">Type delete to remove Mode's data and keys. Your notes folder is not touched; the logs and the installed hooks stay.</div>
      <input className="m-input sg-confirm" aria-label="Type delete to confirm" value={typed} onChange={(e) => setTyped(e.target.value)} />
      {error && <div className="ap-dialog-note is-bad">{error}</div>}
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button
          variant="primary"
          disabled={typed.trim().toLowerCase() !== 'delete'}
          onClick={() =>
            void invoke('data_delete_all', { confirm: 'delete' }).then(
              () => {
                onClose()
                useSessionsStore.getState().showNotice('Mode data deleted')
              },
              (e) => setError(errorText(e)),
            )
          }
        >
          Delete
        </Button>
      </div>
    </Dialog>
  )
}
