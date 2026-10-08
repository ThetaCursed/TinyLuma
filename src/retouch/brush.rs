// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Brush settings and stroke dabbing.

use super::spot::{Spot, SpotKind};

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

/// Rasterized union coverage of a painted path, ready to be tinted and drawn.
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
    pub(crate) fn to_spot(&self, center: [f32; 2], w: usize, h: usize) -> Spot {
        Spot {
            center,
            radius: self.radius_fraction(w, h),
            hardness: self.hardness,
            opacity: 1.0,
            kind: self.kind,
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

    /// Rasterizes the union of the brush along `path` into a compact coverage
    /// map (bounding box only). The union is taken with `max`, so overlapping
    /// dabs never accumulate — the indication stays a single flat region.
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
        let mut data = vec![0.0f32; bw * bh];

        // Uniform dab centers along the whole path.
        let centers = self.resample_path(path, grid_w, grid_h, scale);

        let inner = self.hardness.clamp(0.0, 1.0) * r;
        let feather = (r - inner).max(1.0e-3);
        for c in centers {
            let cx = c[0] * fw - x0 as f32;
            let cy = c[1] * fh - y0 as f32;
            let px0 = (cx - r).floor().max(0.0) as usize;
            let py0 = (cy - r).floor().max(0.0) as usize;
            let px1 = ((cx + r).ceil() as isize + 1).clamp(0, bw as isize) as usize;
            let py1 = ((cy + r).ceil() as isize + 1).clamp(0, bh as isize) as usize;
            for y in py0..py1 {
                let dy = y as f32 + 0.5 - cy;
                for x in px0..px1 {
                    let dx = x as f32 + 0.5 - cx;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d >= r {
                        continue;
                    }
                    let cov = if d <= inner {
                        1.0
                    } else {
                        let t = ((r - d) / feather).clamp(0.0, 1.0);
                        t * t * (3.0 - 2.0 * t)
                    };
                    let idx = y * bw + x;
                    if cov > data[idx] {
                        data[idx] = cov;
                    }
                }
            }
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

    /// Stamps a whole painted path: one spot per `spacing * size` pixels plus the
    /// end points. This is what a released stroke is turned into.
    pub(crate) fn spots_along(&self, path: &[[f32; 2]], w: usize, h: usize) -> Vec<Spot> {
        self.resample_path(path, w, h, 1.0)
            .into_iter()
            .map(|p| self.to_spot(p, w, h))
            .collect()
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
