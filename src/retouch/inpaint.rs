// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Fill algorithms.
//!
//! `best_offset` implements the automatic healing source of the Spot Healing
//! Brush: find the displacement whose surrounding *ring* matches the ring around
//! the hole best (sum of squared differences). The matching patch is then
//! cloned through the Poisson solver in `poisson.rs`.
//!
//! `complete` is the content-aware fill (v2): it reconstructs the hole from the
//! surrounding texture with a coarse-to-fine PatchMatch nearest-neighbour field
//! and an overlap vote, the classical non-neural inpainting family used by
//! tools such as Photoshop's Content-Aware Fill and Photopea's Spot Healing.

use super::poisson::membrane_fill;

/// Dilates a binary mask by `iters` steps (4-neighbourhood).
pub(crate) fn dilate(mask: &[bool], w: usize, h: usize, iters: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for _ in 0..iters {
        let src = out.clone();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if src[i] {
                    continue;
                }
                let up = y > 0 && src[i - w];
                let down = y + 1 < h && src[i + w];
                let left = x > 0 && src[i - 1];
                let right = x + 1 < w && src[i + 1];
                if up || down || left || right {
                    out[i] = true;
                }
            }
        }
    }
    out
}

/// Best matching source displacement for a hole.
///
/// * `ring` — half-width of the band around the hole that is compared;
/// * `max_radius` — search window (the displacement is clamped to it).
///
/// Returns `None` when the hole covers everything (or is empty) and no valid
/// source exists.
pub(crate) fn best_offset(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    ring: usize,
    max_radius: i32,
) -> Option<(i32, i32)> {
    if w == 0 || h == 0 || max_radius < 1 {
        return None;
    }
    let (bx0, by0, bx1, by1) = hole_bbox(hole, w, h)?;

    // Integral image over the hole mask for O(1) "does this rectangle contain a
    // hole pixel" checks.
    let stride = w + 1;
    let mut ii = vec![0u32; stride * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += hole[y * w + x] as u32;
            ii[(y + 1) * stride + (x + 1)] = ii[y * stride + (x + 1)] + row;
        }
    }
    let count = |x0: i32, y0: i32, x1: i32, y1: i32| -> u32 {
        let x0 = x0.max(0) as usize;
        let y0 = y0.max(0) as usize;
        let x1 = x1.max(0) as usize;
        let y1 = y1.max(0) as usize;
        let a = ii[y1 * stride + x1];
        let b = ii[y0 * stride + x1];
        let c = ii[y1 * stride + x0];
        let d = ii[y0 * stride + x0];
        a + d - b - c
    };

    let valid = |dx: i32, dy: i32| -> bool {
        let sx0 = bx0 + dx;
        let sy0 = by0 + dy;
        let sx1 = bx1 + dx;
        let sy1 = by1 + dy;
        if sx0 < 0 || sy0 < 0 || sx1 > w as i32 || sy1 > h as i32 {
            return false;
        }
        // The whole source rectangle must be free of hole pixels.
        count(sx0, sy0, sx1, sy1) == 0
    };

    // Ring = known pixels within `ring` of the hole.
    let dil = dilate(hole, w, h, ring);
    let mut ring_pixels: Vec<usize> = (0..w * h)
        .filter(|&i| dil[i] && !hole[i])
        .collect();
    if ring_pixels.is_empty() {
        return None;
    }
    // Cap the number of samples so the search stays cheap on big spots.
    const MAX_SAMPLES: usize = 1500;
    if ring_pixels.len() > MAX_SAMPLES {
        let step = ring_pixels.len().div_ceil(MAX_SAMPLES);
        ring_pixels = ring_pixels.into_iter().step_by(step).collect();
    }

    let score = |dx: i32, dy: i32| -> f64 {
        let mut ssd = 0.0f64;
        for &i in &ring_pixels {
            let x = (i % w) as i32;
            let y = (i / w) as i32;
            let sx = x + dx;
            let sy = y + dy;
            if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 {
                continue;
            }
            let j = sy as usize * w + sx as usize;
            let bi = i * ch;
            let bj = j * ch;
            for c in 0..ch {
                let d = (img[bi + c] - img[bj + c]) as f64;
                ssd += d * d;
            }
        }
        // Tiny tie-breaker: prefer the nearer source.
        ssd + (dx * dx + dy * dy) as f64 * 1.0e-6
    };

    let coarse = (max_radius / 8).max(1);
    let mut best: Option<(f64, i32, i32)> = None;

    let mut dy = -max_radius;
    while dy <= max_radius {
        let mut dx = -max_radius;
        while dx <= max_radius {
            if (dx != 0 || dy != 0) && valid(dx, dy) {
                let s = score(dx, dy);
                if best.map_or(true, |(bs, _, _)| s < bs) {
                    best = Some((s, dx, dy));
                }
            }
            dx += coarse;
        }
        dy += coarse;
    }

    // Full-resolution refinement around the coarse winner.
    if let Some((_, bdx, bdy)) = best {
        let x0 = (bdx - coarse).max(-max_radius);
        let x1 = (bdx + coarse).min(max_radius);
        let y0 = (bdy - coarse).max(-max_radius);
        let y1 = (bdy + coarse).min(max_radius);
        for ry in y0..=y1 {
            for rx in x0..=x1 {
                if (rx == 0 && ry == 0) || !valid(rx, ry) {
                    continue;
                }
                let s = score(rx, ry);
                if best.map_or(true, |(bs, _, _)| s < bs) {
                    best = Some((s, rx, ry));
                }
            }
        }
    }

    best.map(|(_, dx, dy)| (dx, dy))
}

/// Inclusive min / exclusive max bounding box of the hole pixels.
fn hole_bbox(hole: &[bool], w: usize, h: usize) -> Option<(i32, i32, i32, i32)> {
    let mut x0 = i32::MAX;
    let mut y0 = i32::MAX;
    let mut x1 = -1i32;
    let mut y1 = -1i32;
    for y in 0..h {
        for x in 0..w {
            if hole[y * w + x] {
                x0 = x0.min(x as i32);
                y0 = y0.min(y as i32);
                x1 = x1.max(x as i32);
                y1 = y1.max(y as i32);
            }
        }
    }
    if x1 < 0 {
        None
    } else {
        Some((x0, y0, x1 + 1, y1 + 1))
    }
}

// ---------------------------------------------------------------------------
// Content-aware completion (v2)
// ---------------------------------------------------------------------------
//
// PatchMatch-based completion: the hole is filled by voting the best-matching
// known patches over it, on a coarse-to-fine pyramid so both large structure and
// fine texture are continued. This is the classical, non-neural content-aware
// fill behind tools like Photopea's Spot Healing Brush.
//
// References:
// * Y. Wexler, E. Shechtman, M. Irani, *Space-Time Completion of Video*,
//   IEEE TPAMI 29(3):463-476, 2007 (completion by patch voting).
// * C. Barnes, E. Shechtman, A. Finkelstein, D. B. Goldman, *PatchMatch: A
//   Randomized Correspondence Algorithm for Structural Image Editing*,
//   ACM TOG (SIGGRAPH) 28(3), 2009 (the nearest-neighbour field).

/// Deterministic xorshift64 for the randomized patch search. A fixed seed keeps
/// the heal reproducible (the unit tests rely on it).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }
    #[inline]
    fn range(&mut self, n: i32) -> i32 {
        if n <= 1 {
            0
        } else {
            (self.next_u32() % n as u32) as i32
        }
    }
}

/// One pyramid level: the image, the hole, and the precomputed list of source
/// patch centres (fully known and fully inside the level).
struct Level {
    w: usize,
    h: usize,
    /// Patch half-size at this level.
    ph: usize,
    img: Vec<f32>,
    hole: Vec<bool>,
    /// `valid[t]`: the patch centred at `t` can be used as a source.
    valid: Vec<bool>,
    /// Indices of the valid sources, for O(1) random sampling.
    valid_list: Vec<u32>,
}

/// Builds a level and marks every patch centre whose `(2*ph+1)^2` neighbourhood
/// is fully inside the level and free of hole pixels.
fn make_level(w: usize, h: usize, img: Vec<f32>, hole: Vec<bool>, ph: usize) -> Level {
    let ph = ph.min(w.min(h).saturating_sub(1) / 2).max(1);
    // Integral image of the hole for O(1) "is the source patch hole-free".
    let stride = w + 1;
    let mut ii = vec![0u32; stride * (h + 1)];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += hole[y * w + x] as u32;
            ii[(y + 1) * stride + (x + 1)] = ii[y * stride + (x + 1)] + row;
        }
    }
    let count = |x0: i32, y0: i32, x1: i32, y1: i32| -> u32 {
        let x0 = x0.max(0) as usize;
        let y0 = y0.max(0) as usize;
        let x1 = x1.max(0) as usize;
        let y1 = y1.max(0) as usize;
        ii[y1 * stride + x1] + ii[y0 * stride + x0] - ii[y0 * stride + x1] - ii[y1 * stride + x0]
    };

    let mut valid = vec![false; w * h];
    let mut valid_list = Vec::new();
    let p = ph as i32;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if x - p < 0 || y - p < 0 || x + p >= w as i32 || y + p >= h as i32 {
                continue;
            }
            if count(x - p, y - p, x + p + 1, y + p + 1) == 0 {
                let t = y as usize * w + x as usize;
                valid[t] = true;
                valid_list.push(t as u32);
            }
        }
    }
    Level {
        w,
        h,
        ph,
        img,
        hole,
        valid,
        valid_list,
    }
}

/// Is `(sx, sy)` a usable source patch centre?
#[inline]
fn is_valid(lvl: &Level, sx: i32, sy: i32) -> bool {
    sx >= 0
        && sy >= 0
        && sx < lvl.w as i32
        && sy < lvl.h as i32
        && lvl.valid[sy as usize * lvl.w + sx as usize]
}

/// Sum of squared RGB differences between the target patch at `(tx, ty)` and the
/// source patch at `(sx, sy)`, clipped to the level. `best` allows an early out
/// as soon as the running sum cannot win.
fn patch_dist(lvl: &Level, ch: usize, tx: i32, ty: i32, sx: i32, sy: i32, best: f32) -> f32 {
    let p = lvl.ph as i32;
    let mut sum = 0.0f32;
    for dy in -p..=p {
        let yt = ty + dy;
        let ys = sy + dy;
        if yt < 0 || yt >= lvl.h as i32 || ys < 0 || ys >= lvl.h as i32 {
            continue;
        }
        for dx in -p..=p {
            let xt = tx + dx;
            let xs = sx + dx;
            if xt < 0 || xt >= lvl.w as i32 || xs < 0 || xs >= lvl.w as i32 {
                continue;
            }
            let ti = (yt as usize * lvl.w + xt as usize) * ch;
            let si = (ys as usize * lvl.w + xs as usize) * ch;
            for c in 0..ch {
                let d = lvl.img[ti + c] - lvl.img[si + c];
                sum += d * d;
            }
            if sum >= best {
                return sum;
            }
        }
    }
    sum
}

/// Scores a candidate source for a target centre; returns the new best distance
/// when the candidate is valid and strictly better.
#[inline]
fn try_source(lvl: &Level, ch: usize, tx: i32, ty: i32, sx: i32, sy: i32, cur: f32) -> Option<f32> {
    if !is_valid(lvl, sx, sy) {
        return None;
    }
    let d = patch_dist(lvl, ch, tx, ty, sx, sy, cur);
    if d < cur {
        Some(d)
    } else {
        None
    }
}

/// Computes the nearest-neighbour field (source centre per target pixel) with
/// propagation + random search, optionally seeded from a coarser level.
fn patchmatch(
    lvl: &Level,
    ch: usize,
    iters: usize,
    init: Option<&(Vec<i32>, Vec<i32>)>,
) -> (Vec<i32>, Vec<i32>) {
    let (w, h) = (lvl.w, lvl.h);
    let n = w * h;
    let mut nx = vec![0i32; n];
    let mut ny = vec![0i32; n];
    let mut dist = vec![f32::MAX; n];
    let mut rng = Rng::new(0x5EED_1234);
    let vn = lvl.valid_list.len() as i32;

    // Initial field: the (upscaled) coarse result when valid, otherwise random.
    for t in 0..n {
        let (mut sx, mut sy) = (0i32, 0i32);
        if let Some((ix, iy)) = init {
            if t < ix.len() {
                sx = ix[t];
                sy = iy[t];
            }
        }
        if !is_valid(lvl, sx, sy) {
            let s = lvl.valid_list[rng.range(vn) as usize] as usize;
            sx = (s % w) as i32;
            sy = (s / w) as i32;
        }
        nx[t] = sx;
        ny[t] = sy;
        dist[t] = patch_dist(lvl, ch, (t % w) as i32, (t / w) as i32, sx, sy, f32::MAX);
    }

    for _ in 0..iters {
        // Forward pass: propagate from the left / top neighbour, then random search.
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let t = y as usize * w + x as usize;
                let (mut bx, mut by, mut bd) = (nx[t], ny[t], dist[t]);
                if x > 0 {
                    let lt = t - 1;
                    if let Some(d) = try_source(lvl, ch, x, y, nx[lt] + 1, ny[lt], bd) {
                        bx = nx[lt] + 1;
                        by = ny[lt];
                        bd = d;
                    }
                }
                if y > 0 {
                    let ut = t - w;
                    if let Some(d) = try_source(lvl, ch, x, y, nx[ut], ny[ut] + 1, bd) {
                        bx = nx[ut];
                        by = ny[ut] + 1;
                        bd = d;
                    }
                }
                let mut radius = w.max(h) as i32;
                while radius >= 1 {
                    let cx = bx + rng.range(2 * radius + 1) - radius;
                    let cy = by + rng.range(2 * radius + 1) - radius;
                    if let Some(d) = try_source(lvl, ch, x, y, cx, cy, bd) {
                        bx = cx;
                        by = cy;
                        bd = d;
                    }
                    radius /= 2;
                }
                nx[t] = bx;
                ny[t] = by;
                dist[t] = bd;
            }
        }
        // Backward pass (propagate from the right / bottom neighbour).
        for y in (0..h as i32).rev() {
            for x in (0..w as i32).rev() {
                let t = y as usize * w + x as usize;
                let (mut bx, mut by, mut bd) = (nx[t], ny[t], dist[t]);
                if x + 1 < w as i32 {
                    let rt = t + 1;
                    if let Some(d) = try_source(lvl, ch, x, y, nx[rt] - 1, ny[rt], bd) {
                        bx = nx[rt] - 1;
                        by = ny[rt];
                        bd = d;
                    }
                }
                if y + 1 < h as i32 {
                    let dt = t + w;
                    if let Some(d) = try_source(lvl, ch, x, y, nx[dt], ny[dt] - 1, bd) {
                        bx = nx[dt];
                        by = ny[dt] - 1;
                        bd = d;
                    }
                }
                nx[t] = bx;
                ny[t] = by;
                dist[t] = bd;
            }
        }
    }
    (nx, ny)
}

/// Reconstructs the hole from the nearest-neighbour field: every patch votes for
/// the known pixels it overlaps, weighted by its match quality. Averaging the
/// votes removes the hard patch seams a plain copy would show.
fn vote(lvl: &Level, ch: usize, nx: &[i32], ny: &[i32]) -> Vec<f32> {
    let (w, h, p) = (lvl.w, lvl.h, lvl.ph as i32);
    let mut acc = vec![0.0f32; w * h * ch];
    let mut wsum = vec![0.0f32; w * h];
    let norm = ((2 * p + 1) as f32).powi(2) * ch as f32;

    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let t = y as usize * w + x as usize;
            let (sx, sy) = (nx[t], ny[t]);
            let d = patch_dist(lvl, ch, x, y, sx, sy, f32::MAX);
            let weight = 1.0 / (d / norm + 1.0e-4);
            for dy in -p..=p {
                let yt = y + dy;
                let ys = sy + dy;
                if yt < 0 || yt >= h as i32 || ys < 0 || ys >= h as i32 {
                    continue;
                }
                let th = yt as usize * w;
                let sh = ys as usize * w;
                for dx in -p..=p {
                    let xt = x + dx;
                    let xs = sx + dx;
                    if xt < 0 || xt >= w as i32 || xs < 0 || xs >= w as i32 {
                        continue;
                    }
                    let tp = th + xt as usize;
                    if !lvl.hole[tp] {
                        continue;
                    }
                    let sp = sh + xs as usize;
                    let ti = tp * ch;
                    let si = sp * ch;
                    for c in 0..ch {
                        acc[ti + c] += weight * lvl.img[si + c];
                    }
                    wsum[tp] += weight;
                }
            }
        }
    }

    let mut out = lvl.img.clone();
    for t in 0..w * h {
        if lvl.hole[t] && wsum[t] > 0.0 {
            for c in 0..ch {
                out[t * ch + c] = acc[t * ch + c] / wsum[t];
            }
        }
    }
    out
}

/// Content-aware completion of `hole` in `img` (all `0.0..=1.0` f32, interleaved
/// by `ch` channels). Returns a new buffer where **only** hole pixels changed.
///
/// The algorithm builds a Gaussian pyramid, initialises the hole with a membrane
/// solve and refines it level by level with a PatchMatch field + overlap vote,
/// from coarse (structure) to fine (texture). It is deterministic: a fixed RNG
/// seed makes the same input always heal the same way.
///
/// Falls back to a plain membrane fill when there is no known source patch at
/// all (e.g. the hole covers everything).
pub(crate) fn complete(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool]) -> Vec<f32> {
    if w == 0
        || h == 0
        || ch == 0
        || hole.iter().all(|&b| !b)
        || hole.iter().all(|&b| b)
    {
        return img.to_vec();
    }

    // Patch half-size from the hole size: small defects use a small patch (fast,
    // texture-like), large ones a bigger patch (structure-like).
    let Some((bx0, by0, bx1, by1)) = hole_bbox(hole, w, h) else {
        return img.to_vec();
    };
    let hole_max = (bx1 - bx0).max(by1 - by0) as usize;
    let ph_fine = ((hole_max + 3) / 4).clamp(2, 4);

    // Build the pyramid (index 0 = finest).
    let mut levels = vec![make_level(w, h, img.to_vec(), hole.to_vec(), ph_fine)];
    loop {
        let (lw, lh) = (levels.last().unwrap().w, levels.last().unwrap().h);
        if lw <= 20 || lh <= 20 {
            break;
        }
        let (nw, nh) = ((lw + 1) / 2, (lh + 1) / 2);
        let last = levels.last().unwrap();
        let mut nimg = vec![0.0f32; nw * nh * ch];
        let mut nhole = vec![false; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0.0f32; 3];
                let mut cnt = 0.0f32;
                for (ox, oy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                    let sx = x * 2 + ox;
                    let sy = y * 2 + oy;
                    if sx >= lw || sy >= lh || last.hole[sy * lw + sx] {
                        continue;
                    }
                    let si = (sy * lw + sx) * ch;
                    for c in 0..ch {
                        acc[c] += last.img[si + c];
                    }
                    cnt += 1.0;
                }
                let di = (y * nw + x) * ch;
                if cnt > 0.0 {
                    for c in 0..ch {
                        nimg[di + c] = acc[c] / cnt;
                    }
                } else {
                    nhole[y * nw + x] = true;
                }
            }
        }
        let li = levels.len();
        let nph = (ph_fine >> li).max(1);
        levels.push(make_level(nw, nh, nimg, nhole, nph));
    }

    const ITERS: usize = 3;
    let mut coarse_filled: Option<Vec<f32>> = None;
    let mut coarse_dims = (0usize, 0usize);
    let mut init_nnf: Option<(Vec<i32>, Vec<i32>)> = None;

    // Coarse to fine.
    for li in (0..levels.len()).rev() {
        let (lw, lh) = (levels[li].w, levels[li].h);

        // Initial hole values: the upsampled coarse result, or a membrane solve
        // at the coarsest level.
        if let Some(cf) = &coarse_filled {
            let (cw, chh) = coarse_dims;
            for y in 0..lh {
                for x in 0..lw {
                    let t = y * lw + x;
                    if !levels[li].hole[t] {
                        continue;
                    }
                    let cx = (x / 2).min(cw - 1);
                    let cy = (y / 2).min(chh - 1);
                    let si = (cy * cw + cx) * ch;
                    let di = t * ch;
                    for c in 0..ch {
                        levels[li].img[di + c] = cf[si + c];
                    }
                }
            }
        } else {
            levels[li].img = membrane_fill(lw, lh, ch, &levels[li].img, &levels[li].hole);
        }

        if levels[li].valid_list.is_empty() {
            coarse_filled = Some(membrane_fill(lw, lh, ch, &levels[li].img, &levels[li].hole));
            coarse_dims = (lw, lh);
            init_nnf = None;
            continue;
        }

        let (nx, ny) = patchmatch(&levels[li], ch, ITERS, init_nnf.as_ref());
        levels[li].img = vote(&levels[li], ch, &nx, &ny);
        coarse_filled = Some(levels[li].img.clone());
        coarse_dims = (lw, lh);

        // Upscale the field for the next (finer) level.
        if li > 0 {
            let (fw, fh) = (levels[li - 1].w, levels[li - 1].h);
            let mut ix = vec![0i32; fw * fh];
            let mut iy = vec![0i32; fw * fh];
            for y in 0..fh {
                for x in 0..fw {
                    let cx = (x / 2).min(lw - 1);
                    let cy = (y / 2).min(lh - 1);
                    let ci = cy * lw + cx;
                    ix[y * fw + x] = nx[ci] * 2;
                    iy[y * fw + x] = ny[ci] * 2;
                }
            }
            init_nnf = Some((ix, iy));
        }
    }

    let mut out = img.to_vec();
    if let Some(finest) = levels.first() {
        for t in 0..w * h {
            if hole[t] {
                for c in 0..ch {
                    out[t * ch + c] = finest.img[t * ch + c];
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod complete_tests {
    use super::*;

    /// A striped pattern must be rebuilt inside the hole (structure continues).
    #[test]
    fn complete_restores_stripes() {
        let (w, h) = (48usize, 48usize);
        let mut img = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let stripe = if (x / 4) % 2 == 0 { 0.85 } else { 0.2 };
                img[y * w + x] = (stripe + y as f32 * 0.001).min(1.0);
            }
        }
        let (cx, cy) = (24.0f32, 24.0f32);
        let mut hole = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                if dx * dx + dy * dy <= 36.0 {
                    hole[y * w + x] = true;
                }
            }
        }
        let out = complete(w, h, 1, &img, &hole);
        // Outside the hole nothing changes.
        for i in 0..w * h {
            if !hole[i] {
                assert_eq!(out[i], img[i], "pixel {i} outside hole changed");
            }
        }
        // Inside, the fill must be close to the periodic pattern (a phase shift by
        // a whole stripe period is fine because the pattern repeats).
        let mut err = 0.0f64;
        let mut count = 0.0f64;
        for i in 0..w * h {
            if hole[i] {
                err += (out[i] - img[i]).abs() as f64;
                count += 1.0;
            }
        }
        let mean = err / count;
        assert!(mean < 0.30, "mean fill error too high: {mean}");
        // Something was actually synthesized (not a flat patch).
        let (mut min, mut max) = (f32::MAX, f32::MIN);
        for i in 0..w * h {
            if hole[i] {
                min = min.min(out[i]);
                max = max.max(out[i]);
            }
        }
        assert!(max - min > 0.3, "fill collapsed to a flat value");
    }

    #[test]
    fn complete_is_deterministic() {
        let (w, h) = (32usize, 32usize);
        let img: Vec<f32> = (0..w * h)
            .map(|i| ((i * 7 + i / 3) % 97) as f32 / 97.0)
            .collect();
        let mut hole = vec![false; w * h];
        for y in 12..20 {
            for x in 12..20 {
                hole[y * w + x] = true;
            }
        }
        assert_eq!(complete(w, h, 1, &img, &hole), complete(w, h, 1, &img, &hole));
    }

    #[test]
    fn complete_covers_the_whole_hole() {
        let (w, h) = (24usize, 24usize);
        let img = vec![0.4f32; w * h * 3];
        let mut hole = vec![false; w * h];
        for y in 8..16 {
            for x in 8..16 {
                hole[y * w + x] = true;
            }
        }
        let out = complete(w, h, 3, &img, &hole);
        for t in 0..w * h {
            if hole[t] {
                for c in 0..3 {
                    assert!(out[t * 3 + c].is_finite());
                }
            }
        }
    }

    /// Full hole / empty hole are no-ops.
    #[test]
    fn complete_degenerate_cases_are_noops() {
        let img = vec![0.1f32, 0.2, 0.3, 0.4];
        assert_eq!(complete(2, 2, 1, &img, &[false; 4]), img);
        assert_eq!(complete(2, 2, 1, &img, &[true; 4]), img);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nonzero_offset_on_flat_image() {
        let (w, h) = (40usize, 40usize);
        let img = vec![0.5f32; w * h];
        let mut hole = vec![false; w * h];
        for y in 18..23 {
            for x in 18..23 {
                hole[y * w + x] = true;
            }
        }
        let off = best_offset(w, h, 1, &img, &hole, 3, 12);
        let (dx, dy) = off.expect("flat image has a valid source");
        assert!(dx != 0 || dy != 0, "offset must not be zero: ({dx},{dy})");
        assert!(dx.abs() <= 12 && dy.abs() <= 12);
    }

    /// On a structured image the chosen source ring must match the ring around
    /// the hole better than the hole itself.
    #[test]
    fn picks_matching_source_on_pattern() {
        let (w, h) = (64usize, 64usize);
        let mut img = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                // Vertical stripes with period 8 + a small horizontal gradient.
                img[y * w + x] = if (x / 8) % 2 == 0 { 0.8 } else { 0.2 } + y as f32 * 0.001;
            }
        }
        let mut hole = vec![false; w * h];
        for y in 20..28 {
            for x in 20..28 {
                hole[y * w + x] = true;
            }
        }
        let off = best_offset(w, h, 1, &img, &hole, 4, 20).expect("source found");
        let (dx, dy) = off;
        // A multiple of the stripe period (8) shifts the stripes onto themselves.
        assert_eq!(dx % 8, 0, "dx should be a whole number of stripes: {dx}");
        assert!(dx.abs() >= 8, "source must move away from the hole: {dx}");
        let _ = dy;
    }

    #[test]
    fn none_when_hole_covers_image() {
        let (w, h) = (10usize, 10usize);
        let img = vec![0.5f32; w * h];
        let hole = vec![true; w * h];
        assert!(best_offset(w, h, 1, &img, &hole, 2, 4).is_none());
    }

    #[test]
    fn dilate_grows_by_one_step() {
        let (w, h) = (5usize, 5usize);
        let mut m = vec![false; w * h];
        m[2 * w + 2] = true;
        let d = dilate(&m, w, h, 1);
        assert!(d[2 * w + 1] && d[2 * w + 3] && d[1 * w + 2] && d[3 * w + 2]);
        assert!(!d[0]);
        assert!(!d[4 * w + 4]);
    }
}
