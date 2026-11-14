// Automations step preview: "Morning brief" filled in the way the editor (03 §7.9) takes it, each change
// caused by the caret or the pointer: the name typed with a caret, the pointer opening the When picker
// and choosing Weekdays from its menu, the time typed, the Do picker opened and Ask Chat chosen, the
// prompt typed, then Save pressed; the form fades and the automation is a row with its next run; 8 s a
// loop. The starter's row follows the step's switch.
import type { ReactNode } from 'react'
import { Button } from '../../../kit/button'
import { Menu, MenuItem, Picker, StaticInput } from '../../../kit/controls'
import { List } from '../../../kit/lists'
import { AutomationRow, Form, FormField } from '../../../kit/views'
import { Cursor } from './Cursor'
import { Frame } from './Frame'
import { typed, useLoop } from './loop'

const PERIOD = 8000
const NAME = 'Morning brief'
const TIME = '08:00'
const PROMPT = "Summarize today's inbox and calendar"
/** The editor's choices (Editor.tsx's KIND_LABEL and ACTION_LABEL). */
const WHEN = ['Daily', 'Weekdays', 'Weekly', 'Custom']
const DO = ['Start a Claude Code session', 'Create a note', 'Ask Chat']

// The script, in ms into the loop. A glide is 600 ms (the cursor's transition); a press 160 ms.
const NAME_FROM = 200
const WHEN_MOVE = 1000
const WHEN_PRESS = 1600
const WHEN_ITEM = 1800
const WHEN_HOVER = 2300
const WHEN_PICK = 2450
const TIME_MOVE = 2650
const TIME_PRESS = 3150
const TIME_FROM = 3250
const DO_MOVE = 3700
const DO_PRESS = 4200
const DO_ITEM = 4400
const DO_HOVER = 4900
const DO_PICK = 5050
const PROMPT_FROM = 5250
const SAVE_MOVE = 6400
const SAVE_PRESS = 6950
const SAVED_AT = 7150
const PRESS_MS = 160

const pressing = (t: number, at: number) => t >= at && t < at + PRESS_MS

/** A picker with the menu it opens, drawn in the kit's menu styles under it. */
function Pick({ value, label, open, items, hover, testId }: { value: ReactNode; label: string; open: boolean; items: string[]; hover: string | null; testId: string }) {
  return (
    <div className="m-ob-pick" data-testid={testId}>
      <Picker value={value} label={label} />
      {open && (
        <Menu label={label} className="m-ob-enter">
          {items.map((i) => (
            <MenuItem key={i} className={i === hover ? 'is-hover' : undefined}>
              {i}
            </MenuItem>
          ))}
        </Menu>
      )}
    </div>
  )
}

export function AutomationsPreview({ starterOn = false }: { starterOn?: boolean }) {
  const t = useLoop(PERIOD)
  const saved = t >= SAVED_AT
  const whenOpen = t >= WHEN_PRESS + PRESS_MS && t < WHEN_PICK + PRESS_MS
  const doOpen = t >= DO_PRESS + PRESS_MS && t < DO_PICK + PRESS_MS
  const target =
    t >= SAVE_MOVE
      ? '.m-form .m-btn-primary'
      : t >= DO_ITEM
        ? '[data-testid="pick-do"] .m-menu-item:nth-child(3)'
        : t >= DO_MOVE
          ? '[data-testid="pick-do"] > .m-input'
          : t >= TIME_MOVE
            ? '.m-ob-time'
            : t >= WHEN_ITEM
              ? '[data-testid="pick-when"] .m-menu-item:nth-child(2)'
              : t >= WHEN_MOVE
                ? '[data-testid="pick-when"] > .m-input'
                : null
  const pressed = [WHEN_PRESS, WHEN_PICK, TIME_PRESS, DO_PRESS, DO_PICK, SAVE_PRESS].some((at) => pressing(t, at))
  const caret = t < WHEN_MOVE ? 'name' : t >= TIME_PRESS && t < DO_MOVE ? 'time' : t >= DO_PICK + PRESS_MS && t < SAVE_PRESS ? 'prompt' : null
  return (
    <Frame title="Automations" step="automations">
      <div className="m-ob-view">
        {!saved && (
          <div className={`m-card${t >= SAVE_PRESS ? ' m-ob-leave' : ''}`}>
            <Form>
              <FormField label="Name">
                <StaticInput value={typed(NAME, t, NAME_FROM, 18) || undefined} placeholder="Name" focused={caret === 'name'} />
              </FormField>
              <FormField label="When">
                <div className="m-inline">
                  <Pick testId="pick-when" label="When" value={t >= WHEN_PICK ? 'Weekdays' : 'Daily'} open={whenOpen} items={WHEN} hover={t >= WHEN_HOVER ? 'Weekdays' : null} />
                  <StaticInput className="m-ob-time" value={t >= TIME_FROM ? typed(TIME, t, TIME_FROM, 12) || undefined : undefined} placeholder="09:00" focused={caret === 'time'} />
                </div>
              </FormField>
              <FormField label="Do">
                <Pick testId="pick-do" label="Do" value={t >= DO_PICK ? 'Ask Chat' : DO[0]} open={doOpen} items={DO} hover={t >= DO_HOVER ? 'Ask Chat' : null} />
              </FormField>
              <FormField label="Prompt">
                <StaticInput value={typed(PROMPT, t, PROMPT_FROM, 32) || undefined} placeholder="What should it do?" focused={caret === 'prompt'} />
              </FormField>
              <FormField>
                <div className="m-inline">
                  <Button variant="primary">Save</Button>
                </div>
              </FormField>
            </Form>
          </div>
        )}
        <List>
          {saved && <AutomationRow className="m-ob-enter" name={NAME} on trigger={`Weekdays ${TIME}`} action="Ask Chat · summarize inbox and calendar" nextRun={`tomorrow ${TIME}`} />}
          <AutomationRow name="Nightly tests" on={starterOn} trigger="Daily 23:00" action="Start session · run the suite" nextRun={starterOn ? 'today 23:00' : 'off'} />
        </List>
      </div>
      <Cursor to={saved ? null : target} pressed={pressed} visible={t >= WHEN_MOVE - 200 && t < PERIOD - 500} />
    </Frame>
  )
}
