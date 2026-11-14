// Gallery sections: round 3 (components.html §agent-page).
import { EditableAvatar } from '../kit/avatars'
import { Button } from '../kit/button'
import { Icon } from '../kit/icons'
import { SwatchDots, ThemeSwatch } from '../kit/swatch'
import { Section as KitSection } from '../kit/views'
import { ChipsField, Tab, Tabs } from '../kit/controls'
import { Ex, Flow, type SectionSpec } from './wf'

const PAPER = { bg: '#F3F1EC', surface: '#FAF9F6', fg: '#1E1D1A', accent: '#D9480F', ok: '#2F7D4A', info: '#35669A' }
const NORD = { bg: '#2E3440', surface: '#3B4252', fg: '#ECEFF4', accent: '#D08770', ok: '#A3BE8C', info: '#81A1C1' }

export const SECTIONS_4: SectionSpec[] = [
  {
    id: 'agent-page',
    title: 'Agent page (round 3)',
    note: 'The edit page of an agent (03 §7E.2) follows Settings: a page head, tabs, then forms. The mark carries a pencil badge at its bottom-right that opens the accessory menu; short lists (backup models, tool rules, tables, extra arguments) are chips fields: removable chips and a bare input in one sunken frame, Enter or a comma adds.',
    render: () => (
      <>
        <Ex caption=".m-avatar-editable · .m-avatar-agent.is-40 · .m-avatar-edit (+ .m-icon)">
          <Flow>
            <EditableAvatar size={40} hue={108} variant="specs" label="Change the mark" />
            <EditableAvatar size={40} hue={239} label="Change the mark" />
          </Flow>
        </Ex>
        <Ex caption=".m-tabs · .m-tab (+ .is-active) · the agent pages">
          <Tabs label="Agent pages">
            <Tab>Identity</Tab>
            <Tab active>Brain</Tab>
            <Tab>Knows</Tab>
            <Tab>Does</Tab>
            <Tab>Rules</Tab>
            <Tab>Advanced</Tab>
          </Tabs>
        </Ex>
        <Ex caption=".m-chips · .m-chip (+ .m-x) · input">
          <div style={{ width: 360 }}>
            <ChipsField label="Backup models" values={['sonnet']} placeholder="sonnet, haiku · up to three, in order" max={3} onChange={() => {}} />
          </div>
        </Ex>
        <Ex caption=".m-chips (empty) · input placeholder">
          <div style={{ width: 360 }}>
            <ChipsField label="Allowed tools" values={[]} placeholder="Read, Bash(git diff *) · as --allowedTools takes them" onChange={() => {}} />
          </div>
        </Ex>
      </>
    ),
  },
{
    id: 'swatch',
    title: 'Theme swatch (round 3)',
    note: 'A theme pack\u2019s colors as card art (10 §4.5; 03 §7C.6): a window in the pack\u2019s bg and surface with its accent dot and fg bars, then one dot per role; the colors travel as custom properties on the element. The Appearance gallery card draws both; the Marketplace\u2019s theme pack card the dots. A section head may carry an icon button after its meta: the catalog\u2019s refresh sits beside its \u201cfetched\u201d stamp.',
    render: () => (
      <>
        <Ex caption=".m-swatch · .m-swatch-dots (+ .is-bg … .is-info)">
          <Flow style={{ alignItems: 'flex-start' }}>
            <div style={{ width: 200 }}>
              <ThemeSwatch swatch={PAPER} />
              <div style={{ marginTop: 8 }}>
                <SwatchDots swatch={PAPER} />
              </div>
            </div>
            <div style={{ width: 200 }}>
              <ThemeSwatch swatch={NORD} />
              <div style={{ marginTop: 8 }}>
                <SwatchDots swatch={NORD} />
              </div>
            </div>
          </Flow>
        </Ex>
        <Ex caption=".m-section-head · .m-t-meta · .m-btn.m-btn-icon.is-quiet (+ #i-refresh)">
          <div style={{ width: 360 }}>
            <KitSection
              title="Catalog"
              meta="3 packs · fetched 2 m ago"
              actions={
                <Button variant="icon" quiet aria-label="Refresh catalog">
                  <Icon id="refresh" />
                </Button>
              }
            >
              <div className="m-t-meta">the pack cards</div>
            </KitSection>
          </div>
        </Ex>
      </>
    ),
  },
]
