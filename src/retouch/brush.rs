// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Brush settings and stroke dabbing.

use super::spot::{core_fraction, Spot, SpotKind};

/// Brush configuration, owned by the retouch state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct BrushSettings {
    /// Diameter in working-image pixels (what the UI shows).
    pub(crate) size: f32,
    /// `0..1`, `1` = hard edge.
    pub(crate) hardness: f32,
    /// Fraction of `size` between dabs along a stroke.
    pub(crate) spacing: f32,
    /// Fill algorithm new dabs use (Photopea-style "Type").
    pub(crate) kind: SpotKind,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            size: 60.0,
            hardness: 0.5,
            spacing: 0.25,
            // Content-aware by default — it matches Photopea's Spot Healing
            // Brush and needs no manual source.
            kind: SpotKind::ContentAware,
        }
    }
}

/// Rasterized coverage of a painted path (the capsule swept by the brush),
/// ready to be tinted and drawn.
pub(crate) struct OverlayCoverage {
    /// Normalized bounding box in image space: `[x0, y0, x1, y1]`.
    pub(crate) area: [f32; 4],
    pub(crate) w: usize,
    pub(crate) h: usize,
    /// Coverage `0..1` per pixel (row-major).
    pub(crate) data: Vec<f32>,
}

impl BrushSettings {
    /// Radius normalized to the longer image side.
    pub(crate) fn radius_fraction(&self, w: usize, h: usize) -> f32 {
        (self.size * 0.5) / (w.max(h) as f32).max(1.0)
    }

    /// Builds a proximity-match spot at a normalized position.
    #[allow(dead_code)] // dab helper, exercised by tests / the size fallback
    pub(crate) fn to_spot(&self, center: [f32; 2], w: usize, h: usize) -> Spot {
        Spot {
            center,
            radius: self.radius_fraction(w, h),
            hardness: self.hardness,
            opacity: 1.0,
            kind: self.kind,
            path: None,
        }
    }

    /// Builds a single **stroke** spot from a painted path: the whole gesture is
    /// healed by one union fill instead of one fill per dab. The stored centers
    /// are the dab centers (see [`Self::resample_path`]) and they double as the
    /// stroke's polyline, so the healed capsule matches the overlay indication.
    pub(crate) fn to_stroke(&self, path: &[[f32; 2]], w: usize, h: usize) -> Spot {
        let centers = self.resample_path(path, w, h, 1.0);
        Spot {
            center: centers.first().copied().unwrap_or([0.5, 0.5]),
            radius: self.radius_fraction(w, h),
            hardness: self.hardness,
            opacity: 1.0,
            kind: self.kind,
            path: Some(centers),
        }
    }

    /// Dab centers from `from` (exclusive) to `to` (inclusive), normalized.
    /// Resamples a painted path into dab centers spaced `spacing * size` pixels
    /// apart, carrying the leftover distance across segment boundaries.
    ///
    /// This is the key detail: the pointer emits a point per frame, so individual
    /// segments are often shorter than the spacing; resampling each segment
    /// separately would drop them and leave gaps.
    pub(crate) fn resample_path(
        &self,
        path: &[[f32; 2]],
        w: usize,
        h: usize,
        scale: f32,
    ) -> Vec<[f32; 2]> {
        if path.is_empty() {
            return Vec::new();
        }
        let step = (self.spacing * self.size * scale).max(1.0);
        let fw = w.max(1) as f32;
        let fh = h.max(1) as f32;

        let mut out = Vec::with_capacity(path.len() + 8);
        out.push(path[0]);
        let mut prev = path[0];
        // Distance travelled since the last emitted dab.
        let mut travelled = 0.0f32;

        for &p in &path[1..] {
            let ax = prev[0] * fw;
            let ay = prev[1] * fh;
            let bx = p[0] * fw;
            let by = p[1] * fh;
            let seg = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            if seg < 1.0e-6 {
                prev = p;
                continue;
            }
            let mut d = step - travelled;
            while d <= seg {
                let t = d / seg;
                out.push([(ax + (bx - ax) * t) / fw, (ay + (by - ay) * t) / fh]);
                d += step;
            }
            // Distance from the last emitted dab to the end of this segment.
            travelled = seg - (d - step);
            prev = p;
        }

        if out.last().copied() != path.last().copied() {
            out.push(*path.last().unwrap());
        }
        out
    }

    /// Rasterizes the brush along `path` into a compact coverage map (bounding
    /// box only).
    ///
    /// The coverage is derived from the **distance to the painted polyline**, not
    /// from the union of individual dabs. Because the dab centers are discrete,
    /// taking the `max` of their falloffs made the edge of a stroke scalloped —
    /// the further apart the dabs (larger `spacing`), the deeper the notches. The
    /// distance field is exactly the capsule swept by the brush, so the outline
    /// is smooth and independent of the dab spacing.
    ///
    /// `scale` converts working-image pixels to this (possibly downscaled) grid,
    /// so a large image can use a cheap low-resolution indication.
    pub(crate) fn stroke_coverage(
        &self,
        path: &[[f32; 2]],
        grid_w: usize,
        grid_h: usize,
        scale: f32,
    ) -> Option<OverlayCoverage> {
        if path.is_empty() || grid_w == 0 || grid_h == 0 {
            return None;
        }
        let fw = grid_w as f32;
        let fh = grid_h as f32;
        let r = (self.size * 0.5 * scale).max(1.0);

        let mut minx = f32::MAX;
        let mut miny = f32::MAX;
        let mut maxx = f32::MIN;
        let mut maxy = f32::MIN;
        for p in path {
            let x = p[0] * fw;
            let y = p[1] * fh;
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        let x0 = (minx - r - 2.0).floor().clamp(0.0, fw) as usize;
        let y0 = (miny - r - 2.0).floor().clamp(0.0, fh) as usize;
        let x1 = (maxx + r + 2.0).ceil().clamp(0.0, fw) as usize;
        let y1 = (maxy + r + 2.0).ceil().clamp(0.0, fh) as usize;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let bw = x1 - x0;
        let bh = y1 - y0;

        // Squared distance to the painted polyline, in grid pixels. The dab
        // centers from `resample_path` approximate the pointer path within one
        // spacing step, so connecting them keeps the capsule smooth while the
        // work stays proportional to the stroke length, not to the frame rate.
        let mut dist2 = vec![f32::MAX; bw * bh];
        let poly: Vec<(f32, f32)> = self
            .resample_path(path, grid_w, grid_h, scale)
            .iter()
            .map(|p| (p[0] * fw - x0 as f32, p[1] * fh - y0 as f32))
            .collect();
        if poly.len() == 1 {
            stamp_segment_dist(&mut dist2, bw, bh, poly[0], poly[0], r + 1.0);
        } else {
            for seg in poly.windows(2) {
                stamp_segment_dist(&mut dist2, bw, bh, seg[0], seg[1], r + 1.0);
            }
        }

        // A fully hard brush still needs a sub-pixel ramp at the outline,
        // otherwise the binary edge lands on the grid and reads as stair steps
        // once the texture is magnified. Keep at least a one-grid-pixel
        // smoothstep band regardless of `hardness`.
        const AA: f32 = 1.0;
        let inner = (core_fraction(self.hardness) * r).min(r - AA).max(0.0);
        let feather = (r - inner).max(AA);
        let mut data = vec![0.0f32; bw * bh];
        for (i, &d2) in dist2.iter().enumerate() {
            if d2 == f32::MAX {
                continue;
            }
            let d = d2.sqrt();
            if d >= r {
                continue;
            }
            data[i] = if d <= inner {
                1.0
            } else {
                let t = ((r - d) / feather).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
        }

        Some(OverlayCoverage {
            area: [
                x0 as f32 / fw,
                y0 as f32 / fh,
                x1 as f32 / fw,
                y1 as f32 / fh,
            ],
            w: bw,
            h: bh,
            data,
        })
    }

    /// Stamps a whole painted path into individual dabs. Single dabs are still a
    /// valid op (and the stroke size fallback), so this stays available even
    /// though a normal gesture is now stored as one stroke.
    #[allow(dead_code)]
    pub(crate) fn spots_along(&self, path: &[[f32; 2]], w: usize, h: usize) -> Vec<Spot> {
        self.resample_path(path, w, h, 1.0)
            .into_iter()
            .map(|p| self.to_spot(p, w, h))
            .collect()
    }
}

/// Updates `dist2` (squared distance to the painted polyline, row-major over a
/// `bw x bh` grid) with the distances to the segment `a`–`b`. Only the segment's
/// own bbox, grown by `pad`, is visited; the caller only cares about pixels
/// within the brush radius (plus a margin for the smoothstep edge).
pub(crate) fn stamp_segment_dist(
    dist2: &mut [f32],
    bw: usize,
    bh: usize,
    a: (f32, f32),
    b: (f32, f32),
    pad: f32,
) {
    let px0 = (a.0.min(b.0) - pad).floor().max(0.0) as usize;
    let py0 = (a.1.min(b.1) - pad).floor().max(0.0) as usize;
    let px1 = ((a.0.max(b.0) + pad).ceil() as isize + 1).clamp(0, bw as isize) as usize;
    let py1 = ((a.1.max(b.1) + pad).ceil() as isize + 1).clamp(0, bh as isize) as usize;
    if px1 <= px0 || py1 <= py0 {
        return;
    }
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len2 = vx * vx + vy * vy;
    for y in py0..py1 {
        let py = y as f32 + 0.5;
        for x in px0..px1 {
            let px = x as f32 + 0.5;
            let wx = px - a.0;
            let wy = py - a.1;
            let t = if len2 > 1.0e-12 {
                ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dx = px - (a.0 + t * vx);
            let dy = py - (a.1 + t * vy);
            let d2 = dx * dx + dy * dy;
            let i = y * bw + x;
            if d2 < dist2[i] {
                dist2[i] = d2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_fraction_tracks_longer_side() {
        let b = BrushSettings {
            size: 120.0,
            ..Default::default()
        };
        // 120 px diameter → 60 px radius → 60/1200 of the longer side.
        assert!((b.radius_fraction(1200, 800) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn resample_path_spaces_dabs_evenly() {
        let b = BrushSettings {
            size: 10.0,
            spacing: 0.5, // step = 5 px
            ..Default::default()
        };
        let pts = b.resample_path(&[[0.0, 0.5], [1.0, 0.5]], 100, 100, 1.0);
        // 100 px / 5 px step → 21 points including both ends.
        assert_eq!(pts.len(), 21);
        assert!((pts[0][0] - 0.0).abs() < 1e-6);
        assert!((pts[20][0] - 1.0).abs() < 1e-6);
        for w in pts.windows(2) {
            assert!((w[1][0] - w[0][0] - 0.05).abs() < 1e-4);
        }
    }

    /// Regression: the pointer emits a point every frame, so most segments are
    /// shorter than the spacing. Resampling must carry the distance across them
    /// instead of dropping the dabs (which left gaps in the overlay).
    #[test]
    fn resample_path_bridges_segments_shorter_than_the_step() {
        let b = BrushSettings {
            size: 10.0,
            spacing: 0.5, // step = 5 px
            ..Default::default()
        };
        let path: Vec<[f32; 2]> = (0..=100).map(|i| [i as f32 / 100.0, 0.5]).collect();
        let pts = b.resample_path(&path, 100, 100, 1.0);
        assert!(
            pts.len() >= 20,
            "short segments must still produce dabs: {}",
            pts.len()
        );
        for w in pts.windows(2) {
            let dx = (w[1][0] - w[0][0]) * 100.0;
            assert!(dx <= 5.0 + 1e-3, "gap {dx} px");
        }
    }

    #[test]
    fn spots_along_covers_path_endpoints() {
        let b = BrushSettings {
            size: 10.0,
            spacing: 0.5,
            ..Default::default()
        };
        let path = [[0.0, 0.5], [0.5, 0.5], [1.0, 0.5]];
        let spots = b.spots_along(&path, 100, 100);
        assert!(!spots.is_empty());
        assert_eq!(spots.first().unwrap().center, [0.0, 0.5]);
        assert_eq!(spots.last().unwrap().center, [1.0, 0.5]);
        // 100 px path, 5 px step → at least 20 dabs.
        assert!(spots.len() >= 20, "{} spots", spots.len());
    }

    /// The coverage outline must be the smooth capsule swept by the brush, not
    /// the scalloped union of discrete dabs. On a straight stroke, every column
    /// at the same distance from the line must have the same coverage.
    #[test]
    fn stroke_coverage_edge_is_smooth() {
        let b = BrushSettings {
            size: 40.0,
            hardness: 0.4,
            spacing: 0.25,
            ..Default::default()
        };
        let (gw, gh) = (200usize, 100usize);
        let cov = b
            .stroke_coverage(&[[0.2f32, 0.5], [0.8, 0.5]], gw, gh, 1.0)
            .unwrap();
        let x0 = (cov.area[0] * gw as f32).round() as usize;
        let y0 = (cov.area[1] * gh as f32).round() as usize;
        // Center line is at y = 50; r = 20 and the core is 0.88 r = 17.6, so
        // y = 69 (d = 19) is mid-feather.
        let row = (50 + 19) - y0;
        let start = 45usize.saturating_sub(x0);
        let end = 155usize.saturating_sub(x0).min(cov.w);
        let vals: Vec<f32> = (start..end).map(|x| cov.data[row * cov.w + x]).collect();
        let min = vals.iter().copied().fold(f32::MAX, f32::min);
        let max = vals.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            min > 0.0 && max < 1.0,
            "row must be in the feather band: {min}..{max}"
        );
        assert!(
            max - min < 1.0e-3,
            "coverage edge is scalloped: {min}..{max}"
        );
    }

    /// A fully hard brush must still carry a one-pixel anti-aliased ramp at the
    /// outline, otherwise the edge becomes stair steps once magnified.
    #[test]
    fn stroke_coverage_hard_edge_is_anti_aliased() {
        let b = BrushSettings {
            size: 40.0,
            hardness: 1.0,
            spacing: 0.25,
            ..Default::default()
        };
        let (gw, gh) = (200usize, 100usize);
        let cov = b
            .stroke_coverage(&[[0.2f32, 0.5], [0.8, 0.5]], gw, gh, 1.0)
            .unwrap();
        let x0 = (cov.area[0] * gw as f32).round() as usize;
        let y0 = (cov.area[1] * gh as f32).round() as usize;
        // r = 20; y = 69 gives d = 19.5, i.e. inside the [19, 20] AA band.
        let row = 69 - y0;
        let x = 100 - x0;
        let v = cov.data[row * cov.w + x];
        assert!(v > 0.05 && v < 0.95, "hard edge must be anti-aliased, got {v}");
    }

    #[test]
    fn to_stroke_carries_the_path() {
        let b = BrushSettings {
            size: 10.0,
            spacing: 0.5,
            ..Default::default()
        };
        let s = b.to_stroke(&[[0.0, 0.5], [1.0, 0.5]], 100, 100);
        let path = s.path.expect("a stroke carries its path");
        // 100 px at a 5 px step -> at least 20 dab centres.
        assert!(path.len() >= 20, "{} centres", path.len());
        assert_eq!(s.center, path[0]);
        assert!((s.radius - b.radius_fraction(100, 100)).abs() < 1e-6);
        // A plain dab has no path.
        assert!(b.to_spot([0.5, 0.5], 100, 100).path.is_none());
    }

    #[test]
    fn stroke_coverage_is_a_union_not_a_sum() {
        let b = BrushSettings {
            size: 20.0,
            hardness: 1.0,
            spacing: 0.25,
            ..Default::default()
        };
        // A path that folds back onto itself: the coverage must stay ≤ 1.
        let path = [[0.3, 0.5], [0.7, 0.5], [0.3, 0.5]];
        let cov = b.stroke_coverage(&path, 100, 100, 1.0).unwrap();
        assert!(
            cov.data.iter().all(|&v| v <= 1.0 + 1e-6),
            "coverage must not accumulate"
        );
        assert!(cov.data.iter().any(|&v| v > 0.9), "there must be a solid core");
        // The bbox stays normalized inside the image.
        assert!(cov.area[0] >= 0.0 && cov.area[2] <= 1.0);
        assert!(cov.area[1] >= 0.0 && cov.area[3] <= 1.0);
    }
}
