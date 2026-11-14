// Notes step preview: the list fills with three notes, the capture box types a line and the note
// appears at the top, then the search field types "retry" and the list narrows to the two notes
// that match, then clears; 6.5 s a loop.
import { Capture, StaticInput } from '../../../kit/controls'
import { GroupLabel, List, Task } from '../../../kit/lists'
import { Frame } from './Frame'
import { typed, useLoop } from './loop'

const PERIOD = 6500
export const CAPTURED = 'Call Priya about the retry policy'
const NOTES = [
  { at: 300, glyph: '○', title: 'Release checklist', meta: 'Fri' },
  { at: 700, glyph: '◑', title: 'Retry backoff in the sync worker', meta: '2/5' },
  { at: 1100, glyph: '✓', title: 'Rotate the API keys', done: true },
]
const TYPE_FROM = 1700
const SAVED_AT = 3300
const SEARCH_FROM = 3800
const SEARCH_CLEAR = 5800
export const SEARCH = 'retry'

export function NotesPreview() {
  const t = useLoop(PERIOD)
  const saved = t >= SAVED_AT
  const query = t >= SEARCH_CLEAR ? '' : typed(SEARCH, t, SEARCH_FROM, 10)
  const searching = t >= SEARCH_FROM && t < SEARCH_CLEAR
  const shown = (title: string) => query.length < 3 || title.toLowerCase().includes(query)
  return (
    <Frame title="Notes" step="notes" head={<StaticInput className="m-ob-search" value={query || undefined} placeholder="Search notes" focused={searching} />}>
      <div className="m-ob-view">
        <Capture value={saved ? '' : typed(CAPTURED, t, TYPE_FROM)} onChange={() => {}} saved={saved && !searching} />
        <List>
          <GroupLabel>{query.length >= 3 ? `Matching “${query}”` : 'Unsorted'}</GroupLabel>
          {saved && shown(CAPTURED) && <Task key="captured" className="m-ob-enter" glyph="○" title={CAPTURED} meta="just now" />}
          {NOTES.filter((n) => t >= n.at && shown(n.title)).map((n) => (
            <Task key={n.title} className="m-ob-enter" glyph={n.glyph} title={n.title} meta={n.meta} done={n.done} />
          ))}
        </List>
      </div>
    </Frame>
  )
}
