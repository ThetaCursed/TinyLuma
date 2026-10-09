// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Pixel operations used by crop: the export crop, the fine straighten rotation
//! and the auto-straighten angle detector.

use super::geom::NormRect;

/// Crops a tightly packed RGB8 buffer. Returns the new buffer and its size.
/// An identity crop returns a copy unchanged.
pub(crate) fn crop_rgb(buf: &[u8], img_w: u32, img_h: u32, rect: NormRect) -> (Vec<u8>, u32, u32) {
    if rect.is_full() {
        return (buf.to_vec(), img_w, img_h);
    }
    let (x0, y0, cw, ch) = rect.pixel_rect(img_w, img_h);
    let stride = img_w as usize * 3;
    let row = cw as usize * 3;
    let mut out = Vec::with_capacity(row * ch as usize);
    for y in y0..(y0 + ch) {
        let start = y as usize * stride + x0 as usize * 3;
        out.extend_from_slice(&buf[start..start + row]);
    }
    (out, cw, ch)
}

/// Rotates an RGB8 image by `angle_deg` degrees clockwise around its center
/// (bilinear sampling). The dimensions are unchanged; the corners the rotation
/// leaves empty are black. Used by straighten (display and export).
pub(crate) fn rotate_rgb(buf: &[u8], w: u32, h: u32, angle_deg: f32) -> (Vec<u8>, u32, u32) {
    if w == 0 || h == 0 || angle_deg.abs() < 1e-4 {
        return (buf.to_vec(), w, h);
    }
    let cx = (w as f32 - 1.0) * 0.5;
    let cy = (h as f32 - 1.0) * 0.5;
    let r = angle_deg.to_radians();
    let (s, c) = r.sin_cos();
    let mut out = vec![0u8; buf.len()];
    for oy in 0..h {
        for ox in 0..w {
            let dx = ox as f32 - cx;
            let dy = oy as f32 - cy;
            // Inverse rotation: destination → source.
            let sx = cx + c * dx + s * dy;
            let sy = cy - s * dx + c * dy;
            let di = ((oy as usize * w as usize) + ox as usize) * 3;
            if sx < 0.0 || sy < 0.0 || sx > (w - 1) as f32 || sy > (h - 1) as f32 {
                continue; // empty corner → black
            }
            out[di..di + 3].copy_from_slice(&bilinear(buf, w, h, sx, sy));
        }
    }
    (out, w, h)
}

fn bilinear(buf: &[u8], w: u32, h: u32, x: f32, y: f32) -> [u8; 3] {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = (x0 + 1).min(w as i32 - 1);
    let y1 = (y0 + 1).min(h as i32 - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let mut out = [0u8; 3];
    for c in 0..3 {
        let p00 = px(buf, w, x0, y0, c) as f32;
        let p10 = px(buf, w, x1, y0, c) as f32;
        let p01 = px(buf, w, x0, y1, c) as f32;
        let p11 = px(buf, w, x1, y1, c) as f32;
        let top = p00 + (p10 - p00) * fx;
        let bottom = p01 + (p11 - p01) * fx;
        out[c] = (top + (bottom - top) * fy).clamp(0.0, 255.0) as u8;
    }
    out
}

fn px(buf: &[u8], w: u32, x: i32, y: i32, c: usize) -> u8 {
    let x = x.clamp(0, w as i32 - 1) as usize;
    let y = y.max(0) as usize;
    let i = (y * w as usize + x) * 3 + c;
    buf.get(i).copied().unwrap_or(0)
}

/// Estimates the straighten angle needed to level the dominant near-horizontal
/// or near-vertical lines, in degrees. Returns `0.0` when nothing stands out.
///
/// Method: a magnitude-weighted histogram of edge-gradient directions, mapped to
/// their deviation from the nearest axis, then the peak. Only roughly
/// axis-aligned edges are counted, so it targets horizons and building edges
/// rather than arbitrary texture.
pub(crate) fn detect_straighten_angle(buf: &[u8], w: u32, h: u32) -> f32 {
    if w < 8 || h < 8 {
        return 0.0;
    }
    const MIN_MAG: f32 = 48.0;
    const RANGE: f32 = 15.0;
    const STEP: f32 = 0.25;
    let bins = ((2.0 * RANGE / STEP) as usize) + 1;
    let mut hist = vec![0f32; bins];

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = sobel(buf, w, x, y, true);
            let gy = sobel(buf, w, x, y, false);
            let mag = (gx * gx + gy * gy).sqrt();
            if mag < MIN_MAG {
                continue;
            }
            // Edge direction is perpendicular to the gradient.
            let mut a = gx.atan2(-gy).to_degrees();
            if a >= 90.0 {
                a -= 180.0;
            } else if a < -90.0 {
                a += 180.0;
            }
            // Only near-axis edges (within 30° of horizontal/vertical).
            if a.abs() > 30.0 && (a.abs() - 90.0).abs() > 30.0 {
                continue;
            }
            let dev = if a > 45.0 {
                a - 90.0
            } else if a < -45.0 {
                a + 90.0
            } else {
                a
            };
            if dev < -RANGE || dev > RANGE {
                continue;
            }
            let idx = ((dev + RANGE) / STEP).round() as usize;
            if idx < bins {
                hist[idx] += mag;
            }
        }
    }

    let (best_idx, &best) = hist
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();
    if best <= 0.0 {
        return 0.0;
    }
    // The detected line deviation must be cancelled by rotating the other way.
    let dev = -RANGE + best_idx as f32 * STEP;
    -dev
}

/// One Sobel half (horizontal gradients integrate the Gx kernel, vertical Gy).
fn sobel(buf: &[u8], w: u32, x: u32, y: u32, horizontal: bool) -> f32 {
    let l = |dx: i32, dy: i32| -> f32 {
        let xx = (x as i32 + dx).clamp(0, w as i32 - 1) as u32;
        let yy = (y as i32 + dy).max(0) as u32;
        luma(buf, w, xx, yy)
    };
    if horizontal {
        (l(1, -1) + 2.0 * l(1, 0) + l(1, 1)) - (l(-1, -1) + 2.0 * l(-1, 0) + l(-1, 1))
    } else {
        (l(-1, 1) + 2.0 * l(0, 1) + l(1, 1)) - (l(-1, -1) + 2.0 * l(0, -1) + l(1, -1))
    }
}

fn luma(buf: &[u8], w: u32, x: u32, y: u32) -> f32 {
    let i = ((y * w + x) * 3) as usize;
    if i + 2 >= buf.len() {
        return 0.0;
    }
    0.299 * buf[i] as f32 + 0.587 * buf[i + 1] as f32 + 0.114 * buf[i + 2] as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|i| [i as u8, (i / 2) as u8, 0u8]).collect()
    }

    /// Horizontal stripes (black/white bands) used to test the detector.
    fn stripes(w: u32, h: u32) -> Vec<u8> {
        let mut buf = vec![0u8; (w * h * 3) as usize];
        for y in 0..h {
            let v = if (y / 4) % 2 == 0 { 20u8 } else { 235u8 };
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                buf[i] = v;
                buf[i + 1] = v;
                buf[i + 2] = v;
            }
        }
        buf
    }

    #[test]
    fn identity_is_a_copy() {
        let buf = gradient(4, 4);
        let (out, w, h) = crop_rgb(&buf, 4, 4, NormRect::FULL);
        assert_eq!((w, h), (4, 4));
        assert_eq!(out, buf);
    }

    #[test]
    fn crops_top_left_quadrant() {
        let buf = gradient(4, 4);
        let rect = NormRect {
            x: 0.0,
            y: 0.0,
            w: 0.5,
            h: 0.5,
        };
        let (out, w, h) = crop_rgb(&buf, 4, 4, rect);
        assert_eq!((w, h), (2, 2));
        assert_eq!(&out[0..6], &[0, 0, 0, 1, 0, 0]);
        assert_eq!(out.len(), 2 * 2 * 3);
    }

    #[test]
    fn crops_right_half() {
        let buf = gradient(4, 2);
        let rect = NormRect {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        };
        let (out, w, h) = crop_rgb(&buf, 4, 2, rect);
        assert_eq!((w, h), (2, 2));
        assert_eq!(&out[0..6], &[2, 1, 0, 3, 1, 0]);
        assert_eq!(&out[6..12], &[6, 3, 0, 7, 3, 0]);
    }

    #[test]
    fn rotate_zero_is_a_copy() {
        let buf = gradient(5, 3);
        let (out, w, h) = rotate_rgb(&buf, 5, 3, 0.0);
        assert_eq!((out, w, h), (buf, 5, 3));
    }

    #[test]
    fn rotate_keeps_dimensions_and_length() {
        let buf = stripes(32, 32);
        let (out, w, h) = rotate_rgb(&buf, 32, 32, 12.0);
        assert_eq!((w, h), (32, 32));
        assert_eq!(out.len(), buf.len());
    }

    #[test]
    fn rotate_90_is_not_the_same_as_cw_quarter_turn() {
        // A tiny sanity check: rotating a two-tone image by 90° should move the
        // brightness to a different corner (not identity).
        let mut buf = vec![0u8; (2 * 2 * 3) as usize];
        let last = (1 * 2 + 1) * 3;
        buf[last] = 200;
        buf[last + 1] = 200;
        buf[last + 2] = 200;
        let (out, _, _) = rotate_rgb(&buf, 2, 2, 90.0);
        assert_ne!(out, buf);
    }

    #[test]
    fn detects_zero_on_level_stripes() {
        let buf = stripes(64, 64);
        let a = detect_straighten_angle(&buf, 64, 64);
        assert!(a.abs() < 0.5, "level stripes must not tilt: {a}");
    }

    #[test]
    fn detects_a_known_tilt() {
        // Rotate the stripes by +6°, the detector should ask for about -6°.
        let buf = stripes(96, 96);
        let (tilted, w, h) = rotate_rgb(&buf, 96, 96, 6.0);
        let a = detect_straighten_angle(&tilted, w, h);
        assert!((a + 6.0).abs() < 1.5, "expected ~-6, got {a}");
    }
}
