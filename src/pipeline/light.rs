// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Light panel: global tonal operations in Oklab.
//!
//! Every formula is per-pixel and operates only on `L` (lightness), so the
//! whole pass is baked into a 3D LUT. Order:
//! `exposure → contrast → highlights → shadows → whites → blacks`.
//!
//! References:
//! - exposure: darktable `basicadj.c` + the Oklab property (`cbrt` LMS);
//! - contrast: the darktable `basicadj.c` idea (18.42% grey pivot), but the shape
//!   is an endpoint-preserving logit S-curve (see `apply_contrast`);
//! - highlights/shadows: Lightroom semantics, mask shape from the legacy
//!   `highlights_shadows.rs`;
//! - whites/blacks: masked gamma curve (legacy/`whites_blacks.rs`).
#![allow(dead_code)] // enabled during integration (M4)

/// Contrast pivot: `cbrt(0.1842)` — 18.42% linear grey in Oklab.
pub(crate) const CONTRAST_PIVOT: f32 = 0.5686;

/// Shadow-zone threshold (its own constant, see the plan).
const SHADOW_THRESHOLD: f32 = 0.5;
/// Highlight-zone threshold.
const HIGHLIGHT_THRESHOLD: f32 = 0.5;
/// Shadow-lift strength (gamma exponent).
const SHADOW_STRENGTH: f32 = 1.0;
/// Highlight-recovery strength.
const HIGHLIGHT_STRENGTH: f32 = 1.0;
/// Start of the Whites zone.
const WHITES_EDGE: f32 = 0.6;
/// Start of the Blacks zone.
const BLACKS_EDGE: f32 = 0.4;
/// Strength of the Whites/Blacks gamma deviation from identity.
///
/// `0` — the slider is off, `1` — canonical strength (matches the legacy
/// `powf(curve)` in sign and order of magnitude), higher — more aggressive.
const WB_STRENGTH: f32 = 1.0;

/// Light-panel parameters in internal (normalized) units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LightSettings {
    /// Exposure in stops (EV).
    pub exposure: f32,
    /// Contrast, internal range `-1..=1`.
    pub contrast: f32,
    /// Highlights (recovery when `> 0`), `-1..=1`.
    pub highlights: f32,
    /// Shadows (lift when `> 0`), `-1..=1`.
    pub shadows: f32,
    /// Whites, `-1..=1`.
    pub whites: f32,
    /// Blacks, `-1..=1`.
    pub blacks: f32,
}

impl LightSettings {
    pub(crate) fn is_identity(&self) -> bool {
        self.exposure.abs() < 1e-6
            && self.contrast.abs() < 1e-6
            && self.highlights.abs() < 1e-6
            && self.shadows.abs() < 1e-6
            && self.whites.abs() < 1e-6
            && self.blacks.abs() < 1e-6
    }
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Exposure: scales all channels by `2^(EV/3)`.
///
/// In Oklab the channels are linear in `cbrt(LMS)`, so `2^EV` of linear light
/// gives `2^(EV/3)`. Preserves hue and saturation exactly.
#[inline]
pub(crate) fn apply_exposure(p: &mut [f32; 3], ev: f32) {
    if ev == 0.0 {
        return;
    }
    let f = 2.0f32.powf(ev / 3.0);
    p[0] *= f;
    p[1] *= f;
    p[2] *= f;
}

/// Logit `ln(x / (1-x))`, domain `(0,1)`.
#[inline]
fn logit(x: f32) -> f32 {
    (x / (1.0 - x)).ln()
}

/// Sigmoid `1 / (1 + e^-y)`, domain `(0,1)`, inverse of [`logit`].
#[inline]
fn sigmoid(y: f32) -> f32 {
    1.0 / (1.0 + (-y).exp())
}

/// Contrast: endpoint-preserving S-curve pivoted at 18.42% grey.
///
/// `f(x) = sigmoid(k·logit(x) + (1−k)·logit(pivot))`, `k = 1 + amount`.
/// Properties:
/// - `amount = 0` → `k = 1` → identity; monotonic and smooth;
/// - `f(pivot) = pivot` — the pivot stays fixed for any `amount`;
/// - for `amount ≥ 0` (boost) `f(0) = 0`, `f(1) = 1` — the endpoints are
///   pinned, so nothing exits `[0,1]` and there is no clipping;
/// - for `amount < 0` (reduce) the extremes are pulled toward the pivot —
///   down to flat grey `pivot` at `amount = -1`.
///
/// The previous power form `L^(1+amount)·pivot^(−amount)` stretched the white
/// end up to `pivot^(−amount)` (≈1.76 at the maximum), and `gamut_map` then
/// hard-clipped everything above 1 — the top ~25% of the range burned to white.
#[inline]
pub(crate) fn apply_contrast(p: &mut [f32; 3], amount: f32) {
    if amount.abs() < 1e-6 {
        return;
    }
    // logit/sigmoid require a strictly open interval — stay away from the poles.
    let x = p[0].clamp(1e-4, 1.0 - 1e-4);
    let pivot = CONTRAST_PIVOT.clamp(1e-4, 1.0 - 1e-4);
    let k = 1.0 + amount;
    p[0] = sigmoid(k * logit(x) + (1.0 - k) * logit(pivot));
}

/// Highlights / Shadows: global masks on `L`, endpoint-preserving gamma.
///
/// `L' = L^(gamma_s * gamma_h)`. For both sliders `+` = brighter, `−` = darker:
/// `+shadows` lifts the shadows, `+highlights` brightens the highlights, and
/// `−highlights` recovers (pulls down) blown highlights. The shape preserves the
/// endpoints: `0 → 0`, `1 → 1` (neither a grey veil on black nor greyness on
/// white), monotonic and smooth.
///
/// `+shadows` lifts the shadows, `+highlights` brightens the highlights.
#[inline]
pub(crate) fn apply_highlights_shadows(p: &mut [f32; 3], highlights: f32, shadows: f32) {
    if (highlights.abs() < 1e-6 && shadows.abs() < 1e-6) || p[0] <= 0.0 {
        return;
    }
    let l = p[0];
    // Smooth masks: 1 in shadows/highlights, 0 at the middle.
    let s_mask = {
        let t = (1.0 - l / SHADOW_THRESHOLD).clamp(0.0, 1.0);
        t * t
    };
    let h_mask = {
        let t = ((l - HIGHLIGHT_THRESHOLD) / (1.0 - HIGHLIGHT_THRESHOLD)).clamp(0.0, 1.0);
        t * t
    };
    // `+` on either of them gives gamma < 1 → L^gamma > L → brighter.
    let gamma_s = 1.0 / (1.0 + shadows * SHADOW_STRENGTH * s_mask);
    let gamma_h = 1.0 / (1.0 + highlights * HIGHLIGHT_STRENGTH * h_mask);
    let gamma = (gamma_s * gamma_h).max(1e-3);
    p[0] = l.powf(gamma);
}

/// Whites: gamma curve in the highlight zone, endpoint-preserving.
///
/// `L' = L^γ`, where `γ = 1/(1 + whites·w·WB_STRENGTH)` for `whites > 0`
/// (brighter) and `γ = 1 + |whites|·w·WB_STRENGTH` for `whites < 0` (darker),
/// with `w = smoothstep(WHITES_EDGE, 1.0, L)` localizing the effect to the
/// highlights.
///
/// Since `1^γ = 1` and `0^γ = 0`, the white point never collapses to grey at
/// any `whites` — it only asymptotically approaches the boundary. This is the
/// shape from the reference/legacy (`r.powf(curve)`), unlike the linear `room`,
/// which dragged `L = 1` down to as low as 0.5 at `whites = -1`.
#[inline]
pub(crate) fn apply_whites(p: &mut [f32; 3], whites: f32) {
    if whites.abs() < 1e-6 || p[0] <= 0.0 {
        return;
    }
    let w = smoothstep(WHITES_EDGE, 1.0, p[0]);
    let gamma = if whites > 0.0 {
        1.0 / (1.0 + whites * w * WB_STRENGTH)
    } else {
        1.0 + (-whites) * w * WB_STRENGTH
    };
    p[0] = p[0].powf(gamma);
}

/// Blacks: gamma curve in the shadow zone, endpoint-preserving.
///
/// Mirror of [`apply_whites`]: `γ = 1/(1 + blacks·w·WB_STRENGTH)` for
/// `blacks > 0` (shadow lift), `γ = 1 + |blacks|·w·WB_STRENGTH` for
/// `blacks < 0` (deepening), `w = smoothstep(BLACKS_EDGE, 0.0, L)`.
/// The black point stays exactly `0`, the white point exactly `1`.
#[inline]
pub(crate) fn apply_blacks(p: &mut [f32; 3], blacks: f32) {
    if blacks.abs() < 1e-6 || p[0] <= 0.0 {
        return;
    }
    let w = smoothstep(BLACKS_EDGE, 0.0, p[0]);
    let gamma = if blacks > 0.0 {
        1.0 / (1.0 + blacks * w * WB_STRENGTH)
    } else {
        1.0 + (-blacks) * w * WB_STRENGTH
    };
    p[0] = p[0].powf(gamma);
}

/// Full Light pass over an Oklab pixel (order is fixed).
#[inline]
pub(crate) fn apply_light(p: &mut [f32; 3], s: &LightSettings) {
    apply_exposure(p, s.exposure);
    apply_contrast(p, s.contrast);
    apply_highlights_shadows(p, s.highlights, s.shadows);
    apply_whites(p, s.whites);
    apply_blacks(p, s.blacks);
}

// ─── Slider mapping ───────────────────────────────────────────────

/// Contrast: `slider ∈ [-100, 100]` → `amount = sign(s) * (|s|/100)^2`.
///
/// The square concentrates the useful range in the first half of the slider.
#[inline]
pub(crate) fn contrast_from_slider(slider: f32) -> f32 {
    let n = slider / 100.0;
    n * n.abs()
}

/// Plain slider `[-100, 100]` → `[-1, 1]`.
#[inline]
pub(crate) fn unit_from_slider(slider: f32) -> f32 {
    slider / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::oklab::rgb_to_oklab;

    fn grey(v: f32) -> [f32; 3] {
        rgb_to_oklab(v, v, v)
    }

    #[test]
    fn identity_does_nothing() {
        let mut p = grey(0.3);
        let orig = p;
        apply_light(&mut p, &LightSettings::default());
        assert_eq!(p, orig);
    }

    #[test]
    fn exposure_plus_one_stop_scales_all_channels() {
        let mut p = [0.5f32, 0.1, -0.05];
        apply_exposure(&mut p, 1.0);
        let f = 2.0f32.powf(1.0 / 3.0);
        assert!((p[0] - 0.5 * f).abs() < 1e-6);
        assert!((p[1] - 0.1 * f).abs() < 1e-6);
        assert!((p[2] - -0.05 * f).abs() < 1e-6);
    }

    #[test]
    fn contrast_pivot_is_stable() {
        let mut p = [CONTRAST_PIVOT, 0.0, 0.0];
        apply_contrast(&mut p, 0.8);
        assert!((p[0] - CONTRAST_PIVOT).abs() < 1e-4);
    }

    #[test]
    fn contrast_expands_range() {
        let mut dark = [0.3f32, 0.0, 0.0];
        let mut bright = [0.8f32, 0.0, 0.0];
        apply_contrast(&mut dark, 0.5);
        apply_contrast(&mut bright, 0.5);
        assert!(dark[0] < 0.3, "dark should darken: {}", dark[0]);
        assert!(bright[0] > 0.8, "bright should brighten: {}", bright[0]);
    }

    #[test]
    fn contrast_preserves_endpoints_when_increasing() {
        // For amount >= 0 nothing should be clipped at the endpoints (no clipping).
        for amount in [0.25f32, 0.5, 1.0] {
            let mut black = [0.0f32, 0.0, 0.0];
            let mut white = [1.0f32, 0.0, 0.0];
            apply_contrast(&mut black, amount);
            apply_contrast(&mut white, amount);
            assert!(black[0] < 1e-3, "black drifted: {} @ {amount}", black[0]);
            assert!(
                (white[0] - 1.0).abs() < 1e-3,
                "white drifted: {} @ {amount}",
                white[0]
            );
        }
    }

    #[test]
    fn contrast_negative_pulls_endpoints_inward() {
        // Reducing contrast must pull the extremes toward the pivot.
        let mut black = [0.0f32, 0.0, 0.0];
        let mut white = [1.0f32, 0.0, 0.0];
        apply_contrast(&mut black, -0.5);
        apply_contrast(&mut white, -0.5);
        assert!(black[0] > 0.0 && black[0] < CONTRAST_PIVOT, "black = {}", black[0]);
        assert!(white[0] < 1.0 && white[0] > CONTRAST_PIVOT, "white = {}", white[0]);
    }

    #[test]
    fn contrast_never_exceeds_unit_range() {
        // Previously the white end flew up to ~1.76 and was hard-clipped by the gamut clamp.
        for i in 0..=100 {
            let l = i as f32 / 100.0;
            let mut p = [l, 0.0, 0.0];
            apply_contrast(&mut p, 1.0);
            assert!((0.0..=1.0).contains(&p[0]), "out of range: {} from {l}", p[0]);
        }
    }

    #[test]
    fn contrast_negative_flattens() {
        let mut dark = [0.2f32, 0.0, 0.0];
        let mut bright = [0.9f32, 0.0, 0.0];
        apply_contrast(&mut dark, -0.5);
        apply_contrast(&mut bright, -0.5);
        assert!(bright[0] - dark[0] < 0.7, "range should shrink");
    }

    #[test]
    fn shadows_lift_darks_only() {
        let mut dark = [0.1f32, 0.0, 0.0];
        let mut mid = [0.6f32, 0.0, 0.0];
        apply_highlights_shadows(&mut dark, 0.0, 1.0);
        apply_highlights_shadows(&mut mid, 0.0, 1.0);
        assert!(dark[0] > 0.1, "dark lifted: {}", dark[0]);
        assert!((mid[0] - 0.6).abs() < 1e-6, "mid untouched: {}", mid[0]);
    }

    #[test]
    fn highlights_brighten_brights_only() {
        let mut bright = [0.9f32, 0.0, 0.0];
        let mut mid = [0.4f32, 0.0, 0.0];
        apply_highlights_shadows(&mut bright, 1.0, 0.0);
        apply_highlights_shadows(&mut mid, 1.0, 0.0);
        assert!(bright[0] > 0.9, "bright brightened: {}", bright[0]);
        assert!((mid[0] - 0.4).abs() < 1e-6, "mid untouched: {}", mid[0]);
    }

    #[test]
    fn highlights_negative_recovers_brights() {
        let mut bright = [0.9f32, 0.0, 0.0];
        apply_highlights_shadows(&mut bright, -1.0, 0.0);
        assert!(bright[0] < 0.9 && bright[0] > 0.6, "recovery: {}", bright[0]);
    }

    #[test]
    fn whites_and_blacks_are_local() {
        let mut high = [0.85f32, 0.0, 0.0];
        let mut low = [0.2f32, 0.0, 0.0];
        apply_whites(&mut high, 1.0);
        apply_whites(&mut low, 1.0);
        assert!(high[0] > 0.85, "high brightened: {}", high[0]);
        assert!((low[0] - 0.2).abs() < 1e-6, "low untouched: {}", low[0]);

        let mut dark = [0.1f32, 0.0, 0.0];
        let mut mid = [0.7f32, 0.0, 0.0];
        apply_blacks(&mut dark, -1.0);
        apply_blacks(&mut mid, -1.0);
        assert!(dark[0] < 0.1, "dark crushed: {}", dark[0]);
        assert!((mid[0] - 0.7).abs() < 1e-6, "mid untouched: {}", mid[0]);
    }

    #[test]
    fn blacks_positive_lifts_darks() {
        let mut dark = [0.1f32, 0.0, 0.0];
        apply_blacks(&mut dark, 1.0);
        assert!(dark[0] > 0.1);
    }

    #[test]
    fn whites_blacks_preserve_endpoints() {
        for amount in [-1.0f32, -0.5, 0.5, 1.0] {
            let mut black = [0.0f32, 0.0, 0.0];
            let mut white = [1.0f32, 0.0, 0.0];
            apply_whites(&mut black, amount);
            apply_whites(&mut white, amount);
            assert!(black[0].abs() < 1e-6, "whites moved black: {}", black[0]);
            assert!(
                (white[0] - 1.0).abs() < 1e-6,
                "whites moved white: {}",
                white[0]
            );

            let mut black = [0.0f32, 0.0, 0.0];
            let mut white = [1.0f32, 0.0, 0.0];
            apply_blacks(&mut black, amount);
            apply_blacks(&mut white, amount);
            assert!(black[0].abs() < 1e-6, "blacks moved black: {}", black[0]);
            assert!(
                (white[0] - 1.0).abs() < 1e-6,
                "blacks moved white: {}",
                white[0]
            );
        }
    }

    #[test]
    fn whites_negative_does_not_crush_white() {
        // Before: the linear `room` dragged L=1 down to 0.5 at whites = -1.
        let mut white = [1.0f32, 0.0, 0.0];
        apply_whites(&mut white, -1.0);
        assert!((white[0] - 1.0).abs() < 1e-6, "white crushed: {}", white[0]);
    }

    #[test]
    fn highlights_shadows_preserve_black_and_white() {
        let mut black = [0.0f32, 0.0, 0.0];
        let mut white = [1.0f32, 0.0, 0.0];
        apply_highlights_shadows(&mut black, 1.0, 1.0);
        apply_highlights_shadows(&mut white, 1.0, 1.0);
        assert!(black[0] < 1e-6, "black drifted: {}", black[0]);
        assert!((white[0] - 1.0).abs() < 1e-6, "white drifted: {}", white[0]);
    }

    #[test]
    fn shadows_lift_is_bounded() {
        // Even at maximum the dark end must not fly up toward 0.5.
        let mut dark = [0.1f32, 0.0, 0.0];
        apply_highlights_shadows(&mut dark, 0.0, 1.0);
        assert!(dark[0] > 0.1 && dark[0] < 0.4, "lift out of range: {}", dark[0]);
    }

    #[test]
    fn highlights_boost_is_bounded() {
        let mut bright = [0.9f32, 0.0, 0.0];
        apply_highlights_shadows(&mut bright, 1.0, 0.0);
        assert!(bright[0] > 0.9 && bright[0] < 1.0, "boost out of range: {}", bright[0]);
    }

    #[test]
    fn contrast_slider_mapping() {
        assert!((contrast_from_slider(0.0) - 0.0).abs() < 1e-6);
        assert!((contrast_from_slider(100.0) - 1.0).abs() < 1e-6);
        assert!((contrast_from_slider(-100.0) + 1.0).abs() < 1e-6);
        // Square: 50 → 0.25
        assert!((contrast_from_slider(50.0) - 0.25).abs() < 1e-6);
        assert!((contrast_from_slider(-50.0) + 0.25).abs() < 1e-6);
    }
}
