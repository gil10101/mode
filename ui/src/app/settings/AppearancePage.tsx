// Settings → Appearance (03 §8.5): System / Light / Dark ("this device only")
// and the themes gallery: the theme packs section (`ThemeSection`, package
// 11: one card per pack, import / export / remove) with its ⋮ entries.
import { Seg, SegItem } from '../../kit/controls'
import { MenuButton } from '../../kit/button'
import { useSetting } from '../../state/settings-store'
import { openMenu } from '../ui'
import { Field, Group } from './controls'
// package 11: the theme packs gallery
import { ThemeSection, themeMenuEntries } from './ThemeSection'

const APPEARANCE = [
  ['auto', 'System'],
  ['light', 'Light'],
  ['dark', 'Dark'],
] as const

export function AppearancePage() {
  const [appearance, setAppearance] = useSetting<string>('theme.appearance')
  return (
    <>
      <Group title="Light and dark" id="appearance">
        <Field title="Appearance" desc="System follows the OS between light and dark." k="theme.appearance" testId="appearance-row">
          <Seg label="Appearance">
            {APPEARANCE.map(([v, label]) => (
              <SegItem key={v} active={appearance === v} onClick={() => void setAppearance(v)}>
                {label}
              </SegItem>
            ))}
          </Seg>
          <MenuButton label="Theme options" onClick={(e) => openMenu(e.currentTarget, themeMenuEntries(), 'Theme options')} />
        </Field>
      </Group>
      <section className="m-set-group" aria-label="Themes">
        <div className="m-set-head">Themes</div>
        <ThemeSection />
      </section>
    </>
  )
}
