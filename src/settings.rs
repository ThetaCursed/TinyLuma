// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug)]
pub(crate) enum SaveFormat {
    Png,
    Jpg,
    WebP,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SaveSettings {
    #[serde(default = "default_save_format")]
    pub(crate) format: SaveFormat,
    #[serde(default = "default_quality")]
    pub(crate) quality: u8,
    #[serde(default)]
    pub(crate) last_input_dir: Option<String>,
    #[serde(default)]
    pub(crate) last_output_dir: Option<String>,
    #[serde(default = "default_split_position")]
    pub(crate) split_position: f32,
    /// Open/closed state of the left panel's collapsible groups:
    /// [LIGHT, COLOR, DETAILS, EFFECTS].
    #[serde(default = "default_open_groups")]
    pub(crate) open_groups: [bool; 4],
    /// Postfix for batch export (Save All).
    #[serde(default = "default_postfix")]
    pub(crate) postfix: String,
    /// Carry the source PNG's metadata over to the saved PNG.
    #[serde(default)]
    pub(crate) embed_png_metadata: bool,
    /// Show only favorites in the LUT library (★ filter).
    /// Persisted across runs so the mode is not reset.
    #[serde(default)]
    pub(crate) lut_favorites_only: bool,
    /// Expanded categories (folders) of the LUT library.
    /// We store the names of the EXPANDED categories: new folders are collapsed
    /// by default (as before), and any removed from the config are ignored.
    #[serde(default)]
    pub(crate) open_lut_categories: BTreeSet<String>,
    /// Retouch brush size (working-image pixels). Remembered across runs so the
    /// user starts the next session with the brush they are comfortable with.
    #[serde(default = "default_brush_size")]
    pub(crate) brush_size: f32,
    /// Retouch brush hardness, `0..1`.
    #[serde(default = "default_brush_hardness")]
    pub(crate) brush_hardness: f32,
}

impl Default for SaveSettings {
    fn default() -> Self {
        Self {
            format: default_save_format(),
            quality: default_quality(),
            last_input_dir: None,
            last_output_dir: None,
            split_position: default_split_position(),
            open_groups: default_open_groups(),
            postfix: default_postfix(),
            embed_png_metadata: false,
            lut_favorites_only: false,
            open_lut_categories: BTreeSet::new(),
            brush_size: default_brush_size(),
            brush_hardness: default_brush_hardness(),
        }
    }
}

/// By default all left-panel groups are expanded.
pub(crate) fn default_open_groups() -> [bool; 4] {
    [true; 4]
}

/// Defaults for fields that may be missing from older configs. Kept here so
/// the serde defaults and the fallback in `load_save_settings` don't drift
/// apart.
pub(crate) fn default_save_format() -> SaveFormat {
    SaveFormat::Jpg
}

pub(crate) fn default_quality() -> u8 {
    90
}

pub(crate) fn default_split_position() -> f32 {
    0.5
}

/// Default batch-export postfix.
pub(crate) fn default_postfix() -> String {
    "_edited".to_string()
}

/// Retouch brush defaults. `BrushSettings` is the single source of truth, so the
/// config fallback can never drift from the tool's own default.
pub(crate) fn default_brush_size() -> f32 {
    crate::retouch::BrushSettings::default().size
}

pub(crate) fn default_brush_hardness() -> f32 {
    crate::retouch::BrushSettings::default().hardness
}

/// All settings for a single image (the sliders).
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct FilterSettings {
    pub(crate) exposure: f32,
    pub(crate) contrast: f32,
    pub(crate) whites: f32,
    pub(crate) blacks: f32,
    pub(crate) shadows: f32,
    pub(crate) highlights: f32,
    pub(crate) temp: f32,
    pub(crate) tint: f32, // Added tint
    pub(crate) vibrance: f32,
    pub(crate) saturation: f32,

    pub(crate) texture: f32,
    pub(crate) clarity: f32,
    pub(crate) dehaze: f32,
    pub(crate) sharpen: f32,
    pub(crate) lut_intensity: f32,
    pub(crate) grain: f32,
}

impl Default for FilterSettings {
    fn default() -> Self {
        Self {
            // Everything now defaults to 0.0
            exposure: 0.0,
            contrast: 0.0,
            whites: 0.0,
            blacks: 0.0,
            shadows: 0.0,
            highlights: 0.0,
            temp: 0.0,
            tint: 0.0,
            vibrance: 0.0,
            saturation: 0.0,

            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            sharpen: 0.0,
            lut_intensity: 0.0,
            grain: 0.0,
        }
    }
}
