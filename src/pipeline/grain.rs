// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Film grain based on the darktable `src/iop/grain.c` reference.
//!
//! Model:
//! - 2D noise = 3 octaves of 3D simplex with frequencies/amplitudes tuned to
//!   the spectrum of real film grain;
//! - the grain response by lightness is modeled by a "paper" curve
//!   (`paper_resp` / `paper_resp_inverse`) and baked into a 2D LUT;
//! - the grain is monochrome and is added to lightness.
//!
//! Noise is sampled in coordinates normalized to the short side with a fixed
//! `zoom` — so the grain size does not depend on resolution (identical for
//! preview and export). The `strength` parameter is the Grain slider.

use rayon::prelude::*;
use std::sync::OnceLock;

/// Octave frequencies (darktable `_simplex_2d_noise`).
const OCTAVE_F: [f64; 3] = [0.4910, 0.9441, 1.7280];
/// Octave amplitudes.
const OCTAVE_A: [f64; 3] = [0.2340, 0.7850, 1.2150];
/// Grain scale (roughly ISO ~1600 in darktable at a fixed scale).
const GRAIN_ZOOM: f64 = 0.002;
/// Grain strength scale (darktable `GRAIN_LIGHTNESS_STRENGTH_SCALE`).
const LIGHTNESS_STRENGTH_SCALE: f32 = 0.15;
/// Size of the response 2D LUT.
const LUT_SIZE: usize = 128;
const LUT_N: usize = LUT_SIZE * LUT_SIZE;
/// Paper-curve constants (darktable).
const DELTA_MAX: f32 = 2.0;
const DELTA_MIN: f32 = 0.0001;
const PAPER_GAMMA: f32 = 1.0;
const MIDTONES_BIAS: f32 = 100.0;

/// Ken Perlin's classic permutation table (as in darktable).
const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142,
    8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203,
    117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74,
    165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220,
    105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241,
    81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204, 176,
    115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128,
    195, 78, 66, 215, 61, 156, 180,
];

const GRAD3: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

#[inline]
fn p(idx: usize) -> usize {
    PERM[idx & 255] as usize
}

#[inline]
fn grad_dot(gi: usize, x: f64, y: f64, z: f64) -> f64 {
    let g = GRAD3[gi % 12];
    g[0] * x + g[1] * y + g[2] * z
}

/// 3D simplex noise (Gustavson), range roughly [-1, 1].
fn simplex3(xin: f64, yin: f64, zin: f64) -> f64 {
    const F3: f64 = 1.0 / 3.0;
    const G3: f64 = 1.0 / 6.0;
    let s = (xin + yin + zin) * F3;
    let i = (xin + s).floor();
    let j = (yin + s).floor();
    let k = (zin + s).floor();
    let t = (i + j + k) * G3;
    let x0 = xin - (i - t);
    let y0 = yin - (j - t);
    let z0 = zin - (k - t);

    let (i1, j1, k1, i2, j2, k2);
    if x0 >= y0 {
        if y0 >= z0 {
            (i1, j1, k1) = (1, 0, 0);
            (i2, j2, k2) = (1, 1, 0);
        } else if x0 >= z0 {
            (i1, j1, k1) = (1, 0, 0);
            (i2, j2, k2) = (1, 0, 1);
        } else {
            (i1, j1, k1) = (0, 0, 1);
            (i2, j2, k2) = (1, 0, 1);
        }
    } else if y0 < z0 {
        (i1, j1, k1) = (0, 0, 1);
        (i2, j2, k2) = (0, 1, 1);
    } else if x0 < z0 {
        (i1, j1, k1) = (0, 1, 0);
        (i2, j2, k2) = (0, 1, 1);
    } else {
        (i1, j1, k1) = (0, 1, 0);
        (i2, j2, k2) = (1, 1, 0);
    }

    let x1 = x0 - i1 as f64 + G3;
    let y1 = y0 - j1 as f64 + G3;
    let z1 = z0 - k1 as f64 + G3;
    let x2 = x0 - i2 as f64 + 2.0 * G3;
    let y2 = y0 - j2 as f64 + 2.0 * G3;
    let z2 = z0 - k2 as f64 + 2.0 * G3;
    let x3 = x0 - 1.0 + 3.0 * G3;
    let y3 = y0 - 1.0 + 3.0 * G3;
    let z3 = z0 - 1.0 + 3.0 * G3;

    let ii = (i as i64 & 255) as usize;
    let jj = (j as i64 & 255) as usize;
    let kk = (k as i64 & 255) as usize;

    let gi0 = p(ii + p(jj + p(kk))) % 12;
    let gi1 = p(ii + i1 + p(jj + j1 + p(kk + k1))) % 12;
    let gi2 = p(ii + i2 + p(jj + j2 + p(kk + k2))) % 12;
    let gi3 = p(ii + 1 + p(jj + 1 + p(kk + 1))) % 12;

    let mut n = 0.0;
    for (tt, x, y, z, gi) in [
        (0.6 - x0 * x0 - y0 * y0 - z0 * z0, x0, y0, z0, gi0),
        (0.6 - x1 * x1 - y1 * y1 - z1 * z1, x1, y1, z1, gi1),
        (0.6 - x2 * x2 - y2 * y2 - z2 * z2, x2, y2, z2, gi2),
        (0.6 - x3 * x3 - y3 * y3 - z3 * z3, x3, y3, z3, gi3),
    ] {
        if tt > 0.0 {
            let tt = tt * tt;
            n += tt * tt * grad_dot(gi, x, y, z);
        }
    }
    32.0 * n
}

/// 2D noise from 3 simplex octaves (darktable `_simplex_2d_noise`).
#[inline]
fn simplex_2d(x: f64, y: f64, z: f64) -> f64 {
    let mut total = 0.0;
    for o in 0..3 {
        total += simplex3(x * OCTAVE_F[o] / z, y * OCTAVE_F[o] / z, o as f64) * OCTAVE_A[o];
    }
    total
}

/// Generates a `w×h` grain map (values roughly in [-2.2, 2.2]).
///
/// Coordinates are normalized to the short side → the grain size does not depend
/// on resolution. `seed` shifts the field (different grain for different seeds).
pub(crate) fn generate_map(w: usize, h: usize, seed: u64) -> Vec<f32> {
    let wd = w.min(h).max(1) as f64;
    let offset = (seed % 100_000) as f64 * 0.137;
    let mut out = vec![0f32; w * h];
    out.par_iter_mut().enumerate().for_each(|(i, v)| {
        let x = (i % w) as f64 / wd;
        let y = (i / w) as f64 / wd;
        *v = simplex_2d(x + offset, y, GRAIN_ZOOM) as f32;
    });
    out
}

#[inline]
fn paper_resp(exposure: f32, mb: f32, gp: f32) -> f32 {
    let delta = DELTA_MAX * ((mb / 100.0) * DELTA_MIN.ln()).exp();
    (1.0 + 2.0 * delta) / (1.0 + ((4.0 * gp * (0.5 - exposure)) / (1.0 + 2.0 * delta)).exp()) - delta
}

#[inline]
fn paper_resp_inverse(density: f32, mb: f32, gp: f32) -> f32 {
    let delta = DELTA_MAX * ((mb / 100.0) * DELTA_MIN.ln()).exp();
    -(((1.0 + 2.0 * delta) / (density + delta) - 1.0).ln()) * (1.0 + 2.0 * delta) / (4.0 * gp) + 0.5
}

fn build_lut() -> [f32; LUT_N] {
    let mb = MIDTONES_BIAS;
    let gp = PAPER_GAMMA;
    let mut lut = [0f32; LUT_N];
    for j in 0..LUT_SIZE {
        let l = j as f32 / (LUT_SIZE - 1) as f32;
        let inv = paper_resp_inverse(l, mb, gp);
        for i in 0..LUT_SIZE {
            let gu = i as f32 / (LUT_SIZE - 1) as f32 - 0.5;
            lut[j * LUT_SIZE + i] = 100.0 * (paper_resp(gu + inv, mb, gp) - l);
        }
    }
    lut
}

static LUT: OnceLock<[f32; LUT_N]> = OnceLock::new();

fn grain_lut() -> &'static [f32; LUT_N] {
    LUT.get_or_init(build_lut)
}

/// Bilinear sample of a 2D LUT (darktable `dt_lut_lookup_2d_1c`).
fn lookup(lut: &[f32; LUT_N], x: f32, y: f32) -> f32 {
    let sx = ((x + 0.5) * (LUT_SIZE - 1) as f32).clamp(0.0, (LUT_SIZE - 1) as f32);
    let sy = (y * (LUT_SIZE - 1) as f32).clamp(0.0, (LUT_SIZE - 1) as f32);
    let x0 = (sx as usize).min(LUT_SIZE - 2);
    let y0 = (sy as usize).min(LUT_SIZE - 2);
    let dx = sx - x0 as f32;
    let dy = sy - y0 as f32;
    let l00 = lut[y0 * LUT_SIZE + x0];
    let l01 = lut[y0 * LUT_SIZE + x0 + 1];
    let l10 = lut[(y0 + 1) * LUT_SIZE + x0];
    let l11 = lut[(y0 + 1) * LUT_SIZE + x0 + 1];
    let a = l00 * (1.0 - dy) + l10 * dy;
    let b = l01 * (1.0 - dy) + l11 * dy;
    a * (1.0 - dx) + b * dx
}

/// Lightness increment from grain (in `[0,1]` units).
///
/// `noise` comes from [`generate_map`], `strength` is `[0,1]` (slider / 100),
/// `luma` is the current lightness `[0,1]`.
#[inline]
pub(crate) fn grain_delta(noise: f32, strength: f32, luma: f32) -> f32 {
    let x = noise * strength * LIGHTNESS_STRENGTH_SCALE;
    lookup(grain_lut(), x, luma) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplex3_is_bounded_and_deterministic() {
        for i in 0..1000 {
            let x = i as f64 * 0.37;
            let y = i as f64 * 0.11;
            let z = i as f64 * 0.53;
            let v = simplex3(x, y, z);
            assert!(v.abs() <= 1.0, "simplex out of range: {v}");
            assert_eq!(v, simplex3(x, y, z));
        }
    }

    #[test]
    fn paper_resp_monotonic() {
        for &mb in &[0.0f32, 50.0, 100.0] {
            let mut prev = paper_resp(0.0, mb, PAPER_GAMMA);
            for i in 1..=20 {
                let e = i as f32 / 20.0;
                let v = paper_resp(e, mb, PAPER_GAMMA);
                assert!(v >= prev - 1e-6, "not monotonic at e={e}: {v} < {prev}");
                prev = v;
            }
        }
    }

    #[test]
    fn grain_strongest_in_midtones() {
        // For the same noise, the response in midtones is larger than at the extremes.
        let noise = 1.0f32;
        let strength = 1.0f32;
        let mid = grain_delta(noise, strength, 0.5).abs();
        let shadow = grain_delta(noise, strength, 0.05).abs();
        let high = grain_delta(noise, strength, 0.95).abs();
        assert!(mid > shadow, "mid {mid} vs shadow {shadow}");
        assert!(mid > high, "mid {mid} vs highlight {high}");
    }

    #[test]
    fn zero_strength_no_grain() {
        assert!(grain_delta(1.5, 0.0, 0.5).abs() < 1e-6);
    }

    #[test]
    fn generate_map_shape_and_range() {
        let m = generate_map(32, 16, 42);
        assert_eq!(m.len(), 32 * 16);
        for &v in &m {
            assert!(v.abs() < 4.0, "noise too large: {v}");
        }
    }
}

#[cfg(test)]
mod bench_grain {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn bench_grain_map_1200() {
        let t = Instant::now();
        let m = generate_map(1200, 800, 42);
        println!(
            "grain map 1200x800: {:.1} ms ({} samples)",
            t.elapsed().as_secs_f64() * 1000.0,
            m.len()
        );
        std::hint::black_box(m);
    }
}
