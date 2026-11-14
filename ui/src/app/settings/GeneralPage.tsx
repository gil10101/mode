// Settings → General (03 §8.2) and its Sessions group (§8.6): the profile
// first (picture, name), startup, rows, language, the onboarding re-run, then
// the roster controls and the soft weekly token budget (tokens only, ADR-0032).
import { useState } from 'react'
import { Mascot } from '../../kit/avatars'
import { Button } from '../../kit/button'
import { Select, Switch } from '../../kit/controls'
import { invoke } from '../../tauri'
import { currentOs, useSetting, errorText } from '../../state/settings-store'
import type { PicturePick } from '../../state/contract-settings'
import { useSessionsStore } from '../../state/sessions-store'
import { openOnboarding } from '../onboarding/state'
import { CommitInput, Field, Group, SettingNumber, SettingPicker, SettingSwitch } from './controls'

/** A local file as a webview URL (Tauri's asset protocol; `convertFileSrc` without importing @tauri-apps/api here). */
export function fileSrc(path: string): string {
  // Already a URL (the browser mock's bundled sample picture, a data URL): through as is.
  if (/^(data:|blob:|https?:|\/)/.test(path)) return path
  const enc = encodeURIComponent(path)
  return currentOs() === 'windows' ? `http://asset.localhost/${enc}` : `asset://localhost/${enc}`
}

const WEEKDAYS = [
  [null, 'not set'],
  ['monday', 'Monday'],
  ['tuesday', 'Tuesday'],
  ['wednesday', 'Wednesday'],
  ['thursday', 'Thursday'],
  ['friday', 'Friday'],
  ['saturday', 'Saturday'],
  ['sunday', 'Sunday'],
] as const

export function GeneralPage({ section }: { section?: string }) {
  void section
  const [picture, setPicture] = useSetting<string | null>('profile.picture_path')
  const [name, setName] = useSetting<string>('profile.name')
  const [language, setLanguage] = useSetting<string>('general.language')
  const [updateNote, setUpdateNote] = useState<string | null>(null)

  const choosePicture = async () => {
    try {
      // The copy under `<data dir>/profile/` (not resized); `url` is the same file through the asset protocol (`fileSrc`).
      const r = await invoke<PicturePick>('profile_picture_pick')
      if (r?.path) await setPicture(r.path)
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't use that picture: ${errorText(e)}`)
    }
  }

  return (
    <>
      <Group title="Profile" id="profile">
        <Field title="Picture" desc="The mascot until you choose one." k="profile.picture_path">
          <Mascot size={48} src={picture ? fileSrc(picture) : undefined} picture={Boolean(picture)} alt="Profile picture" />
          <Button onClick={() => void choosePicture()}>Choose picture…</Button>
          <Button disabled={!picture} onClick={() => void setPicture(null)}>
            Use mascot
          </Button>
        </Field>
        <Field title="Name" desc="The OS user name until you change it." k="profile.name" htmlFor="profile-name">
          <CommitInput id="profile-name" label="Profile name" value={name} onCommit={(v) => void setName(String(v).trim())} testId="profile-name" />
        </Field>
      </Group>

      <Group title="Startup" id="startup">
        <SettingSwitch k="general.launch_at_login" title="Launch at login" desc="Mode starts when you sign in." />
        <SettingSwitch
          k="general.status_item"
          title="Status item"
          desc="Show the count badge in the menu bar."
          read={(v) => (v as { badge?: boolean } | null)?.badge !== false}
          write={(on, v) => ({ ...((v as object | null) ?? {}), badge: on })}
        />
        <UpdatesRow note={updateNote} onCheck={() => setUpdateNote('Updates arrive with v1.0; this build is current.')} />
      </Group>

      <Group title="Window" id="rows">
        <SettingSwitch k="general.compact_rows" title="Compact rows" desc="Tighter lists in every view." />
        <Field title="Language" k="general.language">
          <Select label="Language" value={language} options={[['en', 'English']]} onChange={(v) => void setLanguage(v)} />
        </Field>
        <Field title="Onboarding" desc="Walk through the first-run steps again.">
          <Button onClick={() => openOnboarding(true)}>Run again</Button>
        </Field>
      </Group>

      <Group title="Sessions" id="sessions">
        <SettingNumber k="sessions.fold_after_minutes" title="Fold finished" desc="Recently completed sessions fold after this." unit="minutes" />
        <SettingSwitch k="sessions.store_prompt_previews" title="Prompt previews" desc="Store the first line of each prompt." />
        <SettingNumber k="usage.budget_tokens_week" title="Weekly budget" desc="A soft line on Stats; leave empty for none." unit="tokens" nullable />
        <SettingPicker k="usage.reset_weekday" title="Week starts" desc="When the weekly budget resets." options={WEEKDAYS} />
      </Group>
    </>
  )
}

function UpdatesRow({ note, onCheck }: { note: string | null; onCheck: () => void }) {
  const [auto, setAuto] = useSetting<boolean>('general.auto_update')
  return (
    <Field title="Updates" desc={note ?? 'Check now, or let Mode update on its own.'} k="general.auto_update">
      <Button onClick={onCheck}>Check for updates</Button>
      <Switch on={auto} label="Update automatically" onChange={(on) => void setAuto(on)} />
    </Field>
  )
}
