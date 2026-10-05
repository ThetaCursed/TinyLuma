// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Color panel: white balance (Kelvin), saturation, vibrance, gamut.
//!
//! Every operation is per-pixel, so it is baked into a 3D LUT.
//!
//! References:
//! - CCT→xy: Kim et al. 1999 (Planckian locus approximation);
//! - chromatic adaptation: Bradford (constants by Bruce Lindbloom);
//! - saturation/vibrance: darktable `basicadj.c`;
//! - gamut clipping: Ottosson, "sRGB gamut clipping".
#![allow(dead_code)] // enabled during integration (M4)

use crate::color::oklab::oklab_to_linear_rgb;

/// Reference temperature (white-balance identity), K.
pub(crate) const REFERENCE_KELVIN: f32 = 6500.0;
/// Temp slider scale bounds.
const KELVIN_COOL: f32 = 2000.0; // slider = -100
const KELVIN_WARM: f32 = 15000.0; // slider = +100
/// Tint scale: slider ±100 → ±0.02 Duv.
const TINT_DUV_SCALE: f32 = 0.02;
/// Maximum chroma for vibrance (Oklab).
const CHROMA_MAX: f32 = 0.32;
/// Vibrance protection exponent (higher protects saturated colors more).
const VIBRANCE_PROTECTION: f32 = 2.0;
/// Saturation compression: already-saturated colours get less boost, matching
/// the way RapidRAW's RGB gamut limits their saturation growth.
const SATURATION_COMPRESSION: f32 = 0.5;
/// Negative-vibrance desaturation window (as in RapidRAW).
///
/// `sat` is an Oklab-chroma proxy for HSV saturation (`~2·C/CHROMA_MAX`); below
/// `LO` nothing happens, above `HI` a `-100` slider fully desaturates.
const VIBRANCE_DESAT_LO: f32 = 0.05;
const VIBRANCE_DESAT_HI: f32 = 0.75;

type Mat3 = [[f32; 3]; 3];

#[inline]
fn mat3_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let mut s = 0.0;
            for k in 0..3 {
                s += a[i][k] * b[k][j];
            }
            out[i][j] = s;
        }
    }
    out
}

#[inline]
fn mat3_apply(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

// sRGB / D65.
const RGB_TO_XYZ: Mat3 = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175_0],
    [0.019_333_9, 0.119_192_0, 0.950_304_1],
];
const XYZ_TO_RGB: Mat3 = [
    [3.240_454_2, -1.537_138_5, -0.498_531_4],
    [-0.969_266_0, 1.876_010_8, 0.041_556_0],
    [0.055_643_4, -0.204_025_9, 1.057_225_2],
];
// Bradford cone-response matrix and its inverse.
const BRADFORD: Mat3 = [
    [0.895_1, 0.266_4, -0.161_4],
    [-0.750_2, 1.713_5, 0.036_7],
    [0.038_9, -0.068_5, 1.029_6],
];
const BRADFORD_INV: Mat3 = [
    [0.986_992_9, -0.147_054_3, 0.159_962_7],
    [0.432_305_3, 0.518_360_3, 0.049_291_2],
    [-0.008_528_7, 0.040_042_8, 0.968_486_7],
];

/// CIE xy chromaticity for color temperature `kelvin` (Kim et al. 1999).
pub(crate) fn cct_to_xy(kelvin: f32) -> [f32; 2] {
    let t = kelvin.max(1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t3 - 0.234_358_9e6 / t2 + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t3 + 2.107_037_9e6 / t2 + 0.222_634_7e3 / t + 0.240_390
    };
    let x2 = x * x;
    let x3 = x2 * x;
    let y = if t <= 2222.0 {
        -1.106_381_4 * x3 - 1.348_110_2 * x2 + 2.185_558_3 * x - 0.202_196_8
    } else if t <= 4000.0 {
        -0.954_947_6 * x3 - 1.374_185_9 * x2 + 2.091_370_2 * x - 0.167_488_7
    } else {
        3.081_758_0 * x3 - 5.873_386_8 * x2 + 3.751_130_0 * x - 0.370_014_8
    };
    [x, y]
}

/// Temp slider `[-100, 100]` → Kelvin.
///
/// The scale is linear in **mired** (`1e6 / K`) separately toward the warm and
/// cool sides: `0 → 6500K`, `+100 → 15000K` (warmer), `-100 → 2000K` (cooler).
/// Mired is more perceptually uniform than Kelvin.
pub(crate) fn slider_to_kelvin(slider: f32) -> f32 {
    let t = (slider / 100.0).clamp(-1.0, 1.0).abs();
    let m_ref = 1.0e6 / REFERENCE_KELVIN;
    let m = if slider >= 0.0 {
        let m_warm = 1.0e6 / KELVIN_WARM;
        m_ref + (m_warm - m_ref) * t
    } else {
        let m_cool = 1.0e6 / KELVIN_COOL;
        m_ref + (m_cool - m_ref) * t
    };
    1.0e6 / m
}

#[inline]
fn xy_to_xyz(xy: [f32; 2]) -> [f32; 3] {
    let [x, y] = xy;
    [x / y, 1.0, (1.0 - x - y) / y]
}

#[inline]
fn xy_to_uv(xy: [f32; 2]) -> [f32; 2] {
    let [x, y] = xy;
    let d = -2.0 * x + 12.0 * y + 3.0;
    [4.0 * x / d, 6.0 * y / d]
}

#[inline]
fn uv_to_xy(uv: [f32; 2]) -> [f32; 2] {
    let [u, v] = uv;
    let d = 2.0 * u - 8.0 * v + 4.0;
    [3.0 * u / d, 2.0 * v / d]
}

/// Shifts the white point along the perpendicular to the Planckian locus by
/// `duv`.
///
/// A positive `duv` goes toward green; the sign of Tint is applied by the caller.
fn offset_duv(kelvin: f32, duv: f32) -> [f32; 2] {
    let uv = xy_to_uv(cct_to_xy(kelvin));
    // Locus tangent via finite difference; perpendicular is a 90° rotation.
    let uv_next = xy_to_uv(cct_to_xy(kelvin * 1.001));
    let tx = uv_next[0] - uv[0];
    let ty = uv_next[1] - uv[1];
    let len = (tx * tx + ty * ty).sqrt().max(1e-9);
    let (px, py) = (-ty / len, tx / len);
    uv_to_xy([uv[0] + px * duv, uv[1] + py * duv])
}

/// Bradford chromatic-adaptation matrix from `src_xy` to `dst_xy` (in linear RGB).
fn bradford_rgb_matrix(src_xy: [f32; 2], dst_xy: [f32; 2]) -> Mat3 {
    let src_lms = mat3_apply(&BRADFORD, xy_to_xyz(src_xy));
    let dst_lms = mat3_apply(&BRADFORD, xy_to_xyz(dst_xy));

    let mut scale: Mat3 = [[0.0; 3]; 3];
    for i in 0..3 {
        scale[i][i] = dst_lms[i] / src_lms[i].max(1e-9);
    }
    let m_xyz = mat3_mul(&mat3_mul(&BRADFORD_INV, &scale), &BRADFORD);
    mat3_mul(&mat3_mul(&XYZ_TO_RGB, &m_xyz), &RGB_TO_XYZ)
}

/// Precomputed white balance (a matrix in linear RGB).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WhiteBalance {
    m: Mat3,
}

impl WhiteBalance {
    pub(crate) fn new(temp_slider: f32, tint_slider: f32) -> Self {
        if temp_slider == 0.0 && tint_slider == 0.0 {
            return Self {
                m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            };
        }
        let kelvin = slider_to_kelvin(temp_slider);
        // Tint > 0 is magenta (below the locus), hence a negative Duv.
        let duv = -(tint_slider / 100.0).clamp(-1.0, 1.0) * TINT_DUV_SCALE;
        let target_xy = offset_duv(kelvin, duv);
        let reference_xy = cct_to_xy(REFERENCE_KELVIN);
        Self {
            m: bradford_rgb_matrix(target_xy, reference_xy),
        }
    }

    pub(crate) fn is_identity(&self) -> bool {
        const I: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut max = 0.0f32;
        for i in 0..3 {
            for j in 0..3 {
                max = max.max((self.m[i][j] - I[i][j]).abs());
            }
        }
        max < 1e-6
    }

    #[inline]
    pub(crate) fn apply_linear(&self, rgb: &mut [f32; 3]) {
        *rgb = mat3_apply(&self.m, *rgb);
    }
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Saturation: scales chroma `a,b`, with a light compression so already
/// saturated colours grow less (as in RapidRAW, where the RGB gamut limits
/// their growth). Negative amounts stay a plain uniform desaturation.
#[inline]
pub(crate) fn apply_saturation(p: &mut [f32; 3], amount: f32) {
    if amount.abs() < 1e-6 {
        return;
    }
    let f = if amount > 0.0 {
        let chroma = (p[1] * p[1] + p[2] * p[2]).sqrt();
        let norm = (chroma / CHROMA_MAX).min(1.0);
        1.0 + amount * (1.0 - SATURATION_COMPRESSION * norm)
    } else {
        1.0 + amount
    };
    p[1] *= f;
    p[2] *= f;
}

/// Vibrance: boosts weakly saturated colors more than saturated ones.
///
/// Positive side keeps the legacy protection (muted colours change most).
/// Negative side follows RapidRAW: saturated colours are desaturated while
/// near-neutrals are left alone.
#[inline]
pub(crate) fn apply_vibrance(p: &mut [f32; 3], amount: f32) {
    if amount.abs() < 1e-6 {
        return;
    }
    let chroma = (p[1] * p[1] + p[2] * p[2]).sqrt();
    let norm = (chroma / CHROMA_MAX).min(1.0);
    let scale = if amount > 0.0 {
        let pf = (1.0 - norm).powf(VIBRANCE_PROTECTION);
        1.0 + amount * pf
    } else {
        let sat = (norm * 2.0).min(1.0);
        let w = smoothstep(VIBRANCE_DESAT_LO, VIBRANCE_DESAT_HI, sat);
        1.0 + amount * w
    };
    p[1] *= scale;
    p[2] *= scale;
}

#[inline]
fn in_gamut(rgb: [f32; 3]) -> bool {
    const E: f32 = 1e-4;
    rgb[0] >= -E && rgb[0] <= 1.0 + E && rgb[1] >= -E && rgb[1] <= 1.0 + E && rgb[2] >= -E && rgb[2] <= 1.0 + E
}

// ─── Gamut clipping (Ottosson, "sRGB gamut clipping") ───────────────

/// Maximum saturation `S = C/L` achievable in sRGB for a given hue.
///
/// `a`,`b` is a normalized hue vector (`a² + b² = 1`). Polynomial approximation
/// plus one Halley step on the exact "channel equals 0" condition.
fn compute_max_saturation(a: f32, b: f32) -> f32 {
    let (k0, k1, k2, k3, k4, wl, wm, ws);
    if -1.881_703_3 * a - 0.809_364_93 * b > 1.0 {
        // The red channel goes negative first.
        (k0, k1, k2, k3, k4) = (1.190_862_8, 1.765_767_3, 0.596_626_4, 0.755_151_97, 0.567_712_45);
        (wl, wm, ws) = (4.076_741_7, -3.307_711_6, 0.230_969_94);
    } else if 1.814_441_1 * a - 1.194_452_8 * b > 1.0 {
        // Green.
        (k0, k1, k2, k3, k4) = (0.739_565_15, -0.459_544_04, 0.082_854_27, 0.125_410_7, 0.145_032_04);
        (wl, wm, ws) = (-1.268_438, 2.609_757_4, -0.341_319_38);
    } else {
        // Blue.
        (k0, k1, k2, k3, k4) = (1.357_336_5, -0.009_157_99, -1.151_302_1, -0.505_596_06, 0.006_921_67);
        (wl, wm, ws) = (-0.004_196_086_3, -0.703_418_6, 1.707_614_7);
    }

    let s = k0 + k1 * a + k2 * b + k3 * a * a + k4 * a * b;

    let k_l = 0.396_337_78 * a + 0.215_803_76 * b;
    let k_m = -0.105_561_346 * a - 0.063_854_17 * b;
    let k_s = -0.089_484_18 * a - 1.291_485_5 * b;

    let l_ = 1.0 + s * k_l;
    let m_ = 1.0 + s * k_m;
    let s_ = 1.0 + s * k_s;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;

    let l_ds = 3.0 * k_l * l_ * l_;
    let m_ds = 3.0 * k_m * m_ * m_;
    let s_ds = 3.0 * k_s * s_ * s_;

    let l_ds2 = 6.0 * k_l * k_l * l_;
    let m_ds2 = 6.0 * k_m * k_m * m_;
    let s_ds2 = 6.0 * k_s * k_s * s_;

    let f = wl * l + wm * m + ws * s3;
    let f1 = wl * l_ds + wm * m_ds + ws * s_ds;
    let f2 = wl * l_ds2 + wm * m_ds2 + ws * s_ds2;

    s - f * f1 / (f1 * f1 - 0.5 * f * f2)
}

/// Point of maximum chroma (cusp) for a given hue: `(L_cusp, C_cusp)`.
fn find_cusp(a_: f32, b_: f32) -> (f32, f32) {
    let s = compute_max_saturation(a_, b_);
    let rgb = oklab_to_linear_rgb(1.0, s * a_, s * b_);
    let max = rgb[0].max(rgb[1]).max(rgb[2]).max(1e-6);
    let l_cusp = (1.0 / max).cbrt();
    (l_cusp, l_cusp * s)
}

/// Adaptive projection point `L0` on the neutral axis (the `L0_0_5` variant).
///
/// At `α = 0` it is `0.5`; as chroma grows, `L0` shifts toward the neutral:
/// strongly out-of-gamut colors are projected along a shallower ray, which
/// preserves more saturation than keeping `L` fixed.
fn adaptive_l0(l: f32, c: f32) -> f32 {
    const ALPHA: f32 = 0.05;
    let ld = l - 0.5;
    let ad = ld.abs();
    let e1 = 0.5 + ad + ALPHA * c;
    // `e1² − 2·|ld| = (|ld| − 0.5)² + … ≥ 0`, so the root is always real.
    let root = (e1 * e1 - 2.0 * ad).max(0.0).sqrt();
    let sgn = if ld >= 0.0 { 1.0 } else { -1.0 };
    (0.5 * (1.0 + sgn * (e1 - root))).clamp(0.0, 1.0)
}

/// Intersection of the ray `(L0,0) → (L1,C1)` with the sRGB boundary.
///
/// Returns `t ∈ [0,1]`: the point `(L0 + t·(L1−L0), t·C1)` lies on the boundary.
/// The cusp approximates the boundary with the inscribed triangle
/// `(0,0)-(cusp)-(1,0)`, then the boundary is refined by bisection against the
/// exact `in_gamut` predicate (instead of a Halley step — more robust and fast
/// enough while baking the LUT).
fn find_gamut_intersection(
    a_: f32,
    b_: f32,
    l1: f32,
    c1: f32,
    l0: f32,
    cusp: (f32, f32),
) -> f32 {
    let (lc, cc) = cusp;
    let dl = l1 - l0;

    // Intersection with the triangle's two edges.
    let denom_low = c1 * lc - cc * dl;
    let denom_up = c1 * (1.0 - lc) + cc * dl;
    let t_low = if denom_low.abs() > 1e-12 {
        cc * l0 / denom_low
    } else {
        f32::NAN
    };
    let t_up = if denom_up.abs() > 1e-12 {
        cc * (1.0 - l0) / denom_up
    } else {
        f32::NAN
    };

    let valid = |t: f32| t.is_finite() && t > 0.0 && t <= 1.0;
    let t0 = if valid(t_low) && valid(t_up) {
        t_low.min(t_up)
    } else if valid(t_low) {
        t_low
    } else if valid(t_up) {
        t_up
    } else {
        1.0
    };

    // Refine against the true (curved) boundary by bisection.
    let at = |t: f32| oklab_to_linear_rgb(l0 + dl * t, c1 * t * a_, c1 * t * b_);
    let t0 = t0.clamp(0.0, 1.0);
    let (mut lo, mut hi) = if in_gamut(at(t0)) { (t0, 1.0) } else { (0.0, t0) };
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if in_gamut(at(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Gamut-map: preserves hue, adaptively reduces chroma (and nudges `L`) to fit
/// into sRGB.
///
/// Reference: Björn Ottosson, "sRGB gamut clipping" — the `adaptive L0 = 0.5`
/// variant. `L` is clamped to `[0,1]` (SDR output). Unlike simply reducing chroma
/// at constant `L`, the projection onto the boundary runs along the ray toward
/// the neutral `L0`, which preserves more saturation for colors far outside the
/// gamut. For already-valid colors it exits early (nothing changes).
pub(crate) fn gamut_map_linear(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l = l.clamp(0.0, 1.0);
    let rgb = oklab_to_linear_rgb(l, a, b);
    if in_gamut(rgb) {
        return rgb;
    }

    let c = (a * a + b * b).sqrt();
    if c < 1e-8 {
        // Achromatic: `L` is already in [0,1], this is a pure grey.
        return [l, l, l];
    }
    let (a_, b_) = (a / c, b / c);

    let l0 = adaptive_l0(l, c);
    let cusp = find_cusp(a_, b_);
    let t = find_gamut_intersection(a_, b_, l, c, l0, cusp);

    let l_clip = l0 + (l - l0) * t;
    let c_clip = c * t;
    let out = oklab_to_linear_rgb(l_clip, c_clip * a_, c_clip * b_);
    [
        out[0].clamp(0.0, 1.0),
        out[1].clamp(0.0, 1.0),
        out[2].clamp(0.0, 1.0),
    ]
}

/// Color-panel chroma parameters (Temp/Tint are handled by `WhiteBalance`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ColorSettings {
    pub saturation: f32,
    pub vibrance: f32,
}

/// Saturation + Vibrance over an Oklab pixel (order is fixed).
#[inline]
pub(crate) fn apply_chroma(p: &mut [f32; 3], s: &ColorSettings) {
    apply_saturation(p, s.saturation);
    apply_vibrance(p, s.vibrance);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::oklab::rgb_to_oklab;

    fn reference_kelvin_is_identity() {
        let wb = WhiteBalance::new(0.0, 0.0);
        assert!(wb.is_identity());
        let mut rgb = [0.2f32, 0.5, 0.8];
        let orig = rgb;
        wb.apply_linear(&mut rgb);
        for c in 0..3 {
            assert!((rgb[c] - orig[c]).abs() < 1e-6);
        }
    }

    #[test]
    fn kelvin_endpoints() {
        assert!((slider_to_kelvin(0.0) - 6500.0).abs() < 1.0);
        assert!((slider_to_kelvin(100.0) - 15000.0).abs() < 1.0);
        assert!((slider_to_kelvin(-100.0) - 2000.0).abs() < 1.0);
        // Monotonicity: warmer = higher K.
        assert!(slider_to_kelvin(50.0) > slider_to_kelvin(0.0));
        assert!(slider_to_kelvin(-50.0) < slider_to_kelvin(0.0));
    }

    #[test]
    fn cct_to_xy_d65() {
        let [x, y] = cct_to_xy(6500.0);
        assert!((x - 0.3127).abs() < 0.01, "x={x}");
        assert!((y - 0.3290).abs() < 0.01, "y={y}");
    }

    #[test]
    fn warm_shifts_neutral_toward_red_blue() {
        // A neutral grey after warming should become warmer: R > B.
        let mut rgb = [1.0f32, 1.0, 1.0];
        WhiteBalance::new(80.0, 0.0).apply_linear(&mut rgb);
        assert!(rgb[0] > rgb[2], "warm neutral: {rgb:?}");
    }

    #[test]
    fn cool_shifts_neutral_toward_blue() {
        let mut rgb = [1.0f32, 1.0, 1.0];
        WhiteBalance::new(-80.0, 0.0).apply_linear(&mut rgb);
        assert!(rgb[2] > rgb[0], "cool neutral: {rgb:?}");
    }

    #[test]
    fn tint_positive_is_magenta() {
        let mut rgb = [1.0f32, 1.0, 1.0];
        WhiteBalance::new(0.0, 70.0).apply_linear(&mut rgb);
        assert!(rgb[1] < rgb[0] && rgb[1] < rgb[2], "magenta tint: {rgb:?}");
    }

    #[test]
    fn tint_negative_is_green() {
        let mut rgb = [1.0f32, 1.0, 1.0];
        WhiteBalance::new(0.0, -70.0).apply_linear(&mut rgb);
        assert!(rgb[1] > rgb[0] && rgb[1] > rgb[2], "green tint: {rgb:?}");
    }

    #[test]
    fn saturation_identity_and_negative() {
        let mut p = [0.5f32, 0.2, -0.1];
        let orig = p;
        apply_saturation(&mut p, 0.0);
        assert_eq!(p, orig);

        let mut p = [0.5f32, 0.2, -0.1];
        apply_saturation(&mut p, -1.0);
        assert!(p[1].abs() < 1e-6 && p[2].abs() < 1e-6, "grey: {p:?}");
        assert!((p[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn vibrance_protects_saturated() {
        let mut low = [0.5f32, 0.05, 0.0];
        let mut high = [0.5f32, 0.30, 0.0];
        let low_before = (low[1] * low[1] + low[2] * low[2]).sqrt();
        let high_before = (high[1] * high[1] + high[2] * high[2]).sqrt();
        apply_vibrance(&mut low, 1.0);
        apply_vibrance(&mut high, 1.0);
        let low_gain = (low[1] * low[1] + low[2] * low[2]).sqrt() / low_before;
        let high_gain = (high[1] * high[1] + high[2] * high[2]).sqrt() / high_before;
        assert!(low_gain > high_gain, "low {low_gain} vs high {high_gain}");
    }

    #[test]
    fn saturation_compresses_high_chroma() {
        // A saturated colour must gain less than the full `1 + amount`.
        let mut p = [0.5f32, 0.25, 0.0]; // norm ~ 0.78
        let before = (p[1] * p[1] + p[2] * p[2]).sqrt();
        apply_saturation(&mut p, 1.0);
        let gain = (p[1] * p[1] + p[2] * p[2]).sqrt() / before;
        assert!(gain > 1.0 && gain < 2.0, "gain = {gain}");
    }

    #[test]
    fn vibrance_negative_desaturates_saturated_more() {
        // RapidRAW-like: negative vibrance nearly greys saturated colours while
        // leaving near-neutrals almost untouched.
        let mut saturated = [0.5f32, 0.28, 0.0];
        let mut muted = [0.5f32, 0.02, 0.0];
        let sat_before = (saturated[1] * saturated[1] + saturated[2] * saturated[2]).sqrt();
        let muted_before = (muted[1] * muted[1] + muted[2] * muted[2]).sqrt();
        apply_vibrance(&mut saturated, -1.0);
        apply_vibrance(&mut muted, -1.0);
        let sat_gain =
            (saturated[1] * saturated[1] + saturated[2] * saturated[2]).sqrt() / sat_before;
        let muted_gain = (muted[1] * muted[1] + muted[2] * muted[2]).sqrt() / muted_before;
        assert!(
            sat_gain < muted_gain,
            "sat {sat_gain} vs muted {muted_gain}"
        );
        assert!(sat_gain < 0.2, "saturated should grey out: {sat_gain}");
    }

    #[test]
    fn gamut_map_keeps_valid_and_neutral() {
        // A saturated out-of-gamut color must come back into [0,1].
        let rgb = gamut_map_linear(0.6, 0.25, -0.20);
        for c in rgb {
            assert!((0.0..=1.0).contains(&c), "out of gamut: {rgb:?}");
        }
        // A neutral grey stays grey.
        let grey = gamut_map_linear(0.5, 0.0, 0.0);
        assert!((grey[0] - grey[1]).abs() < 1e-4);
        assert!((grey[1] - grey[2]).abs() < 1e-4);
    }

    #[test]
    fn cusp_lies_on_gamut_boundary() {
        // The cusp must lie on the sRGB boundary: max channel ≈ 1, none < 0.
        for i in 0..360 {
            let h = i as f32 * std::f32::consts::TAU / 360.0;
            let (a_, b_) = (h.cos(), h.sin());
            let (lc, cc) = find_cusp(a_, b_);
            let rgb = oklab_to_linear_rgb(lc, cc * a_, cc * b_);
            let max = rgb[0].max(rgb[1]).max(rgb[2]);
            let min = rgb[0].min(rgb[1]).min(rgb[2]);
            assert!((max - 1.0).abs() < 1e-3, "hue {i}: max {max} {rgb:?}");
            assert!(min > -1e-3, "hue {i}: min {min} {rgb:?}");
        }
    }

    #[test]
    fn gamut_map_preserves_hue_and_is_in_gamut() {
        // Sweep hues and chroma levels that are definitely out of gamut.
        for i in 0..180 {
            let h = i as f32 * std::f32::consts::TAU / 180.0;
            for &(l, c) in &[(0.3f32, 0.4f32), (0.5, 0.4), (0.7, 0.4), (0.2, 0.3), (0.9, 0.3)] {
                let (a, b) = (c * h.cos(), c * h.sin());
                let out = gamut_map_linear(l, a, b);
                for v in out {
                    assert!(
                        (0.0..=1.0).contains(&v),
                        "out of gamut: L={l} hue={i} {out:?}"
                    );
                }
                // Hue is preserved: the output direction (a,b) is unchanged.
                let lab = rgb_to_oklab(out[0], out[1], out[2]);
                let c_out = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
                if c_out > 1e-3 {
                    let dot = (lab[1] * a + lab[2] * b) / (c_out * c);
                    assert!(dot > 0.999, "hue drift: L={l} i={i} dot={dot}");
                }
            }
        }
    }
}
