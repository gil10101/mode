// Claude Code step preview: a session asks to run a command; the pointer goes to Allow and presses,
// the card fades, and the session is working again; 5.5 s a loop. Sample sessions, no model named.
import { AgentAvatar } from '../../../kit/avatars'
import { PermissionCard } from '../../../kit/cards'
import { GroupLabel, List, Row } from '../../../kit/lists'
import { Cursor } from './Cursor'
import { Frame } from './Frame'
import { useLoop } from './loop'

const PERIOD = 5500
const MOVE_AT = 700
const PRESS_AT = 2400
const ALLOWED_AT = 2600

export function SessionsPreview() {
  const t = useLoop(PERIOD)
  const waiting = t < ALLOWED_AT
  const api = (
    <Row
      key="api"
      className={waiting ? undefined : 'm-ob-enter'}
      state={waiting ? 'waiting' : 'working'}
      avatar={<AgentAvatar hue={200} variant="beret" />}
      name="api-refactor"
      detail={waiting ? 'wants to run Bash: npm test' : 'running npm test'}
      meta={waiting ? '15 s' : 'now'}
      menu={false}
    />
  )
  return (
    <Frame title="Sessions" step="claude">
      <div className="m-ob-view">
        <List>
          {waiting && <GroupLabel>waiting</GroupLabel>}
          {waiting && api}
          <GroupLabel>working</GroupLabel>
          {!waiting && api}
          <Row state="working" avatar={<AgentAvatar hue={140} variant="specs" />} name="docs-site" detail="editing guides/migration.md" meta="2 min" menu={false} />
          <GroupLabel>finished</GroupLabel>
          <Row state="unread" avatar={<AgentAvatar hue={200} variant="beanie" />} name="rate-limiter" detail="+86 −4 · 4 files" meta="20 min" menu={false} />
        </List>
        {waiting && <PermissionCard tool="Bash" command="npm test" className={t >= PRESS_AT ? 'm-ob-leave' : undefined} />}
      </div>
      <Cursor to={t >= MOVE_AT && t < ALLOWED_AT ? '.m-thread-permission .m-btn-primary' : null} pressed={t >= PRESS_AT && t < PRESS_AT + 160} visible={t >= 300 && t < PERIOD - 400} />
    </Frame>
  )
}
