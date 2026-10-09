// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Pure crop geometry: normalized rectangles, aspect presets and the math that
//! keeps a crop frame inside the image. No egui, no app state, no pixels — so
//! every rule here is unit-testable on plain numbers.
//!
//! Coordinates are normalized to `0..1` in image space (origin top-left, `y`
//! down). Normalized aspect is *not* the same as the pixel aspect: for an image
//! of ratio `R = W/H`, a frame with pixel ratio `r` has `w/h = r / R`.

/// A rectangle in normalized image space, `0..1`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct NormRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A crop frame handle (8 sides/corners).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Handle {
    N,
    S,
    W,
    E,
    Nw,
    Ne,
    Sw,
    Se,
}

/// The smallest side a crop frame may reach, in normalized units (~2%).
pub(crate) const MIN_SIDE: f32 = 0.02;

impl NormRect {
    /// The whole image.
    pub const FULL: NormRect = NormRect {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    pub(crate) fn is_full(&self) -> bool {
        const EPS: f32 = 1e-4;
        self.x.abs() < EPS
            && self.y.abs() < EPS
            && (self.w - 1.0).abs() < EPS
            && (self.h - 1.0).abs() < EPS
    }

    pub(crate) fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    /// Shifts the frame so it lies inside `0..1`. Size is preserved when it can
    /// be; an oversized frame is shrunk first.
    pub(crate) fn clamp_to_unit(&mut self) {
        self.w = self.w.clamp(MIN_SIDE, 1.0);
        self.h = self.h.clamp(MIN_SIDE, 1.0);
        self.x = self.x.clamp(0.0, 1.0 - self.w);
        self.y = self.y.clamp(0.0, 1.0 - self.h);
    }

    /// Integer pixel bounds `(x, y, w, h)` of the frame for an image of
    /// `img_w × img_h`. The result is always at least 1×1 and inside the image.
    pub(crate) fn pixel_rect(&self, img_w: u32, img_h: u32) -> (u32, u32, u32, u32) {
        if img_w == 0 || img_h == 0 {
            return (0, 0, 1, 1);
        }
        let fw = img_w as f32;
        let fh = img_h as f32;
        let mut x0 = (self.x * fw).round().clamp(0.0, fw - 1.0);
        let mut y0 = (self.y * fh).round().clamp(0.0, fh - 1.0);
        let mut x1 = ((self.x + self.w) * fw).round().clamp(x0 + 1.0, fw);
        let mut y1 = ((self.y + self.h) * fh).round().clamp(y0 + 1.0, fh);
        // Guard against rounding ordering surprises.
        if x1 <= x0 {
            x0 = (x0 - 1.0).max(0.0);
            x1 = x0 + 1.0;
        }
        if y1 <= y0 {
            y0 = (y0 - 1.0).max(0.0);
            y1 = y0 + 1.0;
        }
        (
            x0 as u32,
            y0 as u32,
            (x1 - x0) as u32,
            (y1 - y0) as u32,
        )
    }
}

/// An aspect-ratio preset. Orientation is explicit (`Fixed(2, 3)` is portrait),
/// so no separate orientation state is needed; a swap button flips the numbers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum AspectPreset {
    Free,
    Original,
    Fixed(u32, u32),
}

impl AspectPreset {
    /// Pixel aspect `width / height`, or `None` for `Free`.
    pub(crate) fn ratio(&self, image_ratio: f32) -> Option<f32> {
        match self {
            AspectPreset::Free => None,
            AspectPreset::Original => Some(image_ratio),
            AspectPreset::Fixed(w, h) => Some(*w as f32 / *h as f32),
        }
    }

    /// Normalized aspect (`w/h` in `0..1` space), or `None` for `Free`.
    pub(crate) fn norm_ratio(&self, image_ratio: f32) -> Option<f32> {
        self.ratio(image_ratio)
            .map(|r| if image_ratio > 0.0 { r / image_ratio } else { r })
    }

    pub(crate) fn label(&self) -> String {
        match self {
            AspectPreset::Free => "Free".to_string(),
            AspectPreset::Original => "Original".to_string(),
            AspectPreset::Fixed(w, h) => format!("{w}:{h}"),
        }
    }

    /// `Fixed(w, h) → Fixed(h, w)`; `Free`/`Original` are unchanged.
    pub(crate) fn swapped(&self) -> AspectPreset {
        match self {
            AspectPreset::Fixed(w, h) => AspectPreset::Fixed(*h, *w),
            other => *other,
        }
    }
}

/// The preset list offered in the UI. One entry per ratio — orientations are
/// explicit, so both `3:2` and `2:3` are present, but no `prefer_portrait`
/// state is needed.
pub(crate) const RATIO_PRESETS: &[AspectPreset] = &[
    AspectPreset::Original,
    AspectPreset::Free,
    AspectPreset::Fixed(1, 1),
    AspectPreset::Fixed(2, 3),
    AspectPreset::Fixed(3, 2),
    AspectPreset::Fixed(3, 4),
    AspectPreset::Fixed(4, 3),
    AspectPreset::Fixed(4, 5),
    AspectPreset::Fixed(5, 4),
    AspectPreset::Fixed(9, 16),
    AspectPreset::Fixed(16, 9),
    AspectPreset::Fixed(21, 9),
];

/// The largest frame with `target_ratio` (pixel aspect) that fits the image,
/// centered. `Original` → the whole image.
pub(crate) fn largest_centered(image_ratio: f32, target_ratio: f32) -> NormRect {
    let ar = if image_ratio > 0.0 {
        target_ratio / image_ratio
    } else {
        target_ratio
    };
    let (w, h) = if ar >= 1.0 {
        (1.0, (1.0 / ar).min(1.0))
    } else {
        (ar.max(MIN_SIDE), 1.0)
    };
    NormRect {
        x: (1.0 - w) * 0.5,
        y: (1.0 - h) * 0.5,
        w,
        h,
    }
}

/// A frame with `target_ratio` that keeps the *pixel area* of `current` and stays
/// centered on it. Falls back to `largest_centered` if the result would not fit.
pub(crate) fn area_preserving(current: NormRect, image_ratio: f32, target_ratio: f32) -> NormRect {
    let ar = if image_ratio > 0.0 {
        target_ratio / image_ratio
    } else {
        target_ratio
    };
    if ar <= 0.0 {
        return largest_centered(image_ratio, target_ratio);
    }
    let area = current.w * current.h;
    let h = (area / ar).sqrt();
    let w = ar * h;
    let (cx, cy) = current.center();
    let mut rect = NormRect {
        x: cx - w * 0.5,
        y: cy - h * 0.5,
        w,
        h,
    };
    rect.clamp_to_unit();
    // A clamped result may no longer have the requested aspect (tiny image or a
    // frame pushed to the edge) — fall back to the centered fit.
    let got = rect.w / rect.h;
    if (got - ar).abs() > 1e-3 {
        return largest_centered(image_ratio, target_ratio);
    }
    rect
}

/// Moves a frame by a normalized delta, clamped to the image.
pub(crate) fn move_rect(rect: NormRect, dx: f32, dy: f32) -> NormRect {
    let mut out = NormRect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    };
    out.clamp_to_unit();
    out
}

/// Resizes `start` by dragging `handle` to `pointer` (both normalized).
///
/// * corner handles keep the opposite corner fixed;
/// * edge handles move one side and grow the other dimension symmetrically
///   when a ratio is locked;
/// * `norm_ar` is the *normalized* aspect (`None` = free).
pub(crate) fn resize_rect(
    start: NormRect,
    handle: Handle,
    pointer: (f32, f32),
    norm_ar: Option<f32>,
) -> NormRect {
    let (px, py) = pointer;
    let left = start.x;
    let right = start.x + start.w;
    let top = start.y;
    let bottom = start.y + start.h;

    // Build the frame from the fixed edges toward the pointer, then lock ratio.
    let lock = |w: f32, h: f32| -> (f32, f32) {
        match norm_ar {
            None => (w, h),
            Some(a) if a > 0.0 => {
                // Follow whichever axis implies the larger frame, so the drag
                // feels natural regardless of which way the pointer went.
                let h_from_w = w / a;
                if h_from_w >= h {
                    (w, h_from_w)
                } else {
                    (h * a, h)
                }
            }
            Some(_) => (w, h),
        }
    };

    let rect = match handle {
        Handle::Se => {
            let (w, h) = lock((px - left).max(MIN_SIDE), (py - top).max(MIN_SIDE));
            NormRect {
                x: left,
                y: top,
                w,
                h,
            }
        }
        Handle::Nw => {
            let (w, h) = lock((right - px).max(MIN_SIDE), (bottom - py).max(MIN_SIDE));
            NormRect {
                x: right - w,
                y: bottom - h,
                w,
                h,
            }
        }
        Handle::Ne => {
            let (w, h) = lock((px - left).max(MIN_SIDE), (bottom - py).max(MIN_SIDE));
            NormRect {
                x: left,
                y: bottom - h,
                w,
                h,
            }
        }
        Handle::Sw => {
            let (w, h) = lock((right - px).max(MIN_SIDE), (py - top).max(MIN_SIDE));
            NormRect {
                x: right - w,
                y: top,
                w,
                h,
            }
        }
        Handle::E => {
            let w = (px - left).max(MIN_SIDE);
            let h = match norm_ar {
                Some(a) if a > 0.0 => w / a,
                _ => start.h,
            };
            NormRect {
                x: left,
                y: top + (start.h - h) * 0.5,
                w,
                h,
            }
        }
        Handle::W => {
            let w = (right - px).max(MIN_SIDE);
            let h = match norm_ar {
                Some(a) if a > 0.0 => w / a,
                _ => start.h,
            };
            NormRect {
                x: right - w,
                y: top + (start.h - h) * 0.5,
                w,
                h,
            }
        }
        Handle::S => {
            let h = (py - top).max(MIN_SIDE);
            let w = match norm_ar {
                Some(a) if a > 0.0 => h * a,
                _ => start.w,
            };
            NormRect {
                x: left + (start.w - w) * 0.5,
                y: top,
                w,
                h,
            }
        }
        Handle::N => {
            let h = (bottom - py).max(MIN_SIDE);
            let w = match norm_ar {
                Some(a) if a > 0.0 => h * a,
                _ => start.w,
            };
            NormRect {
                x: left + (start.w - w) * 0.5,
                y: bottom - h,
                w,
                h,
            }
        }
    };

    let mut rect = rect;
    rect.clamp_to_unit();
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn full_rect_is_identity() {
        assert!(NormRect::FULL.is_full());
        assert!(!largest_centered(1.5, 1.0).is_full());
    }

    #[test]
    fn centered_square_in_wide_image() {
        // 3:2 image (ratio 1.5), square target → normalized ar = 1/1.5 = 0.666.
        let r = largest_centered(1.5, 1.0);
        assert!(approx(r.w, 2.0 / 3.0));
        assert!(approx(r.h, 1.0));
        assert!(approx(r.x, (1.0 - 2.0 / 3.0) / 2.0));
        assert!(approx(r.y, 0.0));
    }

    #[test]
    fn centered_square_in_tall_image() {
        // 2:3 image (ratio 0.666), square → normalized ar = 1.5.
        let r = largest_centered(2.0 / 3.0, 1.0);
        assert!(approx(r.w, 1.0));
        assert!(approx(r.h, 2.0 / 3.0));
    }

    #[test]
    fn original_preset_yields_full_frame() {
        let img = 1.7;
        let r = largest_centered(img, img);
        assert!(r.is_full());
    }

    #[test]
    fn area_preserving_keeps_area() {
        let img = 1.5;
        let start = largest_centered(img, 1.0); // 0.666 x 1.0 → area 0.666
        let area = start.w * start.h;
        let next = area_preserving(start, img, 16.0 / 9.0);
        assert!(approx(next.w * next.h, area));
        // Requested pixel aspect is preserved.
        let pixel_ar = (next.w * img) / next.h;
        assert!(approx(pixel_ar, 16.0 / 9.0));
    }

    #[test]
    fn move_clamps_to_image() {
        let r = NormRect {
            x: 0.8,
            y: 0.8,
            w: 0.5,
            h: 0.5,
        };
        let moved = move_rect(r, 0.5, 0.5);
        assert!(approx(moved.x, 0.5));
        assert!(approx(moved.y, 0.5));
        assert!(approx(moved.w, 0.5));
    }

    #[test]
    fn resize_corner_keeps_ratio() {
        let img = 1.5;
        let ar = AspectPreset::Fixed(1, 1).norm_ratio(img).unwrap();
        let start = NormRect::FULL;
        let r = resize_rect(start, Handle::Se, (0.5, 1.0), Some(ar));
        assert!(approx(r.w / r.h, ar), "{} vs {}", r.w / r.h, ar);
        assert!(approx(r.x, 0.0));
        assert!(approx(r.y, 0.0));
    }

    #[test]
    fn resize_never_leaves_image() {
        let start = NormRect::FULL;
        for &h in &[Handle::N, Handle::S, Handle::W, Handle::E] {
            let r = resize_rect(start, h, (0.5, 0.5), None);
            assert!(r.x >= -1e-4 && r.y >= -1e-4);
            assert!(r.x + r.w <= 1.0 + 1e-4);
            assert!(r.y + r.h <= 1.0 + 1e-4);
            assert!(r.w >= MIN_SIDE - 1e-4 && r.h >= MIN_SIDE - 1e-4);
        }
    }

    #[test]
    fn pixel_rect_maps_normalized_bounds() {
        // Right half of a 100x50 image.
        let r = NormRect {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        };
        assert_eq!(r.pixel_rect(100, 50), (50, 0, 50, 50));
    }

    #[test]
    fn pixel_rect_is_always_positive() {
        let r = NormRect {
            x: 0.999,
            y: 0.999,
            w: 0.02,
            h: 0.02,
        };
        let (x, y, w, h) = r.pixel_rect(10, 10);
        assert!(w >= 1 && h >= 1);
        assert!(x + w <= 10 && y + h <= 10);
    }

    #[test]
    fn swap_flips_fixed_ratio() {
        assert_eq!(
            AspectPreset::Fixed(16, 9).swapped(),
            AspectPreset::Fixed(9, 16)
        );
        assert_eq!(AspectPreset::Original.swapped(), AspectPreset::Original);
    }
}
