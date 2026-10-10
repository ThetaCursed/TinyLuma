// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Tone curves: a master RGB curve plus per-channel R/G/B curves.
//!
//! Applied on display-referred values inside the color pass, so they bake into
//! the combined 3D LUT like every other control. Interpolation is a monotone
//! cubic Hermite with Fritsch–Carlson tangent limiting (the shape RapidRAW
//! uses), so an S-curve never overshoots.
//!
//! The storage is a fixed `[[f32; 2]; CURVE_MAX_POINTS]` + length, so
//! `FilterSettings` stays `Copy`. A curve serializes as a short point list, so
//! presets stay readable.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The most points a curve can hold.
pub(crate) const CURVE_MAX_POINTS: usize = 16;
/// Minimum spacing between consecutive control-point inputs.
const MIN_INPUT_SPACING: f32 = 1.0 / 4096.0;

/// Which curve of a [`ToneCurves`] the editor is changing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CurveChannel {
    Master,
    Red,
    Green,
    Blue,
}

/// A single monotone point curve, points in `0..1`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Curve {
    points: [[f32; 2]; CURVE_MAX_POINTS],
    len: u8,
}

impl Curve {
    /// A straight diagonal (the neutral curve).
    pub(crate) fn identity() -> Self {
        let mut points = [[0.0f32; 2]; CURVE_MAX_POINTS];
        points[0] = [0.0, 0.0];
        points[1] = [1.0, 1.0];
        Self { points, len: 2 }
    }

    /// A curve from a point list: clamped, sorted by `x`, near-duplicate inputs
    /// dropped. Falls back to the identity when fewer than two points remain.
    pub(crate) fn from_points(pts: &[[f32; 2]]) -> Self {
        let mut curve = Self::identity();
        let take = pts.len().min(CURVE_MAX_POINTS);
        if take < 2 {
            return curve;
        }
        let mut sorted = [[0.0f32; 2]; CURVE_MAX_POINTS];
        for (dst, p) in sorted.iter_mut().zip(&pts[..take]) {
            *dst = [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)];
        }
        sorted[..take].sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut out = [[0.0f32; 2]; CURVE_MAX_POINTS];
        let mut m = 0usize;
        for p in &sorted[..take] {
            if m > 0 && p[0] - out[m - 1][0] < MIN_INPUT_SPACING {
                continue;
            }
            out[m] = *p;
            m += 1;
        }
        if m < 2 {
            return curve;
        }
        curve.points = out;
        curve.len = m as u8;
        curve
    }

    /// The used control points.
    pub(crate) fn points(&self) -> &[[f32; 2]] {
        &self.points[..self.len as usize]
    }

    /// The neutral diagonal: endpoints span 0–1 and every point lies on `y = x`.
    pub(crate) fn is_identity(&self) -> bool {
        let n = self.len as usize;
        self.points[0][0].abs() < 1e-3
            && (self.points[n - 1][0] - 1.0).abs() < 1e-3
            && self.points[..n].iter().all(|p| (p[0] - p[1]).abs() < 1e-3)
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::identity();
    }

    /// Inserts a point (clamped), returning its index. A point that lands on an
    /// existing input returns that point instead; a full curve is left as is.
    pub(crate) fn insert(&mut self, p: [f32; 2]) -> usize {
        let x = p[0].clamp(0.0, 1.0);
        let y = p[1].clamp(0.0, 1.0);
        let n = self.len as usize;
        for i in 0..n {
            if (self.points[i][0] - x).abs() < MIN_INPUT_SPACING {
                return i;
            }
        }
        let mut idx = n;
        for i in 0..n {
            if self.points[i][0] > x {
                idx = i;
                break;
            }
        }
        if n == CURVE_MAX_POINTS {
            return idx.min(CURVE_MAX_POINTS - 1);
        }
        for j in (idx..n).rev() {
            self.points[j + 1] = self.points[j];
        }
        self.points[idx] = [x, y];
        self.len += 1;
        idx
    }

    /// Moves a point. The endpoints keep their input fixed (0 and 1) but may
    /// move vertically; an interior point stays strictly between its neighbours.
    pub(crate) fn move_point(&mut self, i: usize, p: [f32; 2]) {
        let n = self.len as usize;
        if i >= n {
            return;
        }
        let y = p[1].clamp(0.0, 1.0);
        let x = if i == 0 {
            0.0
        } else if i + 1 == n {
            1.0
        } else {
            let lo = self.points[i - 1][0] + MIN_INPUT_SPACING;
            let hi = self.points[i + 1][0] - MIN_INPUT_SPACING;
            if lo <= hi {
                p[0].clamp(lo, hi)
            } else {
                self.points[i][0]
            }
        };
        self.points[i] = [x, y];
    }

    /// Removes an interior point; the endpoints cannot be removed.
    pub(crate) fn remove(&mut self, i: usize) -> bool {
        let n = self.len as usize;
        if i == 0 || i + 1 >= n {
            return false;
        }
        for j in i..n - 1 {
            self.points[j] = self.points[j + 1];
        }
        self.points[n - 1] = [0.0, 0.0];
        self.len -= 1;
        true
    }

    fn slope(&self, i: usize) -> f32 {
        let a = self.points[i];
        let b = self.points[i + 1];
        (b[1] - a[1]) / (b[0] - a[0]).max(1e-6)
    }

    /// Monotone (Fritsch–Carlson) tangent at point `i`.
    fn tangent(&self, i: usize) -> f32 {
        let n = self.len as usize;
        if i == 0 {
            return self.slope(0);
        }
        if i + 1 == n {
            return self.slope(n - 2);
        }
        let a = self.slope(i - 1);
        let b = self.slope(i);
        if a * b <= 0.0 {
            return 0.0;
        }
        let h0 = self.points[i][0] - self.points[i - 1][0];
        let h1 = self.points[i + 1][0] - self.points[i][0];
        let w0 = 2.0 * h1 + h0;
        let w1 = h1 + 2.0 * h0;
        (w0 + w1) / (w0 / a + w1 / b)
    }

    /// Evaluates the curve at `x` (`0..1`).
    pub(crate) fn evaluate(&self, x: f32) -> f32 {
        let n = self.len as usize;
        let pts = &self.points[..n];
        let x = x.clamp(0.0, 1.0);
        if x <= pts[0][0] {
            return pts[0][1];
        }
        if x >= pts[n - 1][0] {
            return pts[n - 1][1];
        }
        let mut i = 0;
        while i + 1 < n && x > pts[i + 1][0] {
            i += 1;
        }
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let h = (p2[0] - p1[0]).max(1e-6);
        let t = (x - p1[0]) / h;
        let m1 = self.tangent(i);
        let m2 = self.tangent(i + 1);
        let t2 = t * t;
        let t3 = t2 * t;
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * p1[1]
            + (t3 - 2.0 * t2 + t) * h * m1
            + (-2.0 * t3 + 3.0 * t2) * p2[1]
            + (t3 - t2) * h * m2;
        // Belt and braces with the tangent limiter: never leave the segment range.
        y.clamp(p1[1].min(p2[1]), p1[1].max(p2[1]))
    }
}

impl Default for Curve {
    fn default() -> Self {
        Self::identity()
    }
}

/// Equality is over the *used* points, so stale slots left by `remove` can never
/// make a neutral curve compare unequal to the default.
impl PartialEq for Curve {
    fn eq(&self, other: &Self) -> bool {
        self.points() == other.points()
    }
}

impl Serialize for Curve {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.points().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Curve {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pts = Vec::<[f32; 2]>::deserialize(deserializer)?;
        Ok(Self::from_points(&pts))
    }
}

/// The master curve plus the three per-channel curves.
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ToneCurves {
    pub(crate) master: Curve,
    pub(crate) red: Curve,
    pub(crate) green: Curve,
    pub(crate) blue: Curve,
}

impl ToneCurves {
    pub(crate) fn is_identity(&self) -> bool {
        self.master.is_identity()
            && self.red.is_identity()
            && self.green.is_identity()
            && self.blue.is_identity()
    }

    pub(crate) fn curve(&self, ch: CurveChannel) -> &Curve {
        match ch {
            CurveChannel::Master => &self.master,
            CurveChannel::Red => &self.red,
            CurveChannel::Green => &self.green,
            CurveChannel::Blue => &self.blue,
        }
    }

    pub(crate) fn curve_mut(&mut self, ch: CurveChannel) -> &mut Curve {
        match ch {
            CurveChannel::Master => &mut self.master,
            CurveChannel::Red => &mut self.red,
            CurveChannel::Green => &mut self.green,
            CurveChannel::Blue => &mut self.blue,
        }
    }

    /// Bakes the composed per-channel transfer into 256-sample tables:
    /// `table_c[i] = channel_c(master(i / 255))`.
    pub(crate) fn bake(&self) -> CurveTables {
        let mut tables = CurveTables {
            r: [0.0; 256],
            g: [0.0; 256],
            b: [0.0; 256],
        };
        for i in 0..256 {
            let x = i as f32 / 255.0;
            let m = self.master.evaluate(x);
            tables.r[i] = self.red.evaluate(m);
            tables.g[i] = self.green.evaluate(m);
            tables.b[i] = self.blue.evaluate(m);
        }
        tables
    }
}

/// 256-sample per-channel transfer tables: the curve, ready for per-pixel use.
pub(crate) struct CurveTables {
    pub(crate) r: [f32; 256],
    pub(crate) g: [f32; 256],
    pub(crate) b: [f32; 256],
}

impl CurveTables {
    /// Reads a channel table with linear interpolation.
    #[inline]
    pub(crate) fn read(table: &[f32; 256], v: f32) -> f32 {
        let x = v.clamp(0.0, 1.0) * 255.0;
        let i = x as usize;
        if i >= 255 {
            return table[255];
        }
        let f = x - i as f32;
        table[i] + (table[i + 1] - table[i]) * f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_a_no_op() {
        let c = Curve::identity();
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            assert!((c.evaluate(x) - x).abs() < 1e-4, "{x}");
        }
        assert!(c.is_identity());
    }

    #[test]
    fn two_point_curve_interpolates_linearly() {
        // A straight line from (0,0) to (1,0.5): y = x/2.
        let c = Curve::from_points(&[[0.0, 0.0], [1.0, 0.5]]);
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((c.evaluate(x) - x * 0.5).abs() < 1e-4, "{x}");
        }
    }

    #[test]
    fn s_curve_is_monotone_and_does_not_overshoot() {
        let c = Curve::from_points(&[[0.0, 0.0], [0.25, 0.1], [0.75, 0.9], [1.0, 1.0]]);
        let mut prev = -1.0;
        for i in 0..=1000 {
            let y = c.evaluate(i as f32 / 1000.0);
            assert!(y >= prev - 1e-5, "{y} < {prev}");
            assert!((0.0..=1.0).contains(&y));
            prev = y;
        }
    }

    #[test]
    fn endpoints_move_vertically_but_not_horizontally() {
        let mut c = Curve::identity();
        c.move_point(0, [0.3, 0.05]);
        assert_eq!(c.points()[0], [0.0, 0.05]);
        c.move_point(1, [0.7, 0.9]);
        assert_eq!(c.points()[1], [1.0, 0.9]);
    }

    #[test]
    fn insert_move_remove_returns_to_identity() {
        let mut c = Curve::identity();
        let i = c.insert([0.4, 0.3]);
        assert_eq!(i, 1);
        assert_eq!(c.points().len(), 3);
        c.move_point(i, [0.6, 0.7]);
        assert_eq!(c.points()[1], [0.6, 0.7]);
        assert!(!c.remove(0));
        assert!(c.remove(1));
        assert_eq!(c.points().len(), 2);
        assert_eq!(c, Curve::identity());
    }

    #[test]
    fn bake_identity_is_the_identity_table() {
        let t = ToneCurves::default().bake();
        for i in 0..256 {
            let x = i as f32 / 255.0;
            assert!((CurveTables::read(&t.r, x) - x).abs() < 1e-4);
            assert!((CurveTables::read(&t.g, x) - x).abs() < 1e-4);
            assert!((CurveTables::read(&t.b, x) - x).abs() < 1e-4);
        }
    }

    #[test]
    fn channel_curve_composes_after_master() {
        let mut c = ToneCurves::default();
        // Master halves everything; red identity so it passes the master through.
        c.master = Curve::from_points(&[[0.0, 0.0], [1.0, 0.5]]);
        let t = c.bake();
        assert!((CurveTables::read(&t.r, 1.0) - 0.5).abs() < 1e-3);
        assert!((CurveTables::read(&t.g, 1.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn identity_detection_uses_all_channels() {
        assert!(ToneCurves::default().is_identity());
        let mut c = ToneCurves::default();
        c.red = Curve::from_points(&[[0.0, 0.0], [1.0, 0.9]]);
        assert!(!c.is_identity());
    }
}
