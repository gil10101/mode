// Settings → Overlay (03 §8.3; semantics and ranges owned by 02 §17):
// placement, host display, the AppBar reservation on Windows, what it does
// with nothing to show (`overlay.idle`), the timings, pinning, full-screen
// behavior, rows, sounds, the mascot, Now playing (`overlay.media`, with the
// macOS Automation path when the permission was refused, and the Music tab's
// Clear history) and motion.
import { useEffect, useState } from 'react'
import { Button } from '../../kit/button'
import { Notice } from '../../kit/controls'
import { EVENTS, type MediaChanged } from '../../state/contract'
import { useSetting, useSettingsStore } from '../../state/settings-store'
import { invoke, listen } from '../../tauri'
import { useSessionsStore } from '../../state/sessions-store'
import { Block, Field, Group, SettingNumber, SettingPicker, SettingSeg, SettingSwitch } from './controls'

/** The macOS Automation pane (03 §8.3); the app window's opener allows exactly this URL. */
const AUTOMATION_PANE = 'x-apple.systempreferences:com.apple.preference.security?Privacy_Automation'

/** Whether the core reported a refused Automation permission (`media.changed.permission`). */
function useMediaDenied(): boolean {
  const [denied, setDenied] = useState(false)
  useEffect(() => {
    const off = listen<MediaChanged>(EVENTS.media, (p) => setDenied(p?.permission === 'denied'))
    return () => void off.then((u) => u())
  }, [])
  return denied
}

export function OverlayPage() {
  const platform = useSettingsStore((s) => s.platform)
  const placements: (readonly [string, string])[] = [
    ...(platform?.notch ? ([['notch', 'Notch']] as const) : []),
    ['top-bar', 'Top bar'],
    ['right-edge', 'Right edge'],
  ]
  const displays: (readonly [string, string])[] = [
    ['main', 'Main display'],
    ['cursor', 'Display with the pointer'],
    ...(platform?.displays ?? []).filter((d) => !d.main).map((d) => [d.id, d.name] as const),
  ]
  const reduced = platform?.reduced_motion
  const mac = platform?.os === 'macos'
  const [media] = useSetting<unknown>('overlay.media')
  const denied = useMediaDenied() && media !== false
  return (
    <>
      <Group title="Where it sits" id="placement">
        <SettingSeg k="overlay.placement" title="Placement" desc="The notch, the top bar or the right edge of the screen." options={placements} />
        <SettingPicker k="overlay.display" title="Host display" options={displays} />
        {platform?.os === 'windows' && (
          <SettingSeg
            k="overlay.right_edge_dock"
            title="Right edge"
            desc="Reserving space keeps windows from sliding under it."
            options={[
              ['float', 'Float'],
              ['appbar', 'Reserve screen space'],
            ]}
          />
        )}
        <SettingPicker
          k="overlay.idle"
          title="When nothing is happening"
          desc="The resting mascot shows app actions."
          options={[
            ['mascot', 'Show the mascot in the notch'],
            ['hidden', 'Hide the overlay'],
          ]}
        />
      </Group>

      <Group title="Behavior" id="behaviour">
        <SettingNumber k="overlay.peek_delay_ms" title="Peek delay" desc="0–1000 ms before it peeks." unit="ms" />
        <SettingSwitch k="overlay.expand_on_hover" title="Expand on hover" desc="Open the panel when the pointer rests on it." />
        <SettingNumber k="overlay.expand_hover_ms" title="Hover to expand" desc="300–2000 ms of hover." unit="ms" />
        <SettingNumber k="overlay.collapse_delay_ms" title="Collapse after" desc="After the pointer leaves; 500–10000 ms." unit="ms" />
        <SettingNumber k="overlay.auto_collapse_ms" title="Auto-collapse" desc="3000–60000 ms, or 0 for never." unit="ms" />
        <SettingSwitch k="overlay.pinned" title="Pinned by default" desc="Keep the panel open until you unpin it." />
        <SettingSeg
          k="overlay.default_tab"
          title="Opens on"
          options={[
            ['auto', 'Auto'],
            ['sessions', 'Sessions'],
            ['inbox', 'Inbox'],
            ['notes', 'Notes'],
            ['calendar', 'Calendar'],
            ['music', 'Music'],
          ]}
        />
        <SettingPicker
          k="overlay.fullscreen"
          title="Full screen"
          desc="What it does over a full-screen app."
          options={[
            ['presentation-hides', 'Hide while presenting'],
            ['show', 'Show'],
            ['hide', 'Hide'],
          ]}
        />
        <SettingNumber k="overlay.max_rows" title="Rows per tab" desc="4–10 rows in each overlay tab." />
      </Group>

      <Group title="Presence" id="presence">
        <SettingSwitch k="sounds.enabled" title="Sounds" desc="Off by default." />
        <SettingSwitch k="overlay.show_mascot" title="Mascot" desc="A state dot replaces it when off." />
        <SettingSwitch
          k="overlay.media"
          title="Now playing"
          desc={mac ? 'Shows what Spotify or Music is playing, with controls, in the overlay.' : 'Shows what is playing, with controls, in the overlay.'}
        />
        <Field title="Music history" desc="The Music tab lists the tracks Mode saw playing, on this device only.">
          <Button onClick={() => void invoke('media_history_clear').then(() => useSessionsStore.getState().showNotice('Music history cleared'), () => {})} data-testid="media-history-clear">
            Clear history
          </Button>
        </Field>
        {mac && denied && (
          <Block testId="media-denied">
            <Notice
              state="info"
              action={
                <Button quiet onClick={() => void invoke('plugin:opener|open_url', { url: AUTOMATION_PANE }).catch(() => {})}>
                  Open System Settings
                </Button>
              }
            >
              Allow Mode to control Spotify in System Settings › Privacy &amp; Security › Automation
            </Notice>
          </Block>
        )}
        <SettingSeg
          k="overlay.motion"
          title="Motion"
          desc={reduced === undefined ? 'System follows the OS setting.' : `System follows the OS setting (now ${reduced ? 'reduced' : 'full'}).`}
          options={[
            ['system', 'System'],
            ['reduced', 'Reduced'],
            ['full', 'Full'],
          ]}
        />
        {platform?.os === 'macos' && <SettingNumber k="overlay.scale" title="Scale" desc="100–150 %." unit="%" />}
      </Group>
    </>
  )
}
