// Gallery sections: overlay … stats (components.html §overlay–§stats).
import { AgentAvatar, Mascot } from '../kit/avatars'
import { Button, MenuButton } from '../kit/button'
import { Card, CardActions, CardHead, Composer, PermissionCard, QuestionCard, SessionHead, Thread, ThreadAssistant, ThreadTool, ThreadUser } from '../kit/cards'
import { Activity, ActivityRow, BubbleUser, Chat, ChatInput, ChatMeta, ChatThread, Reply, Shimmer, SlashPop } from '../kit/chat'
import { Capture, Menu, MenuItem, MenuSep, Seg, SegItem, Tab, Tabs } from '../kit/controls'
import { Diff, GroupLabel, List, StateCol, Task } from '../kit/lists'
import { Overlay, Peek, Pill } from '../kit/overlay'
import { Inline, Meta, MetaLine } from '../kit/primitives'
import { HBars, Stat, StatRow, Stats, StatsBars, StatsHead } from '../kit/stats'
import { WIREFRAME_TODAY, wireframeSample } from '../app/stats/sample'
import { SpecTable } from '../app/stats/table'
import { dayBars, HOURS_HEADING, hourSection, metricTiles, modelTable, spendTiles, stateCounts } from '../app/stats/text'
import { modelShares } from '../app/stats/tracker'
import { SESSION_STATES } from '../kit/types'
import { Messages, S, SessionRow } from './data'
import { Ex, Flow, Item, type SectionSpec } from './wf'

const OVERLAY_TABS = [
  { id: 'sessions', label: 'Sessions' },
  { id: 'inbox', label: 'Inbox' },
  { id: 'notes', label: 'Notes' },
  { id: 'calendar', label: 'Calendar' },
  { id: 'music', label: 'Music' },
]

const LONG_PROMPT =
  'Summarize what changed in api-refactor this week and draft a release note for #release. Include the rate limiter, the auth fixture fix and anything Lena flagged in the docs PR. Keep it under 120 words, lead with what a user would notice, and end with the migration steps from guides/migration.md. If the invoice-parser failure is related, say so in one line; otherwise leave it out. Mention that the webhook spike is parked. Use the same tone as the last two release notes in CHANGELOG.md, no exclamation marks, and link the PR numbers. When you are done, pin the note so I can find it from the overlay, and tell me which files you read so I can double-check them before I post. Also check whether Priya\'s thread about the v2 rollout changes any of the dates.'

export const SECTIONS_2: SectionSpec[] = [
  {
    id: 'overlay',
    title: 'Overlay',
    note: 'Notch = the bezel: --m-fg background with the token set inverted, in either theme. Top-bar and right-edge are surface panels at radius 14. Rows are 32 px, tabs are text, the compact pill is mascot + one phrase + count.',
    render: () => (
      <>
        <Ex caption=".m-overlay.is-notch (expanded, Sessions tab)">
          <Overlay placement="notch" badge="waiting" phrase="waiting · api-refactor" count={3} tabs={OVERLAY_TABS} activeTab="sessions" footer={<Capture />}>
            <List>
              <GroupLabel>waiting</GroupLabel>
              <SessionRow s={S.apiRefactor} dense />
              <GroupLabel>working</GroupLabel>
              <SessionRow s={S.docsSite} dense />
              <GroupLabel>finished</GroupLabel>
              <SessionRow s={S.rateLimiter} dense />
              <GroupLabel>idle</GroupLabel>
              <SessionRow s={S.webhookSpike} dense />
            </List>
          </Overlay>
        </Ex>
        <Ex caption=".m-overlay.is-topbar (expanded, Inbox tab)">
          <Overlay placement="top-bar" badge="waiting" phrase="waiting · api-refactor" count={3} tabs={OVERLAY_TABS} activeTab="inbox" footer={<Capture />}>
            <List divided>
              <Messages />
            </List>
          </Overlay>
        </Ex>
        <Ex caption=".m-overlay.is-rightedge (expanded, Notes tab, capture just saved)">
          <Overlay placement="right-edge" badge="waiting" phrase="waiting · api-refactor" count={3} tabs={OVERLAY_TABS} activeTab="notes" footer={<Capture saved />}>
            <List style={{ padding: '4px 8px' }}>
              <GroupLabel style={{ paddingLeft: 4 }}>today</GroupLabel>
              <Task glyph="✱" title="Fix flaky auth test" meta="today" />
              <Task glyph="◑" title="Draft v2 changelog" meta="tomorrow" />
              <Task glyph="○" title="Book dentist" />
              <Task glyph="✓" title="Reply to Lena about PR 412" done />
            </List>
          </Overlay>
        </Ex>
        <Ex caption=".m-pill (compact) · .m-pill.is-notch">
          <Flow>
            <Pill badge="waiting" phrase="waiting · api-refactor" count={1} />
            <Pill badge="working" phrase="working · docs-site" />
            <Pill badge="unread" phrase="Sam · #release" count={3} />
            <Pill badge="waiting" phrase="waiting · api-refactor" count={1} notch />
          </Flow>
        </Ex>
        <Ex caption=".m-peek · .m-peek.is-top">
          <Flow>
            <Peek badge="waiting" />
            <Meta>right-edge peek</Meta>
            <Peek badge="waiting" top label="3 items" />
            <Meta>top-bar peek</Meta>
          </Flow>
        </Ex>
      </>
    ),
  },
  {
    id: 'mascot',
    title: 'Mascot and state badge',
    note: 'The duck artwork as an <img> at 20 / 32 / 48 px. Poses are not drawn: the 8-px badge at the bottom-right carries the state in the state colors.',
    render: () => (
      <>
        <Ex caption=".m-mascot .is-20 / default 32 / .is-48">
          <Flow>
            <Item>
              <Mascot size={20} badge="waiting" />
              <Meta>20</Meta>
            </Item>
            <Item>
              <Mascot badge="working" />
              <Meta>32</Meta>
            </Item>
            <Item>
              <Mascot size={48} badge="unread" />
              <Meta>48</Meta>
            </Item>
          </Flow>
        </Ex>
        <Ex caption=".m-badge-state .is-waiting … .is-untracked">
          <Flow>
            {SESSION_STATES.map((s) => (
              <Item key={s}>
                <Mascot badge={s} />
                <Meta>{s}</Meta>
              </Item>
            ))}
          </Flow>
        </Ex>
      </>
    ),
  },
  {
    id: 'cards',
    title: 'Cards',
    note: "Surface, 1-px line, radius 10, no shadow; never inside another bordered box. Query cards keep their pattern: title · count · ↗ · ⋮ · ×, rows 32 px with the ○ ◑ ✱ ✓ glyphs.",
    render: () => (
      <>
        <Ex caption=".m-card (session card, cards view)">
          <Card style={{ maxWidth: 320 }}>
            <CardHead avatar={<AgentAvatar hue={200} variant="beret" />} title="api-refactor" actions={<MenuButton />} />
            <MetaLine>
              mode · <code>feat/api</code> · 15 m
            </MetaLine>
            <Inline style={{ marginTop: 10 }}>
              <StateCol state="waiting" />
              <Diff add={43} del={18} />
              <Meta>· 4 files</Meta>
            </Inline>
            <CardActions>
              <Button variant="primary">Resume</Button>
            </CardActions>
          </Card>
        </Ex>
        <Ex caption=".m-card (query card) · .m-task · .m-task-glyph · .is-done">
          <Card style={{ maxWidth: 360 }}>
            <CardHead
              title="Urgent"
              count={4}
              actions={
                <>
                  <Button variant="icon" quiet aria-label="Expand">
                    ↗
                  </Button>
                  <MenuButton />
                  <Button variant="icon" quiet aria-label="Close">
                    ×
                  </Button>
                </>
              }
            />
            <Task glyph="✱" title="Fix flaky auth test" meta="today" />
            <Task glyph="◑" title="Draft v2 changelog" meta="tomorrow" />
            <Task glyph="○" title="Book dentist" />
            <Task glyph="✓" title="Reply to Lena about PR 412" done />
          </Card>
        </Ex>
      </>
    ),
  },
  {
    id: 'header',
    title: 'Session detail header',
    note: '32-px artwork avatar, title on one line, one meta line (repo · branch · model · elapsed: the model is named here and nowhere else), state in the column style, diff and usage, Resume + ⋮. External sessions add the word “external” to the meta line and nothing else.',
    render: () => (
      <>
        <Ex caption=".m-session-head · .m-session-title · .m-session-sub · .m-tabs">
          <SessionHead
            avatar={<AgentAvatar hue={200} size={32} variant="beret" />}
            title="api-refactor"
            repo="mode"
            branch="feat/api"
            model="claude-opus-5-5"
            elapsed="15 m"
            state="waiting"
            diff={{ add: 43, del: 18 }}
            files={4}
            usage="137k tokens · 6 turns"
          />
          <div style={{ marginTop: 10 }}>
            <Tabs>
              <Tab active>Overview</Tab>
              <Tab>Terminal</Tab>
              <Tab count={4}>Files</Tab>
              <Tab>Transcript</Tab>
            </Tabs>
          </div>
        </Ex>
        <Ex caption=".m-session-head (external session)">
          <SessionHead avatar={<AgentAvatar untracked variant="squares" />} title="scratch" path="~/code/scratch" external model="claude-opus-5-5" elapsed="2 d" primaryLabel="Open" primary={false} />
        </Ex>
      </>
    ),
  },
  {
    id: 'thread',
    title: 'Thread and composer',
    note: 'User prompt right-aligned in sunken; assistant in surface with a 1-px line; tool cards are one-line rows that expand; permission card = tool + command + Allow / Deny / Always; question card = radio and checkbox lists + Send. Composer = .m-input + Send, no helper text.',
    render: () => (
      <>
        <Ex caption=".m-thread · .m-thread-user / -assistant / -tool (+ .is-open) / -permission / -question">
          <Thread>
            <ThreadUser>Make the auth test deterministic. It fails about one run in five on CI.</ThreadUser>
            <ThreadAssistant>
              <p>
                The flake comes from a shared fixture: <code>client</code> reuses one database across tests and the token clock drifts. I will pin the clock and give each test its own transaction.
              </p>
              <p>Two files change; the suite should stay under a second.</p>
            </ThreadAssistant>
            <ThreadTool name="Edit" arg="tests/test_auth.py" diff={{ add: 12, del: 4 }} />
            <ThreadTool name="Read" arg="src/auth/fixtures.py" meta="84 lines" open>
              {'@pytest.fixture\ndef client(db):\n    yield TestClient(app, db=db)'}
            </ThreadTool>
            <PermissionCard tool="Bash" command="pytest tests/test_auth.py -q" />
            <QuestionCard
              question="Which retry policy for the rate limiter?"
              groups={[
                {
                  id: 'q1',
                  options: [
                    { id: 'exp', label: 'Exponential backoff, 5 attempts', checked: true },
                    { id: 'fixed', label: 'Fixed 2 s, 3 attempts' },
                    { id: 'none', label: 'No retries' },
                  ],
                },
                {
                  id: 'also',
                  legend: 'Also apply to',
                  multiple: true,
                  options: [
                    { id: 'webhook', label: 'Webhook delivery', checked: true },
                    { id: 'email', label: 'Outbound email' },
                  ],
                },
              ]}
            />
          </Thread>
        </Ex>
        <Ex caption=".m-composer">
          <Composer placeholder="Reply to api-refactor…" />
        </Ex>
      </>
    ),
  },
  {
    id: 'chat',
    title: 'Chat',
    darkFirst: true,
    note: "§6: one column, max 720 px, 24-px rhythm. User messages are sunken bubbles (radius 16) that clamp at eight lines; assistant replies are plain text on the canvas with copy · pin · time underneath, and nothing else. Tool use folds into activity rows; a pending reply shimmers. The input is a radius-24 sunken container with + and ↵, and the Auto / model line below. Shown dark first, as the owner's reference is dark.",
    render: () => (
      <>
        <Ex caption=".m-chat · .m-chat-thread · .m-bubble-user (+ .is-clamped, .m-show-more) · .m-activity (+ .is-open, .is-active) · .m-activity-row · .m-thumb · .m-reply · .m-reply-actions · .m-shimmer">
          <Chat>
            <ChatThread>
              <BubbleUser clamp>{LONG_PROMPT}</BubbleUser>
              <Activity label="Ran 3 commands" />
              <Activity label="Read 8 files" open status="4 background tasks completed">
                <ActivityRow thumb="md">guides/migration.md</ActivityRow>
                <ActivityRow thumb="py">tests/test_auth.py</ActivityRow>
                <ActivityRow thumb="ts">src/auth/limiter.ts</ActivityRow>
                <ActivityRow thumb="md">CHANGELOG.md</ActivityRow>
                <ActivityRow>Checked Playwright availability and loaded the migration guide draft from the docs-site session</ActivityRow>
              </Activity>
              <Reply time="just now">
                <p>
                  Three things changed in <code>api-refactor</code> this week:
                </p>
                <ul>
                  <li>The rate limiter now retries with exponential backoff (PR 418).</li>
                  <li>The auth test is deterministic: the fixture pins the clock and isolates the database (PR 417).</li>
                  <li>Lena's docs PR 412 adds the migration guide; the dates match Priya's thread.</li>
                </ul>
                <p>The invoice-parser failure is unrelated, and the webhook spike is parked.</p>
              </Reply>
              <BubbleUser>Draft it for #release.</BubbleUser>
              <Activity label="Updating the release note" active />
              <Shimmer />
            </ChatThread>
          </Chat>
        </Ex>
        <Ex caption=".m-chat-input (+ .m-plus, .m-send, .has-text) · .m-chat-meta">
          <Chat>
            <ChatInput editable={false} />
            <ChatMeta effort="Auto" model="opus-5-5" busy />
            <ChatInput editable={false} value="Draft it for #release." style={{ marginTop: 16 }} />
            <ChatMeta effort="Thorough" model="opus-5-5" />
          </Chat>
        </Ex>
        <Ex caption=".m-menu.is-plus (+ .is-dimmed, .m-reason, .m-hint)">
          <Menu plus>
            <MenuItem icon="notes">Attach note</MenuItem>
            <MenuItem icon="inbox" dimmed reason="Connect Gmail or Slack first">
              Attach message
            </MenuItem>
            <MenuItem icon="sessions">Attach session</MenuItem>
            <MenuItem icon="file">Add file</MenuItem>
            <MenuItem icon="slash" hint="/">
              Slash commands
            </MenuItem>
            <MenuSep />
            <MenuItem icon="link" hint="›">
              Connectors
            </MenuItem>
          </Menu>
        </Ex>
        <Ex caption=".m-slash-pop · .m-slash-row.is-active">
          <SlashPop
            active="/summarize"
            commands={[
              { name: '/summarize', description: 'Summarize the attached note, session or thread' },
              { name: '/todo', description: 'Turn the reply into a todo note' },
              { name: '/snooze', description: 'Snooze a message until a time' },
              { name: '/open', description: 'Open a note, session or message' },
              { name: '/link', description: 'Link the current note to a session' },
            ]}
          />
        </Ex>
      </>
    ),
  },
  {
    id: 'stats',
    title: 'Stats',
    note: 'The Stats view\u2019s pieces (03 §7A): the spend row with its hero figure (estimates at list price, ADR-0051: the one place currency appears), the metric tiles, a group label with a control at its right, horizontal share bars, a plain table of figures, tokens per day as neutral bars (today in fg-2), messages per hour, and sessions by state as a vertical list with counts.',
    render: () => {
      const { summary } = wireframeSample()
      const d = dayBars(summary, WIREFRAME_TODAY, 'this_week', 'monday')
      const h = hourSection(summary)
      const tiles = (list: ReturnType<typeof metricTiles>) => list.map((t, i) => <Stat key={i} value={t.value} label={t.label} title={t.title} hero={t.hero} />)
      return (
        <>
          <Ex caption=".m-stats.is-spend · .m-stat.is-hero">
            <Stats className="is-spend">{tiles(spendTiles(summary, 'this week'))}</Stats>
          </Ex>
          <Ex caption=".m-stats.is-metrics · .m-stat">
            <Stats className="is-metrics">{tiles(metricTiles(summary))}</Stats>
          </Ex>
          <Ex caption=".m-stats-head (a group label with a control)">
            <StatsHead label="breakdown">
              <Seg label="Breakdown">
                <SegItem active>Models</SegItem>
                <SegItem>Agents</SegItem>
                <SegItem>Days</SegItem>
              </Seg>
            </StatsHead>
          </Ex>
          <Ex caption=".m-hbars · .m-hbar-l · .m-hbar · .m-hbar-n">
            <div style={{ maxWidth: 520 }}>
              <HBars rows={modelShares(summary.by_model)} />
            </div>
          </Ex>
          <Ex caption=".m-table · th/td.is-num · .is-dim">
            <SpecTable spec={modelTable(summary)} />
          </Ex>
          <Ex caption=".m-stats-bars · .m-stats-bar.is-today · .m-stats-days">
            <div style={{ maxWidth: 420 }}>
              <Meta block>{d.heading}</Meta>
              <StatsBars values={d.bars.values} todayIndex={d.bars.todayIndex} days={d.bars.labels} titles={d.titles} />
            </div>
          </Ex>
          <Ex caption=".m-stats-bars.is-hours · .m-stats-days.is-hours">
            <div style={{ maxWidth: 420 }}>
              <Meta block>{HOURS_HEADING}</Meta>
              <StatsBars values={h.values} todayIndex={h.peak} days={h.labels} variant="hours" titles={h.titles} />
            </div>
          </Ex>
          <Ex caption=".m-stat-row (sessions by state)">
            <div style={{ maxWidth: 260 }}>
              <Meta block style={{ marginBottom: 4 }}>
                Sessions by state
              </Meta>
              {stateCounts(summary).map(({ state, count }) => (
                <StatRow key={state} state={state} count={count} />
              ))}
            </div>
          </Ex>
        </>
      )
    },
  },

]
