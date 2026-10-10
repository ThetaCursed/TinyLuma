// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Unified application theme.
//!
//! All interface colors are collected here as named tokens, and the
//! [`apply`] function configures `egui::Visuals`. The reference is Apple's dark design:
//! semantic system colors (systemBlue, systemGreen, systemRed,
//! systemOrange, systemYellow), `label` / `secondaryLabel` and `separator`.
//!
//! Rule: UI code must not contain raw `Color32::from_*` — only `theme::*`.

use eframe::egui;
use egui::{Color32, Rounding, Stroke};

// ─────────────────────────────────────────────────────────────────────────────
// Backgrounds
// ─────────────────────────────────────────────────────────────────────────────

/// Main background of the panels and window. #1C1C1E (Apple secondarySystemBackground).
pub const BG_WINDOW: Color32 = Color32::from_rgb(28, 28, 30);
/// The darkest background: the letterbox under the image, thumbnail placeholders. #121214.
pub const BG_DEEP: Color32 = Color32::from_rgb(18, 18, 20);
/// Canvas grid dots in the letterbox. A barely visible translucent white: it gives
/// the feel of a working area, but does not fight with the photo or affect color
/// perception. Used only under/around the frame, not over the image.
pub const CANVAS_DOT: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);
/// Filmstrip "tray" background: slightly lighter than the canvas so the strip reads as a separate
/// surface without outline lines, but without sharp contrast with the window.
pub const FILMSTRIP_BG: Color32 = Color32::from_rgb(26, 26, 28);
/// Surface of cards and grouped blocks. #2C2C2E (tertiarySystemBackground).
pub const BG_SURFACE: Color32 = Color32::from_rgb(44, 44, 46);
/// Elevated surfaces: popovers, windows, toasts. #3A3A3C.
pub const BG_ELEVATED: Color32 = Color32::from_rgb(58, 58, 60);
/// Input field background. #1C1C1E.
pub const BG_INPUT: Color32 = Color32::from_rgb(28, 28, 30);
/// Dark translucent backing of the floating plaques (toolbar, frame index).
/// Shared by both pills so the background/density match. Dense enough
/// that the canvas content underneath barely shows through and does not interfere with reading the icons.
pub const NAV_BG: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 242);
/// Rim of the floating pills (toolbar, frame index). Light and translucent,
/// noticeably lighter than the usual [BORDER]: the pill reads clearly on any
/// image underneath — dark or light. Shared by both pills.
pub const NAV_BORDER: Color32 = Color32::from_rgba_premultiplied(145, 145, 145, 145);

// ─────────────────────────────────────────────────────────────────────────────
// Borders and dividers
// ─────────────────────────────────────────────────────────────────────────────

/// Default separators and strokes. #38383A (separatorOpaque).
pub const SEPARATOR: Color32 = Color32::from_rgb(56, 56, 58);
/// A more noticeable border (hover, selection frame). #48484A.
pub const BORDER: Color32 = Color32::from_rgb(72, 72, 74);
/// The line under the window/dialog title and other structural dividers
/// (under the toolbar, the filmstrip edges, the verticals in the toolbar).
/// Translucent white, not grey: on any dark background it gives a neat
/// light edge and adapts to the backing, unlike a fixed
/// grey which looked a bit dirty.
pub const DIVIDER: Color32 = Color32::from_rgba_premultiplied(42, 42, 42, 42);

// ─────────────────────────────────────────────────────────────────────────────
// Text
// ─────────────────────────────────────────────────────────────────────────────

/// Primary text (Apple `label`). #F5F5F7.
pub const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
/// Secondary text and captions (Apple `secondaryLabel`). #98989F.
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(152, 152, 159);
/// Tertiary text, hints, hint text (Apple `tertiaryLabel`). #6D6D72.
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(109, 109, 114);
/// Disabled elements (Apple `quaternaryLabel`). #48484A.
pub const TEXT_DISABLED: Color32 = Color32::from_rgb(72, 72, 74);

// ─────────────────────────────────────────────────────────────────────────────
// Accent and semantic colors
// ─────────────────────────────────────────────────────────────────────────────

/// System accent — Apple systemBlue. #0A84FF.
pub const ACCENT: Color32 = Color32::from_rgb(10, 132, 255);
/// Accent on hover (for the hover state of accent buttons).
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(64, 156, 255);
/// Accent on press.
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0, 106, 220);
/// Success — systemGreen. #30D158.
pub const SUCCESS: Color32 = Color32::from_rgb(48, 209, 88);
/// Warning — systemOrange. #FF9F0A.
pub const WARNING: Color32 = Color32::from_rgb(255, 159, 10);
/// Error / destructive action — systemRed. #FF453A.
pub const DANGER: Color32 = Color32::from_rgb(255, 69, 58);
/// Gold for "favorites" — systemYellow. #FFD60A.
/// Currently unused: in the UI favorites are shown in white (see the theme above),
/// so as not to stand out from the overall palette. Kept as a semantic token.
#[allow(dead_code)]
pub const STAR: Color32 = Color32::from_rgb(255, 214, 10);

// ─────────────────────────────────────────────────────────────────────────────
// Colour-axis slider tracks
// ─────────────────────────────────────────────────────────────────────────────
// Gradient rails for bipolar colour controls. These are the only *coloured*
// UI elements outside the accent: the colour carries the semantics (cool↔warm,
// green↔magenta), so they read as an intentional exception rather than a style
// break. Muted tones that sit calmly on `BG_SURFACE`.

/// Temperature track: cool blue → cyan → neutral grey → warm yellow → orange.
pub const TEMP_TRACK: [Color32; 5] = [
    Color32::from_rgb(45, 74, 116),
    Color32::from_rgb(63, 148, 182),
    Color32::from_rgb(138, 138, 138),
    Color32::from_rgb(199, 197, 73),
    Color32::from_rgb(199, 134, 60),
];

/// Tint track: green → neutral grey → magenta.
pub const TINT_TRACK: [Color32; 5] = [
    Color32::from_rgb(68, 141, 65),
    Color32::from_rgb(89, 212, 89),
    Color32::from_rgb(138, 138, 138),
    Color32::from_rgb(156, 84, 138),
    Color32::from_rgb(190, 64, 159),
];

// ─────────────────────────────────────────────────────────────────────────────
// Histogram
// ─────────────────────────────────────────────────────────────────────────────
// The channel colours are data-visualisation colours: they carry the meaning
// (R/G/B) and their overlaps mix to yellow / cyan / magenta / grey.

/// Histogram channel bars.
pub const HIST_RED: Color32 = Color32::from_rgb(196, 58, 52);
pub const HIST_GREEN: Color32 = Color32::from_rgb(62, 170, 70);
pub const HIST_BLUE: Color32 = Color32::from_rgb(56, 104, 220);
/// Overlap of two channels.
pub const HIST_OVERLAP_RG: Color32 = Color32::from_rgb(190, 176, 60);
pub const HIST_OVERLAP_GB: Color32 = Color32::from_rgb(58, 168, 186);
pub const HIST_OVERLAP_RB: Color32 = Color32::from_rgb(170, 70, 170);
/// Neutral base where all three channels overlap.
pub const HIST_BASE: Color32 = Color32::from_gray(150);
/// Clipping triangle when idle / when shown (on or hovered).
pub const HIST_TRIANGLE_IDLE: Color32 = Color32::from_gray(90);
pub const HIST_TRIANGLE_ACTIVE: Color32 = Color32::from_gray(150);

/// The colour of a clipping triangle: the clipping channels mixed, white when
/// all three clip; `None` when none clips.
pub fn clip_indicator(channels: [bool; 3]) -> Option<Color32> {
    let [r, g, b] = channels.map(|on| if on { 235 } else { 60 });
    channels.contains(&true).then(|| Color32::from_rgb(r, g, b))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tone curve
// ─────────────────────────────────────────────────────────────────────────────
// Line colours of the curve editor. The master curve uses [TEXT] (neutral);
// the per-channel curves carry the channel's colour.

pub const CURVE_RED: Color32 = Color32::from_rgb(230, 92, 86);
pub const CURVE_GREEN: Color32 = Color32::from_rgb(92, 200, 105);
pub const CURVE_BLUE: Color32 = Color32::from_rgb(96, 148, 240);

// ─────────────────────────────────────────────────────────────────────────────
// Overlays over images
// ─────────────────────────────────────────────────────────────────────────────

/// Almost opaque dark backing over the image: round cross buttons,
/// badges. Dense enough that the selection frame does not show through underneath.
pub const OVERLAY_DARK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 200);
/// A thin light rim over the dark overlays — softly separates them from the image.
pub const OVERLAY_RIM: Color32 = Color32::from_rgba_premultiplied(52, 52, 52, 52);
/// A dense dark halo under the accent indicators: keeps them readable
/// on both light and dark areas of the photo.
pub const HALO_DARK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 150);

// ─────────────────────────────────────────────────────────────────────────────
// Corner radii
// ─────────────────────────────────────────────────────────────────────────────

/// Small elements: buttons, fields, chips.
pub const RADIUS_SM: f32 = 5.0;
/// Medium elements: groups, popovers.
pub const RADIUS_MD: f32 = 8.0;
/// Large elements: windows, cards.
pub const RADIUS_LG: f32 = 12.0;

/// The same color but with the given alpha (for overlays and handles).
pub fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

// ─────────────────────────────────────────────────────────────────────────────
// Application
// ─────────────────────────────────────────────────────────────────────────────

/// Apply the theme to the egui context. Call once at startup.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    ctx.set_style(style);
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    v.panel_fill = BG_WINDOW;
    v.window_fill = BG_ELEVATED;
    v.extreme_bg_color = BG_INPUT;
    v.faint_bg_color = Color32::from_rgb(38, 38, 40);
    v.code_bg_color = BG_INPUT;

    v.window_stroke = Stroke::new(1.0, SEPARATOR);
    v.window_rounding = Rounding::same(RADIUS_LG);
    v.menu_rounding = Rounding::same(RADIUS_MD);

    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;

    v.selection.bg_fill = Color32::from_rgba_unmultiplied(10, 132, 255, 90);
    v.selection.stroke = Stroke::new(1.0, TEXT);

    let rounding = Rounding::same(RADIUS_SM);
    v.widgets = egui::style::Widgets {
        // Backgrounds, group frames, dividers.
        noninteractive: egui::style::WidgetVisuals {
            bg_fill: BG_SURFACE,
            weak_bg_fill: BG_SURFACE,
            bg_stroke: Stroke::new(1.0, SEPARATOR),
            rounding,
            fg_stroke: Stroke::new(1.0, TEXT),
            expansion: 0.0,
        },
        // Normal buttons / fields.
        inactive: egui::style::WidgetVisuals {
            bg_fill: BG_SURFACE,
            weak_bg_fill: BG_SURFACE,
            bg_stroke: Stroke::NONE,
            rounding,
            fg_stroke: Stroke::new(1.0, TEXT),
            expansion: 0.0,
        },
        // Hover.
        hovered: egui::style::WidgetVisuals {
            bg_fill: BG_ELEVATED,
            weak_bg_fill: BG_ELEVATED,
            bg_stroke: Stroke::new(1.0, BORDER),
            rounding,
            fg_stroke: Stroke::new(1.5, TEXT),
            expansion: 0.0,
        },
        // Press / selected state.
        active: egui::style::WidgetVisuals {
            bg_fill: ACCENT_ACTIVE,
            weak_bg_fill: ACCENT_ACTIVE,
            bg_stroke: Stroke::NONE,
            rounding,
            fg_stroke: Stroke::new(1.5, Color32::WHITE),
            expansion: 0.0,
        },
        // Expanded headers / open menus.
        open: egui::style::WidgetVisuals {
            bg_fill: BG_ELEVATED,
            weak_bg_fill: BG_ELEVATED,
            bg_stroke: Stroke::new(1.0, SEPARATOR),
            rounding,
            fg_stroke: Stroke::new(1.0, TEXT),
            expansion: 0.0,
        },
    };

    v
}
