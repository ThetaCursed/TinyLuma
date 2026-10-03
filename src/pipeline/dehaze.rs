// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Dehaze: Dark Channel Prior.
//!
//! Reference: He, Sun, Tang, "Single Image Haze Removal Using Dark Channel
//! Prior" (CVPR 2009 / TPAMI 2011). Full original pipeline:
//! dark channel → atmospheric light → transmission → guided filter → recover.
//!
//! The haze map is computed at a **working resolution** (no more than `WORK_MAX`
//! on the long side) and upsampled to the full frame. This:
//! - makes preview and export look the same (radii are in working-grid pixels);
//! - bounds memory and time on large exports.
#![allow(dead_code)]

use rayon::prelude::*;
use std::collections::VecDeque;

/// Maximum long side of the haze working map.
///
/// Haze maps are low-frequency, so we compute them at reduced resolution and
/// upsample: ~4× faster and visually almost indistinguishable. The value is the
/// same for preview and export, so the look matches.
const WORK_MAX: usize = 640;
/// Working resolution of the haze map in fast (drag) mode. Smaller is faster; the
/// map is low-frequency, so the difference is subtle, and full quality returns on
/// release. 480 is a compromise: 320 would save ~3 ms more but look noticeably
/// coarser.
const FAST_WORK_MAX: usize = 480;
/// Radius of the min filter (window 2r+1) for the dark channel and transmission.
const DCP_RADIUS: usize = 7;
/// Guided filter radius.
const GUIDED_RADIUS: usize = 30;
/// Guided filter regularization.
const GUIDED_EPS: f32 = 1e-3;
/// Fraction of haze to subtract (0.95 from the paper).
const OMEGA: f32 = 0.95;
/// Lower bound on transmittance.
const T0: f32 = 0.1;
/// Strength of atmospheric-light `A` neutralization.
///
/// `0.0` — as in the paper (per-channel `A`): if `A` is colored (e.g. blue sky),
/// recovery produces a color cast — yellow/orange.
/// `1.0` — `A` is brought to its luminance (cast removed). Intermediate values
/// trade color-haze accuracy against the absence of a cast.
const ATMOSPHERIC_NEUTRALIZE: f32 = 1.0;
/// "Fog" strength (negative dehaze).
const FOG_STRENGTH: f32 = 0.5;
/// Airlight-perspective color for fog.
const HAZE: [f32; 3] = [0.80, 0.82, 0.85];

/// Applies dehaze (`amount > 0`) or fog (`amount < 0`) to an RGB buffer.
///
/// `amount` is in [-1, 1] (slider / 100). `out` and `input` have the same length
/// `width * height * 3`.
pub(crate) fn apply(input: &[u8], out: &mut [u8], width: usize, height: usize, amount: f32) {
    apply_with(input, out, width, height, amount, false);
}

/// Fast variant for interactive drag: the haze map is computed at a smaller
/// working resolution and without the guided filter. Full quality comes on
/// release (`apply`). The haze map is low-frequency, so the visual difference is
/// small.
pub(crate) fn apply_fast(input: &[u8], out: &mut [u8], width: usize, height: usize, amount: f32) {
    apply_with(input, out, width, height, amount, true);
}

fn apply_with(
    input: &[u8],
    out: &mut [u8],
    width: usize,
    height: usize,
    amount: f32,
    fast: bool,
) {
    if amount.abs() < 1e-6 {
        out.copy_from_slice(input);
        return;
    }
    if amount < 0.0 {
        apply_fog(input, out, amount);
        return;
    }
    let work_max = if fast { FAST_WORK_MAX } else { WORK_MAX };
    apply_dcp(input, out, width, height, amount, work_max);
}

fn apply_fog(input: &[u8], out: &mut [u8], amount: f32) {
    let f = (-amount).min(1.0) * FOG_STRENGTH;
    out.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        let base = i * 3;
        for c in 0..3 {
            let v = input[base + c] as f32 / 255.0;
            let m = v * (1.0 - f) + HAZE[c] * f;
            px[c] = (m.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
}

fn apply_dcp(
    input: &[u8],
    out: &mut [u8],
    width: usize,
    height: usize,
    amount: f32,
    work_max: usize,
) {
    let (sw, sh) = work_size_with(width, height, work_max);
    let work = downscale(input, width, height, sw, sh);
    let n = sw * sh;

    // 1. Dark channel.
    let mut dark = vec![0f32; n];
    dark.par_iter_mut().enumerate().for_each(|(i, d)| {
        *d = work[i * 3].min(work[i * 3 + 1]).min(work[i * 3 + 2]);
    });
    let dc = min_filter(&dark, sw, sh, DCP_RADIUS);

    // 2. Atmospheric light.
    //    Neutralize the color of `A`: otherwise a per-channel `A` (e.g. blue sky)
    //    produces a yellow cast during recovery.
    let a = neutralize_atmospheric_light(
        atmospheric_light(&work, &dc, sw, sh),
        ATMOSPHERIC_NEUTRALIZE,
    );

    // 3. Transmission.
    let mut tmap = vec![0f32; n];
    tmap.par_iter_mut().enumerate().for_each(|(i, t)| {
        let r = work[i * 3] / a[0].max(1e-3);
        let g = work[i * 3 + 1] / a[1].max(1e-3);
        let b = work[i * 3 + 2] / a[2].max(1e-3);
        *t = 1.0 - OMEGA * r.min(g).min(b);
    });
    let tmap = min_filter(&tmap, sw, sh, DCP_RADIUS);

    // 4. Guided filter (guide = luma). Keep it in fast mode too: it is what
    //    makes the map edge-aware and removes halos. Fast differs only in the
    //    smaller working resolution of the map.
    let guide: Vec<f32> = (0..n)
        .map(|i| 0.2126 * work[i * 3] + 0.7152 * work[i * 3 + 1] + 0.0722 * work[i * 3 + 2])
        .collect();
    let t_ref = guided_filter(&guide, &tmap, sw, sh, GUIDED_RADIUS, GUIDED_EPS);

    // 5. Recover at full resolution.
    out.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        let x = i % width;
        let y = i / width;
        let t = sample_bilinear(&t_ref, sw, sh, x, y, width, height).max(T0);
        for c in 0..3 {
            let v = input[i * 3 + c] as f32 / 255.0;
            let j = (v - a[c]) / t + a[c];
            let m = v + amount * (j - v);
            px[c] = (m.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    });
}

fn work_size(width: usize, height: usize) -> (usize, usize) {
    work_size_with(width, height, WORK_MAX)
}

fn work_size_with(width: usize, height: usize, max_dim_cap: usize) -> (usize, usize) {
    let max_dim = width.max(height);
    if max_dim <= max_dim_cap {
        (width.max(1), height.max(1))
    } else {
        let scale = max_dim_cap as f32 / max_dim as f32;
        (
            ((width as f32 * scale).round() as usize).max(1),
            ((height as f32 * scale).round() as usize).max(1),
        )
    }
}

/// Box-downscale into f32 [0,1], interleaved RGB.
fn downscale(input: &[u8], w: usize, h: usize, sw: usize, sh: usize) -> Vec<f32> {
    let mut out = vec![0f32; sw * sh * 3];
    if sw == w && sh == h {
        out.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
            px[0] = input[i * 3] as f32 / 255.0;
            px[1] = input[i * 3 + 1] as f32 / 255.0;
            px[2] = input[i * 3 + 2] as f32 / 255.0;
        });
        return out;
    }
    let sx = w as f32 / sw as f32;
    let sy = h as f32 / sh as f32;
    out.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        let dx = i % sw;
        let dy = i / sw;
        let x0 = (dx as f32 * sx) as usize;
        let x1 = (((dx + 1) as f32 * sx) as usize).clamp(x0 + 1, w);
        let y0 = (dy as f32 * sy) as usize;
        let y1 = (((dy + 1) as f32 * sy) as usize).clamp(y0 + 1, h);
        let mut acc = [0f32; 3];
        let mut cnt = 0f32;
        for y in y0..y1 {
            let row = y * w;
            for x in x0..x1 {
                let b = (row + x) * 3;
                acc[0] += input[b] as f32;
                acc[1] += input[b + 1] as f32;
                acc[2] += input[b + 2] as f32;
                cnt += 1.0;
            }
        }
        px[0] = acc[0] / cnt / 255.0;
        px[1] = acc[1] / cnt / 255.0;
        px[2] = acc[2] / cnt / 255.0;
    });
    out
}

/// Separable min filter, O(1) per pixel (monotonic deque).
fn min_filter(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; w * h];
    min_filter_h(src, &mut tmp, w, h, r);
    // Vertical pass via transpose: horizontal over rows = parallel.
    let mut tmp_t = vec![0f32; w * h];
    transpose(&tmp, &mut tmp_t, w, h);
    let mut out_t = vec![0f32; w * h];
    min_filter_h(&tmp_t, &mut out_t, h, w, r);
    let mut out = vec![0f32; w * h];
    transpose(&out_t, &mut out, h, w);
    out
}

/// `dst` is the transpose of `src` (w×h → w rows of h).
fn transpose(src: &[f32], dst: &mut [f32], w: usize, h: usize) {
    dst.par_chunks_mut(h).enumerate().for_each(|(x, row)| {
        for (y, v) in row.iter_mut().enumerate() {
            *v = src[y * w + x];
        }
    });
}

/// Horizontal min filter (window `2r+1`), parallel over rows.
fn min_filter_h(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    if w == 0 || h == 0 {
        return;
    }
    let r = r.min(w.saturating_sub(1));
    dst.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let base = y * w;
        let mut dq: VecDeque<usize> = VecDeque::with_capacity(2 * r + 2);
        let mut added: isize = -1;
        for x in 0..w {
            let right = (x + r).min(w - 1) as isize;
            while added < right {
                added += 1;
                let i = added as usize;
                while let Some(&b) = dq.back() {
                    if src[base + b] >= src[base + i] {
                        dq.pop_back();
                    } else {
                        break;
                    }
                }
                dq.push_back(i);
            }
            let left = x.saturating_sub(r);
            while let Some(&f) = dq.front() {
                if f < left {
                    dq.pop_front();
                } else {
                    break;
                }
            }
            row[x] = src[base + *dq.front().unwrap()];
        }
    });
}

/// Blends the atmospheric-light color toward its luminance.
///
/// At `strength = 1` a colored `A` becomes neutral (luminance preserved); at `0`
/// it stays as in the paper.
fn neutralize_atmospheric_light(mut a: [f32; 3], strength: f32) -> [f32; 3] {
    let lum = 0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2];
    for c in &mut a {
        *c += strength * (lum - *c);
    }
    a
}

/// Mean of the brightest 0.1% of pixels by dark channel.
fn atmospheric_light(work: &[f32], dc: &[f32], w: usize, h: usize) -> [f32; 3] {
    let n = w * h;
    let mut hist = [0u32; 256];
    for &d in dc {
        hist[(d.clamp(0.0, 1.0) * 255.0) as usize] += 1;
    }
    let target = ((n as f32 * 0.001).ceil() as u32).max(1);
    let mut acc = 0u32;
    let mut thresh = 0usize;
    for i in (0..256).rev() {
        acc += hist[i];
        if acc >= target {
            thresh = i;
            break;
        }
    }
    let mut a = [0f32; 3];
    let mut cnt = 0u32;
    for i in 0..n {
        if (dc[i].clamp(0.0, 1.0) * 255.0) as usize >= thresh {
            a[0] += work[i * 3];
            a[1] += work[i * 3 + 1];
            a[2] += work[i * 3 + 2];
            cnt += 1;
        }
    }
    if cnt == 0 {
        return [1.0, 1.0, 1.0];
    }
    [a[0] / cnt as f32, a[1] / cnt as f32, a[2] / cnt as f32]
}

/// Guided filter (He, Sun, Tang 2013) with a box filter via the integral image.
fn guided_filter(guide: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let n = w * h;
    let mean_i = box_filter(guide, w, h, r);
    let mean_p = box_filter(p, w, h, r);
    let ip: Vec<f32> = (0..n).map(|k| guide[k] * p[k]).collect();
    let ii: Vec<f32> = guide.iter().map(|v| v * v).collect();
    let mean_ip = box_filter(&ip, w, h, r);
    let mean_ii = box_filter(&ii, w, h, r);

    let mut a = vec![0f32; n];
    let mut b = vec![0f32; n];
    for k in 0..n {
        let var = mean_ii[k] - mean_i[k] * mean_i[k];
        let cov = mean_ip[k] - mean_i[k] * mean_p[k];
        a[k] = cov / (var + eps);
        b[k] = mean_p[k] - a[k] * mean_i[k];
    }
    let mean_a = box_filter(&a, w, h, r);
    let mean_b = box_filter(&b, w, h, r);
    (0..n).map(|k| mean_a[k] * guide[k] + mean_b[k]).collect()
}

/// Box-mean with clamped edges, via an integral image (f32).
fn box_filter(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let iw = w + 1;
    let mut integ = vec![0f32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0f32;
        for x in 0..w {
            row += src[y * w + x];
            integ[(y + 1) * iw + x + 1] = integ[y * iw + x + 1] + row;
        }
    }
    let mut out = vec![0f32; w * h];
    out.par_iter_mut().enumerate().for_each(|(k, o)| {
        let x = k % w;
        let y = k / w;
        let x0 = x.saturating_sub(r);
        let x1 = (x + r).min(w - 1);
        let y0 = y.saturating_sub(r);
        let y1 = (y + r).min(h - 1);
        let area = ((x1 - x0 + 1) * (y1 - y0 + 1)) as f32;
        let s = integ[(y1 + 1) * iw + x1 + 1] - integ[y0 * iw + x1 + 1]
            - integ[(y1 + 1) * iw + x0]
            + integ[y0 * iw + x0];
        *o = s / area;
    });
    out
}

/// Bilinear sample of a single-channel `sw×sh` map at a full-frame coordinate.
fn sample_bilinear(map: &[f32], sw: usize, sh: usize, x: usize, y: usize, w: usize, h: usize) -> f32 {
    let fx = (x as f32 + 0.5) * sw as f32 / w as f32 - 0.5;
    let fy = (y as f32 + 0.5) * sh as f32 / h as f32 - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let xi = |v: f32| (v as isize).clamp(0, sw as isize - 1) as usize;
    let yi = |v: f32| (v as isize).clamp(0, sh as isize - 1) as usize;
    let (xa, xb) = (xi(x0), xi(x0 + 1.0));
    let (ya, yb) = (yi(y0), yi(y0 + 1.0));
    let v00 = map[ya * sw + xa];
    let v10 = map[ya * sw + xb];
    let v01 = map[yb * sw + xa];
    let v11 = map[yb * sw + xb];
    let top = v00 * (1.0 - tx) + v10 * tx;
    let bot = v01 * (1.0 - tx) + v11 * tx;
    top * (1.0 - ty) + bot * ty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_amount_is_identity() {
        let input: Vec<u8> = (0..30).map(|i| (i * 7 % 256) as u8).collect();
        let mut out = vec![0u8; input.len()];
        apply(&input, &mut out, 5, 2, 0.0);
        assert_eq!(input, out);
    }

    #[test]
    fn min_filter_matches_naive() {
        let (w, h, r) = (17usize, 11usize, 3usize);
        let src: Vec<f32> = (0..w * h)
            .map(|i| ((i * 2654435761usize) % 1000) as f32 / 1000.0)
            .collect();
        let fast = min_filter(&src, w, h, r);
        for y in 0..h {
            for x in 0..w {
                let mut m = f32::MAX;
                for dy in -(r as isize)..=(r as isize) {
                    for dx in -(r as isize)..=(r as isize) {
                        let nx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
                        let ny = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                        m = m.min(src[ny * w + nx]);
                    }
                }
                assert!(
                    (fast[y * w + x] - m).abs() < 1e-6,
                    "at ({x},{y}): {} vs {m}",
                    fast[y * w + x]
                );
            }
        }
    }

    #[test]
    fn neutral_constant_image_unchanged_under_dcp() {
        // A flat NEUTRAL frame is the degenerate case: dehaze changes nothing.
        // A colored flat frame is now neutralized together with `A` — see
        // `neutral_scene_stays_neutral_under_colored_haze`.
        let (w, h) = (24usize, 16usize);
        let input: Vec<u8> = (0..w * h).flat_map(|_| [100u8, 100, 100]).collect();
        let mut out = vec![0u8; input.len()];
        apply(&input, &mut out, w, h, 1.0);
        for (a, b) in input.iter().zip(out.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "changed {a} -> {b}");
        }
    }

    #[test]
    fn atmospheric_neutralization_preserves_luminance() {
        let a = [0.667f32, 0.745, 0.863];
        let keep = neutralize_atmospheric_light(a, 0.0);
        for c in 0..3 {
            assert!((keep[c] - a[c]).abs() < 1e-9, "strength 0 changed A");
        }
        let n = neutralize_atmospheric_light(a, 1.0);
        assert!((n[0] - n[1]).abs() < 1e-6 && (n[1] - n[2]).abs() < 1e-6);
        let lum = |v: [f32; 3]| 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        assert!((lum(a) - lum(n)).abs() < 1e-6, "luminance not preserved");
    }

    #[test]
    fn neutral_scene_stays_neutral_under_colored_haze() {
        // A blue "sky" sets a colored `A`; a grey subject under haze must not
        // turn orange/yellow after dehaze (regression guard for the cast).
        let (w, h) = (64usize, 64usize);
        let mut input = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let b = (y * w + x) * 3;
                let (r, g, bl) = if y < h / 3 {
                    (170u8, 190, 220)
                } else {
                    (150, 150, 150)
                };
                input[b] = r;
                input[b + 1] = g;
                input[b + 2] = bl;
            }
        }
        let mut out = vec![0u8; input.len()];
        apply(&input, &mut out, w, h, 1.0);
        let mid = ((h * 2 / 3) * w + w / 2) * 3;
        let (r, g, b) = (out[mid] as i32, out[mid + 1] as i32, out[mid + 2] as i32);
        assert!(
            (r - g).abs() <= 10 && (g - b).abs() <= 10,
            "color cast remained: {:?}",
            &out[mid..mid + 3]
        );
    }

    #[test]
    fn fog_brightens_dark() {
        let (w, h) = (8usize, 8usize);
        let input = vec![10u8; w * h * 3];
        let mut out = vec![0u8; input.len()];
        apply(&input, &mut out, w, h, -1.0);
        assert!(out[0] > 10, "fog should lighten: {}", out[0]);
    }

    #[test]
    fn work_size_bounds_long_side() {
        assert_eq!(work_size(640, 480), (640, 480));
        assert_eq!(work_size(1200, 800), (640, 427));
        assert_eq!(work_size(6000, 4000), (640, 427));
        assert_eq!(work_size(4000, 6000), (427, 640));
    }

    #[test]
    fn dark_object_darkens_under_haze() {
        let (w, h) = (32usize, 32usize);
        let mut input = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let b = (y * w + x) * 3;
                if (8..24).contains(&x) && (8..24).contains(&y) {
                    input[b] = 50;
                    input[b + 1] = 60;
                    input[b + 2] = 70;
                } else {
                    input[b] = 200;
                    input[b + 1] = 205;
                    input[b + 2] = 210;
                }
            }
        }
        let mut out = vec![0u8; input.len()];
        apply(&input, &mut out, w, h, 1.0);
        let center = (16 * w + 16) * 3;
        assert!(out[center] < 50, "dark object should darken: {}", out[center]);
    }

    #[test]
    fn fast_mode_stays_close_to_full() {
        // Fast mode (drag) uses a smaller working resolution for the haze map
        // (the guided filter is kept). We check that the divergence from the
        // full variant stays small — otherwise release would show a visible "jump".
        let (w, h) = (1200usize, 800usize);
        let mut input = vec![0u8; w * h * 3];
        let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.45);
        for y in 0..h {
            for x in 0..w {
                let b = (y * w + x) * 3;
                // Light "haze" + a soft dark object in the center (realistic).
                let dx = (x as f32 - cx) / (w as f32 * 0.22);
                let dy = (y as f32 - cy) / (h as f32 * 0.28);
                let r = (dx * dx + dy * dy).sqrt();
                let mut obj = (1.0 - r).clamp(0.0, 1.0);
                obj = obj * obj * (3.0 - 2.0 * obj);
                let v = (200.0 - 130.0 * obj).clamp(0.0, 255.0) as u8;
                input[b] = v;
                input[b + 1] = v.saturating_add(5);
                input[b + 2] = v.saturating_add(10);
            }
        }
        let mut full = vec![0u8; input.len()];
        let mut fast = vec![0u8; input.len()];
        apply(&input, &mut full, w, h, 0.7);
        apply_fast(&input, &mut fast, w, h, 0.7);

        let mut max = 0i32;
        let mut sum = 0f64;
        for (a, b) in full.iter().zip(fast.iter()) {
            let d = (*a as i32 - *b as i32).abs();
            max = max.max(d);
            sum += d as f64;
        }
        let mean = sum / full.len() as f64;
        // Thresholds are a coarse regression guard: fast mode is for drag, not
        // a reference.
        assert!(max <= 30, "fast diverge: max={max} mean={mean:.2}");
        assert!(mean <= 4.0, "fast diverge: max={max} mean={mean:.2}");
    }

    #[test]
    #[ignore]
    fn tune_fast_mode() {
        use std::time::Instant;
        let (w, h) = (1200usize, 800usize);
        let mut input = vec![0u8; w * h * 3];
        let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.45);
        for y in 0..h {
            for x in 0..w {
                let b = (y * w + x) * 3;
                // Light haze + a soft dark object in the center.
                let dx = (x as f32 - cx) / (w as f32 * 0.22);
                let dy = (y as f32 - cy) / (h as f32 * 0.28);
                let r = (dx * dx + dy * dy).sqrt();
                let mut obj = (1.0 - r).clamp(0.0, 1.0);
                obj = obj * obj * (3.0 - 2.0 * obj);
                let grad = 12.0 * (y as f32 / h as f32);
                let v = (200.0 - 130.0 * obj + grad).clamp(0.0, 255.0) as u8;
                input[b] = v;
                input[b + 1] = v.saturating_add(6);
                input[b + 2] = v.saturating_add(12);
            }
        }
        let mut full = vec![0u8; input.len()];
        apply(&input, &mut full, w, h, 0.7);

        println!("\n=== dehaze fast mode: tuning work_max (reference — 640) ===");
        println!("{:>8} {:>8} {:>8} {:>8}", "work", "ms", "max|Δ|", "mean|Δ|");
        for wm in [640usize, 480, 320, 240, 160] {
            let mut out = vec![0u8; input.len()];
            let t = Instant::now();
            apply_dcp(&input, &mut out, w, h, 0.7, wm);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let mut max = 0i32;
            let mut sum = 0f64;
            for (a, b) in out.iter().zip(full.iter()) {
                let d = (*a as i32 - *b as i32).abs();
                max = max.max(d);
                sum += d as f64;
            }
            println!(
                "{:>8} {:>8.1} {:>8} {:>8.2}",
                wm,
                ms,
                max,
                sum / out.len() as f64
            );
        }
    }

    #[test]
    #[ignore]
    fn bench_dehaze_1200() {
        use std::time::Instant;
        let (w, h) = (1200usize, 800usize);
        let mut input = vec![128u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let b = (y * w + x) * 3;
                let v = ((x * 7 + y * 13) % 200) as u8;
                input[b] = v;
                input[b + 1] = v.saturating_add(10);
                input[b + 2] = v.saturating_add(20);
            }
        }
        let mut out = vec![0u8; input.len()];
        let t = Instant::now();
        apply(&input, &mut out, w, h, 1.0);
        println!(
            "dehaze 1200x800 (rayon): {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
}
