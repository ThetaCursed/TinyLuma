// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! HSL / colour mixer: 8 hue bands (Red, Orange, Yellow, Green, Aqua, Blue,
//! Purple, Magenta), each with Hue / Saturation / Luminance sliders.
//!
//! Works in **OkLCh** — the same colour space the rest of the pipeline uses —
//! and is applied in the color pass after Light and before `apply_chroma`, so
//! it bakes into the combined 3D LUT for free.
//!
//! The band influence is a **partition of unity**: a raised-cosine blend
//! between the two neighbouring band centres, so tones mix smoothly instead of
//! stepping at a boundary. Hue and luminance changes fade out on near-neutral
//! colours (chroma gating), so greys do not drift.
//!
//! 8-band OkLCh mixer and partition-of-unity weights — based on lightcraft
//! (MIT OR Apache-2.0). See `docs/HSL.md`.

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::color::oklab::{rgb_to_oklab, srgb_to_linear};

/// Number of mixer bands.
pub(crate) const HSL_BANDS: usize = 8;

/// Band display names, in Lightroom order.
pub(crate) const BAND_NAMES: [&str; HSL_BANDS] = [
    "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta",
];

/// Band centre hues as HSV hue angles (degrees), in Lightroom order.
const MIXER_HUES: [f32; HSL_BANDS] = [0.0, 30.0, 60.0, 120.0, 180.0, 225.0, 270.0, 315.0];

/// Hue-shift scale: slider ±100 → ±0.5 rad (≈ ±28.6°).
const HUE_SCALE: f32 = 0.5;
/// Luminance-shift scale: slider ±100 → ±0.18 in Oklab `L`.
const LUM_SCALE: f32 = 0.18;
/// Chroma at which the hue/luminance gating reaches full strength (Oklab).
const CHROMA_GATE: f32 = 0.12;

/// Mixer settings: `[band][0=hue, 1=saturation, 2=luminance]`, each slider in
/// `-100..=100`.
///
/// Fixed-size arrays keep `FilterSettings: Copy`; `serde(default)` on the field
/// in `FilterSettings` keeps older presets/configs loading as a no-op mixer.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize, Debug)]
pub(crate) struct HslSettings {
    pub(crate) bands: [[f32; 3]; HSL_BANDS],
}

impl HslSettings {
    /// True when every slider is zero (the mixer is a no-op).
    pub(crate) fn is_identity(&self) -> bool {
        self.bands.iter().flatten().all(|v| v.abs() < 1e-6)
    }
}

/// HSV→RGB for a fully saturated colour at hue `deg` (`s = v = 1`).
fn hsv_hue_rgb(deg: f32) -> [f32; 3] {
    let h = (deg / 60.0).rem_euclid(6.0);
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as i32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// OkLCh hue (radians) of a pure sRGB colour with HSV hue `deg`.
fn oklch_hue_of_srgb_hue(deg: f32) -> f32 {
    let rgb = hsv_hue_rgb(deg).map(srgb_to_linear);
    let lab = rgb_to_oklab(rgb[0], rgb[1], rgb[2]);
    lab[2].atan2(lab[1])
}

/// OkLCh hue (radians) of each band centre, computed once.
pub(crate) fn band_hues() -> &'static [f32; HSL_BANDS] {
    static H: OnceLock<[f32; HSL_BANDS]> = OnceLock::new();
    H.get_or_init(|| MIXER_HUES.map(oklch_hue_of_srgb_hue))
}

/// Wraps an angle to `-π..π`.
#[inline]
fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

/// Partition-of-unity weights of hue `h` (radians) over the 8 bands.
///
/// Between two band centres the weight is a raised cosine `0.5 - 0.5·cos(πt)`,
/// so at every hue the weights sum to 1 and at a band centre that band
/// dominates. The blend interval is the angular gap to the next centre, which
/// handles the uneven Lightroom spacing.
pub(crate) fn band_weights(h: f32) -> [f32; HSL_BANDS] {
    let hues = band_hues();
    let mut w = [0.0f32; HSL_BANDS];
    for i in 0..HSL_BANDS {
        let a = hues[i];
        let b = hues[(i + 1) % HSL_BANDS];
        let span = wrap(b - a).rem_euclid(TAU);
        let d = wrap(h - a).rem_euclid(TAU);
        if d <= span {
            let t = d / span;
            let s = 0.5 - 0.5 * (t * PI).cos();
            w[i] += 1.0 - s;
            w[(i + 1) % HSL_BANDS] += s;
            break;
        }
    }
    w
}

/// Applies the mixer to an Oklab pixel `[L, a, b]`.
///
/// No-op when every band is neutral.
#[inline]
pub(crate) fn apply_hsl(p: &mut [f32; 3], s: &HslSettings) {
    if s.is_identity() {
        return;
    }
    let c0 = (p[1] * p[1] + p[2] * p[2]).sqrt();
    let chroma_w = (c0 / CHROMA_GATE).min(1.0);
    let h = p[2].atan2(p[1]);
    let w = band_weights(h);

    let mut dh = 0.0;
    let mut ds = 0.0;
    let mut dl = 0.0;
    for (i, b) in s.bands.iter().enumerate() {
        dh += w[i] * (b[0] / 100.0) * HUE_SCALE;
        ds += w[i] * (b[1] / 100.0);
        dl += w[i] * (b[2] / 100.0) * LUM_SCALE;
    }

    // Saturation acts on chroma directly; hue/luminance fade out on neutrals.
    let c = c0 * (1.0 + ds).max(0.0);
    let h = h + dh * chroma_w;
    p[0] += dl * chroma_w * p[0].max(0.05).sqrt();
    p[1] = c * h.cos();
    p[2] = c * h.sin();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_partition_unity() {
        for i in 0..720 {
            let h = (i as f32 / 720.0) * TAU - PI;
            let w = band_weights(h);
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "{h}: {sum} {w:?}");
            assert!(w.iter().all(|v| *v >= -1e-6), "{h}: {w:?}");
        }
    }

    #[test]
    fn band_centre_dominates() {
        for i in 0..HSL_BANDS {
            let w = band_weights(band_hues()[i]);
            assert!(w[i] > 0.99, "band {i}: {w:?}");
        }
    }

    #[test]
    fn identity_is_a_no_op() {
        let s = HslSettings::default();
        let mut p = [0.5, 0.1, -0.05];
        let before = p;
        apply_hsl(&mut p, &s);
        assert_eq!(p, before);
    }

    #[test]
    fn red_reacts_only_to_the_red_band() {
        let base = rgb_to_oklab(srgb_to_linear(1.0), 0.0, 0.0);
        let mut s = HslSettings::default();
        s.bands[0][1] = -100.0; // desaturate reds
        let mut p = base;
        apply_hsl(&mut p, &s);
        let c0 = (base[1].powi(2) + base[2].powi(2)).sqrt();
        let c1 = (p[1].powi(2) + p[2].powi(2)).sqrt();
        assert!(c1 < c0 * 0.2, "red chroma {c0} -> {c1}");

        // A saturated blue must be untouched by the red band.
        let base_b = rgb_to_oklab(0.0, 0.0, srgb_to_linear(1.0));
        let mut p_b = base_b;
        apply_hsl(&mut p_b, &s);
        assert!((p_b[1] - base_b[1]).abs() < 1e-4, "{p_b:?}");
        assert!((p_b[2] - base_b[2]).abs() < 1e-4, "{p_b:?}");
    }

    #[test]
    fn neutral_stays_neutral() {
        let mut s = HslSettings::default();
        for b in s.bands.iter_mut() {
            b[0] = 40.0;
            b[1] = 60.0;
            b[2] = 30.0;
        }
        let mut p = rgb_to_oklab(0.2, 0.2, 0.2);
        let before = p;
        apply_hsl(&mut p, &s);
        assert!((p[0] - before[0]).abs() < 1e-5);
        assert!(p[1].abs() < 1e-6 && p[2].abs() < 1e-6);
    }

    #[test]
    fn lightness_band_moves_l() {
        let mut s = HslSettings::default();
        s.bands[3][2] = 100.0; // brighten greens
        let base = rgb_to_oklab(0.0, srgb_to_linear(1.0), 0.0);
        let mut p = base;
        apply_hsl(&mut p, &s);
        assert!(p[0] > base[0], "L {} -> {}", base[0], p[0]);
    }

    #[test]
    fn serialization_roundtrip() {
        let mut s = HslSettings::default();
        s.bands[2][1] = 42.0;
        let json = serde_json::to_string(&s).unwrap();
        let back: HslSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
