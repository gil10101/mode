// The gallery's sample data: sessions api-refactor, docs-site, invoice-parser
// across the repos mode (hue 200) and docs-site (hue 140), Sam's Slack
// message in #release, Priya's email.
import type { ReactNode } from 'react'
import { AgentAvatar, PersonAvatar } from '../kit/avatars'
import { MsgRow, Row, type MsgRowProps, type RowProps } from '../kit/lists'
import type { SidebarItemSpec } from '../kit/layout'
import type { SessionState } from '../kit/types'

export interface SessionSample {
  state: SessionState
  hue?: number
  /** 05 §11.2: the accessory the bot head wears (the same per name as the wireframes). */
  variant?: string
  untracked?: boolean
  unread?: boolean
  name: string
  detail: ReactNode
  meta: string
}

export const S = {
  apiRefactor: {
    state: 'waiting',
    hue: 200,
    variant: 'beret',
    name: 'api-refactor',
    detail: (
      <>
        mode · <code>feat/api</code> · permission for pytest
      </>
    ),
    meta: '15 m',
  },
  docsSite: {
    state: 'working',
    hue: 140,
    variant: 'specs',
    name: 'docs-site',
    detail: (
      <>
        docs-site · <code>main</code> · editing guides/migration.md
      </>
    ),
    meta: '2 m',
  },
  invoiceParser: {
    state: 'failed',
    hue: 200,
    variant: 'tophat',
    name: 'invoice-parser',
    detail: (
      <>
        mode · <code>feat/invoices</code> · pytest exited 1
      </>
    ),
    meta: '1 h',
  },
  rateLimiter: {
    state: 'unread',
    unread: true,
    hue: 200,
    variant: 'beanie',
    name: 'rate-limiter',
    detail: (
      <>
        mode · <code>feat/api</code> · 4 files changed
      </>
    ),
    meta: '20 m',
  },
  readmeTidy: {
    state: 'finished',
    hue: 140,
    variant: 'mustache',
    name: 'readme-tidy',
    detail: (
      <>
        docs-site · <code>main</code> · 1 file changed
      </>
    ),
    meta: '3 h',
  },
  webhookSpike: {
    state: 'idle',
    hue: 200,
    variant: 'cap',
    name: 'webhook-spike',
    detail: (
      <>
        mode · <code>spike/webhooks</code> · no activity
      </>
    ),
    meta: '1 d',
  },
  scratch: {
    state: 'untracked',
    untracked: true,
    variant: 'squares',
    name: 'scratch',
    detail: (
      <>
        <code>~/code/scratch</code> · no hooks
      </>
    ),
    meta: '2 d',
  },
} satisfies Record<string, SessionSample>

type RowFlags = Pick<RowProps, 'selected' | 'dense' | 'detail' | 'onClick'>

export function SessionRow({ s, ...flags }: { s: SessionSample } & RowFlags) {
  return <Row state={s.state} unread={s.unread} avatar={<AgentAvatar hue={s.hue} variant={s.variant} untracked={s.untracked} />} name={s.name} detail={s.detail} meta={s.meta} {...flags} />
}

export const MESSAGES: MsgRowProps[] = [
  {
    avatar: <PersonAvatar initials="SO" source="slack" />,
    sender: 'Sam',
    channel: '#release',
    mentioned: true,
    time: '09:41',
    preview: 'can we ship v2 behind the flag today? need your sign-off before the 3 pm train',
    urgent: true,
    unread: true,
  },
  { avatar: <PersonAvatar initials="PN" source="gmail" />, sender: 'Priya', time: '08:12', preview: 'Re: API v2 rollout', unread: true },
  { avatar: <PersonAvatar initials="LV" source="slack" />, sender: 'Lena', channel: '#docs', time: 'yesterday', preview: 'pushed the migration guide draft, PR 412 is up' },
]

export function Messages() {
  return (
    <>
      {MESSAGES.map((m) => (
        <MsgRow key={m.sender as string} {...m} />
      ))}
    </>
  )
}

export const NAV: SidebarItemSpec[] = [
  { id: 'sessions', label: 'Sessions', icon: 'sessions', count: 7, waiting: true },
  { id: 'inbox', label: 'Inbox', icon: 'inbox', count: 12 },
  { id: 'notes', label: 'Notes', icon: 'notes' },
  { id: 'calendar', label: 'Calendar', icon: 'calendar' },
  { id: 'chat', label: 'Chat', icon: 'chat' },
  { id: 'agents', label: 'Agents', icon: 'agents' },
  { id: 'automations', label: 'Automations', icon: 'automations' },
  { id: 'integrations', label: 'Integrations', icon: 'integrations' },
  { id: 'shortcuts', label: 'Shortcuts', icon: 'shortcuts' },
]

export const NAV_BOTTOM: SidebarItemSpec[] = [
  { id: 'stats', label: 'Stats', icon: 'stats' },
  { id: 'settings', label: 'Settings', icon: 'settings' },
]
