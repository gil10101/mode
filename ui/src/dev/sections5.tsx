// Gallery sections: the design-consistency pass (components.html §select, §settings-rows, §swatches, §notice), the skeleton (§skeleton) and Now playing (§nowplaying).
import { AgentAvatar } from '../kit/avatars'
import { Button } from '../kit/button'
import { Combo, Menu, MenuItem, Notice, Seg, SegItem, Select, Switch, SwatchGrid, TextArea } from '../kit/controls'
import { AVATAR_VARIANTS, nowPlayingArt } from '../kit/mascot'
import { Equalizer, NowPlayingStrip } from '../kit/nowplaying'
import { SettingError, SettingGroup, SettingRow, SettingsNav, SettingsPage } from '../kit/settings'
import { Skeleton } from '../kit/skeleton'
import { Stat, Stats } from '../kit/stats'
import { wireframeSample } from '../app/stats/sample'
import { DAY_BARS, SPEND } from '../app/stats/skeleton'
import { metricTiles } from '../app/stats/text'
import { SKELETON_NOTE } from '../app/stats/wireframe'
import { Ex, Flow, type SectionSpec } from './wf'

const noop = () => {}

const TIMEZONES = [
  ['berlin', 'Europe/Berlin'],
  ['london', 'Europe/London'],
  ['newyork', 'America/New_York'],
] as const

const START_ON = [
  ['sessions', 'Sessions'],
  ['inbox', 'Inbox'],
  ['notes', 'Notes'],
] as const

const VARIANT_NAME: Record<string, string> = { plain: 'Plain', beret: 'Beret', specs: 'Specs', tophat: 'Top hat', mustache: 'Mustache', beanie: 'Beanie', cap: 'Cap', squares: 'Squares', monocle: 'Monocle' }

export const SECTIONS_5: SectionSpec[] = [
  {
    id: 'select',
    title: 'Select and combobox',
    note: 'One select design (10 §7.4.5): a 28-px button on the surface with a 1-px inset line, the value ellipsised, a 12-px chevron in fg-3; it opens the window’s .m-menu under it, at least as wide, with a check on the current option. The combobox is free text with suggestions: an .m-input with the same chevron inside it. The switch is a 36 × 20 track with a white 16-px knob, green when on.',
    render: () => (
      <>
        <Ex caption=".m-select · .m-select-label · .m-select-chev (+ .is-placeholder, [disabled])">
          <Flow>
            <Select label="Timezone" value="berlin" options={TIMEZONES} onChange={noop} />
            <Select label="Repo" value={undefined} options={START_ON} placeholder="Choose a repo" onChange={noop} />
            <Select label="Start on" value="inbox" options={START_ON} disabled onChange={noop} />
          </Flow>
        </Ex>
        <Ex caption=".m-select open · .m-menu under it, at its width · the current option checked">
          <div className="m-stack" style={{ alignItems: 'flex-start', gap: 4 }}>
            <Select label="Start on" value="sessions" options={START_ON} onChange={noop} />
            <Menu label="Start on" style={{ minWidth: 180 }}>
              <MenuItem hint="✓">Sessions</MenuItem>
              <MenuItem>Inbox</MenuItem>
              <MenuItem>Notes</MenuItem>
            </Menu>
          </div>
        </Ex>
        <Ex caption=".m-combo · .m-input · .m-combo-chev">
          <div style={{ width: 280 }}>
            <Combo label="Folder" value="~/code/mode" suggestions={['~/code/mode', '~/code/docs-site']} onChange={noop} />
          </div>
        </Ex>
        <Ex caption=".m-switch (+ .is-on, [disabled]) · 36 × 20">
          <Flow>
            <Switch on={false} label="Off" />
            <Switch on label="On" />
            <Switch on={false} label="Off, disabled" disabled />
            <Switch on label="On, disabled" disabled />
          </Flow>
        </Ex>
      </>
    ),
  },
  {
    id: 'settings-rows',
    title: 'Settings nav, cards and rows',
    stacked: true,
    note: 'The Settings pattern (10 §7.4.19): a 200-px page list with the filter on top and hairline-separated groups, then a page column centered at 680 px: the page title, a muted group heading, a surface card whose rows carry a title (and the laptop glyph of a device-scope key) with one description line on the left and the control on the right. A wide control stacks under the text; an inline error sits under the control. Dialogs use the compact group: no card surface, 40-px rows.',
    render: () => (
      <>
        <Ex caption=".m-snav · .m-snav-search · .m-snav-item (+ .is-active) · .m-snav-sep · .m-set-page · .m-set-group · .m-set-head · .m-set-card · .m-set-row (+ .is-stack) · .m-set-title · .m-set-device · .m-set-desc · .m-set-ctl · .m-set-unit · .m-set-err">
          <div className="m-window">
            <SettingsNav
              label="Settings pages"
              active="general"
              onSelect={noop}
              search={<input className="m-input" placeholder="Search settings" aria-label="Search settings" />}
              groups={[
                {
                  items: [
                    { id: 'general', label: 'General', icon: 'settings' },
                    { id: 'appearance', label: 'Appearance', icon: 'palette' },
                    { id: 'overlay', label: 'Overlay', icon: 'overlay' },
                    { id: 'dictation', label: 'Dictation', icon: 'mic' },
                  ],
                },
                {
                  items: [
                    { id: 'chat', label: 'Chat', icon: 'chat' },
                    { id: 'alerts', label: 'Alerts', icon: 'bell' },
                    { id: 'notes', label: 'Notes', icon: 'notes' },
                    { id: 'urgency', label: 'Urgency', icon: 'flag' },
                    { id: 'ai', label: 'AI', icon: 'sparkle' },
                  ],
                },
                {
                  items: [
                    { id: 'privacy', label: 'Privacy', icon: 'shield' },
                    { id: 'advanced', label: 'Advanced', icon: 'wrench' },
                  ],
                },
              ]}
            />
            <div className="m-main">
              <SettingsPage title="General">
                <SettingGroup title="Window" id="window">
                  <SettingRow title="Open at login" desc="Mode starts when you sign in.">
                    <Switch on label="Open at login" />
                  </SettingRow>
                  <SettingRow title="Start on" desc="The view the window opens to.">
                    <Select label="Start on" value="sessions" options={START_ON} onChange={noop} />
                  </SettingRow>
                  <SettingRow title="Density" device>
                    <Seg label="Density">
                      <SegItem active>Comfortable</SegItem>
                      <SegItem>Compact</SegItem>
                    </Seg>
                  </SettingRow>
                  <SettingRow title="Keep finished sessions" desc="Older ones leave the list.">
                    <input className="m-input" type="number" defaultValue={14} aria-label="Keep finished sessions" />
                    <span className="m-set-unit">days</span>
                  </SettingRow>
                </SettingGroup>
                <SettingGroup title="Profile" id="profile">
                  <SettingRow title="Name">
                    <input className="m-input" defaultValue="Ana" aria-label="Name" />
                  </SettingRow>
                  <SettingRow title="Onboarding" desc="Walk through the setup again.">
                    <Button quiet>Run again</Button>
                  </SettingRow>
                  <SettingRow title="Signature" desc="Added under the notes you share." stack error={<SettingError>Keep it under 280 characters.</SettingError>}>
                    <TextArea aria-label="Signature" defaultValue="Ana · sent from Mode" />
                  </SettingRow>
                </SettingGroup>
              </SettingsPage>
            </div>
          </div>
        </Ex>
        <Ex caption=".m-set-group.is-compact (dialogs)">
          <div style={{ width: 440 }}>
            <SettingGroup compact>
              <SettingRow title="Folder">
                <Combo label="Folder" value="~/code/mode" suggestions={['~/code/mode']} onChange={noop} />
              </SettingRow>
              <SettingRow title="Permission mode">
                <Seg label="Permission mode">
                  <SegItem>Ask</SegItem>
                  <SegItem active>Accept edits</SegItem>
                </Seg>
              </SettingRow>
            </SettingGroup>
          </div>
        </Ex>
      </>
    ),
  },
  {
    id: 'swatches',
    title: 'Swatch grid',
    note: 'A picker of pictures in a popover (10 §7.4.19): five to a row, 8-px gaps, each a round button holding the picture; the chosen one carries a 2-px fg ring, hover is --m-hover. No visible names: the name is the button’s label and tooltip. Arrows move in the grid, Enter or Space picks, Esc closes. The agent accessory uses it with the 40-px bot head.',
    render: () => (
      <>
        <Ex caption=".m-swatches · .m-swatch-cell (+ .is-on) · .m-avatar-agent.is-40">
          <Menu label="Accessory" style={{ padding: 0 }}>
            <SwatchGrid
              label="Accessory"
              value="specs"
              onPick={noop}
              options={['plain', ...AVATAR_VARIANTS].map((v) => ({
                id: v,
                label: VARIANT_NAME[v] ?? v,
                node: <AgentAvatar size={40} hue={108} variant={v === 'plain' ? undefined : v} />,
              }))}
            />
          </Menu>
        </Ex>
      </>
    ),
  },
  {
    id: 'notice',
    title: 'Notices',
    note: 'An inline notice (10 §7.4.20): sunken, a 12-px state glyph in the state color, one sentence in fg-2, at most one quiet action at the right. Never a colored side bar. It rises in.',
    render: () => (
      <>
        <Ex caption=".m-notice.is-bad · .m-notice-glyph · .m-btn.is-quiet">
          <div style={{ maxWidth: 480 }}>
            <Notice state="bad" action={<Button quiet>Retry</Button>}>
              Ollama did not answer at localhost:11434.
            </Notice>
          </div>
        </Ex>
        <Ex caption=".m-notice.is-info">
          <div style={{ maxWidth: 480 }}>
            <Notice>Chat keeps 90 days of history; older turns go at launch.</Notice>
          </div>
        </Ex>
      </>
    ),
  },
  {
    id: 'skeleton',
    title: 'Skeleton',
    note: SKELETON_NOTE,
    render: () => (
      <>
        <Ex caption=".m-skel.is-text · .w-80 / .w-60 / .w-40">
          <div style={{ maxWidth: 320 }}>
            <Skeleton width="80%" />
            <Skeleton width="60%" />
            <Skeleton width="40%" />
          </div>
        </Ex>
        <Ex caption=".m-stats.is-spend · .m-stat-n > .m-skel.is-n (32 px in the hero) · .m-stat-l > .m-skel.is-text">
          <Stats className="is-spend">
            {SPEND.map((t, i) => (
              <Stat key={i} hero={i === 0} value={<Skeleton variant="n" width={t.n} />} label={<Skeleton width={t.l} />} />
            ))}
          </Stats>
        </Ex>
        <Ex caption=".m-tracker-grid · .m-skel.is-square (the sweep starts 20 ms later per week column)">
          <div className="m-tracker">
            <div className="m-tracker-grid">
              {Array.from({ length: 7 * 12 }, (_, i) => (
                <Skeleton key={i} variant="square" column={Math.floor(i / 7)} />
              ))}
            </div>
          </div>
        </Ex>
        <Ex caption=".m-stats-bars · .m-skel.is-bar">
          <div style={{ maxWidth: 420 }}>
            <div className="m-stats-bars">
              {DAY_BARS.map((h, i) => (
                <Skeleton key={i} variant="bar" height={h} />
              ))}
            </div>
          </div>
        </Ex>
        <Ex caption=".m-stats.is-metrics.is-stale (a refetch in flight)">
          <Stats className="is-metrics is-stale">
            {metricTiles(wireframeSample().summary)
              .slice(0, 4)
              .map((t, i) => (
                <Stat key={i} value={t.value} label={t.label} title={t.title} />
              ))}
          </Stats>
        </Ex>
      </>
    ),
  },
  {
    id: 'nowplaying',
    title: 'Now playing',
    note: 'The local player in the overlay (10 §7.4.22, 02 §6.1, §7.1): the peek’s equalizer while a track plays, and the strip under the panel’s header with the artwork (a ♪ tile without one), title, artist, previous / play-pause / next and a 2-px progress hairline. The fill is drawn still here, as when paused or under reduced motion; the pending control holds the spinner.',
    render: () => (
      <>
        <Ex caption=".m-eq.is-playing · .m-eq (paused: resting heights)">
          <Flow>
            <Equalizer playing />
            <Equalizer playing={false} />
          </Flow>
        </Ex>
        <Ex caption=".m-nowplaying · .m-nowplaying-art · .m-nowplaying-fill.is-static">
          <div style={{ maxWidth: 520 }}>
            <NowPlayingStrip title="Northeaster" artist="The Eragny Quartet" artworkUrl={nowPlayingArt} playing reduced positionMs={65_000} durationMs={222_000} atMs={0} />
          </div>
        </Ex>
        <Ex caption=".m-nowplaying · ♪ tile · paused · next pending (.m-spinner)">
          <div style={{ maxWidth: 520 }}>
            <NowPlayingStrip title="Harbour Lights" artist="The Eragny Quartet" playing={false} pending="next" positionMs={30_000} durationMs={200_000} atMs={0} />
          </div>
        </Ex>
      </>
    ),
  },
]
