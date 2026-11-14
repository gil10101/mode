# Brand marks

The marks Mode draws for the external apps and services it connects to or monitors ([UI/UX](../../docs/10-ui-ux.md) §7.4.16, [ADR-0053](../../docs/13-decisions.md#adr-0053-every-external-app-carries-its-mark)). One SVG per mark, kept as its owner published it (its own viewBox, its own fills and gradients); `manifest.json` lists each mark's id, label, colour mode, source, the URL it was fetched from and, where the source states one per file, its `licence`. A mark is **fetched, never drawn**: `node ui/scripts/fetch-brand.mjs <id> <url> --label … --source …` downloads one from a public set, normalises the wrapper and writes its manifest line.

| Source | Meaning |
|---|---|
| `logos` | The owner's full-colour mark from [gilbarbara/logos](https://github.com/gilbarbara/logos) (MIT): Gmail, Slack, Zoom, Microsoft Teams, Google Calendar, Google Meet, Tux, Azure (Microsoft Foundry). |
| `vscode-icons` | The owner's full-colour tile from [vscode-icons](https://github.com/vscode-icons/vscode-icons) (MIT), where the sets above have none: PowerShell. |
| `fluent` | Microsoft's multicolour product icon from the Fluent UI brand-icons CDN (`res-1.cdn.office.net/files/fabric-cdn-prod_*/assets/brand-icons/product/svg/`), under the [Fluent UI assets licence](https://aka.ms/fluentui-assets-license): Outlook. |
| `commons` | Apple's app icon as filed on [Wikimedia Commons](https://commons.wikimedia.org), with the licence its file page states in the mark's `licence` (nominative use; the owner accepted the trademark risk on 2026-11-08): Apple Mail, Apple Calendar. Editor exports are cleaned by `fetch-brand.mjs` (DOCTYPE, metadata, Inkscape nodes, class sheets written onto the elements). |
| `simple-icons` | A monochrome path from [Simple Icons](https://simpleicons.org) (CC0 1.0), for a brand whose mark is one colour (GitHub, Notion, Linear, the terminals, the AI providers it carries). |
| `lobe-icons` | A monochrome AI mark from [lobe-icons](https://github.com/lobehub/lobe-icons) (MIT) where Simple Icons has none: Groq, OpenAI, Amazon Bedrock, Grok. |
| `drawn` | Mode's own glyph, kept only where no set carries a mark and the thing is not a brand's logo: the plain terminal window, the Kitty cat. Never a company's logo redrawn. |

Every mark keeps its owner's trademark; the About page carries the note, and the release check (XC-36) is where the marks are verified against each owner's guidelines before a public build.

## Colour, and why a mark went wrong on dark

`color` in the manifest is one of three things. `own`: the file carries every colour, the whites included, and the stylesheet sets none; this is the right mode for any brand whose logo has more than one colour or a white interior (Gmail's envelope, Teams' T, Calendar's 31, Zoom's camera, Outlook's O, Tux's belly). A hex: a one-colour mark in the brand's colour. `fg`: a near-black one-colour mark that follows the text colour.

The first versions of the Gmail, Slack, Zoom, Teams, Outlook, Google Calendar, Google Meet and Tux marks were one-colour knockouts (Simple Icons silhouettes or drawings), where the logo's white parts were holes. On a light ground a hole reads as white and the mark looks right; on a dark ground the same hole shows the dark surface, so the T, the 31, the camera, the O and the belly went dark and the marks read inverted. The legibility step then moved some brand colours in lightness to pass 3:1, which changed the brand's identity on top. Full-colour fetched marks have no holes to fill and their colours are not touched, so they read the same on both grounds. Rule for every future mark: fetch the owner's colour version from a public set with `fetch-brand.mjs`; a monochrome path is right only for a brand whose logo is itself one colour.

A hex colour must still read at 3:1 on both grounds (WCAG 1.4.11): the generator keeps it where it does and, where it would sink into a theme pack's surface, moves its lightness in OKLab (hue and chroma held) until it reads against `grounds.light` (the darkest fill of a built-in light pack) and `grounds.dark` (the lightest surface of a built-in dark pack), emitting the dark value under `[data-theme="dark"]`. A new built-in pack that goes past those extremes updates `grounds`.

## Adding a mark

1. `node ui/scripts/fetch-brand.mjs <id> <url> --label "Name" --source logos` (add `--color fg` or a hex for a one-colour mark); it writes `<id>.svg` and the manifest line.
2. Run `node ui/scripts/brands.mjs`: it rewrites `ui/src/kit/brands.gen.ts`, the colour block of `docs/wireframes/mode.css` and `ui/src/kit/mode.css`, and `docs/wireframes/tools/brands.svg.html`; then `python3 docs/wireframes/tools/embed.py` splices the symbols into the wireframes.
3. Draw it with `<BrandMark id="<id>" />` (18 px), `size={16}` in rows and menus or `size={12}` before a name; map Mode's own names to it in `ui/src/kit/brands.tsx` when a helper applies.
4. CI runs `node ui/scripts/brands.mjs --check`; stale generated files fail the build.

