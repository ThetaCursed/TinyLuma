# Application icon

`icon.svg` is the **master source**. `icon.ico` and `icon_256.png` are generated
from it and committed, so a normal `cargo build`/`cargo test` does **not** need
the generator's dependencies.

| File | Purpose |
|---|---|
| `icon.svg` | Design source. Mirrors the hero-screen tile in `src/ui/center_panel.rs` (colors from `src/theme.rs`). |
| `icon.ico` | Multi-size Windows icon (16, 24, 32, 48, 64, 128, 256). Embedded into the `.exe` by `build.rs` (`winresource`) — Explorer, taskbar, Alt-Tab. |
| `icon_256.png` | 256×256 PNG. Embedded into the binary (`include_bytes!`) and set as the runtime window icon in `src/main.rs` via `egui::ViewportBuilder::with_icon`. |

## Regenerate

After editing `icon.svg`:

```sh
cargo run --release --example gen_icon
```

This rasterizes the SVG with `resvg` (a dev-dependency) and rewrites
`icon.ico` + `icon_256.png`. The generator also sanity-checks the result
(transparent rounded corner, `#2C2C2E` tile fill, `#0A84FF` glyph).

## Design

The glyph is Phosphor's **aperture** (Regular), the same icon the app uses at
runtime (`egui_phosphor::regular::APERTURE`). Scale math to match the 54 px
hero tile:

- tile / corner radius / stroke / glyph size: `54 / 12 / 1 / 48` px
- aperture ink is `0.8125 em` (Phosphor `units_per_em = 1024`), so at 48 px the
  glyph covers ≈ 72 % of the tile — reproduced by drawing the 256-unit
  aperture at 8/9 scale.
