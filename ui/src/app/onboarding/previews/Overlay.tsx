// Overlay step preview: the pill at the chosen placement, waiting on a
// permission with Allow, then allowed; 5 s a loop.
import { Button } from '../../../kit/button'
import { Pill } from '../../../kit/overlay'
import { Frame, type Placement } from './Frame'
import { useLoop } from './loop'

const PERIOD = 5000

export function OverlayPreview({ placement }: { placement: Placement }) {
  const t = useLoop(PERIOD)
  const waiting = t < 2500
  return (
    <Frame step="placement">
      <div className={`m-ob-screen is-${placement}`}>
        <div className="m-ob-spot">
          <Pill
            notch={placement === 'notch'}
            badge={waiting ? 'waiting' : 'working'}
            phrase={waiting ? 'api-refactor · Bash: npm test' : 'Allowed · api-refactor'}
            actions={waiting ? <Button variant="primary">Allow</Button> : undefined}
          />
        </div>
      </div>
    </Frame>
  )
}
