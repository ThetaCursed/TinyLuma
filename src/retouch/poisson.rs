// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Gradient-domain blending.
//!
//! Independent adaptation of the classical methods:
//! * membrane / Laplace fill — smooth interpolation over a hole;
//! * Poisson image editing (Pérez, Gangnet, Blake, SIGGRAPH 2003) — makes a
//!   pasted patch blend into its surroundings so the seam disappears.

/// Iterative membrane (Laplace) fill of `hole` pixels.
///
/// Solves `Δf = 0` inside the hole with the known pixels as Dirichlet boundary.
/// Used to initialise a fallback fill when no healing source is found.
pub(crate) fn membrane_fill(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool]) -> Vec<f32> {
    let mut out = img.to_vec();
    if w == 0 || h == 0 || hole.iter().all(|&b| !b) {
        return out;
    }

    let max_iters = mask_iters(hole, w, h);
    const TOL: f32 = 1.0e-4;

    for _ in 0..max_iters {
        let mut max_diff = 0.0f32;
        // Gauss–Seidel in place: updated values are immediately visible to the
        // next pixel, which converges roughly twice as fast as Jacobi.
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if !hole[i] {
                    continue;
                }
                let mut sum = [0.0f32; 4];
                let mut count = 0.0f32;
                // 4-neighbourhood, clamped at the image border.
                for (nx, ny) in neighbors(x, y, w, h) {
                    let j = ny * w + nx;
                    for c in 0..ch {
                        sum[c] += out[j * ch + c];
                    }
                    count += 1.0;
                }
                if count == 0.0 {
                    continue;
                }
                let base = i * ch;
                for c in 0..ch {
                    let v = sum[c] / count;
                    let d = (v - out[base + c]).abs();
                    if d > max_diff {
                        max_diff = d;
                    }
                    out[base + c] = v;
                }
            }
        }
        if max_diff < TOL {
            break;
        }
    }
    out
}

/// Gradient-domain clone (Poisson image editing).
///
/// Keeps the *gradients* of `src` inside `mask` and matches the values of `dst`
/// on the mask boundary: `Δf = Δsrc` inside the mask, `f = dst` outside.
pub(crate) fn seamless_clone(
    w: usize,
    h: usize,
    ch: usize,
    src: &[f32],
    dst: &[f32],
    mask: &[bool],
) -> Vec<f32> {
    let mut out = dst.to_vec();
    if w == 0 || h == 0 || mask.iter().all(|&b| !b) {
        return out;
    }

    // Copy the source into the mask as the initial guess — the solver then only
    // has to remove the low-frequency seam. Also build the red/black pixel lists
    // so each solver sweep touches only mask pixels, not the whole region.
    let mut red = Vec::new();
    let mut black = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !mask[i] {
                continue;
            }
            for c in 0..ch {
                out[i * ch + c] = src[i * ch + c];
            }
            if (x + y) % 2 == 0 {
                red.push(i);
            } else {
                black.push(i);
            }
        }
    }

    let max_iters = mask_iters(mask, w, h);
    const TOL: f32 = 1.0e-4;

    for _ in 0..max_iters {
        let mut max_diff = 0.0f32;
        // Red–black ordering: proper Gauss–Seidel, no directional bias.
        for list in [&red, &black] {
            for &i in list {
                let x = i % w;
                let y = i / w;
                let base = i * ch;
                let mut sum = [0.0f32; 4];
                let mut guide = [0.0f32; 4];
                let mut count = 0.0f32;
                for (nx, ny) in neighbors(x, y, w, h) {
                    let j = ny * w + nx;
                    for c in 0..ch {
                        sum[c] += out[j * ch + c];
                        // Guidance field: the discrete Laplacian of the source.
                        guide[c] += src[base + c] - src[j * ch + c];
                    }
                    count += 1.0;
                }
                if count == 0.0 {
                    continue;
                }
                for c in 0..ch {
                    let v = (sum[c] + guide[c]) / count;
                    let d = (v - out[base + c]).abs();
                    if d > max_diff {
                        max_diff = d;
                    }
                    out[base + c] = v;
                }
            }
        }
        if max_diff < TOL {
            break;
        }
    }
    out
}

/// Iteration budget for the iterative solvers: proportional to the mask
/// diagonal (Gauss–Seidel needs O(D) sweeps for an O(D) region) but clamped so
/// tiny spots are cheap and huge ones stay bounded.
fn mask_iters(mask: &[bool], w: usize, h: usize) -> usize {
    let mut x0 = w;
    let mut y0 = h;
    let mut x1 = 0usize;
    let mut y1 = 0usize;
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if !any {
        return 1;
    }
    let diag = (((x1 - x0 + 1).pow(2) + (y1 - y0 + 1).pow(2)) as f32).sqrt();
    ((diag * 1.5) as usize).clamp(48, 600)
}

/// The up-to-four in-bounds 4-neighbours of `(x, y)` (clamped at the border).
#[inline]
fn neighbors(x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut buf = [(x, y); 4];
    let mut n = 0;
    if x > 0 {
        buf[n] = (x - 1, y);
        n += 1;
    }
    if x + 1 < w {
        buf[n] = (x + 1, y);
        n += 1;
    }
    if y > 0 {
        buf[n] = (x, y - 1);
        n += 1;
    }
    if y + 1 < h {
        buf[n] = (x, y + 1);
        n += 1;
    }
    buf.into_iter().take(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hole in a linear gradient must be filled with the same gradient.
    #[test]
    fn membrane_reproduces_linear_gradient() {
        let (w, h) = (21usize, 9usize);
        let mut img = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                img[y * w + x] = x as f32 / (w - 1) as f32;
            }
        }
        let mut hole = vec![false; w * h];
        // A 5x5 hole in the middle.
        for y in 2..7 {
            for x in 8..13 {
                hole[y * w + x] = true;
            }
        }
        let out = membrane_fill(w, h, 1, &img, &hole);
        for y in 0..h {
            for x in 0..w {
                let expected = x as f32 / (w - 1) as f32;
                assert!(
                    (out[y * w + x] - expected).abs() < 2.0e-2,
                    "({x},{y}) {} vs {expected}",
                    out[y * w + x]
                );
            }
        }
    }

    /// Outside the mask the clone must leave the destination byte-identical.
    #[test]
    fn seamless_clone_keeps_outside_untouched() {
        let (w, h) = (12usize, 12usize);
        let src: Vec<f32> = (0..w * h * 3).map(|i| (i % 17) as f32 / 17.0).collect();
        let dst: Vec<f32> = (0..w * h * 3).map(|i| (i % 5) as f32 / 5.0).collect();
        let mut mask = vec![false; w * h];
        for y in 4..8 {
            for x in 4..8 {
                mask[y * w + x] = true;
            }
        }
        let out = seamless_clone(w, h, 3, &src, &dst, &mask);
        for i in 0..w * h {
            if !mask[i] {
                for c in 0..3 {
                    assert_eq!(out[i * 3 + c], dst[i * 3 + c], "pixel {i} changed");
                }
            }
        }
    }

    /// With a flat source the Poisson result inside the mask equals the
    /// destination boundary value (no gradients to preserve).
    #[test]
    fn seamless_clone_flat_source_matches_boundary() {
        let (w, h) = (16usize, 16usize);
        let src = vec![0.25f32; w * h];
        let dst = vec![0.75f32; w * h];
        let mut mask = vec![false; w * h];
        for y in 6..10 {
            for x in 6..10 {
                mask[y * w + x] = true;
            }
        }
        let out = seamless_clone(w, h, 1, &src, &dst, &mask);
        for i in 0..w * h {
            assert!((out[i] - 0.75).abs() < 2.0e-3, "{}", out[i]);
        }
    }
}
