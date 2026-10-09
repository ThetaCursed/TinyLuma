// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Glue between the pure crop module (`src/crop/`) and the app state.
//!
//! The crop is non-destructive: the pipeline output is never resized. The
//! orientation (quarter turns + straighten angle) is baked into the *display*
//! texture on upload, so the canvas can draw a plain UV sub-rect; export applies
//! the same transform to the full-resolution render and then crops. This file
//! only tracks the value, the tool state, undo/session persistence and the
//! display math — no pixel work.

use eframe::egui;

use super::TinyLumaApp;
use crate::crop::orient::{
    fit_rect_inside_rotated, flip_rect_h, flip_rect_v, rotate_rect_ccw, rotate_rect_cw,
};
use crate::crop::{AspectPreset, Crop, Handle, NormRect, Orientation};

/// What the current crop drag is doing. Held so a gesture keeps its anchor even
/// as the pointer moves (and one history entry is produced per gesture).
#[derive(Clone, Copy, Debug)]
pub(crate) enum CropDrag {
    /// Moving the whole frame. `grab` is the normalized pointer at drag start.
    Move { start: NormRect, grab: (f32, f32) },
    /// Resizing by a handle, with the opposite side/corner fixed.
    Resize { handle: Handle, start: NormRect },
}

/// Live crop tool state. `Crop` is the undoable value; `active`/`dragging`/`drag`
/// are UI-only and are never snapshotted.
pub(crate) struct CropState {
    pub(crate) crop: Crop,
    /// The tool is active (full image shown, overlay + handles drawn).
    pub(crate) active: bool,
    /// A frame/handle drag is in progress.
    pub(crate) dragging: bool,
    /// The current drag gesture, if any.
    pub(crate) drag: Option<CropDrag>,
    /// The crop value that is currently recorded in the history. Updated on
    /// commit, undo/redo and frame switch.
    pub(crate) committed: Crop,
    /// The crop at the moment the tool was (re)activated — the baseline for the
    /// ONE history entry a crop session produces. `Some` until that entry is
    /// written, `None` once the session has been recorded (further edits in the
    /// same session do not add entries).
    pub(crate) session_baseline: Option<Crop>,
    /// The un-straightened frame: the straighten fit recomputes `crop.rect` from
    /// this base for the current angle, so lowering the angle restores the frame
    /// instead of only ever shrinking it. `None` when the angle is zero.
    pub(crate) angle_base: Option<NormRect>,
}

impl Default for CropState {
    fn default() -> Self {
        Self {
            crop: Crop::default(),
            active: false,
            dragging: false,
            drag: None,
            committed: Crop::default(),
            session_baseline: None,
            angle_base: None,
        }
    }
}

impl TinyLumaApp {
    /// Pixel size of the base image after the right-angle orientation.
    pub(crate) fn crop_image_size(&self) -> (u32, u32) {
        let (w, h) = self
            .preview_base
            .as_ref()
            .map_or((1, 1), |b| (b.width(), b.height()));
        self.crop.crop.orientation.dimensions(w, h)
    }

    /// Pixel aspect of the oriented image. The crop frame lives in oriented
    /// space, so preset ratios must use this, not the raw texture ratio.
    pub(crate) fn crop_image_ratio(&self) -> f32 {
        let (w, h) = self.crop_image_size();
        if h == 0 {
            1.0
        } else {
            w as f32 / h as f32
        }
    }

    /// The transform applied to the image for display: `(orientation, angle)`.
    /// The retouch tool always wants the raw, unoriented working image.
    pub(crate) fn crop_display_transform(&self) -> (Orientation, f32) {
        if self.retouch.active {
            (Orientation::IDENTITY, 0.0)
        } else {
            (self.crop.crop.orientation, self.crop.crop.angle)
        }
    }

    /// `(display_size, visible_rect, orientation, angle)` for the canvas.
    /// `full` is the raw texture size. While the crop tool (or retouch) is active
    /// the visible rect is the whole image; otherwise it is the crop frame.
    pub(crate) fn crop_display_state(
        &self,
        full: egui::Vec2,
    ) -> (egui::Vec2, NormRect, Orientation, f32) {
        let (orientation, angle) = self.crop_display_transform();
        let visible = if self.crop.active || self.retouch.active {
            NormRect::FULL
        } else {
            self.crop.crop.rect
        };
        let oriented = if orientation.swaps_dims() {
            egui::vec2(full.y, full.x)
        } else {
            full
        };
        (
            egui::vec2(oriented.x * visible.w, oriented.y * visible.h),
            visible,
            orientation,
            angle,
        )
    }

    /// Toggles the crop tool. Crop and retouch both own the canvas pointer and
    /// the full-image view, so enabling one disables the other.
    pub(crate) fn crop_toggle(&mut self) {
        if self.crop.dragging {
            self.crop_end_gesture();
        }
        if self.crop.active {
            self.crop.active = false;
            self.crop_options_rect = None;
            self.crop.session_baseline = None;
            self.crop.angle_base = None;
            self.is_panning = false;
            self.crop_save_session();
        } else {
            if self.retouch.active {
                self.retouch_toggle();
            }
            self.crop.active = true;
            self.crop.session_baseline = Some(self.crop.crop);
            self.crop.angle_base = None;
            self.zoom_scale = 1.0;
            self.pan_offset = egui::Vec2::ZERO;
            self.is_panning = false;
        }
    }

    /// Selects an aspect preset (a discrete action; committed at frame end).
    pub(crate) fn crop_set_preset(&mut self, preset: AspectPreset) {
        self.crop.crop.set_preset(preset, self.crop_image_ratio());
        // The preset fit becomes the new un-straightened base, then the current
        // straighten angle is re-applied on top of it.
        self.crop.angle_base = Some(self.crop.crop.rect);
        self.crop_refit_angle();
        self.crop_save_session();
    }

    pub(crate) fn crop_reset(&mut self) {
        self.crop.crop.reset();
        self.crop.angle_base = None;
        self.crop_save_session();
    }

    pub(crate) fn crop_rotate_cw(&mut self) {
        self.crop.crop.rotate_cw();
        self.crop.angle_base = self.crop.angle_base.map(rotate_rect_cw);
        self.crop_refit_angle();
        self.crop_save_session();
    }

    pub(crate) fn crop_rotate_ccw(&mut self) {
        self.crop.crop.rotate_ccw();
        self.crop.angle_base = self.crop.angle_base.map(rotate_rect_ccw);
        self.crop_refit_angle();
        self.crop_save_session();
    }

    pub(crate) fn crop_flip_h(&mut self) {
        self.crop.crop.flip_h();
        // Mirroring the displayed image mirrors the tilt too.
        self.crop.crop.angle = -self.crop.crop.angle;
        self.crop.angle_base = self.crop.angle_base.map(flip_rect_h);
        self.crop_refit_angle();
        self.crop_save_session();
    }

    pub(crate) fn crop_flip_v(&mut self) {
        self.crop.crop.flip_v();
        // Mirroring the displayed image mirrors the tilt too.
        self.crop.crop.angle = -self.crop.crop.angle;
        self.crop.angle_base = self.crop.angle_base.map(flip_rect_v);
        self.crop_refit_angle();
        self.crop_save_session();
    }

    /// Recomputes the frame from the un-straightened base for the current angle.
    ///
    /// At angle 0 the base is simply restored, so dragging the straighten slider
    /// back to zero grows the frame back instead of leaving the last fit in
    /// place. At a non-zero angle the base is recorded on first use and the
    /// frame is the largest copy of it that still fits the rotated image (so no
    /// empty corners are shown). The fit uses the oriented image ratio, so the
    /// rotation is a true (non-sheared) one.
    fn crop_refit_angle(&mut self) {
        let ratio = self.crop_image_ratio();
        let angle = self.crop.crop.angle;
        if angle.abs() < 1e-4 {
            if let Some(base) = self.crop.angle_base.take() {
                self.crop.crop.rect = base;
            }
            return;
        }
        let base = match self.crop.angle_base {
            Some(base) => base,
            None => {
                let base = self.crop.crop.rect;
                self.crop.angle_base = Some(base);
                base
            }
        };
        self.crop.crop.rect = fit_rect_inside_rotated(base, angle, ratio);
    }

    /// Marks the start of a drag gesture. Nothing is recorded yet: the baseline
    /// is `self.crop.committed` (see `commit_crop_history`).
    pub(crate) fn crop_begin_gesture(&mut self) {
        self.crop.dragging = true;
    }

    /// Marks the end of a drag gesture. The history entry is committed at frame
    /// end by `commit_crop_history`, so a swallowed release cannot lose it.
    pub(crate) fn crop_end_gesture(&mut self) {
        self.crop.drag = None;
        self.crop.dragging = false;
        // A manual frame edit takes over as the un-straightened base (it already
        // fits the current angle, so the refit leaves it alone).
        if self.crop.crop.angle.abs() >= 1e-4 {
            self.crop.angle_base = Some(self.crop.crop.rect);
        }
        self.crop_save_session();
    }

    /// Sets the straighten angle and re-fits the frame inside the rotated image.
    pub(crate) fn crop_set_angle(&mut self, angle: f32) {
        self.crop.crop.set_angle(angle);
        self.crop_refit_angle();
        self.crop_save_session();
    }

    /// Detects the dominant tilt and applies it. Returns the applied angle.
    pub(crate) fn crop_auto_straighten(&mut self) -> f32 {
        let Some(base) = self.preview_base.as_ref() else {
            return 0.0;
        };
        let (bw, bh) = (base.width(), base.height());
        let (pixels, w, h) = self
            .crop
            .crop
            .orientation
            .apply_rgb(base.as_raw(), bw, bh);
        let angle = crate::crop::pixel::detect_straighten_angle(&pixels, w, h);
        self.crop.crop.set_angle(angle);
        self.crop_refit_angle();
        self.crop_save_session();
        angle
    }

    /// Commits crop changes to the history.
    ///
    /// While the crop tool is active, the **whole session is ONE undo entry**:
    /// the first change records the pre-session crop as the baseline, and every
    /// later change in the same session just follows the live value. So after
    /// cropping, one Ctrl+Z returns the original frame. Leaving the tool (or
    /// an undo/redo) ends the session, so the next crop starts a fresh entry.
    ///
    /// Called once per frame after the UI; does nothing while a drag is in
    /// progress.
    pub(crate) fn commit_crop_history(&mut self) {
        if self.crop.dragging || self.crop.crop == self.crop.committed {
            return;
        }
        if self.crop.active {
            if let Some(base) = self.crop.session_baseline.take() {
                let mut base_snap = self.snapshot();
                base_snap.crop = base;
                self.history.push(base_snap);
            }
        } else {
            let mut base_snap = self.snapshot();
            base_snap.crop = self.crop.committed;
            self.history.push(base_snap);
        }
        self.crop.committed = self.crop.crop;
        self.crop_save_session();
    }

    /// Stores the crop of the current frame in the session.
    pub(crate) fn crop_save_session(&mut self) {
        if let Some(path) = self.image_path.clone() {
            self.session.save_crop(&path, self.crop.crop);
        }
    }
}
