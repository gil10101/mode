// Gallery sections: round 2 (components.html §sidebar-v2–§palette).
import { anaPicture } from '../kit/mascot'
import { AgentAvatar, Brand } from '../kit/avatars'
import { Button, ButtonGroup, MenuButton } from '../kit/button'
import { Agenda, Calendar, Month, MonthCell } from '../kit/calendar'
import { Crumb, Input, Menu, MenuItem, MenuSep, Palette, Picker, Seg, SegItem, StaticInput, Switch, Tab, Tabs, TextArea } from '../kit/controls'
import { Main, MainBody, MainHead, ProfileRow, Sidebar, SidebarItem, SidebarMore, SidebarSubItem, Window } from '../kit/layout'
import { GroupLabel, List, Row } from '../kit/lists'
import { Inline, Meta, Stack } from '../kit/primitives'
import { Tracker, type TrackerDay } from '../kit/stats'
import { wireframeSample } from '../app/stats/sample'
import { ACTIVITY_META, streakLine } from '../app/stats/text'
import { BrandMark } from '../kit/brands'
import { AutomationRow, ConnectorRow, Form, FormField, IntegrationCard, Integrations, Shortcut } from '../kit/views'
import { NAV, NAV_BOTTOM, S, SessionRow } from './data'
import { Ex, Flow, Item, type SectionSpec } from './wf'

const CONVERSATIONS = ['Release note for api-refactor', "Tomorrow's 1:1 with Priya", 'Is the retry policy final?', 'Standup notes for Monday', 'Why did invoice-parser fail?']

/** The sidebar with the Chat list open (03 §2.3.2). */
function ChatSidebarWindow() {
  const sub = (
    <>
      {CONVERSATIONS.map((t, i) => (
        <SidebarSubItem key={t} active={i === 0} title={t} trailing={<MenuButton label={`${t} options`} />}>
          {t}
        </SidebarSubItem>
      ))}
      <SidebarMore>Show 3 more</SidebarMore>
    </>
  )
  return (
    <Window>
      <Sidebar
        items={NAV.map((i) => (i.id === 'chat' ? { ...i, sub } : i))}
        bottom={NAV_BOTTOM}
        active="chat"
        profile={<ProfileRow name="Ana" detail="1.2M tokens this week" src={anaPicture} />}
      />
      <Main>
        <MainHead title="Chat">
          <Button>New conversation</Button>
          <MenuButton label="More" />
        </MainHead>
        <MainBody />
      </Main>
    </Window>
  )
}

function SessionsWindow({ collapsed }: { collapsed?: boolean }) {
  return (
    <Window>
      <Sidebar items={NAV} bottom={NAV_BOTTOM} active="sessions" collapsed={collapsed} profile={<ProfileRow name="Ana" detail="1.2M tokens this week" src={anaPicture} />} />
      <Main>
        <MainHead title="Sessions">
          <Seg>
            <SegItem active>List</SegItem>
            <SegItem>Board</SegItem>
          </Seg>
          <MenuButton />
        </MainHead>
        <MainBody>
          <List>
            <GroupLabel>waiting</GroupLabel>
            <SessionRow s={S.apiRefactor} />
            <GroupLabel>working</GroupLabel>
            <SessionRow s={S.docsSite} />
            <GroupLabel>finished</GroupLabel>
            <SessionRow s={S.rateLimiter} />
          </List>
        </MainBody>
      </Main>
    </Window>
  )
}

const WEEK = [
  { key: 'mon', num: 19, label: 'Mon' },
  { key: 'tue', num: 20, label: 'Tue' },
  { key: 'wed', num: 21, label: 'Wed', today: true },
  { key: 'thu', num: 22, label: 'Thu' },
  { key: 'fri', num: 23, label: 'Fri' },
  { key: 'sat', num: 24, label: 'Sat' },
  { key: 'sun', num: 25, label: 'Sun' },
]

const EVENTS = [
  { id: 'freeze', day: 3, allDay: true, title: 'Docs freeze' },
  { id: 'standup', day: 2, start: 9 * 60 + 30, end: 10 * 60, title: 'Standup' },
  { id: 'release', day: 4, start: 10 * 60, end: 12 * 60, title: 'Release' },
  { id: 'review', day: 2, start: 11 * 60, end: 12 * 60, title: 'Review' },
  { id: 'dentist', day: 3, start: 13 * 60, end: 13 * 60 + 30, title: 'Dentist' },
]

// The year of Mode activity the wireframe draws, the tooltip of Fri 16 Oct open.
const TRACKER = wireframeSample()
const TRACKER_DAYS: TrackerDay[] = TRACKER.model.days.map((d, i) => (i === TRACKER.openIndex ? { ...d, tip: TRACKER.tips[i] } : d))

export const SECTIONS_3: SectionSpec[] = [
  {
    id: 'sidebar-v2',
    title: 'Sidebar v2 (round 2)',
    note: 'No app name in the window chrome: the title-bar slot holds only the traffic lights. Eight main items in the §7.1 order, a 1-px divider, Stats and Settings, then the profile row: the mascot as the default picture, the OS user name (editable in Settings → General) and “1.2M tokens this week”. The old footer line is gone. Collapsed = icons, divider, two icons, the picture. The Chat entry carries its conversations as a nested list that opens with the view (round 3, ADR-0055): five rows, “Show n more”, ⋮ on hover, the retention note.',
    render: () => (
      <>
        <Ex caption=".m-window · .m-sidebar · .m-sidebar-bottom · .m-profile">
          <SessionsWindow />
        </Ex>
        <Ex caption=".m-sidebar.is-collapsed (+ .m-sidebar-bottom, .m-profile)">
          <SessionsWindow collapsed />
        </Ex>
        <Ex caption=".m-sidebar-sub.is-open under Chat · .m-sidebar-subitem · .m-sidebar-more · .m-sidebar-subnote">
          <ChatSidebarWindow />
        </Ex>
      </>
    ),
  },
  {
    id: 'profile',
    title: 'Profile menu',
    note: 'Clicking the profile row opens a .m-menu: Edit profile, Quiet hours as a .m-switch, Settings, Quit. The switch is a 28 × 16 pill: sunken off, --m-ok on, never accent.',
    render: () => (
      <>
        <Ex caption=".m-profile (hover) · .m-menu · .m-switch (off)">
          <Flow style={{ alignItems: 'flex-end' }}>
            <div style={{ width: 204, padding: 8, border: '1px solid var(--m-line)', borderRadius: 'var(--m-r-card)' }}>
              <div className="m-sidebar-bottom" style={{ marginTop: 0 }}>
                <SidebarItem id="stats" label="Stats" icon="stats" />
                <SidebarItem id="settings" label="Settings" icon="settings" />
                <ProfileRow name="Ana" detail="1.2M tokens this week" src={anaPicture} style={{ background: 'var(--m-hover)' }} />
              </div>
            </div>
            <Menu>
              <MenuItem>Edit profile</MenuItem>
              <MenuItem trailing={<Switch on={false} label="Quiet hours" style={{ marginLeft: 'auto' }} />}>Quiet hours</MenuItem>
              <MenuSep />
              <MenuItem>Settings</MenuItem>
              <MenuItem>Quit</MenuItem>
            </Menu>
          </Flow>
        </Ex>
        <Ex caption=".m-switch · .m-switch.is-on">
          <Flow>
            <Item>
              <Switch on={false} />
              <Meta>off</Meta>
            </Item>
            <Item>
              <Switch on />
              <Meta>on</Meta>
            </Item>
          </Flow>
        </Ex>
      </>
    ),
  },
  {
    id: 'calendar-week',
    title: 'Calendar week (round 2)',
    stacked: true,
    note: '§7.2 remodel. Header = month and week range left; Today, ‹ ›, Day / Week / Month right. All-day row, hours 7–19 with “+ earlier / + later” strips, 1-px --m-line rules, today tinted --m-selected, a 1-px --m-fg now-line with a 6-px dot at the gutter. Event chips are flat sunken blocks: title (500) and time (11 px fg-3), no colored bar, no dot, no calendar color; selected = --m-selected. The 240-px agenda rail lists Today and Tomorrow as plain rows with a Join button only where a link exists.',
    render: () => (
      <Ex caption=".m-calendar · .m-cal-allday · .m-cal-more · .m-cal-now · .m-event (+ .is-selected) · .m-agenda">
        <Inline style={{ justifyContent: 'space-between', marginBottom: 8 }}>
          <span className="m-t-title">October 2026 · 19–25</span>
          <ButtonGroup>
            <Button>Today</Button>
            <Button variant="icon" aria-label="Previous week">
              ‹
            </Button>
            <Button variant="icon" aria-label="Next week">
              ›
            </Button>
            <Seg>
              <SegItem>Day</SegItem>
              <SegItem active>Week</SegItem>
              <SegItem>Month</SegItem>
            </Seg>
          </ButtonGroup>
        </Inline>
        <div style={{ display: 'flex', gap: 16, alignItems: 'flex-start', minWidth: 0 }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <Calendar days={WEEK} events={EVENTS} now={{ day: 2, minutes: 10 * 60 + 38 }} selectedId="review" />
          </div>
          <Agenda
            groups={[
              {
                label: 'Today',
                items: [
                  { id: 'standup', time: '09:30', title: 'Standup', join: true },
                  { id: 'review', time: '11:00', title: 'Review', join: true },
                ],
              },
              {
                label: 'Tomorrow',
                items: [
                  { id: 'freeze', time: 'all day', title: 'Docs freeze' },
                  { id: 'dentist', time: '13:00', title: 'Dentist' },
                ],
              },
            ]}
          />
        </div>
      </Ex>
    ),
  },
  {
    id: 'calendar-month',
    title: 'Calendar month cell row',
    note: 'Compact grid: day numbers top-left, up to three one-line .m-event chips, then a “+n” line; today tinted. Same chip, no color.',
    render: () => (
      <Ex caption=".m-month · .m-month-cell (+ .is-today) · .m-event">
        <Month>
          <MonthCell day={19} events={[{ id: 'standup', title: 'Standup' }]} />
          <MonthCell day={20} />
          <MonthCell
            day={21}
            today
            events={[
              { id: 'standup', title: 'Standup' },
              { id: 'review', title: 'Review' },
              { id: 'pairing', title: 'Pairing · Lena' },
              { id: 'retro', title: 'Retro' },
            ]}
          />
          <MonthCell
            day={22}
            events={[
              { id: 'freeze', title: 'Docs freeze' },
              { id: 'dentist', title: 'Dentist' },
            ]}
          />
          <MonthCell day={23} events={[{ id: 'release', title: 'Release' }]} />
          <MonthCell day={24} />
          <MonthCell day={25} />
        </Month>
      </Ex>
    ),
  },
  {
    id: 'tracker',
    title: 'Day tracker',
    note: '§7.3: GitHub-style grid of Mode activity only, 53 weeks × 7 days (a year ending today), 11-px squares with 2-px gaps, radius 2. Five steps: --m-sunken, then --m-ok mixed at 25 / 50 / 75 / 100 %. Month labels above, weekday letters left, the legend bottom-right, one streak line. The tooltip opens on hover or focus; one is drawn open here. It opens the breakdown of Stats.',
    render: () => (
      <Ex caption=".m-tracker · .m-tracker-months / -days / -grid · .lvl-0 … .lvl-4 · .m-tracker-tip (open) · .m-tracker-foot · .m-tracker-legend">
        <Meta block style={{ marginBottom: 8 }}>
          {ACTIVITY_META}
        </Meta>
        <Tracker days={TRACKER_DAYS} months={TRACKER.model.months} weekdays={TRACKER.model.weekdays} foot={streakLine(TRACKER.model.streak, TRACKER.model.empty)} openIndex={TRACKER.openIndex} />
      </Ex>
    ),
  },

  {
    id: 'subagents',
    title: 'Sessions list with subagents (round 2)',
    note: '§7.4: a parent whose session spawned agents ends its detail line with “▾ 2 agents”; expanded, .m-row.is-agent rows nest under it with a 24-px indent, the state column, the agent type as the name, the current tool as the detail and the elapsed time as meta. No avatar: a 16-px bot-head at 60 % stands in. The overlay shows only a “· 2 agents” suffix.',
    render: () => (
      <Ex caption=".m-list · .m-row (parent, “▾ 2 agents”) · .m-row.is-agent × 2">
        <List>
          <GroupLabel>waiting</GroupLabel>
          <SessionRow
            s={S.apiRefactor}
            detail={
              <>
                mode · <code>feat/api</code> · permission for pytest · ▾ 2 agents
              </>
            }
          />
          <Row
            state="working"
            agent
            avatar={<AgentAvatar />}
            name="code-reviewer"
            detail={
              <>
                <code>Read</code> src/auth/limiter.ts
              </>
            }
            meta="40 s"
          />
          <Row
            state="finished"
            agent
            avatar={<AgentAvatar />}
            name="test-runner"
            detail={
              <>
                <code>Bash</code> pytest -q · 48 passed
              </>
            }
            meta="12 s"
          />
          <GroupLabel>working</GroupLabel>
          <SessionRow s={S.docsSite} />
        </List>
      </Ex>
    ),
  },
  {
    id: 'crumb',
    title: 'Breadcrumb',
    note: '§7.5 drill-in rule: list → detail panel → full page → tabs → row → sub-detail. Every sub-detail page starts with a 12-px fg-3 breadcrumb, › separators, the last item in fg; Esc or ‹ goes up one level.',
    render: () => (
      <Ex caption=".m-crumb (three depths) · ‹ back">
        <Stack style={{ gap: 12 }}>
          <Crumb items={['api-refactor', 'Files', <code key="f">src/auth/limiter.ts</code>]} />
          <Crumb items={['api-refactor', 'Agents', 'code-reviewer']} />
          <Inline>
            <Button variant="icon" quiet aria-label="Back">
              ‹
            </Button>
            <Crumb items={['Automations', 'Morning brief', 'Tue 21 Oct · 08:00']} />
          </Inline>
        </Stack>
      </Ex>
    ),
  },
  {
    id: 'connectors',
    title: 'Session Connectors tab (round 2)',
    note: '§7.6: what this session can reach, read-only, from its init event and hooks. Rows are .m-row without the avatar column and a status glyph in the state column: connected ◉ ok, pending ○, failed ✕; permission rules use ✓ allowed and ✕ denied. A row drills into Integrations.',
    render: () => (
      <Ex caption=".m-tabs (Agents, Connectors) · .m-row.m-connector-row (+ .has-mark, .m-brand-mark.is-16) · .m-state.is-connected / .is-pending / .is-failed / .is-allowed / .is-denied">
        <Tabs>
          <Tab>Overview</Tab>
          <Tab>Terminal</Tab>
          <Tab count={4}>Files</Tab>
          <Tab count={2}>Agents</Tab>
          <Tab active>Connectors</Tab>
          <Tab>Transcript</Tab>
        </Tabs>
        <List style={{ borderRadius: '0 0 var(--m-r-card) var(--m-r-card)' }}>
          <GroupLabel>MCP servers</GroupLabel>
          <ConnectorRow state="connected" mark={<BrandMark id="github" size={16} label="" />} name="github" detail="stdio · github-mcp 1.4" meta="12 tools" />
          <ConnectorRow state="pending" mark={<BrandMark id="modelcontextprotocol" size={16} label="" />} name="playwright" detail="starting · browser not launched yet" meta="0 tools" />
          <ConnectorRow state="failed" mark={<BrandMark id="linear" size={16} label="" />} name="linear" detail="auth expired · reconnect in Integrations" meta="—" />
          <GroupLabel>Permission rules · acceptEdits</GroupLabel>
          <ConnectorRow
            state="allowed"
            name={
              <>
                <code>Read</code> · <code>Edit</code> · <code>Bash(pytest *)</code>
              </>
            }
            detail="from .claude/settings.json"
            meta="3 rules"
          />
          <ConnectorRow
            state="denied"
            name={
              <>
                <code>Bash(rm *)</code> · <code>WebFetch</code>
              </>
            }
            detail="from the project rules"
            meta="2 rules"
          />
          <GroupLabel>Plugins</GroupLabel>
          <ConnectorRow state="connected" stateLabel="loaded" name="design" detail="8 skills · 1 agent" meta="v0.4" />
          <GroupLabel>Hook relay</GroupLabel>
          <ConnectorRow state="connected" name="Mode relay" detail="last event 12 s ago · 5 hooks" meta="v3" />
        </List>
      </Ex>
    ),
  },
  {
    id: 'shortcuts',
    title: 'Shortcuts view (round 2)',
    note: '§7.7: the only place keys appear (ADR-0032). Search at the top, groups by surface, rows = action name, key caps (sunken, mono 11 px) that become editable on click, scope word in fg-3, conflicts flagged inline in --m-bad. ⋮ holds Reset to defaults, Import, Export. Replaces Settings → Hotkeys.',
    render: () => (
      <Ex caption=".m-input (search) · .m-group-label · .m-shortcut · .m-keys · .m-key · .is-conflict · editing (focus ring)">
        <List>
          <Inline style={{ padding: '8px 12px', flexWrap: 'nowrap' }}>
            <Input placeholder="Search shortcuts…" aria-label="Search shortcuts" style={{ flex: 1 }} />
            <MenuButton />
          </Inline>
          <GroupLabel>Overlay</GroupLabel>
          <Shortcut name="Toggle overlay" keys={['⌥', '⇧', 'Space']} scope="global" />
          <Shortcut name="Pin overlay" keys={['⌥', '⇧', 'P']} scope="global" />
          <GroupLabel>App window</GroupLabel>
          <Shortcut name="Command palette" keys={['⌘', 'K']} scope="app" />
          <Shortcut name="Capture a note" keys={['⌥', 'Space']} scope="global" editing />
          <GroupLabel>Sessions</GroupLabel>
          <Shortcut name="Resume" keys={['R']} scope="list" />
          <Shortcut name="Open" keys={['O']} scope="list" conflict="Also bound to Open in editor (Files)" />
        </List>
      </Ex>
    ),
  },
  {
    id: 'integrations',
    title: 'Integrations view (round 2)',
    stacked: true,
    note: '§7.8: a grid of cards, two per row: icon (every external app’s mark, 10 §7.4.16: Claude Code, Gmail, Slack, Google Calendar, Claude for the AI backends), name, status line, two count lines, one button and ⋮ (Settings, Diagnostics, Remove). The one accent on the screen is the card that needs the user: Slack’s Reconnect. “Available later” cards are dimmed with a one-line reason. Replaces Settings → Connectors and → Accounts.',
    render: () => (
      <>
        <Ex caption=".m-integrations > .m-integration · .m-brand with a .m-brand-mark · .m-state.is-connected / .is-idle / .is-failed · .m-switch">
          <Integrations>
            <IntegrationCard
              icon={<Brand brand="claudecode" label="Claude Code" />}
              name="Claude Code"
              status={{ state: 'connected', detail: 'hooks installed · relay connected' }}
              lines={[
                <>
                  CLI 2.1.4 · hooks v3 in <code>~/.claude/settings.json</code>
                </>,
                'last hook event 12 s ago · 7 sessions tracked',
              ]}
              action={{ label: 'Install hooks' }}
            />
            <IntegrationCard
              icon={<Brand brand="gmail" label="Gmail" />}
              name="Gmail"
              status={{ state: 'connected', detail: '2 accounts · synced 2 m ago' }}
              lines={['2 accounts · read-only scope', '412 messages this week · 12 unread']}
              action={{ label: 'Add account' }}
            />
            <IntegrationCard
              icon={<Brand brand="slack" label="Slack" />}
              name="Slack"
              status={{ state: 'failed', label: 'needs sign-in', detail: '1 workspace' }}
              lines={['docs-site.slack.com · token expired yesterday', 'watched: #release, #docs, #general']}
              action={{ label: 'Reconnect', primary: true }}
            />
            <IntegrationCard
              icon={<Brand brand="googlecalendar" label="Google Calendar" />}
              name="Google Calendar"
              status={{ state: 'idle', label: 'not connected' }}
              lines={['Events show in Calendar and the overlay pill', 'Nothing is written back']}
              action={{ label: 'Connect' }}
            />
            <IntegrationCard
              icon={<Brand brand="claude" label="AI backends" />}
              name="AI backends"
              status={{ state: 'connected', detail: 'Claude API key present' }}
              lines={[
                <>
                  <BrandMark id="ollama" size={12} label="" />
                  Ollama · <code>localhost:11434</code> · llama-3.3 ready
                </>,
                'LM Studio · not running',
              ]}
              action={{ label: 'Manage keys' }}
              extra={
                <span className="m-inline" style={{ marginLeft: 4, gap: 6 }}>
                  <Switch on={false} label="Local-only" />
                  <span className="m-t-dense m-fg-2">Local-only</span>
                </span>
              }
            />
          </Integrations>
        </Ex>
        <Ex caption=".m-integration.is-later">
          <GroupLabel style={{ paddingLeft: 0 }}>Available later</GroupLabel>
          <Integrations>
            <IntegrationCard
              later
              icon={<Brand brand="github" label="GitHub" />}
              name="GitHub"
              lines={['Available later · PR and issue triggers arrive with the GitHub trigger on the roadmap']}
            />
            <IntegrationCard
              later
              icon={<Brand brand="outlook" label="Outlook" />}
              name="Outlook"
              lines={['Available later · needs the Microsoft Graph connector; IMAP comes with it']}
            />
          </Integrations>
        </Ex>
      </>
    ),
  },
  {
    id: 'automations',
    title: 'Automations view (round 2)',
    stacked: true,
    note: '§7.9: schedule-triggered only in v1. Rows = .m-switch, name (500), trigger phrase, action phrase, last run (state glyph + relative time), next run in fg-3, ⋮ (Run now, Duplicate, History, Delete). The editor follows the drill-in rule and starts with a breadcrumb: Name; Schedule as a segmented control plus time and timezone; Action picker with its fields; Options; Save is the primary. Run history rows link to the created session or note.',
    render: () => (
      <>
        <Ex caption=".m-automation · .m-switch (+ .is-on) · .m-state (last run) · .m-row-meta (next run)">
          <List>
            <AutomationRow name="Morning brief" on trigger="Weekdays 08:00" action="Start session in mode · summarize inbox and calendar" lastRun={{ state: 'finished', label: '2 h ago' }} nextRun="tomorrow 08:00" />
            <AutomationRow name="Nightly test run" on trigger="Daily 23:00" action="Start session in mode · run pytest and note failures" lastRun={{ state: 'failed', label: '11 h ago' }} nextRun="today 23:00" />
            <AutomationRow name="Weekly changelog" on={false} trigger="Fridays 17:00" action="Ask Chat and save the answer as a note · Work/Mode" lastRun={{ state: 'finished', label: '4 d ago' }} nextRun="paused" />
          </List>
        </Ex>
        <Ex caption=".m-crumb · .m-form · .m-seg (schedule) · .m-input · .m-switch · .m-btn-primary (Save)">
          <Crumb items={['Automations', 'Morning brief']} />
          <Form style={{ marginTop: 12 }}>
            <FormField label="Name">
              <Input defaultValue="Morning brief" aria-label="Name" />
            </FormField>
            <FormField label="Schedule">
              <Inline>
                <Seg>
                  <SegItem>Daily</SegItem>
                  <SegItem active>Weekdays</SegItem>
                  <SegItem>Weekly</SegItem>
                  <SegItem>Custom</SegItem>
                </Seg>
                <Input defaultValue="08:00" style={{ width: 72 }} aria-label="Time" />
                <Picker value="Europe/Berlin" label="Timezone" />
              </Inline>
            </FormField>
            <FormField label="Action">
              <Picker value="Start a Claude Code session" label="Action" />
            </FormField>
            <FormField label="Repo">
              <Picker value="mode" label="Repo" />
            </FormField>
            <FormField label="Prompt">
              <TextArea aria-label="Prompt" defaultValue="Summarize today’s inbox and calendar into a note in Work/Mode; flag anything from Priya or #release." />
            </FormField>
            <FormField label="Permission mode">
              <Seg>
                <SegItem>Ask</SegItem>
                <SegItem active>Accept edits</SegItem>
                <SegItem>Bypass</SegItem>
              </Seg>
            </FormField>
            <FormField label="Run as">
              <Seg>
                <SegItem active>Headless</SegItem>
                <SegItem>Embedded</SegItem>
              </Seg>
            </FormField>
            <FormField label="Token cap">
              <Input defaultValue="200k" style={{ width: 88 }} aria-label="Token cap" />
            </FormField>
            <FormField label="Notify on failure">
              <Switch on label="Notify on failure" />
            </FormField>
            <FormField label="Keep">
              <Inline>
                <Input defaultValue="20" style={{ width: 64 }} aria-label="Keep last runs" />
                <span className="m-t-dense m-fg-2">last runs</span>
              </Inline>
            </FormField>
            <FormField>
              <ButtonGroup>
                <Button variant="primary">Save</Button>
                <MenuButton />
              </ButtonGroup>
            </FormField>
          </Form>
        </Ex>
        <Ex caption=".m-row.is-plain (run history: state · started · duration · link)">
          <List>
            <GroupLabel>Run history · Morning brief</GroupLabel>
            <Row
              state="finished"
              plain
              name="Tue 21 Oct · 08:00"
              detail={
                <>
                  2 m 14 s · <a href="#">session · morning-brief</a>
                </>
              }
              meta="2 h ago"
            />
            <Row
              state="finished"
              plain
              name="Mon 20 Oct · 08:00"
              detail={
                <>
                  1 m 58 s · <a href="#">session · morning-brief</a>
                </>
              }
              meta="1 d ago"
            />
            <Row state="failed" plain name="Fri 16 Oct · 08:00" detail="0 m 12 s · backend unreachable" meta="5 d ago" />
          </List>
        </Ex>
      </>
    ),
  },
  {
    id: 'palette',
    title: 'Command palette (proposed)',
    note: '§7.11, marked as an assumption: ⌘K opens a centered 560-px popover over sessions, notes, messages, automations and settings. One input, grouped result rows, the active row in --m-hover; matching is fuzzy (“re”).',
    render: () => (
      <Ex caption=".m-palette · .m-input.is-focused · .m-group-label · .m-palette-row (+ .is-active)">
        <Palette
          input={<StaticInput lg focused value="re" style={{ marginBottom: 4 }} />}
          activeId="rate-limiter"
          groups={[
            {
              label: 'Sessions',
              rows: [
                { id: 'rate-limiter', icon: 'sessions', label: 'rate-limiter', meta: 'finished · mode' },
                { id: 'api-refactor', icon: 'sessions', label: 'api-refactor', meta: 'waiting · mode' },
              ],
            },
            { label: 'Notes', rows: [{ id: 'note', icon: 'notes', label: 'Reply to Lena about PR 412', meta: 'done · Work/Mode' }] },
            { label: 'Inbox', rows: [{ id: 'msg', icon: 'inbox', label: 'Sam · #release · can we ship v2 behind the flag today?', meta: '09:41' }] },
            { label: 'Automations', rows: [{ id: 'auto', icon: 'automations', label: 'Morning brief', meta: 'weekdays 08:00' }] },
            { label: 'Settings', rows: [{ id: 'appearance', icon: 'settings', label: 'Appearance', meta: 'Settings' }] },
          ]}
        />
      </Ex>
    ),
  },
]
