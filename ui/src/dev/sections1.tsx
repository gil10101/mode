// Gallery sections: state column … folder tree (components.html §state–§tree).
import { AgentAvatar, PersonAvatar } from '../kit/avatars'
import { Button, ButtonGroup, MenuButton } from '../kit/button'
import { Capture, Chip, Input, Menu, MenuItem, MenuSep, Query, Seg, SegItem, StaticInput, Tab, Tabs, TextArea, Tree, TreeItem } from '../kit/controls'
import { Diff, GroupLabel, List, StateCol, Usage } from '../kit/lists'
import { Meta, Stack } from '../kit/primitives'
import { SESSION_STATES } from '../kit/types'
import { Messages, S, SessionRow } from './data'
import { Ex, Flow, Item, type SectionSpec } from './wf'

export const SECTIONS_1: SectionSpec[] = [
  {
    id: 'state',
    title: 'State column',
    note: 'Fixed 88 px, glyph box 16 px + lowercase word, so a list scans as a column. .is-unread is a finished session not yet looked at: green glyph, bold name. The working glyph is the braille spinner (steps(10) over 800 ms; static ⠿ under reduced motion).',
    render: () => (
      <>
        <Ex caption=".m-state .is-waiting … .is-untracked">
          <div className="wf-cols" style={{ gridTemplateColumns: 'repeat(4,88px)', gap: '10px 12px' }}>
            {SESSION_STATES.map((s) => (
              <StateCol key={s} state={s} />
            ))}
          </div>
        </Ex>
        <Ex caption="State column demo — .m-list + .m-group-label + .m-row, seven states in the design system's order">
          <List>
            <GroupLabel>waiting</GroupLabel>
            <SessionRow s={S.apiRefactor} />
            <GroupLabel>working</GroupLabel>
            <SessionRow s={S.docsSite} />
            <GroupLabel>failed</GroupLabel>
            <SessionRow s={S.invoiceParser} />
            <GroupLabel>finished</GroupLabel>
            <SessionRow s={S.rateLimiter} />
            <SessionRow s={S.readmeTidy} />
            <GroupLabel>idle</GroupLabel>
            <SessionRow s={S.webhookSpike} />
            <GroupLabel>untracked</GroupLabel>
            <SessionRow s={S.scratch} />
          </List>
        </Ex>
      </>
    ),
  },
  {
    id: 'rows',
    title: 'List rows',
    note: '[state 88] [avatar 24] [name / detail] [meta 11 px] [⋮]. 40 px in the app, 32 px in the overlay. Selected = background only. Every text cell has min-width:0 and clips with an ellipsis.',
    render: () => (
      <>
        <Ex caption=".m-row (40 px) · .is-selected · .is-unread">
          <List>
            <SessionRow s={S.apiRefactor} selected />
            <SessionRow s={S.rateLimiter} />
            <SessionRow
              s={S.docsSite}
              detail={
                <>
                  docs-site · <code>main</code> · a very long detail line that must clip with an ellipsis rather than wrap or push the meta column
                </>
              }
            />
          </List>
        </Ex>
        <Ex caption=".m-row.is-dense (32 px, overlay)">
          <List>
            <SessionRow s={S.apiRefactor} dense />
            <SessionRow s={S.invoiceParser} dense />
          </List>
        </Ex>
      </>
    ),
  },
  {
    id: 'avatars',
    title: 'Avatars',
    note: 'Session avatar = the bot-head artwork inside a circle tinted per repo (--repo-hue through color-mix); no letters, no >_. Sender avatar = photo or neutral initials, with a 12-px source mark on a surface disc: simplified Gmail and Slack marks in their brand colors (trademark use noted in the docs).',
    render: () => (
      <>
        <Ex caption=".m-avatar-agent (+ .is-32, .is-untracked)">
          <Flow>
            <Item>
              <AgentAvatar hue={200} variant="specs" />
              <Meta>mode</Meta>
            </Item>
            <Item>
              <AgentAvatar hue={140} variant="specs" />
              <Meta>docs-site</Meta>
            </Item>
            <Item>
              <AgentAvatar hue={200} size={32} />
              <Meta>32 px</Meta>
            </Item>
            <Item>
              <AgentAvatar untracked />
              <Meta>untracked</Meta>
            </Item>
          </Flow>
        </Ex>
        <Ex caption=".m-avatar-person (+ .m-src-gmail / .m-src-slack)">
          <Flow>
            <Item>
              <PersonAvatar initials="PN" />
              <Meta>initials</Meta>
            </Item>
            <Item>
              <PersonAvatar initials="PN" source="gmail" />
              <Meta>.m-src-gmail</Meta>
            </Item>
            <Item>
              <PersonAvatar initials="SO" source="slack" />
              <Meta>.m-src-slack</Meta>
            </Item>
            <Item>
              <PersonAvatar initials="SO" source="slack" size={32} />
              <Meta>32 px</Meta>
            </Item>
          </Flow>
        </Ex>
      </>
    ),
  },
  {
    id: 'messages',
    title: 'Message rows',
    note: 'Line 1 = sender · #channel pill or Inbox · @ only when mentioned · time. Line 2 = subject or first text line. Urgent = a 2-px accent bar on the left edge, nothing else. No tier words, no why line (that lives in the detail panel).',
    render: () => (
      <Ex caption=".m-msg · .is-urgent · .is-unread · .m-tag · .m-at">
        <List divided>
          <Messages />
        </List>
      </Ex>
    ),
  },
  {
    id: 'diff',
    title: 'Diff and usage',
    note: 'Mono 12 px, always together. Usage is tokens and turns with a 4-px neutral bar against the weekly soft token budget.',
    render: () => (
      <>
        <Ex caption=".m-diff .add / .del">
          <Flow>
            <Diff add={43} del={18} />
            <Meta>· 4 files</Meta>
            <Diff add={2} del={0} />
          </Flow>
        </Ex>
        <Ex caption=".m-usage + .m-usage-bar">
          <Usage tokens="1.2M" turns={37} budget="3M" pct={40} style={{ maxWidth: 280 }} />
        </Ex>
      </>
    ),
  },
  {
    id: 'buttons',
    title: 'Buttons and menus',
    note: '28 px, radius 6, sunken fill, no border. One primary per screen. Everything secondary goes behind ⋮; shortcut letters appear only in menu rows and the Shortcuts sheet, as plain muted text.',
    render: () => (
      <>
        <Ex caption=".m-btn · .m-btn-primary · .m-btn-icon · .is-quiet">
          <ButtonGroup>
            <Button variant="primary">Resume</Button>
            <Button>Open</Button>
            <Button>Deny</Button>
            <Button disabled>Always</Button>
            <Button variant="icon" aria-label="Open in new window">
              ↗
            </Button>
            <MenuButton />
          </ButtonGroup>
        </Ex>
        <Ex caption=".m-menu (+ --m-shadow-pop)">
          <Menu>
            <MenuItem shortcut="O">Open</MenuItem>
            <MenuItem shortcut="T">Open in terminal</MenuItem>
            <MenuItem>Note from session</MenuItem>
            <MenuItem>Summarize</MenuItem>
            <MenuSep />
            <MenuItem>Shortcuts…</MenuItem>
            <MenuItem danger>Remove</MenuItem>
          </Menu>
        </Ex>
      </>
    ),
  },
  {
    id: 'inputs',
    title: 'Inputs and capture',
    note: 'Sunken fill, no border; focus = a 1-px neutral ring on :focus-visible only, never accent. Capture is one input; after saving, “Saved · Open” replaces the placeholder for 3 s. No icons, no shortcut text.',
    render: () => (
      <>
        <Ex caption=".m-input · .is-focused · textarea.m-input">
          <Stack style={{ maxWidth: 360 }}>
            <Input placeholder="Search notes…" aria-label="Search notes" />
            <StaticInput focused value="rate limiter" />
            <TextArea aria-label="Note body" defaultValue="Pin the clock in the auth fixture; isolate the DB per test." />
          </Stack>
        </Ex>
        <Ex caption=".m-capture — empty · typing · .m-capture-saved">
          <List style={{ maxWidth: 420, padding: 0 }}>
            <Capture />
            <Capture>
              <StaticInput focused value="Call Priya about the v2 rollout" />
            </Capture>
            <Capture saved />
          </List>
        </Ex>
      </>
    ),
  },
  {
    id: 'tabs',
    title: 'Tabs, segmented control, chips',
    note: 'Text tabs with a 2-px underline in fg that slides to the chosen tab; counts as plain muted numbers. Segmented control on a sunken track, its surface thumb sliding between segments (160 ms; anchor positioning, a crossfade where it is missing). Chips are sunken pills; only the person chip in the query bar uses --m-accent-bg.',
    render: () => (
      <>
        <Ex caption=".m-tabs · .m-tab.is-active · .m-n">
          <Tabs>
            <Tab active>Overview</Tab>
            <Tab>Terminal</Tab>
            <Tab count={4}>Files</Tab>
            <Tab>Transcript</Tab>
          </Tabs>
        </Ex>
        <Ex caption=".m-seg · .m-seg-item.is-active">
          <Flow>
            <Seg>
              <SegItem active>List</SegItem>
              <SegItem>Board</SegItem>
              <SegItem caret>Status</SegItem>
            </Seg>
            <Seg>
              <SegItem>Editor</SegItem>
              <SegItem active>Split</SegItem>
              <SegItem>Preview</SegItem>
            </Seg>
          </Flow>
        </Ex>
        <Ex caption=".m-chip · .is-person">
          <Flow>
            <Chip>#design</Chip>
            <Chip removable>#launch</Chip>
            <Chip person={{ initials: 'P' }} removable>
              Priya
            </Chip>
            <Chip>type · note</Chip>
            <Chip>this week</Chip>
          </Flow>
        </Ex>
        <Ex caption=".m-query (tag search in the query bar)">
          <Query style={{ maxWidth: 520 }} placeholder="Search notes…">
            <Chip person={{ initials: 'P' }} removable>
              Priya
            </Chip>
            <Chip removable>#launch</Chip>
            <Chip>this week</Chip>
          </Query>
        </Ex>
      </>
    ),
  },
  {
    id: 'tree',
    title: 'Folder tree and views',
    note: '▸/▾ chevrons, 16-px nesting, counts right. Premade views carry the task glyph; custom groupings (Groceries, Gym) are listed last.',
    render: () => (
      <Ex caption=".m-tree · .m-tree-item.is-open / .is-leaf / .is-active · .m-tree-children">
        <Tree
          style={{ maxWidth: 300 }}
          expanded={['work', 'mode']}
          activeId="mode"
          nodes={[
            {
              id: 'work',
              label: 'Work',
              count: 12,
              children: [
                {
                  id: 'mode',
                  label: 'Mode',
                  count: 8,
                  children: [
                    { id: 'launch', label: 'Launch', count: 3 },
                    { id: 'design', label: 'Design', count: 5 },
                  ],
                },
                { id: 'clients', label: 'Clients', count: 4, children: [] },
              ],
            },
            { id: 'personal', label: 'Personal', count: 6, children: [] },
          ]}
        >
          <GroupLabel>Views</GroupLabel>
          <TreeItem leaf glyph="○" count={6}>
            Todo
          </TreeItem>
          <TreeItem leaf glyph="◑" count={2}>
            Doing
          </TreeItem>
          <TreeItem leaf glyph="✱" count={1}>
            Urgent
          </TreeItem>
          <TreeItem leaf glyph="✓" count={23}>
            Done
          </TreeItem>
          <TreeItem leaf glyph="" count={3}>
            Today
          </TreeItem>
          <TreeItem leaf glyph="" count={2}>
            Tomorrow
          </TreeItem>
          <TreeItem leaf glyph="#" count={4}>
            Groceries
          </TreeItem>
          <TreeItem leaf glyph="#" count={2}>
            Gym
          </TreeItem>
        </Tree>
      </Ex>
    ),
  },
]
