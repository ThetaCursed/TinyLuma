// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Glue between the pure retouch layer (`src/retouch/`) and the app state,
//! caches and pipeline. No pixel math lives here.

use eframe::egui;
use image::RgbImage;
use std::path::{Path, PathBuf};

use super::TinyLumaApp;
use crate::history::Snapshot;
use crate::retouch::{BrushSettings, RetouchLayer, Spot};
use crate::retouch::spot::{apply_spot_cached, SpotCache};

/// Longest side of the full-resolution working image used while the retouch tool
/// is active. Big enough to see real pixels, small enough to stay responsive.
const RETOUCH_MAX_DIM: u32 = 4096;

/// Retouch tool + layer + caches, grouped so `TinyLumaApp` stays readable.
pub(crate) struct RetouchState {
    /// The edit itself (undoable).
    pub(crate) layer: RetouchLayer,
    /// Tool is active (canvas shows the retouch cursor, options bar appears).
    pub(crate) active: bool,
    /// Current brush.
    pub(crate) brush: BrushSettings,
    /// Cached healed preview base + the layer revision it was built from.
    pub(crate) healed_cache: Option<(u64, RgbImage)>,
    /// Bumped on every layer change; the heal cache is valid only when equal.
    pub(crate) revision: u64,
    /// Memoized healed regions, so rebuilding the layer (undo/redo, frame
    /// switch, export) does not re-run the expensive content-aware fills.
    pub(crate) spot_cache: SpotCache,
    /// A pointer gesture is in progress (one history entry per gesture).
    pub(crate) stroke_active: bool,
    /// State before the current gesture (pushed to history on release).
    pub(crate) stroke_baseline: Option<Snapshot>,
    /// Painted path of the current gesture, normalized `0..1`. Nothing is healed
    /// until the pointer is released (Photopea-style live indication).
    pub(crate) pending_path: Vec<[f32; 2]>,
    /// Cached indication texture for the pending path + the image area it covers.
    pub(crate) overlay_tex: Option<egui::TextureHandle>,
    pub(crate) overlay_area: Option<[f32; 4]>,
    /// The pending path changed — the indication texture must be rebuilt.
    pub(crate) overlay_dirty: bool,
    /// Full-resolution original (long side ≤ 4096), loaded while the tool is active.
    pub(crate) full_base: Option<RgbImage>,
    /// Full-resolution healed image, rebuilt from `full_base` when needed.
    pub(crate) full_healed: Option<RgbImage>,
    /// Source path of `full_base`, to reload it on a frame switch.
    pub(crate) full_path: Option<PathBuf>,
    /// The full-resolution textures must be (re)uploaded.
    pub(crate) full_texture_dirty: bool,
}

impl Default for RetouchState {
    fn default() -> Self {
        Self {
            layer: RetouchLayer::default(),
            active: false,
            brush: BrushSettings::default(),
            healed_cache: None,
            revision: 0,
            spot_cache: SpotCache::default(),
            stroke_active: false,
            stroke_baseline: None,
            pending_path: Vec::new(),
            overlay_tex: None,
            overlay_area: None,
            overlay_dirty: false,
            full_base: None,
            full_healed: None,
            full_path: None,
            full_texture_dirty: false,
        }
    }
}

impl RetouchState {
    /// Applies `spots` in order, consulting the memoization cache. Replaying an
    /// unchanged prefix (undo/redo, rebuild) hits the cache and skips the fills.
    fn apply_spots_cached(cache: &mut SpotCache, spots: &[Spot], img: &mut RgbImage) {
        for spot in spots {
            apply_spot_cached(cache, img, spot);
        }
    }

    /// Rebuilds the healed base if the cache is stale for the current layer and
    /// image size.
    pub(crate) fn ensure_healed(&mut self, base: &RgbImage) {
        if self.layer.is_empty() {
            self.healed_cache = None;
            return;
        }
        let dims = (base.width(), base.height());
        if let Some((rev, img)) = &self.healed_cache {
            if *rev == self.revision && (img.width(), img.height()) == dims {
                return;
            }
        }
        let mut img = base.clone();
        Self::apply_spots_cached(&mut self.spot_cache, &self.layer.spots, &mut img);
        self.healed_cache = Some((self.revision, img));
    }

    /// The current healed base, rebuilding it if needed.
    pub(crate) fn healed_base<'a>(&'a mut self, base: &'a RgbImage) -> &'a RgbImage {
        self.ensure_healed(base);
        match &self.healed_cache {
            Some((_, img)) => img,
            None => base,
        }
    }

    /// Replaces the layer (frame switch, undo/redo) and invalidates the cache.
    pub(crate) fn set_layer(&mut self, layer: RetouchLayer) {
        self.layer = layer;
        self.revision = self.revision.wrapping_add(1);
        self.healed_cache = None;
        self.full_healed = None;
        self.full_texture_dirty = true;
        self.stroke_active = false;
        self.stroke_baseline = None;
        self.pending_path.clear();
        self.overlay_tex = None;
        self.overlay_area = None;
        self.overlay_dirty = false;
    }

    /// Frees the full-resolution buffers (on tool deactivation).
    pub(crate) fn unload_full(&mut self) {
        self.full_base = None;
        self.full_healed = None;
        self.full_path = None;
        self.full_texture_dirty = false;
    }

    /// Loads the full-resolution original for `path` if it is not already loaded.
    pub(crate) fn ensure_full_base(&mut self, path: Option<&Path>) {
        let Some(path) = path else {
            return;
        };
        if self.full_path.as_deref() == Some(path) {
            return;
        }
        let Ok(img) = image::open(path) else {
            return;
        };
        let (w, h) = (img.width(), img.height());
        let scale = (RETOUCH_MAX_DIM as f32 / w.max(h) as f32).min(1.0);
        let full = if scale < 1.0 {
            img.resize(
                (w as f32 * scale).round().max(1.0) as u32,
                (h as f32 * scale).round().max(1.0) as u32,
                image::imageops::FilterType::Triangle,
            )
            .to_rgb8()
        } else {
            img.to_rgb8()
        };
        self.full_base = Some(full);
        self.full_healed = None;
        self.full_path = Some(path.to_path_buf());
        self.full_texture_dirty = true;
    }

    /// Rebuilds the healed full-resolution image if it is missing.
    pub(crate) fn ensure_full_healed(&mut self) {
        if self.full_healed.is_some() || self.full_base.is_none() {
            return;
        }
        let mut healed = self.full_base.as_ref().unwrap().clone();
        Self::apply_spots_cached(&mut self.spot_cache, &self.layer.spots, &mut healed);
        self.full_healed = Some(healed);
        self.full_texture_dirty = true;
    }

    /// Adds a spot, updating the heal cache incrementally when possible.
    pub(crate) fn add_spot(&mut self, spot: Spot, base: &RgbImage) {
        self.layer.push(spot);
        let dims = (base.width(), base.height());
        let incremental = matches!(
            &self.healed_cache,
            Some((rev, img)) if *rev == self.revision && (img.width(), img.height()) == dims
        );
        if incremental {
            // Take the image out so the cache and the image can be borrowed at
            // the same time (they are disjoint fields, but a method call cannot split them).
            let mut img = self.healed_cache.take().unwrap().1;
            apply_spot_cached(&mut self.spot_cache, &mut img, &spot);
            self.revision = self.revision.wrapping_add(1);
            self.healed_cache = Some((self.revision, img));
            return;
        }
        self.revision = self.revision.wrapping_add(1);
        self.ensure_healed(base);
    }
}

impl TinyLumaApp {
    /// Records the pre-gesture state and starts a stroke. Nothing is healed yet:
    /// the painted region is previewed as a translucent overlay and only applied
    /// when the pointer is released.
    pub(crate) fn retouch_begin_stroke(&mut self, pos: [f32; 2]) {
        self.retouch.stroke_active = true;
        self.retouch.pending_path.clear();
        self.retouch.pending_path.push(pos);
        self.retouch.overlay_dirty = true;
        self.retouch.stroke_baseline = Some(self.snapshot());
    }

    /// Extends the pending stroke with a pointer position (normalized).
    pub(crate) fn retouch_extend_stroke(&mut self, pos: [f32; 2]) {
        if self
            .retouch
            .pending_path
            .last()
            .map_or(true, |last| *last != pos)
        {
            self.retouch.pending_path.push(pos);
            self.retouch.overlay_dirty = true;
        }
    }

    /// Ends a gesture: heals the whole painted path and records one history
    /// entry for it.
    pub(crate) fn retouch_commit_stroke(&mut self) {
        self.retouch.stroke_active = false;
        let baseline = self.retouch.stroke_baseline.take();
        let path = std::mem::take(&mut self.retouch.pending_path);

        if !path.is_empty() {
            let (ww, wh) = self.retouch_work_dims();
            let spots = self.retouch.brush.spots_along(&path, ww, wh);
            if let Some(base) = self.preview_base.as_ref() {
                for spot in &spots {
                    // Preview cache (used when the tool is off / exported preview).
                    self.retouch.add_spot(*spot, base);
                }
            }
            // Full-resolution working image (shown while the tool is active).
            if let Some(mut full) = self.retouch.full_healed.take() {
                RetouchState::apply_spots_cached(
                    &mut self.retouch.spot_cache,
                    &spots,
                    &mut full,
                );
                self.retouch.full_healed = Some(full);
                self.retouch.full_texture_dirty = true;
            }
        }

        if let Some(base_snap) = baseline {
            if base_snap.retouch != self.retouch.layer {
                self.history.push(base_snap);
            }
        }
        if let Some(path) = self.image_path.clone() {
            let layer = self.retouch.layer.clone();
            self.session.save_retouch(&path, layer);
        }
        self.mark_retouch_dirty();
    }

    /// Cancels an in-progress gesture: the painted region is discarded, nothing
    /// was applied to the layer yet.
    pub(crate) fn retouch_cancel_stroke(&mut self) {
        self.retouch.stroke_active = false;
        self.retouch.pending_path.clear();
        self.retouch.stroke_baseline = None;
    }

    /// Toggles the retouch tool and forces a re-render.
    pub(crate) fn retouch_toggle(&mut self) {
        // Never leave a half-finished gesture behind.
        if self.retouch.stroke_active {
            self.retouch_commit_stroke();
        }
        // Crop and retouch both own the canvas: leaving retouch is implicit.
        self.crop.active = false;
        self.retouch.active = !self.retouch.active;
        self.retouch.stroke_active = false;
        self.retouch.pending_path.clear();
        self.retouch.stroke_baseline = None;
        if !self.retouch.active {
            // Leaving the tool frees the full-resolution buffers.
            self.retouch.unload_full();
        }
        // Entering: keep the current zoom and pan. `zoom_scale` is relative to
        // "fit" and `pan_offset` is in screen pixels, so the framing is
        // preserved when the canvas switches from the preview to the
        // full-resolution working image. Resetting to fit here would throw away
        // the user's zoom right where they spotted the defect.
        self.color_dirty = true;
        self.spatial_dirty = true;
        self.full_render_pending = true;
        self.mark_retouch_dirty();
        // Persist the brush now (not only on exit): toggling is infrequent, and a
        // crash would otherwise lose the user's comfortable size/hardness.
        self.save_save_settings();
    }

    /// Flags a retouch change: re-run the heal + pipeline and drop the stale
    /// cached render of this frame.
    pub(crate) fn mark_retouch_dirty(&mut self) {
        self.retouch_dirty = true;
        self.color_dirty = true;
        self.spatial_dirty = true;
        if let Some(path) = &self.image_path {
            self.preview_cache.drop_render(path);
        }
    }

    /// Pixel dimensions the brush operates in: the full-resolution working image
    /// while the tool is active, otherwise the preview.
    pub(crate) fn retouch_work_dims(&self) -> (usize, usize) {
        if self.retouch.active {
            if let Some(full) = &self.retouch.full_base {
                return (full.width() as usize, full.height() as usize);
            }
        }
        self.preview_base
            .as_ref()
            .map_or((1, 1), |b| (b.width() as usize, b.height() as usize))
    }

    /// (Re)uploads the full-resolution retouch textures: the healed image into
    /// `self.texture` (shown on the canvas) and the original into
    /// `self.original_texture` (before/after split).
    pub(crate) fn upload_full_retouch_textures(&mut self, ctx: &egui::Context) {
        let Some(healed) = self.retouch.full_healed.as_ref() else {
            return;
        };
        let (w, h) = (healed.width() as usize, healed.height() as usize);
        let healed_img = egui::ColorImage::from_rgb([w, h], healed.as_raw());

        let original_img = self
            .retouch
            .full_base
            .as_ref()
            .map(|base| egui::ColorImage::from_rgb([w, h], base.as_raw()));

        if let Some(tex) = &mut self.texture {
            tex.set(healed_img, egui::TextureOptions::LINEAR);
        } else {
            self.texture = Some(ctx.load_texture(
                "retouch_full",
                healed_img,
                egui::TextureOptions::LINEAR,
            ));
        }

        if let Some(original_img) = original_img {
            if let Some(tex) = &mut self.original_texture {
                tex.set(original_img, egui::TextureOptions::LINEAR);
            } else {
                self.original_texture = Some(ctx.load_texture(
                    "retouch_full_orig",
                    original_img,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
    }
}
