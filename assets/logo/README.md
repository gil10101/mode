# Mode logo candidates

Generated with the [`ip-as-logo`](https://github.com/s1dashu/ip-as-logo-skill) workflow. Odd-numbered candidates emerge from the lower-left, even-numbered from the lower-right. Every result is delivered exactly as returned (no filtering, retries or post-processing); earlier rounds are kept on record.

**Model:** Canva `generate-image` (model not disclosed by the connector; used with the owner's explicit consent, as the skill requires for a non-listed model). **Constraint delivery:** main-prompt `Constraints:` line. **Returned size:** 200 × 200 px JPEG, converted losslessly to PNG. Full-resolution versions are available from the Canva links (open and download); the 32/128/512 exports in this folder are upscales of the 200 px originals and must be regenerated from the full-resolution files before shipping.

## Chosen marks (owner's pick, 2026-10-18)

| Mark | Source | File | Use |
|---|---|---|---|
| Duck, light | round 2 A1 | `final/duck-light.png` | Primary mark on light backgrounds; README header, app icon (light) |
| Duck, dark | round 2/3 A2 | `final/duck-dark.png` | Primary mark on dark backgrounds |
| Bot head, light | round 3 C1 | `final/bot-light.png` | Secondary mark on light backgrounds: session / agent contexts (roster avatars fallback, overlay session pill) |
| Bot head, dark | round 3 C2 | `final/bot-dark.png` | Secondary mark on dark backgrounds |

Each has 32 / 128 / 512 px exports next to it (upscaled from the 200 px originals; regenerate from the Canva full-resolution files before shipping). `mode-logo.png` / `mode-logo-dark.png` at the top level are copies of the duck marks. The split of duck = product, bot = sessions is a proposal; see [12-open-questions](../../docs/12-open-questions.md).

## Round 4 (record, `candidates/round4/`) — session-avatar variants of the bot head

The owner asked for session avatars that tell sessions apart: the approved bot head with one accessory each (headwear, hair, eyewear or a mustache), kept within the design of the marks. Eight variants, hats and facial accessories only (the owner dropped hair after the first draws): `beret`, `specs`, `tophat`, `mustache`, `beanie`, `cap`, `squares`, `monocle`; the core assigns them least-used-first (`docs/05-sessions.md` §11.2, ADR-0050). Every Canva result is kept as returned (200 × 200), in three sub-folders:

- `centred/`: a first draw with a paraphrased prompt and a centered head (rejected: not the marks' framing).
- `corner/`: a second draw with the corner emergence asked for in plain words (rejected by the owner for quality).
- `skeleton/`: draws with the [ip-as-logo](https://github.com/s1dashu/ip-as-logo-skill) prompt skeleton verbatim, the accessory as the one defining feature, the two-color rule (eyes and cut lines in the background color) and the bot exclusions on the `Constraints:` line, reviewed three at a time.
- `template/`: draws that take the approved light bot head (Canva media `MAHYZAbhYEs`) as the image reference and ask for one accessory added with nothing else changed, so the head keeps the mark's size and position. These are the source of the shipped files.

Each variant is drawn once, in the light treatment. `scripts/make-avatar-variants.py` makes the two files the app bundles from an approved draw: it finds the two capsule eyes, scales and moves the draw until they sit exactly on the mark's eyes (so every variant's head has the same size and position), writes `final/bot-<variant>-light.png`, and derives `final/bot-<variant>-dark.png` as the mirror image with the colors swapped to the dark mark's (head from the lower-right, like `bot-dark.png`). The owner asked for this derivation rather than a second draw per variant.

| Label | Variant | Corner | Colors | File | Canva |
|---|---|---|---|---|---|
| T1 | Beret (light, from the template) | lower-left | charcoal head and beret, eyes and brim line in background white | `candidates/round4/template/T1-beret-light.png` | [MAHZWwPmsg4](https://www.canva.com/M/MAHZWwPmsg4) |
| T2 | Round spectacles (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T2-specs-light.png` | [MAHZW66PrjQ](https://www.canva.com/M/MAHZW66PrjQ) |
| T3 | Top hat (light, from the template) | lower-left | charcoal head and hat, eyes, band and brim line in background white | `candidates/round4/template/T3-tophat-light.png` | [MAHZW52EcB8](https://www.canva.com/M/MAHZW52EcB8) |
| T4 | Mustache (light, from the template) | lower-left | charcoal head, eyes and mustache in background white | `candidates/round4/template/T4-mustache-light.png` | [MAHZW5nRbqs](https://www.canva.com/M/MAHZW5nRbqs) |
| T5 | Beanie (light, from the template) | lower-left | charcoal head and beanie, eyes, brim line and pom-pom ring in background white | `candidates/round4/template/T5-beanie-light.png` | [MAHZW51RfUo](https://www.canva.com/M/MAHZW51RfUo) |
| T6 | Sunglasses (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T6-shades-light.png` | [MAHZW9tFCYc](https://www.canva.com/M/MAHZW9tFCYc) |
| T7 | Baseball cap (light, from the template) | lower-left | charcoal head and cap, eyes, brim line and seam in background white | `candidates/round4/template/T7-cap-light.png` | [MAHZWyAxOT0](https://www.canva.com/M/MAHZWyAxOT0) |
| T8 | Monocle (light, from the template) | lower-left | charcoal head, eyes, frame and cord in background white | `candidates/round4/template/T8-monocle-light.png` | [MAHZWxgCDLI](https://www.canva.com/M/MAHZWxgCDLI) |
| T9 | Cowboy hat (light, from the template) | lower-left | charcoal head and hat, eyes, brim line and band in background white | `candidates/round4/template/T9-cowboy-light.png` | [MAHZW5BmnBs](https://www.canva.com/M/MAHZW5BmnBs) |
| T10 | Beanie, redrawn with the gap (light, from the template) | lower-left | charcoal head and beanie, eyes, brim line and pom-pom ring in background white | `candidates/round4/template/T10-beanie-light.png` | [MAHZWzrL3y0](https://www.canva.com/M/MAHZWzrL3y0) |
| T11 | Baseball cap, redrawn with the gap (light, from the template) | lower-left | charcoal head and cap, eyes and brim line in background white | `candidates/round4/template/T11-cap-light.png` | [MAHZW6GagSU](https://www.canva.com/M/MAHZW6GagSU) |
| T12 | Square glasses, thin frames (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T12-squares-light.png` | [MAHZW4XAeuE](https://www.canva.com/M/MAHZW4XAeuE) |
| T13 | Round spectacles, redrawn with the mark's eye spacing (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T13-specs-light.png` | [MAHZW_yxpsA](https://www.canva.com/M/MAHZW_yxpsA) |
| T14 | Baseball cap, redrawn with the mark's eye spacing (light, from the template) | lower-left | charcoal head and cap, eyes and brim line in background white | `candidates/round4/template/T14-cap-light.png` | [MAHZWzjjBCM](https://www.canva.com/M/MAHZWzjjBCM) |
| T15 | Round spectacles, lenses centered on the eyes (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T15-specs-light.png` | [MAHZW_TNTEA](https://www.canva.com/M/MAHZW_TNTEA) |
| T16 | Square glasses, lenses centered on the eyes (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T16-squares-light.png` | [MAHZW-4TvGw](https://www.canva.com/M/MAHZW-4TvGw) |
| T17 | Beanie with a clear folded brim (light, from the template) | lower-left | charcoal head and beanie, eyes, two brim lines and pom-pom ring in background white | `candidates/round4/template/T17-beanie-light.png` | [MAHZW8CaOVo](https://www.canva.com/M/MAHZW8CaOVo) |
| T18 | Baseball cap worn at an angle (light, from the template) | lower-left | charcoal head and cap, eyes and brim line in background white | `candidates/round4/template/T18-cap-light.png` | [MAHZWwxjlOM](https://www.canva.com/M/MAHZWwxjlOM) |
| T19 | Square glasses, each lens centered on its eye (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T19-squares-light.png` | [MAHZW3EF7YI](https://www.canva.com/M/MAHZW3EF7YI) |
| T20 | Round spectacles, each circle centered on its eye (light, from the template) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/template/T20-specs-light.png` | [MAHZW7G3OO8](https://www.canva.com/M/MAHZW7G3OO8) |

The owner's review rules, checked by the script on every pinned variant: the head's right edge on the eye row and both eyes sit where the mark's do (the pin matches the head's width and the eye height, so a draw whose eyes are spaced differently fails on the right eye); only the two tones appear; headwear shows a background-color line between itself and the head. T5 and T7 failed the line and were redrawn as T10 and T11; T6's thick sunglasses became T12; T2 and T11 failed the eye spacing and were redrawn as T13 and T14; T9 (cowboy) failed the eye spacing and was dropped.

The owner's second review (2026-10-29): glasses must sit with the eye in the middle of the lens (the script now measures the charcoal room on each side of the eye inside its frame), the beanie's folded brim must read clearly, and the cap must sit at an angle; T15–T20 answer that.

**Shipped set** (`final/bot-<variant>-{light,dark}.png`, fourteen variants in the core's order): `beret` (T1), `specs` (T20), `tophat` (T3), `mustache` (T4), `beanie` (T17), `cap` (T18), `squares` (T19), `monocle` (T8), then round 5's `crown`, `headphones`, `ears`, `wizard`, `patch`, `chef`.

**Round 5** (2026-11-15, `candidates/round5/`): the owner asked for more costumes. A first set drawn as plain geometry in code was rejected ("very badly drawn") and is not kept. The shipped six are template draws like round 4's: the approved light bot head (Canva media `MAHYZAbhYEs`) as the image reference, one accessory added, nothing else changed.

| Label | Variant | File | Canva |
|---|---|---|---|
| U1 | Crown | `candidates/round5/U1-crown-light.png` | [MAHbAjdiQIs](https://www.canva.com/M/MAHbAjdiQIs) |
| U2 | Headphones | `candidates/round5/U2-headphones-light.png` | [MAHbAgg8zIU](https://www.canva.com/M/MAHbAgg8zIU) |
| U3 | Cat ears | `candidates/round5/U3-ears-light.png` | [MAHbAh5Y9To](https://www.canva.com/M/MAHbAh5Y9To) |
| U4 | Wizard hat | `candidates/round5/U4-wizard-light.png` | [MAHbAiQbe5Y](https://www.canva.com/M/MAHbAiQbe5Y) |
| U5 | Eye patch | `candidates/round5/U5-patch-light.png` | [MAHbAvatGvs](https://www.canva.com/M/MAHbAvatGvs) |
| U6 | Chef hat, first draw (rejected: the head was redrawn narrower and the hat ran off the frame) | `candidates/round5/U6-chef-light.png` | [MAHbAoYXFRU](https://www.canva.com/M/MAHbAoYXFRU) |
| U7 | Chef hat, smaller, the head kept in place | `candidates/round5/U7-chef-light.png` | [MAHbAsz6puM](https://www.canva.com/M/MAHbAsz6puM) |

The pin step learned three things for them: the headphones are scaled by the eye spacing (the cup sits where the head's right edge is read), the wizard hat's eyes get the eyewear's tolerance, and where a moved draw stops short of the left or bottom edge its last column or row carries on to it.

| Label | Variant | Corner | Colors | File | Canva |
|---|---|---|---|---|---|
| B1 | Beret (light) | lower-left | charcoal head and beret, eyes and brim line in background white | `candidates/round4/skeleton/B1-beret-light.png` | [MAHZW_skvUE](https://www.canva.com/M/MAHZW_skvUE) |
| B2 | Round spectacles (light) | lower-left | charcoal head, eyes and frames in background white | `candidates/round4/skeleton/B2-specs-light.png` | [MAHZW5f1LaY](https://www.canva.com/M/MAHZW5f1LaY) |
| B3 | Top hat (light) | lower-left | charcoal head and hat, eyes, band and brim line in background white | `candidates/round4/skeleton/B3-tophat-light.png` | [MAHZW9CQqDc](https://www.canva.com/M/MAHZW9CQqDc) |

The corner draw's Canva links, in variant order (light, dark): beret [MAHZW5ZyVKU](https://www.canva.com/M/MAHZW5ZyVKU) / [MAHZW3Li-BU](https://www.canva.com/M/MAHZW3Li-BU); specs [MAHZW6C-eYI](https://www.canva.com/M/MAHZW6C-eYI) / [MAHZW_Qx1SY](https://www.canva.com/M/MAHZW_Qx1SY); top hat [MAHZW9tsycM](https://www.canva.com/M/MAHZW9tsycM) / [MAHZW7BQ04s](https://www.canva.com/M/MAHZW7BQ04s); mustache [MAHZW6_n_EM](https://www.canva.com/M/MAHZW6_n_EM) / [MAHZW54xsuA](https://www.canva.com/M/MAHZW54xsuA); beanie [MAHZWxcA3z0](https://www.canva.com/M/MAHZWxcA3z0) / [MAHZW9vAk2Y](https://www.canva.com/M/MAHZW9vAk2Y); curls [MAHZW_C4XI4](https://www.canva.com/M/MAHZW_C4XI4) / [MAHZW2s-uwE](https://www.canva.com/M/MAHZW2s-uwE); shades [MAHZW-JWDQc](https://www.canva.com/M/MAHZW-JWDQc) / [MAHZW43VbPk](https://www.canva.com/M/MAHZW43VbPk); bob [MAHZW5Vyres](https://www.canva.com/M/MAHZW5Vyres) / [MAHZW6R020A](https://www.canva.com/M/MAHZW6R020A). The centered draw: beret [MAHZW-_WtxU](https://www.canva.com/M/MAHZW-_WtxU) / [MAHZW62OiQs](https://www.canva.com/M/MAHZW62OiQs), specs [MAHZW03XJtI](https://www.canva.com/M/MAHZW03XJtI) / [MAHZW3VasIw](https://www.canva.com/M/MAHZW3VasIw), top hat [MAHZW1Q8tv4](https://www.canva.com/M/MAHZW1Q8tv4).

**Ana's picture** (`assets/sample/ana.png`, the sample user's profile picture in the wireframes and the browser mock) is not generated: it is `final/duck-light.png` recoloured in Psyduck tones (body `#F4CE52`, bill `#F1E3C2`) by a three-level palette map of the mark's gray levels, with the ground made transparent (`python3 scripts/make-ana.py`), so the picture sits on the `.m-mascot.is-picture` disc in the theme's `--m-sunken`: the cream ground it had before stood out too hard on the dark pack.

## Round 3 (record, `candidates/round3/`) — monochrome, theme-adaptive, head-first

Owner feedback after round 2: chameleons dropped; A2 kept; A1 and C1 redrawn; the bot direction is head only. Light variants are black-on-white, dark variants white-on-black, so the app swaps them with the system appearance. All are original designs: the duck follows the owner's reference mark (rounded head, two capsule eyes, flat wide bill); the bot follows the Grok Bot mascot style (round geometric head, two vertical capsule eyes, no mouth) without copying it.

| Label | Direction | Corner | Colors | File | Canva |
|---|---|---|---|---|---|
| A1 | Duck — large head, body just visible, flat bill, capsule eyes (light) | lower-left | charcoal body, cool gray bill, eyes in background white | `candidates/round3/A1.png` | [MAHYZBM4lTI](https://www.canva.com/M/MAHYZBM4lTI) |
| A1b | Duck — head only, cropped at the bottom (light) | lower-left | charcoal head, cool gray bill, eyes in background white | `candidates/round3/A1b.png` | [MAHYZNcrdYI](https://www.canva.com/M/MAHYZNcrdYI) |
| A2 | Duck (dark), kept from round 2 | lower-right | white body, light gray bill, eyes in background black | `candidates/round3/A2.png` | [MAHYZJwqCjg](https://www.canva.com/M/MAHYZJwqCjg) |
| C1 | Bot — squashed squircle head only, two capsule eyes, no mouth (light) | lower-left | charcoal head, eyes in background white | `candidates/round3/C1.png` | [MAHYZAbhYEs](https://www.canva.com/M/MAHYZAbhYEs) |
| C1b | Bot — rounder head only (light) | lower-left | charcoal head, eyes in background white | `candidates/round3/C1b.png` | [MAHYZNtPovo](https://www.canva.com/M/MAHYZNtPovo) |
| C2 | Bot — squashed squircle head only (dark) | lower-right | white head, eyes in background black | `candidates/round3/C2.png` | [MAHYZPugKbw](https://www.canva.com/M/MAHYZPugKbw) |

The bot candidates use two colors in total (head plus background, which also fills the eyes) at the owner's request for a monochrome mark; the skill's three-color default was set aside deliberately.

## Round 2 (record, `candidates/round2/`)

| Label | Direction | Corner | Colors | Canva |
|---|---|---|---|---|
| A1 | Duck with body (light) | lower-left | charcoal, warm gray bill | [MAHYZKEjP7Y](https://www.canva.com/M/MAHYZKEjP7Y) |
| B1 | Chameleon, curled blunt tail | lower-left | leaf green, cream on muted sand | [MAHYZBf1Kxg](https://www.canva.com/M/MAHYZBf1Kxg) |
| B2 | Chameleon | lower-right | mint, pale lilac on muted plum | [MAHYZPDGy54](https://www.canva.com/M/MAHYZPDGy54) |
| C1 | Bot head + body (light); came back with a floor reflection | lower-left | charcoal, warm gray | [MAHYZH3k-_0](https://www.canva.com/M/MAHYZH3k-_0) |
| C2 | Bot head + body (dark); came back with a floor reflection | lower-right | white, light gray | [MAHYZBipjj8](https://www.canva.com/M/MAHYZBipjj8) |

## Round 1 (record, `candidates/round1/`)

| Label | Direction | Corner | Colors | Canva |
|---|---|---|---|---|
| A1 | Duck — yellow body, beige bill | lower-left | warm yellow, beige on muted navy | [MAHYZO2copg](https://www.canva.com/M/MAHYZO2copg) |
| A2 | Duck | lower-right | warm yellow, beige on muted sage | [MAHYZJK5BwA](https://www.canva.com/M/MAHYZJK5BwA) |
| C1 | Generic round-headed character, bump on the head | lower-left | cream, coral on muted slate | [MAHYZJKxIfM](https://www.canva.com/M/MAHYZJKxIfM) |
| C2 | Generic round-headed character | lower-right | charcoal, off-white on muted mustard | [MAHYZEBwzPo](https://www.canva.com/M/MAHYZEBwzPo) |

## Shared prompt skeleton

Every candidate used the skill's prompt skeleton verbatim, with these variables filled per row: background color, subject description and defining feature, the character colors (and the rule that eyes use the background color for the monochrome marks), the emergence corner, and direction-specific exclusions appended to the `Constraints:` line (no hair tufts, feathers or cheeks for the duck; no seams, screws, panels or antennae for the bot; no body, floor, reflection or mouth for the head-only marks). The prompt never names the image as a logo or icon.
