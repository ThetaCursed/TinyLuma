// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The undoable crop value and its preset logic.

use super::geom::{area_preserving, largest_centered, AspectPreset, NormRect};
use super::orient::{
    flip_rect_h, flip_rect_v, rotate_rect_ccw, rotate_rect_cw, Orientation,
};

/// The manual straighten range, in degrees.
pub(crate) const MAX_ANGLE: f32 = 45.0;

/// A crop: a normalized frame plus the aspect preset that produced it.
///
/// This is the value stored in snapshots and in the per-image session; it is
/// `Copy`, cheap to clone and cheap to compare. Everything transient (tool
/// active, in-progress drag) lives in the app layer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Crop {
    /// The frame, in *oriented* space (after `orientation` and `angle`).
    pub rect: NormRect,
    pub preset: AspectPreset,
    /// Right-angle rotation / mirror of the whole image.
    pub orientation: Orientation,
    /// Fine straighten angle in degrees (applied after the quarter turns).
    pub angle: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            rect: NormRect::FULL,
            preset: AspectPreset::Original,
            orientation: Orientation::IDENTITY,
            angle: 0.0,
        }
    }
}

impl Crop {
    /// `true` when the crop does not change the image (full frame, no transform).
    pub(crate) fn is_identity(&self) -> bool {
        self.rect.is_full() && self.orientation.is_identity() && self.angle.abs() < 1e-3
    }

    /// Normalized aspect of the active preset (`None` for `Free`).
    pub(crate) fn norm_ratio(&self, image_ratio: f32) -> Option<f32> {
        self.preset.norm_ratio(image_ratio)
    }

    /// Resets everything to the original image: whole frame, `Original` preset,
    /// no right-angle orientation and no straighten angle. Equivalent to a fresh
    /// `Crop::default()`.
    pub(crate) fn reset(&mut self) {
        *self = Crop::default();
    }

    /// Sets the straighten angle (clamped) **without** touching the frame. The
    /// frame fit lives in the app layer, which owns the un-straightened base so
    /// lowering the angle restores the frame instead of only ever shrinking it.
    pub(crate) fn set_angle(&mut self, angle: f32) {
        self.angle = angle.clamp(-MAX_ANGLE, MAX_ANGLE);
    }

    /// Switches the aspect preset, re-fitting the frame:
    /// * `Free` keeps the current frame;
    /// * `Original` returns the full image;
    /// * a fixed ratio keeps the current frame's area and center when it is
    ///   already cropped, otherwise fits the largest centered frame.
    pub(crate) fn set_preset(&mut self, preset: AspectPreset, image_ratio: f32) {
        self.preset = preset;
        self.rect = match preset {
            AspectPreset::Free => self.rect,
            AspectPreset::Original => NormRect::FULL,
            AspectPreset::Fixed(..) => {
                let target = preset.ratio(image_ratio).unwrap_or(1.0);
                if self.rect.is_full() {
                    largest_centered(image_ratio, target)
                } else {
                    area_preserving(self.rect, image_ratio, target)
                }
            }
        };
    }

    /// Flips a fixed ratio (`16:9 → 9:16`) and re-fits the frame.
    #[allow(dead_code)] // Kept as a small pure API; the UI relies on the preset list.
    pub(crate) fn swap_orientation(&mut self, image_ratio: f32) {
        if matches!(self.preset, AspectPreset::Fixed(..)) {
            self.set_preset(self.preset.swapped(), image_ratio);
        }
    }

    /// Rotates the whole image 90° clockwise, carrying the frame with the
    /// content. A fixed aspect preset is swapped so its label keeps matching.
    pub(crate) fn rotate_cw(&mut self) {
        self.rect = rotate_rect_cw(self.rect);
        self.orientation = self.orientation.rotate_cw();
        if matches!(self.preset, AspectPreset::Fixed(..)) {
            self.preset = self.preset.swapped();
        }
    }

    /// Rotates the whole image 90° counter-clockwise.
    pub(crate) fn rotate_ccw(&mut self) {
        self.rect = rotate_rect_ccw(self.rect);
        self.orientation = self.orientation.rotate_ccw();
        if matches!(self.preset, AspectPreset::Fixed(..)) {
            self.preset = self.preset.swapped();
        }
    }

    /// Mirrors the whole image horizontally.
    pub(crate) fn flip_h(&mut self) {
        self.rect = flip_rect_h(self.rect);
        self.orientation = self.orientation.flip_h();
    }

    /// Mirrors the whole image vertically.
    pub(crate) fn flip_v(&mut self) {
        self.rect = flip_rect_v(self.rect);
        self.orientation = self.orientation.flip_v();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_identity() {
        assert!(Crop::default().is_identity());
    }

    #[test]
    fn original_preset_returns_full_frame() {
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(1, 1), 1.5);
        assert!(!c.is_identity());
        c.set_preset(AspectPreset::Original, 1.5);
        assert!(c.is_identity());
    }

    #[test]
    fn free_keeps_frame() {
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(4, 3), 1.5);
        let rect = c.rect;
        c.set_preset(AspectPreset::Free, 1.5);
        assert_eq!(c.rect, rect);
        assert!(c.norm_ratio(1.5).is_none());
    }

    #[test]
    fn swap_flips_frame_too() {
        let img = 1.5;
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(16, 9), img);
        let before = (c.rect.w * img) / c.rect.h;
        c.swap_orientation(img);
        assert_eq!(c.preset, AspectPreset::Fixed(9, 16));
        let after = (c.rect.w * img) / c.rect.h;
        assert!((after - 1.0 / before).abs() < 1e-3, "{after} vs {}", 1.0 / before);
    }

    #[test]
    fn rotate_swaps_fixed_preset_and_frame() {
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(16, 9), 1.5);
        c.rotate_cw();
        assert_eq!(c.preset, AspectPreset::Fixed(9, 16));
        assert!(!c.orientation.is_identity());
        // Rotating four times returns home.
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(3, 2), 1.5);
        c.rotate_cw();
        c.rotate_cw();
        c.rotate_cw();
        c.rotate_cw();
        assert_eq!(c.preset, AspectPreset::Fixed(3, 2));
        assert!(c.orientation.is_identity());
    }

    #[test]
    fn reset_clears_everything() {
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(1, 1), 1.0);
        c.rotate_cw();
        c.flip_h();
        c.set_angle(12.0);
        assert!(!c.is_identity());
        c.reset();
        assert!(c.is_identity(), "{c:?}");
        assert_eq!(c, Crop::default());
    }
}
