// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Oklab and sRGB transfers.
//!
//! Oklab is Björn Ottosson's perceptually uniform color space (2020):
//! <https://bottosson.github.io/posts/oklab/>
//!
//! Dependency-free math only:
//! - exact sRGB transfers (`srgb_to_linear` / `linear_to_srgb`) instead of the
//!   `x*x` / `sqrt(x)` approximation used by the old Light path;
//! - linear sRGB ↔ Oklab.
//!
//! Every operation is a pure function of a single pixel, so the whole
//! Light/Color pipeline stays bakeable into a 3D LUT.

/// sRGB → linear sRGB (EOTF, piecewise-linear toe).
#[inline]
pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear sRGB → sRGB (OETF).
#[inline]
pub(crate) fn linear_to_srgb(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear sRGB → sRGB, straight into `u8` (0..=255) with clamping.
#[allow(dead_code)] // used in tests/diagnostics
#[inline]
pub(crate) fn linear_to_srgb_u8(c: f32) -> u8 {
    (linear_to_srgb(c).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// `u8` sRGB → linear, a 256-entry table (keeps `powf` out of the loop).
pub(crate) fn srgb_u8_linear_table() -> [f32; 256] {
    let mut t = [0.0f32; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = srgb_to_linear(i as f32 / 255.0);
    }
    t
}

/// Linear RGB → Oklab (Ottosson's matrices for linear sRGB / D65).
#[inline]
pub(crate) fn rgb_to_oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();

    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Oklab → linear RGB.
#[inline]
pub(crate) fn oklab_to_linear_rgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn srgb_transfer_roundtrip() {
        for i in 0..=1000 {
            let x = i as f32 / 1000.0;
            let back = linear_to_srgb(srgb_to_linear(x));
            assert!((back - x).abs() < 1e-5, "roundtrip {x} -> {back}");
        }
    }

    #[test]
    fn srgb_transfer_known_points() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < EPS);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < EPS);
        // sRGB 0.5 ≈ linear 0.2140
        assert!((srgb_to_linear(0.5) - 0.21404114).abs() < 1e-4);
        // u8 128 ≈ 0.2158
        assert!((linear_to_srgb_u8(0.2158) as i32 - 128).abs() <= 1);
    }

    #[test]
    fn neutral_grey_is_l_cbrt_and_zero_chroma() {
        for &v in &[0.01f32, 0.05, 0.1842, 0.5, 0.9, 1.0] {
            let [l, a, b] = rgb_to_oklab(v, v, v);
            assert!((l - v.cbrt()).abs() < 1e-4, "L for grey {v}: {l}");
            assert!(a.abs() < 1e-5, "a for grey {v}: {a}");
            assert!(b.abs() < 1e-5, "b for grey {v}: {b}");
        }
    }

    #[test]
    fn middle_grey_matches_contrast_pivot() {
        // 18.42% linear grey → Oklab L ≈ 0.5686 = the contrast pivot.
        let [l, _, _] = rgb_to_oklab(0.1842, 0.1842, 0.1842);
        assert!((l - 0.5686).abs() < 1e-3, "pivot mismatch: {l}");
    }

    #[test]
    fn oklab_roundtrip() {
        let samples = [
            [0.0f32, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.18, 0.42, 0.77],
            [0.9, 0.1, 0.35],
        ];
        for s in samples {
            let lab = rgb_to_oklab(s[0], s[1], s[2]);
            let rgb = oklab_to_linear_rgb(lab[0], lab[1], lab[2]);
            for c in 0..3 {
                assert!(
                    (rgb[c] - s[c]).abs() < 1e-4,
                    "roundtrip {s:?} channel {c}: {} vs {}",
                    rgb[c],
                    s[c]
                );
            }
        }
    }

    #[test]
    fn primaries_have_expected_chroma_signs() {
        let red = rgb_to_oklab(1.0, 0.0, 0.0);
        let green = rgb_to_oklab(0.0, 1.0, 0.0);
        let blue = rgb_to_oklab(0.0, 0.0, 1.0);
        // Red: +a (magenta/red), +b (yellow).
        assert!(red[1] > 0.0 && red[2] > 0.0, "red {red:?}");
        // Green: -a.
        assert!(green[1] < 0.0, "green {green:?}");
        // Blue: -b.
        assert!(blue[2] < 0.0, "blue {blue:?}");
        // Lightness: green > red > blue.
        assert!(green[0] > red[0] && red[0] > blue[0]);
    }

    #[test]
    fn white_is_l_one() {
        let [l, a, b] = rgb_to_oklab(1.0, 1.0, 1.0);
        assert!((l - 1.0).abs() < 1e-4);
        assert!(a.abs() < 1e-5 && b.abs() < 1e-5);
    }
}
