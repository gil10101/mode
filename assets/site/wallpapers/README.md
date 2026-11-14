# Landing wallpapers

The six `.webp` files are crops of public-domain paintings, cut by `scripts/make-site-wallpapers.py` from the scans in `sources/` (moved here from the repo root on 2026-11-08). Light: Eragny (sessions), Autumn Poplars (notes), Thunderstorm in the Alps (inbox), At the Waterfall (agents), Rocky Landscape with Sheep (marketplace). Dark: Northeaster (sessions, inbox, marketplace), Thunderstorm (notes, agents).

| Source scan | Work | Where it comes from |
|---|---|---|
| `Autumn_Poplars_by_Camille_Pissarro_1600px.jpg` | Camille Pissarro, Autumn, Poplars, Éragny (1894) | Wikimedia Commons, public domain |
| `winslow.jpg` | Winslow Homer, Northeaster (1895) | The Met, public domain |
| `atthewaterfall.jpg` | David Claypoole Johnston, At the Waterfall | The Met, accession 1978.512, CC0 |
| `rockysheep.jpg` | Rocky Landscape with Sheep | The Met, accession 2007.121, CC0 |
| `thunderstorm.jpg` | Thunderstorm in the Alps | The Met, accession 2006.331, CC0 |

The scans' XMP carries the Met object URLs and the CC0 rights; the landing page footer and the README credit the paintings. Regenerate the crops with `python3 scripts/make-site-wallpapers.py` (optionally pass another source folder).
