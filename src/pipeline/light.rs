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
//! - contrast: the darktable `basicadj.c` idea (18.42% grey pivot) with an
//!   endpoint-preserving logit S-curve plus a midtone chroma coupling (see
//!   `apply_contrast`);
//! - highlights/shadows: Lightroom semantics, mask shape from the legacy
//!   `highlights_shadows.rs`;
//! - whites: hybrid of a global gain and a masked highlight gamma; blacks: a
//!   masked gamma curve.
#![allow(dead_code)] // enabled during integration (M4)

/// Contrast pivot: `cbrt(0.1842)` — 18.42% linear grey in Oklab.
pub(crate) const CONTRAST_PIVOT: f32 = 0.5686;
/// Tone-curve strength exponent: `k = 2^(amount · CONTRAST_STRENGTH)`.
///
/// Mirrors RapidRAW's `strength = 2^(con · 1.25)`, so `k ∈ [0.42, 2.38]` over
/// `amount ∈ [-1, 1]` and the curve never collapses to flat grey.
const CONTRAST_STRENGTH: f32 = 1.25;
/// Chroma-coupling strength for positive contrast (saturation boost).
///
/// At the midtone-mask peak this gives `+60 %` chroma at `amount = +1`.
const CONTRAST_CHROMA: f32 = 0.6;
/// Chroma-coupling strength for negative contrast (flattening).
///
/// Slightly weaker than the positive side, matching RapidRAW, which
/// desaturates a little less than it saturates.
const CONTRAST_CHROMA_NEG: f32 = 0.5;

/// Shadow-zone threshold (its own constant, see the plan).
const SHADOW_THRESHOLD: f32 = 0.5;
/// Highlight-zone threshold.
const HIGHLIGHT_THRESHOLD: f32 = 0.5;
/// Shadow-lift strength (gamma exponent).
const SHADOW_STRENGTH: f32 = 1.0;
/// Highlight-recovery/boost strength (exponential gamma).
///
/// `gamma = 2^(-highlights · HIGHLIGHT_STRENGTH · mask)`. Raised from the old
/// linear `1/(1+…)` form to match RapidRAW's more aggressive roll-off.
const HIGHLIGHT_STRENGTH: f32 = 3.0;
/// Start of the Whites zone.
const WHITES_EDGE: f32 = 0.6;
/// Global (whole-range) exposure component of the Whites hybrid, in stops at
/// `whites = ±1`. RapidRAW's Whites is a pure global gain; the local gamma
/// below alone diverged strongly, so the two are combined here.
const WHITES_GLOBAL_EV: f32 = 0.8;
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

/// Midtone mask for the contrast → chroma coupling.
///
/// `1` at the contrast pivot, falling smoothly to `0` at both endpoints, so
/// the extreme zones receive little to no saturation change.
#[inline]
fn contrast_chroma_mask(l: f32, pivot: f32) -> f32 {
    let span = if l < pivot { pivot } else { 1.0 - pivot };
    let t = ((l - pivot) / span.max(1e-6)).abs().clamp(0.0, 1.0);
    let m = 1.0 - t * t;
    m * m
}

/// Contrast: endpoint-preserving S-curve pivoted at 18.42% grey, plus a
/// midtone-masked chroma coupling.
///
/// `f(x) = sigmoid(k·logit(x) + (1−k)·logit(pivot))`,
/// `k = 2^(amount · CONTRAST_STRENGTH)`.
/// Properties:
/// - `amount = 0` → `k = 1` → identity; monotonic and smooth;
/// - `f(pivot) = pivot` — the pivot stays fixed for any `amount`;
/// - for `amount ≥ 0` (boost) `f(0) = 0`, `f(1) = 1` — the endpoints are
///   pinned, so nothing exits `[0,1]` and there is no clipping;
/// - for `amount < 0` (reduce) the extremes are pulled toward the pivot, but
///   the exponential mapping keeps a floor (`k ≈ 0.42` at `amount = -1`), so
///   the image flattens without collapsing to a single grey (the old
///   `k = 1 + amount` hit `k = 0` and erased all tonal structure).
///
/// On top of the tone curve, `a`/`b` are scaled by a midtone mask, so raising
/// contrast also raises saturation and lowering it flattens colours. Lightroom
/// and RapidRAW get this as a side effect of applying the S-curve per RGB
/// channel (which stretches the channel differences); doing it only on `L`
/// left chroma untouched. Scaling both axes together preserves hue exactly and
/// leaves neutrals (`a = b = 0`) neutral.
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
    let k = 2.0f32.powf(amount * CONTRAST_STRENGTH);
    p[0] = sigmoid(k * logit(x) + (1.0 - k) * logit(pivot));

    // Contrast → saturation coupling. Slightly weaker on the negative side,
    // matching RapidRAW's per-channel behaviour.
    let coupling = if amount >= 0.0 {
        CONTRAST_CHROMA
    } else {
        CONTRAST_CHROMA_NEG
    };
    let scale = 1.0 + amount * coupling * contrast_chroma_mask(x, pivot);
    p[1] *= scale;
    p[2] *= scale;
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
    // Exponential highlight gamma: stays strictly positive for both signs (the
    // old `1/(1+…)` hit a zero denominator at `highlights = -1`) and lets the
    // strength exceed 1 without inverting the direction.
    let gamma_h = 2.0f32.powf(-highlights * HIGHLIGHT_STRENGTH * h_mask);
    let gamma = (gamma_s * gamma_h).max(1e-3);
    p[0] = l.powf(gamma);
}

/// Whites: hybrid of a global gain and a local highlight gamma.
///
/// RapidRAW (and Lightroom) treat Whites as a white-point move that affects the
/// whole range, while the legacy TinyLuma shape was purely local. Here a small
/// global exposure (`2^(whites · WHITES_GLOBAL_EV / 3)` in Oklab `L`, i.e.
/// `2^(whites · WHITES_GLOBAL_EV)` in linear light) is applied first, then the
/// endpoint-friendly highlight gamma `L^γ` with
/// `w = smoothstep(WHITES_EDGE, 1.0, L)`:
/// `γ = 1/(1 + whites·w·WB_STRENGTH)` for `whites > 0` (brighter) and
/// `γ = 1 + |whites|·w·WB_STRENGTH` for `whites < 0` (darker).
///
/// The global part is kept below 1 stop at the extreme so the missing tone
/// mapper does not blow the highlights; `gamut_map` soft-clips anything above 1.
#[inline]
pub(crate) fn apply_whites(p: &mut [f32; 3], whites: f32) {
    if whites.abs() < 1e-6 || p[0] <= 0.0 {
        return;
    }
    // Global component (the RapidRAW "white point" part).
    p[0] *= 2.0f32.powf(whites * WHITES_GLOBAL_EV / 3.0);

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

/// Contrast: `slider ∈ [-100, 100]` → `amount = slider / 100` (linear).
///
/// The curve then applies `k = 2^(amount · CONTRAST_STRENGTH)` in
/// [`apply_contrast`], mirroring RapidRAW/Lightroom's slider response.
#[inline]
pub(crate) fn contrast_from_slider(slider: f32) -> f32 {
    slider / 100.0
}

/// Plain slider `[-100, 100]` → `[-1, 1]`.
#[inline]
pub(crate) fn unit_from_slider(slider: f32) -> f32 {
    slider / 100.0
}

/// Exposure slider full-scale, in EV (the ends of the `-5..=5` slider).
const EXPOSURE_MAX_EV: f32 = 5.0;
/// Exponent of the non-linear exposure response.
///
/// `1.0` is the old linear behaviour; `1.5` gives finer control around 0 EV
/// (where the useful range is) and compresses the extremes. Symmetric:
/// `EV = EXPOSURE_MAX_EV · sign(n)·|n|^p`, `n = slider / EXPOSURE_MAX_EV`.
const EXPOSURE_CURVE: f32 = 1.5;

/// Exposure slider → EV mapping (soft around 0, steeper at the ends).
#[inline]
pub(crate) fn exposure_from_slider(slider: f32) -> f32 {
    let n = (slider / EXPOSURE_MAX_EV).clamp(-1.0, 1.0);
    EXPOSURE_MAX_EV * n * n.abs().powf(EXPOSURE_CURVE - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::oklab::rgb_to_oklab;

    fn grey(v: f32) -> [f32; 3] {
        rgb_to_oklab(v, v, v)
    }

    fn chroma(p: [f32; 3]) -> f32 {
        (p[1] * p[1] + p[2] * p[2]).sqrt()
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
    fn contrast_max_negative_does_not_collapse() {
        // amount = -1 must keep a floor via k = 2^(-1.25) ~= 0.42; the old
        // `k = 1 + amount` became 0 and flattened everything to the pivot.
        let mut dark = [0.27f32, 0.0, 0.0];
        let mut bright = [0.89f32, 0.0, 0.0];
        apply_contrast(&mut dark, -1.0);
        apply_contrast(&mut bright, -1.0);
        assert!(dark[0] < CONTRAST_PIVOT && bright[0] > CONTRAST_PIVOT);
        assert!(
            bright[0] - dark[0] > 0.2,
            "range collapsed: {}..{}",
            dark[0],
            bright[0]
        );
    }

    #[test]
    fn contrast_boosts_chroma_of_colors() {
        // A midtone saturated colour must gain saturation, like the per-channel
        // S-curves in Lightroom/RapidRAW.
        let mut p = rgb_to_oklab(0.5, 0.15, 0.05);
        let before = chroma(p);
        apply_contrast(&mut p, 0.5);
        assert!(chroma(p) > before, "chroma {} -> {}", before, chroma(p));
    }

    #[test]
    fn contrast_negative_reduces_chroma() {
        let mut p = rgb_to_oklab(0.5, 0.15, 0.05);
        let before = chroma(p);
        apply_contrast(&mut p, -0.5);
        assert!(chroma(p) < before, "chroma {} -> {}", before, chroma(p));
    }

    #[test]
    fn contrast_chroma_coupling_preserves_hue() {
        // Scaling `a`/`b` together must not rotate the hue (null cross product).
        let mut p = rgb_to_oklab(0.5, 0.15, 0.05);
        let (a0, b0) = (p[1], p[2]);
        apply_contrast(&mut p, 0.7);
        let cross = a0 * p[2] - b0 * p[1];
        assert!(cross.abs() < 1e-6, "hue rotated: cross = {cross}");
    }

    #[test]
    fn contrast_chroma_coupling_leaves_neutrals_neutral() {
        let mut p = grey(0.4);
        let orig = p[0];
        apply_contrast(&mut p, 1.0);
        assert!(
            p[1].abs() < 1e-6 && p[2].abs() < 1e-6,
            "grey gained colour: {p:?}"
        );
        assert!(
            (p[0] - orig).abs() > 1e-3,
            "tone curve stopped working: {orig} -> {}",
            p[0]
        );
    }

    #[test]
    fn contrast_chroma_coupling_is_masked_at_extremes() {
        // A near-black coloured pixel gets a smaller relative boost than a
        // midtone one, because the coupling mask fades toward the endpoints.
        let mut mid = rgb_to_oklab(0.5, 0.15, 0.05);
        let mut dark = rgb_to_oklab(0.02, 0.006, 0.002);
        let mid_before = chroma(mid);
        let dark_before = chroma(dark);
        apply_contrast(&mut mid, 1.0);
        apply_contrast(&mut dark, 1.0);
        let mid_gain = chroma(mid) / mid_before;
        let dark_gain = chroma(dark) / dark_before;
        assert!(mid_gain > dark_gain, "mid {mid_gain} vs dark {dark_gain}");
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
    fn whites_hybrid_moves_whole_range_blacks_are_local() {
        // Whites is a hybrid: the global term lifts the whole range...
        let mut high = [0.85f32, 0.0, 0.0];
        let mut low = [0.2f32, 0.0, 0.0];
        apply_whites(&mut high, 1.0);
        apply_whites(&mut low, 1.0);
        assert!(high[0] > 0.85, "high brightened: {}", high[0]);
        assert!(low[0] > 0.2, "low lifted by the global term: {}", low[0]);

        // ...while Blacks stays local to the shadow zone.
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
    fn blacks_preserve_endpoints() {
        for amount in [-1.0f32, -0.5, 0.5, 1.0] {
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
    fn whites_hybrid_moves_white_point_but_keeps_black() {
        // Black stays pinned; the white point moves (global gain), unlike the
        // old purely-local shape.
        let mut black = [0.0f32, 0.0, 0.0];
        apply_whites(&mut black, 1.0);
        apply_whites(&mut black, -1.0);
        assert!(black[0].abs() < 1e-6, "whites moved black: {}", black[0]);

        let mut up = [1.0f32, 0.0, 0.0];
        apply_whites(&mut up, 1.0);
        assert!(up[0] > 1.0, "white point should rise: {}", up[0]);

        let mut down = [1.0f32, 0.0, 0.0];
        apply_whites(&mut down, -1.0);
        assert!(down[0] < 1.0, "white point should drop: {}", down[0]);
    }

    #[test]
    fn whites_negative_keeps_white_reasonable() {
        // Global gain + local gamma darken the top end, but must not collapse it
        // to mid grey: at whites = -1 the white point stays well above 0.6.
        let mut white = [1.0f32, 0.0, 0.0];
        apply_whites(&mut white, -1.0);
        assert!(white[0] > 0.6, "white crushed: {}", white[0]);
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
        // Linear: 50 -> 0.5
        assert!((contrast_from_slider(50.0) - 0.5).abs() < 1e-6);
        assert!((contrast_from_slider(-50.0) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn exposure_slider_curve_is_fine_near_zero() {
        // Endpoints fixed, symmetric and monotonic.
        assert!(exposure_from_slider(0.0).abs() < 1e-6);
        assert!((exposure_from_slider(5.0) - 5.0).abs() < 1e-6);
        assert!((exposure_from_slider(-5.0) + 5.0).abs() < 1e-6);
        let mut prev = f32::NEG_INFINITY;
        for i in -50..=50 {
            let v = exposure_from_slider(i as f32 / 10.0);
            assert!(v >= prev, "not monotonic at {i}: {v} < {prev}");
            prev = v;
        }
        // Fine around 0: 1.0 on the slider is now well under 1 stop.
        let e1 = exposure_from_slider(1.0);
        assert!(e1 > 0.3 && e1 < 0.6, "1.0 slider = {e1} EV");
        // The whole first half still moves slower than linear.
        assert!(exposure_from_slider(2.5) < 2.5, "sub-linear at 2.5");
    }
}
