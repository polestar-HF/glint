# qingicon

UI icons in Glint come from **qingicon**, the icon set published by QingCloud.

- Site: https://qingicon.com
- Package: `@pitrix/qingicon-react` 2.1.10 (npm)
- License: Apache License 2.0 (`LICENSE` in this folder, copied verbatim from the package)

## What Glint uses and what changed

`scripts/build-ui-icons.cjs` extracts the glyphs Glint needs from the package's SVG sprite
and writes their path data to `ui/src/lib/icons.ts`. The `MAP` table in that script lists
each glyph.

Changes from the original:

- Only a subset of the glyphs is included.
- The duotone secondary layer is drawn in `currentColor` at reduced opacity. The original
  uses a fixed gray (`#b6c2cd`).
- `chevron-small` is `chevron-down` drawn smaller in the same box.

The installer bundles this license as `licenses/qingicon-LICENSE.txt` in the install folder.
