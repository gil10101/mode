# The Marketplace's kind marks

Three marks for the kinds a pack carries ([App window](../../docs/03-app-window-spec.md) §7C.6;
[UI/UX](../../docs/10-ui-ux.md) §7.4.18). Agents wear the bot head
(`assets/logo/final/`); these three are the rest. Each is a flat silhouette like
the mascot art, but colored for its pack rather than drawn in the text ink
(a solid near-black silhouette read too bold on a light surface) and never
inverted for the other pack: `scripts/make-kind-marks.py` fills the silhouette
in a mid tone of each pack, leaves the cut-outs and the ground transparent so the
surface shows through, and paints the palette's blobs in the pack's own colors.

| Mark | For | Fill (light / dark) | Files | Source |
|---|---|---|---|---|
| Parcel box, flaps open | the Marketplace itself: the Integrations card's tile and the page head | cardboard `#B08D5E` / `#C4A071` | `marketplace-light.png` / `marketplace-dark.png` | `source/marketplace.png`, Canva [MAHZdaCDJmA](https://www.canva.com/M/MAHZdaCDJmA) |
| Script: a scroll with a `>_` prompt | an automation (the kind mark on catalog cards and in the pack dialogs) | `#7D7870` / `#A09B92` | `automation-light.png` / `automation-dark.png` | `source/automation.png`, Canva [MAHZf9FHE8o](https://www.canva.com/M/MAHZf9FHE8o) |
| Painter's palette | a theme pack (its own colors are the six dots beside it) | `#7D7870` / `#A09B92`; blobs accent, ok, info and a yellow of the pack | `theme-light.png` / `theme-dark.png` | `source/theme.png`, Canva [MAHZdZQHvg0](https://www.canva.com/M/MAHZdZQHvg0) |

The sources are what Canva drew on 2026-10-30 from text prompts of the form "a
simple flat minimal icon of <the object>, solid dark charcoal silhouette with
<its lines> as thin white cut-outs, in the style of a flat sticker or app mascot
mark: thick rounded shapes, no outline strokes, no shadows, no gradients, no
texture, no text, centered with generous margin on a plain white background".
Two other automation candidates were drawn and set aside: a terminal window
with a clock badge ([MAHZf-BYixQ](https://www.canva.com/M/MAHZf-BYixQ)) and a
code file with a bolt ([MAHZfygbVIE](https://www.canva.com/M/MAHZfygbVIE)); the
scroll says "script" plainest at 24 px. An alarm clock
([MAHZdZEp0Tg](https://www.canva.com/M/MAHZdZEp0Tg)) stood in for a day. The
script crops each source to its ink, pads it to a square with a 10 % margin and
resizes it to 200 × 200; the alpha comes from the gray ramp, so the edges stay
soft. They are Mode's own marks, not any company's (ADR-0021), and the only
places they appear are the ones the table names.
