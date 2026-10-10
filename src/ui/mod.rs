// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

/// Image extensions this application supports.
/// Used in the open/save dialog filters so the user cannot pick an unsupported
/// file.
pub(crate) const SUPPORTED_IMAGE_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"];

/// Application name (for the window title and the wordmark).
pub(crate) const APP_NAME: &str = "TinyLuma";
/// Size of the compact hero window (also the minimum, so it cannot be shrunk
/// further).
pub(crate) const HERO_W: f32 = 760.0;
pub(crate) const HERO_H: f32 = 520.0;
/// Width of the side panels. Both (left and right) use one value, so the layout
/// stays symmetric.
pub(crate) const PANEL_W: f32 = 280.0;

/// How much room on a side panel to reserve for the floating scrollbar.
/// The scrollbar sits on the right, so the content is narrowed on the right by
/// this much; both side panels also add the same amount to their inner left
/// margin, so the group gutters match on either side (and the two panels mirror
/// each other).
pub(crate) const SCROLL_RESERVE: f32 = 12.0;
/// Height of the buttons in the side-panel footers ("Reset all settings",
/// "Load LUT…"). Slightly taller than the standard 28px: such a button is much
/// easier to hit with the mouse while not looking bulky next to the dense groups
/// and chips. Shared by both panels so the footers stay symmetric.
pub(crate) const FOOTER_BTN_H: f32 = 32.0;
/// Minimum editing-mode window size: both side panels (PANEL_W * 2) plus a
/// working center where the toolbar and the button row fit without overlapping.
pub(crate) const EDIT_MIN_W: f32 = 1020.0;
pub(crate) const EDIT_MIN_H: f32 = 600.0;
/// "Normal" editor window size: restore down returns to this. Otherwise the
/// window would restore to the hero size and the layout would be squeezed.
pub(crate) const EDIT_DEFAULT_W: f32 = 1280.0;
pub(crate) const EDIT_DEFAULT_H: f32 = 800.0;
/// Tagline: what this is and who it is for.
pub(crate) const APP_TAGLINE: &str = "Post-processing for AI images";
/// Application version (for the label).
pub(crate) const APP_VERSION: &str = "1.0";
/// Author — for the label on the start screen and in the interface.
pub(crate) const AUTHOR: &str = "ThetaCursed";
/// Author profile — the clickable label points here.
pub(crate) const GITHUB_URL: &str = "https://github.com/ThetaCursed/TinyLuma";
/// Feedback form (Google Forms) — bug reports and general feedback. Works
/// without a GitHub account, so it is the primary channel for users. Linked
/// from the editor's left-panel footer; the hero screen keeps only the author
/// credit.
pub(crate) const FEEDBACK_URL: &str = "https://forms.gle/W2iT49f5ikqdEqQ28";

pub(crate) mod app_impl;
pub(crate) mod center_panel;
pub(crate) mod crop_tool;
pub(crate) mod curve_editor;
pub(crate) mod dialogs;
pub(crate) mod filmstrip;
pub(crate) mod histogram_panel;
pub(crate) mod left_panel;
pub(crate) mod lut_panel;
pub(crate) mod mixer;
pub(crate) mod notification;
pub(crate) mod presets_panel;
pub(crate) mod retouch_tool;
pub(crate) mod toolbar;
pub(crate) mod widgets;
