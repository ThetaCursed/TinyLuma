// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Image orientation: the 8 symmetries of a rectangle (rotations by 90° plus
//! mirroring). Stored canonically as "mirror horizontally, then rotate N×90°
//! clockwise", which is a complete representation of the dihedral group D4 and
//! makes composition a few arithmetic lines.

use super::geom::NormRect;

/// A right-angle orientation: mirror + quarter turns. `Copy`, `Eq`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Orientation {
    /// Number of 90° clockwise turns, `0..3`.
    pub turns: u8,
    /// Mirror horizontally (before the rotation).
    pub mirrored: bool,
}

impl Orientation {
    pub(crate) const IDENTITY: Orientation = Orientation {
        turns: 0,
        mirrored: false,
    };

    pub(crate) fn is_identity(&self) -> bool {
        self.turns == 0 && !self.mirrored
    }

    /// Whether a rotation by 90°/270° swaps the image dimensions.
    pub(crate) fn swaps_dims(&self) -> bool {
        self.turns % 2 == 1
    }

    /// Pixel dimensions of the oriented image.
    pub(crate) fn dimensions(&self, w: u32, h: u32) -> (u32, u32) {
        if self.swaps_dims() {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Applies a clockwise 90° turn to the *displayed* image:
    /// `orientation = R(+1) ∘ orientation`.
    pub(crate) fn rotate_cw(self) -> Orientation {
        Orientation {
            turns: (self.turns + 1) % 4,
            mirrored: self.mirrored,
        }
    }

    /// Counter-clockwise 90°: `R(-1) ∘ orientation`.
    pub(crate) fn rotate_ccw(self) -> Orientation {
        Orientation {
            turns: (self.turns + 3) % 4,
            mirrored: self.mirrored,
        }
    }

    /// Horizontal mirror of the displayed image: `M ∘ orientation`.
    /// `M·R(q) = R(-q)·M`, so the turn count inverts and the mirror flips.
    pub(crate) fn flip_h(self) -> Orientation {
        Orientation {
            turns: (4 - self.turns) % 4,
            mirrored: !self.mirrored,
        }
    }

    /// Vertical mirror: `R(2)·M ∘ orientation`.
    pub(crate) fn flip_v(self) -> Orientation {
        Orientation {
            turns: (6 - self.turns) % 4,
            mirrored: !self.mirrored,
        }
    }

    /// Maps a point from oriented space back to the source texture (`0..1`).
    /// Inverse of the stored forward map `R(turns) ∘ M(mirrored)`.
    pub(crate) fn to_texture(&self, x: f32, y: f32) -> (f32, f32) {
        // Undo the rotation (R(-q)), then undo the mirror.
        let mut p = (x, y);
        for _ in 0..self.turns {
            p = rot_ccw(p);
        }
        if self.mirrored {
            p.0 = 1.0 - p.0;
        }
        p
    }

    /// Produces the oriented RGB8 buffer (used by export).
    pub(crate) fn apply_rgb(&self, buf: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
        if self.is_identity() {
            return (buf.to_vec(), w, h);
        }
        let (ow, oh) = self.dimensions(w, h);
        let mut out = vec![0u8; buf.len()];
        for oy in 0..oh {
            for ox in 0..ow {
                let nx = (ox as f32 + 0.5) / ow as f32;
                let ny = (oy as f32 + 0.5) / oh as f32;
                let (tx, ty) = self.to_texture(nx, ny);
                let sx = ((tx * w as f32).floor() as i32).clamp(0, w as i32 - 1) as u32;
                let sy = ((ty * h as f32).floor() as i32).clamp(0, h as i32 - 1) as u32;
                let si = ((sy * w + sx) * 3) as usize;
                let di = ((oy * ow + ox) * 3) as usize;
                out[di..di + 3].copy_from_slice(&buf[si..si + 3]);
            }
        }
        (out, ow, oh)
    }
}

/// A point rotated 90° counter-clockwise: `(x, y) → (y, 1 - x)`.
fn rot_ccw(p: (f32, f32)) -> (f32, f32) {
    (p.1, 1.0 - p.0)
}

/// Rotates a frame 90° clockwise (content-preserving).
pub(crate) fn rotate_rect_cw(r: NormRect) -> NormRect {
    NormRect {
        x: 1.0 - (r.y + r.h),
        y: r.x,
        w: r.h,
        h: r.w,
    }
}

/// Rotates a frame 90° counter-clockwise.
pub(crate) fn rotate_rect_ccw(r: NormRect) -> NormRect {
    NormRect {
        x: r.y,
        y: 1.0 - (r.x + r.w),
        w: r.h,
        h: r.w,
    }
}

/// Mirrors a frame horizontally.
pub(crate) fn flip_rect_h(r: NormRect) -> NormRect {
    NormRect {
        x: 1.0 - (r.x + r.w),
        ..r
    }
}

/// Mirrors a frame vertically.
pub(crate) fn flip_rect_v(r: NormRect) -> NormRect {
    NormRect {
        y: 1.0 - (r.y + r.h),
        ..r
    }
}

/// Rotates a point around the image center `(0.5, 0.5)` by `deg` degrees
/// (positive = clockwise in screen space, `y` down). `aspect` is the image's
/// `width / height`: the rotation is a true Euclidean rotation, so it must be
/// applied in physical (pixel) space — normalizing both axes to `0..1` and then
/// rotating would shear a non-square image.
pub(crate) fn rotate_point(x: f32, y: f32, deg: f32, aspect: f32) -> (f32, f32) {
    let aspect = if aspect > 0.0 { aspect } else { 1.0 };
    let r = deg.to_radians();
    let (s, c) = r.sin_cos();
    let dx = x - 0.5;
    let dy = y - 0.5;
    // x is measured in `width` units, y in `height` units: scale y by `W/H`
    // before rotating and undo it afterwards.
    (
        0.5 + c * dx - s * dy / aspect,
        0.5 + s * dx * aspect + c * dy,
    )
}

/// Whether a frame has no empty corners after a straighten rotation, i.e. all
/// its corners map back inside the source image. `aspect` is `width / height`.
pub(crate) fn rect_inside_rotated(rect: NormRect, angle: f32, aspect: f32) -> bool {
    let corners = [
        (rect.x, rect.y),
        (rect.x + rect.w, rect.y),
        (rect.x + rect.w, rect.y + rect.h),
        (rect.x, rect.y + rect.h),
    ];
    const EPS: f32 = 1e-3;
    corners.iter().all(|&(x, y)| {
        let (sx, sy) = rotate_point(x, y, -angle, aspect);
        sx >= -EPS && sx <= 1.0 + EPS && sy >= -EPS && sy <= 1.0 + EPS
    })
}

fn scale_rect_about(rect: NormRect, cx: f32, cy: f32, s: f32) -> NormRect {
    NormRect {
        x: cx + (rect.x - cx) * s,
        y: cy + (rect.y - cy) * s,
        w: rect.w * s,
        h: rect.h * s,
    }
}

/// Shrinks `rect` toward its center until it lies fully inside the image rotated
/// by `angle`. Used so a straighten never reveals empty corners. `aspect` is
/// `width / height`, so the fit accounts for the true (non-sheared) rotation.
pub(crate) fn fit_rect_inside_rotated(rect: NormRect, angle: f32, aspect: f32) -> NormRect {
    if angle.abs() < 1e-4 || rect_inside_rotated(rect, angle, aspect) {
        return rect;
    }
    let (cx, cy) = rect.center();
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    let mut best = rect;
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        let cand = scale_rect_about(rect, cx, cy, mid);
        if rect_inside_rotated(cand, angle, aspect) {
            best = cand;
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mut out = best;
    out.clamp_to_unit();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> NormRect {
        NormRect { x, y, w, h }
    }

    #[test]
    fn identity_is_identity() {
        assert!(Orientation::IDENTITY.is_identity());
        assert!(!Orientation::IDENTITY.rotate_cw().is_identity());
    }

    #[test]
    fn four_turns_return_home() {
        let o = Orientation::IDENTITY;
        let o = o.rotate_cw().rotate_cw().rotate_cw().rotate_cw();
        assert!(o.is_identity());
    }

    #[test]
    fn flip_h_applied_twice_is_identity() {
        let o = Orientation::IDENTITY.flip_h().flip_h();
        assert!(o.is_identity(), "{o:?}");
    }

    #[test]
    fn flip_v_equals_rotate_180_then_flip_h() {
        // R(2)∘M == flip_v
        let a = Orientation::IDENTITY.rotate_cw().rotate_cw().flip_h();
        let b = Orientation::IDENTITY.flip_v();
        assert_eq!(a, b);
    }

    #[test]
    fn dimensions_swap_on_odd_turns() {
        let o = Orientation::IDENTITY.rotate_cw();
        assert_eq!(o.dimensions(800, 600), (600, 800));
        let o = o.rotate_cw();
        assert_eq!(o.dimensions(800, 600), (800, 600));
    }

    #[test]
    fn rotate_cw_maps_orientation_corners() {
        // Oriented top-left must sample the texture bottom-left after a CW turn.
        let o = Orientation::IDENTITY.rotate_cw();
        let (tx, ty) = o.to_texture(0.0, 0.0);
        assert!(approx(tx, 0.0) && approx(ty, 1.0), "{tx},{ty}");
        let (tx, ty) = o.to_texture(1.0, 0.0);
        assert!(approx(tx, 0.0) && approx(ty, 0.0), "{tx},{ty}");
    }

    #[test]
    fn flip_h_maps_x_only() {
        let o = Orientation::IDENTITY.flip_h();
        let (tx, ty) = o.to_texture(0.0, 0.25);
        assert!(approx(tx, 1.0) && approx(ty, 0.25), "{tx},{ty}");
    }

    #[test]
    fn rotate_rect_cw_keeps_content() {
        // A frame in the top-left corner stays top-left after CW (content moves
        // with the image, so its normalized place moves to the top-right band).
        let r = rect(0.0, 0.0, 0.25, 0.5);
        let out = rotate_rect_cw(r);
        // Point map: (0,0)->(1,0); (0.25,0.5)->(0.5,0.25).
        assert!(approx(out.x, 0.5) && approx(out.y, 0.0));
        assert!(approx(out.w, 0.5) && approx(out.h, 0.25));
    }

    #[test]
    fn rect_flips_mirror_around_center() {
        let r = rect(0.1, 0.2, 0.3, 0.4);
        let fh = flip_rect_h(r);
        assert!(approx(fh.x, 0.6) && approx(fh.y, 0.2) && approx(fh.w, 0.3));
        let fv = flip_rect_v(r);
        assert!(approx(fv.x, 0.1) && approx(fv.y, 0.4) && approx(fv.h, 0.4));
    }

    #[test]
    fn apply_rgb_rotates_clockwise() {
        // 2x1 texture: [A, B]. Rotating CW gives a 1x2 image with [A] on top.
        let buf = vec![10, 10, 10, 20, 20, 20];
        let o = Orientation::IDENTITY.rotate_cw();
        let (out, w, h) = o.apply_rgb(&buf, 2, 1);
        assert_eq!((w, h), (1, 2));
        // Oriented top pixel samples texture bottom-left (A); bottom samples B.
        assert_eq!(&out[0..3], &[10, 10, 10]);
        assert_eq!(&out[3..6], &[20, 20, 20]);
    }

    #[test]
    fn apply_rgb_flip_h_reverses_a_row() {
        let buf = vec![1, 1, 1, 2, 2, 2, 3, 3, 3];
        let o = Orientation::IDENTITY.flip_h();
        let (out, w, h) = o.apply_rgb(&buf, 3, 1);
        assert_eq!((w, h), (3, 1));
        assert_eq!(&out[0..3], &[3, 3, 3]);
        assert_eq!(&out[6..9], &[1, 1, 1]);
    }

    #[test]
    fn apply_rgb_is_identity_when_identity() {
        let buf = vec![1, 2, 3, 4, 5, 6];
        let (out, w, h) = Orientation::IDENTITY.apply_rgb(&buf, 2, 1);
        assert_eq!((out, w, h), (buf, 2, 1));
    }

    #[test]
    fn apply_rgb_preserves_length() {
        let buf = vec![7u8; 4 * 6 * 3];
        for o in [
            Orientation::IDENTITY.rotate_cw(),
            Orientation::IDENTITY.rotate_cw().rotate_cw().rotate_cw(),
            Orientation::IDENTITY.flip_h(),
            Orientation::IDENTITY.flip_v(),
        ] {
            let (out, w, h) = o.apply_rgb(&buf, 4, 6);
            assert_eq!(out.len(), buf.len());
            assert_eq!((w, h), o.dimensions(4, 6));
        }
    }

    #[test]
    fn fit_shrinks_a_full_frame_when_tilted() {
        let fitted = fit_rect_inside_rotated(NormRect::FULL, 10.0, 1.0);
        assert!(fitted.w < 1.0 && fitted.h < 1.0);
        assert!(rect_inside_rotated(fitted, 10.0, 1.0));
        // Still centered.
        assert!((fitted.x + fitted.w * 0.5 - 0.5).abs() < 1e-3);
    }

    #[test]
    fn fit_keeps_a_valid_frame_untouched() {
        // A centered half-size frame survives a small tilt.
        let rect = NormRect {
            x: 0.25,
            y: 0.25,
            w: 0.5,
            h: 0.5,
        };
        let fitted = fit_rect_inside_rotated(rect, 5.0, 1.0);
        assert_eq!(fitted, rect);
    }

    #[test]
    fn fit_is_identity_without_angle() {
        assert_eq!(
            fit_rect_inside_rotated(NormRect::FULL, 0.0, 1.5),
            NormRect::FULL
        );
    }

    #[test]
    fn rotation_is_aspect_correct_not_a_shear() {
        // 2:1 image (aspect 2), rotate the top-right corner by 90° CW. In pixel
        // space the corner (W, 0) must land at (W, H) after one CW quarter-ish
        // rotation; here we check a small angle in *pixel* space for equality
        // with a direct pixel-space rotation.
        let aspect = 2.0f32;
        let (x, y) = (0.75f32, 0.2f32);
        let deg = 17.0f32;
        let (rx, ry) = rotate_point(x, y, deg, aspect);
        // Expected: rotate around (0.5, 0.5) with y scaled by `aspect`.
        let r = deg.to_radians();
        let (s, c) = r.sin_cos();
        let dx = (x - 0.5) * aspect;
        let dy = y - 0.5;
        let ex = 0.5 + (c * dx - s * dy) / aspect;
        let ey = 0.5 + (s * dx + c * dy);
        assert!(approx(rx, ex) && approx(ry, ey), "{rx},{ry} vs {ex},{ey}");
    }

    #[test]
    fn inverse_rotation_round_trips() {
        for aspect in [0.5f32, 1.0, 1.7777, 3.0] {
            let (x, y) = (0.31f32, 0.62f32);
            let (rx, ry) = rotate_point(x, y, 23.0, aspect);
            let (bx, by) = rotate_point(rx, ry, -23.0, aspect);
            assert!(approx(bx, x) && approx(by, y), "aspect {aspect}");
        }
    }

    #[test]
    fn refitting_from_the_same_base_grows_back() {
        // The app keeps an un-straightened base and refits from it for every
        // angle, so lowering the angle restores the frame instead of only ever
        // shrinking it.
        let aspect = 1.7777f32;
        let base = NormRect::FULL;
        let a10 = fit_rect_inside_rotated(base, 10.0, aspect);
        let a5 = fit_rect_inside_rotated(base, 5.0, aspect);
        let a0 = fit_rect_inside_rotated(base, 0.0, aspect);
        assert!(a10.w < a5.w, "{} vs {}", a10.w, a5.w);
        assert!(a5.w < a0.w);
        assert_eq!(a0, NormRect::FULL);
    }

    #[test]
    fn flip_maps_a_tilted_fit_onto_the_negated_angle_fit() {
        // A horizontal mirror must mirror the fit as well: flipping the base and
        // negating the angle gives the same frame as flipping the fitted frame.
        let aspect = 1.7777f32;
        let base = NormRect {
            x: 0.1,
            y: 0.2,
            w: 0.7,
            h: 0.6,
        };
        let angle = 12.0f32;
        let fitted = fit_rect_inside_rotated(base, angle, aspect);
        let flipped_fit = flip_rect_h(fitted);
        let fit_of_flipped = fit_rect_inside_rotated(flip_rect_h(base), -angle, aspect);
        assert!(approx(flipped_fit.x, fit_of_flipped.x), "{flipped_fit:?}");
        assert!(approx(flipped_fit.y, fit_of_flipped.y));
        assert!(approx(flipped_fit.w, fit_of_flipped.w));
        assert!(approx(flipped_fit.h, fit_of_flipped.h));
    }

    #[test]
    fn fit_matches_the_analytic_scale_for_a_wide_image() {
        // A full frame tilted by θ shrinks uniformly by
        //   k = 1 / (cosθ + sinθ · aspect),
        // the tightest of the corner constraints in pixel space.
        let aspect = 1.7777f32;
        let angle = 10.0f32;
        let r = angle.to_radians();
        let (s, c) = r.sin_cos();
        let k = 1.0 / (c + s * aspect);
        let fitted = fit_rect_inside_rotated(NormRect::FULL, angle, aspect);
        // Tolerance covers the EPS slack of the validity predicate.
        assert!((fitted.w - k).abs() < 2e-3, "{} vs {k}", fitted.w);
        assert!((fitted.h - k).abs() < 2e-3);
        assert!(rect_inside_rotated(fitted, angle, aspect));
    }
}
