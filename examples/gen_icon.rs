// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Generates the application icon from the master SVG.
//!
//! Reads `assets/icon.svg` and writes:
//! * `assets/icon.ico`     — multi-size Windows icon embedded into the `.exe`
//!                           by `build.rs` (via `winresource`);
//! * `assets/icon_256.png` — a single 256×256 PNG used as the runtime window
//!                           icon (`egui::ViewportBuilder::with_icon`).
//!
//! Run with:
//! ```text
//! cargo run --release --example gen_icon
//! ```
//!
//! The generated files are committed, so a normal `cargo build` does not need
//! `resvg` (it is only a dev-dependency, used by this tool).

use std::fs;

/// Sizes packed into the `.ico`. Windows uses 16/32/48/256 for the shell,
/// taskbar and Alt-Tab; the rest improve scaling in Explorer.
const SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];

fn main() {
    let svg = fs::read_to_string("assets/icon.svg").expect("read assets/icon.svg");

    // Runtime window icon: a plain 256×256 PNG.
    let pm256 = render(&svg, 256);
    // Sanity-check the rasterizer: a transparent rounded corner, the filled
    // tile, and the accent glyph.
    assert_eq!(pm256.pixel(2, 2).unwrap().alpha(), 0, "corner must be transparent");
    let tile = pm256.pixel(128, 128).unwrap().demultiply();
    assert_eq!(
        (tile.red(), tile.green(), tile.blue()),
        (44, 44, 46),
        "tile fill must be #2C2C2E"
    );
    let accent = pm256
        .pixels()
        .iter()
        .filter(|p| {
            let c = p.demultiply();
            (c.red(), c.green(), c.blue()) == (10, 132, 255)
        })
        .count();
    assert!(
        accent > 1500,
        "accent glyph looks missing or too small: {accent} px"
    );
    let png_256 = pm256.encode_png().expect("encode icon_256.png");
    fs::write("assets/icon_256.png", &png_256).expect("write assets/icon_256.png");

    // Windows icon: PNG-compressed frames in an ICO container (Vista+).
    let frames: Vec<(u32, Vec<u8>)> = SIZES
        .iter()
        .map(|&size| {
            let png = render(&svg, size)
                .encode_png()
                .unwrap_or_else(|e| panic!("encode {size}px frame: {e}"));
            (size, png)
        })
        .collect();

    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    ico.extend_from_slice(&(frames.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * frames.len() as u32;
    for (size, png) in &frames {
        // 0 means 256 in the ICO directory.
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        ico.push(dim); // width
        ico.push(dim); // height
        ico.push(0); // palette size
        ico.push(0); // reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &frames {
        ico.extend_from_slice(png);
    }
    fs::write("assets/icon.ico", &ico).expect("write assets/icon.ico");

    println!(
        "wrote assets/icon.ico ({} frames: {:?}) and assets/icon_256.png ({accent} accent px)",
        frames.len(),
        SIZES
    );
}

/// Rasterizes the SVG into a square pixmap of the given size.
fn render(svg: &str, size: u32) -> resvg::tiny_skia::Pixmap {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())
        .expect("parse assets/icon.svg");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)
        .unwrap_or_else(|| panic!("allocate {size}px pixmap"));
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
}
