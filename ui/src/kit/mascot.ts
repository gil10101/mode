// The duck mark, the bot head (ADR-0031) and the bot head's accessory variants
// (05 §11.2, ADR-0050), imported through Vite from the repo's assets so the UI
// never duplicates the artwork. The dark variants are for the dark pack and for
// the notch bezel, which is always dark. Ana's picture is the sample user's
// profile picture (the browser mock and the wireframes); the kind marks are
// the Marketplace's (assets/kinds); the sample track's artwork is Now playing's.
import duckLight from '../../../assets/logo/final/duck-light.png'
import duckDark from '../../../assets/logo/final/duck-dark.png'
import botLight from '../../../assets/logo/final/bot-light.png'
import botDark from '../../../assets/logo/final/bot-dark.png'
import beretLight from '../../../assets/logo/final/bot-beret-light.png'
import beretDark from '../../../assets/logo/final/bot-beret-dark.png'
import specsLight from '../../../assets/logo/final/bot-specs-light.png'
import specsDark from '../../../assets/logo/final/bot-specs-dark.png'
import tophatLight from '../../../assets/logo/final/bot-tophat-light.png'
import tophatDark from '../../../assets/logo/final/bot-tophat-dark.png'
import mustacheLight from '../../../assets/logo/final/bot-mustache-light.png'
import mustacheDark from '../../../assets/logo/final/bot-mustache-dark.png'
import beanieLight from '../../../assets/logo/final/bot-beanie-light.png'
import beanieDark from '../../../assets/logo/final/bot-beanie-dark.png'
import capLight from '../../../assets/logo/final/bot-cap-light.png'
import capDark from '../../../assets/logo/final/bot-cap-dark.png'
import squaresLight from '../../../assets/logo/final/bot-squares-light.png'
import squaresDark from '../../../assets/logo/final/bot-squares-dark.png'
import monocleLight from '../../../assets/logo/final/bot-monocle-light.png'
import monocleDark from '../../../assets/logo/final/bot-monocle-dark.png'
import anaPicture from '../../../assets/sample/ana.png'
// The sample track's artwork (Now playing, 02 §7.1): a crop of Winslow Homer's Northeaster (public domain; scripts/make-sample-artwork.py).
import nowPlayingArt from '../../../assets/sample/nowplaying.png'
import marketplaceLight from '../../../assets/kinds/marketplace-light.png'
import marketplaceDark from '../../../assets/kinds/marketplace-dark.png'
import automationLight from '../../../assets/kinds/automation-light.png'
import automationDark from '../../../assets/kinds/automation-dark.png'
import themeLight from '../../../assets/kinds/theme-light.png'
import themeDark from '../../../assets/kinds/theme-dark.png'

export { duckLight, duckDark, botLight, botDark, anaPicture, nowPlayingArt }

/** The accessory variants of the bot head, in the core's assignment order (`naming::AVATAR_VARIANTS`). */
export const AVATAR_VARIANTS = ['beret', 'specs', 'tophat', 'mustache', 'beanie', 'cap', 'squares', 'monocle'] as const
export type AvatarVariant = (typeof AVATAR_VARIANTS)[number]

/** The Marketplace's kinds (03 §7C.6): the parcel for the Marketplace itself, the alarm clock for automations, the palette for themes; agents wear the bot. */
export type KindArt = 'marketplace' | 'automation' | 'theme'

export interface Artwork {
  duck: string
  bot: string
  /** The bot head wearing each accessory. */
  bots: Readonly<Record<AvatarVariant, string>>
  /** The kind marks (assets/kinds), drawn like the bot: a silhouette on the pack's ground. */
  kinds: Readonly<Record<KindArt, string>>
}

export const artwork: Readonly<Record<'light' | 'dark', Artwork>> = {
  light: {
    duck: duckLight,
    bot: botLight,
    bots: { beret: beretLight, specs: specsLight, tophat: tophatLight, mustache: mustacheLight, beanie: beanieLight, cap: capLight, squares: squaresLight, monocle: monocleLight },
    kinds: { marketplace: marketplaceLight, automation: automationLight, theme: themeLight },
  },
  dark: {
    duck: duckDark,
    bot: botDark,
    bots: { beret: beretDark, specs: specsDark, tophat: tophatDark, mustache: mustacheDark, beanie: beanieDark, cap: capDark, squares: squaresDark, monocle: monocleDark },
    kinds: { marketplace: marketplaceDark, automation: automationDark, theme: themeDark },
  },
}

/** The bot head for a session: its variant's file, or the plain mark for `''`, `null` or an id this build does not know. */
export function botArt(art: Artwork, variant?: string | null): string {
  return (variant && (art.bots as Readonly<Record<string, string | undefined>>)[variant]) || art.bot
}
