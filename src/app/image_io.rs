// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use eframe::egui;
use egui_phosphor::regular as ph;
use image::{GenericImageView, RgbImage};
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;

use super::TinyLumaApp;
use crate::history::History;
use crate::retouch::RetouchLayer;
use crate::session::Session;
use crate::settings::FilterSettings;
use crate::ui::notification::ToastKind;

/// Background preview prefetch control: current session index and stop signal.
pub(crate) struct PrefetchControl {
    pub(crate) current: AtomicUsize,
    pub(crate) stop: AtomicBool,
}

impl PrefetchControl {
    pub(crate) fn new() -> Self {
        Self {
            current: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        }
    }
}

/// Cache entry: decoded frame + the last rendered result.
struct PreviewEntry {
    base: RgbImage,
    /// Ready result (color + spatial) for `state`/`lut_path`.
    processed: Option<Vec<u8>>,
    state: FilterSettings,
    lut_path: Option<PathBuf>,
    has_render: bool,
}

/// Preview cache budget (`base` + `render`) in bytes. Bounds memory
/// regardless of the number of open images and their aspect ratios.
/// 1024 MiB ≈ ~180 1200×800 previews or ~120 square 1200×1200 —
/// essentially an entire typical session.
pub(crate) const PREVIEW_BUDGET_BYTES: usize = 1024 * 1024 * 1024;

/// Byte-budgeted LRU preview cache: stores both the decoded frame
/// and the ready render. Returning to a recent frame — no decode or recompute.
/// Evicts the oldest until the total size fits the budget.
pub(crate) struct PreviewCache {
    map: HashMap<PathBuf, PreviewEntry>,
    order: VecDeque<PathBuf>,
    /// Current total size of base+render across all entries (in bytes).
    bytes: usize,
    budget: usize,
    /// The currently displayed frame — not evicted by background prefetch.
    pinned: Option<PathBuf>,
}

impl PreviewCache {
    pub(crate) fn new(budget_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            budget: budget_bytes.max(1),
            pinned: None,
        }
    }

    /// Size of one entry in bytes (decode + render).
    fn entry_size(entry: &PreviewEntry) -> usize {
        entry.base.as_raw().len() + entry.processed.as_ref().map_or(0, |p| p.len())
    }

    /// Pin the current frame (it must not be evicted).
    pub(crate) fn set_pinned(&mut self, path: Option<PathBuf>) {
        self.pinned = path;
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.map.contains_key(path)
    }

    /// Move the path to the back of the queue (most recent).
    fn touch(&mut self, path: &Path) {
        if let Some(pos) = self.order.iter().position(|p| p == path) {
            self.order.remove(pos);
        }
        self.order.push_back(path.to_path_buf());
    }

    /// Evict the oldest (except the pinned one) until we fit the budget.
    fn evict(&mut self) {
        while self.bytes > self.budget {
            let victim = self
                .order
                .iter()
                .position(|p| Some(p) != self.pinned.as_ref());
            match victim {
                Some(i) => {
                    if let Some(old) = self.order.remove(i) {
                        if let Some(entry) = self.map.remove(&old) {
                            self.bytes = self.bytes.saturating_sub(Self::entry_size(&entry));
                        }
                    }
                }
                None => break,
            }
        }
    }

    /// Decoded frame (if present).
    pub(crate) fn base(&mut self, path: &Path) -> Option<RgbImage> {
        let img = self.map.get(path)?.base.clone();
        self.touch(path);
        Some(img)
    }

    /// Insert/update the decoded frame; the ready render is reset.
    pub(crate) fn insert_base(&mut self, path: PathBuf, base: RgbImage) {
        if let Some(old) = self.map.get(&path) {
            self.bytes = self.bytes.saturating_sub(Self::entry_size(old));
        }
        let entry = PreviewEntry {
            base,
            processed: None,
            state: FilterSettings::default(),
            lut_path: None,
            has_render: false,
        };
        self.bytes += Self::entry_size(&entry);
        self.map.insert(path.clone(), entry);
        self.touch(&path);
        self.evict();
    }

    /// Ready render, if it is up to date for the given settings and LUT.
    pub(crate) fn render(
        &mut self,
        path: &Path,
        state: &FilterSettings,
        lut_path: &Option<PathBuf>,
    ) -> Option<Vec<u8>> {
        let entry = self.map.get(path)?;
        if entry.has_render && &entry.state == state && &entry.lut_path == lut_path {
            let processed = entry.processed.clone()?;
            self.touch(path);
            Some(processed)
        } else {
            None
        }
    }

    /// Store the ready render for the current settings.
    pub(crate) fn store_render(
        &mut self,
        path: &Path,
        state: FilterSettings,
        lut_path: Option<PathBuf>,
        processed: Vec<u8>,
    ) {
        if let Some(entry) = self.map.get_mut(path) {
            let old = Self::entry_size(entry);
            entry.processed = Some(processed);
            entry.state = state;
            entry.lut_path = lut_path;
            entry.has_render = true;
            let new = Self::entry_size(entry);
            self.bytes = self.bytes.saturating_sub(old).saturating_add(new);
        }
        self.evict();
    }

    /// Removes a frame from the cache (e.g. when deleted from the session).
    pub(crate) fn remove(&mut self, path: &Path) {
        if let Some(entry) = self.map.remove(path) {
            self.bytes = self.bytes.saturating_sub(Self::entry_size(&entry));
        }
        if let Some(pos) = self.order.iter().position(|p| p == path) {
            self.order.remove(pos);
        }
        if self.pinned.as_deref() == Some(path) {
            self.pinned = None;
        }
    }

    /// Drop ALL ready renders, keeping the decoded frames.
    /// Needed after a mass settings change (apply-to-all): the old
    /// frame renders become stale and only waste budget.
    pub(crate) fn drop_renders(&mut self) {
        for entry in self.map.values_mut() {
            if entry.processed.is_some() {
                let old = Self::entry_size(entry);
                entry.processed = None;
                entry.has_render = false;
                let new = Self::entry_size(entry);
                self.bytes = self.bytes.saturating_sub(old.saturating_sub(new));
            }
        }
    }

    /// Drop the ready render of a single frame (a retouch edit changes only it).
    pub(crate) fn drop_render(&mut self, path: &Path) {
        if let Some(entry) = self.map.get_mut(path) {
            if entry.processed.is_some() {
                let old = Self::entry_size(entry);
                entry.processed = None;
                entry.has_render = false;
                let new = Self::entry_size(entry);
                self.bytes = self.bytes.saturating_sub(old.saturating_sub(new));
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
        self.pinned = None;
    }
}

impl TinyLumaApp {
    pub(crate) fn close_image(&mut self) {
        self.preview_base = None;
        self.color_buffer.clear();
        self.processed_pixels.clear();
        self.histogram = crate::app::histogram::Histogram::EMPTY;
        self.histogram_display_path = None;
        self.clipping.clear();
        self.clip_overlay_applied = crate::app::clipping::ClipOverlay::NONE;
        self.color_buffer_valid = false;
        self.luma_cache.clear();
        self.luma_cache_valid = false;
        self.clarity_cache.clear();
        self.clarity_cache_valid = false;
        self.spatial_base = Default::default();
        self.spatial_base_valid = false;
        self.dehaze_cached_fast = false;
        self.texture = None;
        self.original_texture = None;
        self.image_path = None;
        self.is_dragging_split = false;
        self.is_panning = false;
        self.zoom_scale = 1.0;
        self.pan_offset = egui::Vec2::ZERO;
        self.grain_map.clear();
        self.grain_map_dims = (0, 0);
        self.grain_map_size = crate::settings::default_grain_size();
        self.drag_active = false;
        self.was_dragging = false;
        self.full_render_pending = false;
        // The retouch tool/layer are tied to the frame.
        self.retouch.set_layer(RetouchLayer::default());
        self.retouch.active = false;
        self.retouch.unload_full();
        self.retouch.session_base = None;
        self.retouch.session_hist_len = 0;
        self.crop.crop.reset();
        self.crop.active = false;
        self.crop.dragging = false;
        self.crop.drag = None;
        self.crop.committed = self.crop.crop;
        self.crop.session_baseline = None;
        self.crop.session_hist_len = 0;
        self.crop.angle_base = None;
        // Free the memoized heal regions (they are only useful while a frame is open).
        self.retouch.spot_cache.clear();
        self.retouch_dirty = false;
        self.retouch_options_rect = None;
        self.tools_rect = None;
        self.thumbnails.clear();
        self.preview_cache.clear();
        self.thumb_rx = None;
        self.thumb_tx = None;
        self.prefetch.stop.store(true, AtomicOrdering::Relaxed);
        self.prefetch_rx = None;
        self.active_lut = None;
        self.lut_path = None;
        self.combined_lut = None;
        self.lut_lib.selected_lut_name = None;
        self.settings = FilterSettings::default();
        self.color_dirty = false;
        self.spatial_dirty = false;
        self.notifications.clear();
        self.show_save_dialog = false;
        self.show_batch_progress = false;
        self.batch_progress_message.clear();
        // The edit history is tied to the image — reset it.
        self.history.clear();
        self.drag_baseline = None;
        self.drag_preset_dirty = None;
        self.drag_is_default = None;
        // The selected preset belongs to the previous frame — clear the selection.
        self.preset_manager.selected_index = None;
        println!("🔄 Image closed, screen cleared");
    }

    pub(crate) fn close_session(&mut self) {
        // Save the current settings before clearing
        if let Some(path) = &self.image_path {
            let preset_name = self.current_preset_name();
            self.session.save_current_settings(
                path,
                self.settings,
                self.lut_path.clone(),
                preset_name,
            );
        }
        self.close_image();
        self.session = Session::new();
        println!("🗑 Session cleared");
    }

    /// Remembers the source folder of the last opened files. It is used by
    /// the "Open Images/Folder" dialogs and persisted between runs.
    /// Saved immediately so the path is not lost if the app is closed
    /// without any other action. Single entry point for the dialog and drag & drop.
    fn remember_input_dir(&mut self, dir: Option<&std::path::Path>) {
        let Some(dir) = dir else { return };
        if dir.as_os_str().is_empty() || self.last_input_dir.as_path() == dir {
            return;
        }
        self.last_input_dir = dir.to_path_buf();
        self.save_save_settings();
    }

    pub(crate) fn open_images(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() {
            return;
        }

        // Folder of the first file in the original (unsorted) list.
        self.remember_input_dir(paths.first().and_then(|p| p.parent()));

        // Filmstrip: newest frames first, oldest last. Done centrally
        // so the order is the same for a folder, file selection and drag&drop.
        let mut paths = paths;
        sort_paths_newest_first(&mut paths);

        // Close the current session
        self.close_image();
        self.session = Session::new();
        // New set of files — fresh prefetch control.
        self.prefetch = Arc::new(PrefetchControl::new());

        // Load all paths
        let paths_for_thumbs = paths.clone();
        let paths_for_prefetch = paths.clone();
        self.session.image_list = paths;
        self.session.current_index = 0;

        // Filmstrip thumbnails and preview prefetch run in the background (decode on separate threads)
        self.spawn_thumbnail_loader(paths_for_thumbs, ctx);
        self.spawn_prefetch_loader(paths_for_prefetch, ctx);
        self.filmstrip_needs_scroll = true;

        // Load the first image
        let first = self.session.image_list[0].clone();
        self.load_new_image(first, ctx);
        println!("📂 Loaded {} images", self.session.total());
    }

    /// Adds images to an ALREADY open session without resetting it.
    ///
    /// The current frame stays active — the open image is not lost,
    /// the new ones are simply appended to the end of the filmstrip. Duplicates (a file
    /// already in the session) are skipped. If the session is empty — behaves like [`Self::open_images`].
    pub(crate) fn add_images(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() {
            return;
        }
        if self.session.image_list.is_empty() {
            self.open_images(paths, ctx);
            return;
        }

        // Same order as on open: newest frames first.
        let mut paths = paths;
        sort_paths_newest_first(&mut paths);

        // Filter out what is already open. Compare canonical paths so
        // the same file is not duplicated due to different path spellings
        // (relative/absolute, case, symlink).
        let mut seen: std::collections::HashSet<PathBuf> = self
            .session
            .image_list
            .iter()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
            .collect();
        let mut duplicates = 0usize;
        let mut new_paths: Vec<PathBuf> = Vec::with_capacity(paths.len());
        for p in paths {
            let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if seen.insert(key) {
                new_paths.push(p);
            } else {
                duplicates += 1;
            }
        }

        if new_paths.is_empty() {
            self.notify(ToastKind::Warning, "These images are already open");
            return;
        }

        // Drag & drop previously did not remember the source folder — now it behaves
        // the same as opening via the dialog.
        self.remember_input_dir(new_paths.first().and_then(|p| p.parent()));

        // Append to the end of the filmstrip and load thumbnails only for the new files.
        self.session.image_list.extend(new_paths.iter().cloned());
        self.spawn_thumbnail_loader(new_paths.clone(), ctx);

        // The background prefetch thread keeps its own copy of the list — update it.
        self.restart_prefetch(ctx);

        // Do NOT switch the current frame: the user keeps working on the same
        // image, and the new frames are visible in the filmstrip on the right.
        self.filmstrip_needs_scroll = true;

        let msg = if duplicates > 0 {
            format!(
                "Added {} image(s) — {} already open",
                new_paths.len(),
                duplicates
            )
        } else {
            format!("Added {} image(s)", new_paths.len())
        };
        self.notify(ToastKind::Success, msg);
        println!(
            "➕ Added {} images, session now {}",
            new_paths.len(),
            self.session.total()
        );
        ctx.request_repaint();
    }

    pub(crate) fn switch_to_image(&mut self, new_index: usize, ctx: &egui::Context) {
        if new_index >= self.session.total() {
            return;
        }
        if new_index == self.session.current_index {
            return;
        }

        // 1. Save the settings and history of the current image
        if let Some(path) = &self.image_path {
            let preset_name = self.current_preset_name();
            self.session.save_current_settings(
                path,
                self.settings,
                self.lut_path.clone(),
                preset_name,
            );
            // The undo/redo stack belongs to the frame — save it under the path.
            let hist = std::mem::replace(&mut self.history, History::new());
            self.session.history_map.insert(path.clone(), hist);
            // The retouch layer is per image as well.
            let layer = self.retouch.layer.clone();
            self.session.save_retouch(path, layer);
            self.session.save_crop(path, self.crop.crop);
        }

        // 2. Update the index
        self.session.current_index = new_index;
        self.prefetch
            .current
            .store(new_index, AtomicOrdering::Relaxed);
        self.filmstrip_needs_scroll = true;

        // 3-5. Apply the per-file state (settings/LUT/preset/history).
        let new_path = self.session.image_list[new_index].clone();
        self.apply_session_state(&new_path);

        // 6. Load the preview
        self.load_new_image(new_path, ctx);
    }

    /// Applies a frame's per-file data to the working state: settings, LUT,
    /// preset and undo/redo history. Does not touch `current_index` or the preview.
    pub(crate) fn apply_session_state(&mut self, path: &PathBuf) {
        let (saved_settings, saved_lut, saved_preset) = self.session.load_settings(path);
        self.settings = saved_settings;

        // First UNCONDITIONALLY reset the previous LUT, otherwise if the file is missing
        // on the new frame the previous image's LUT would remain.
        self.active_lut = None;
        self.lut_path = None;
        self.lut_lib.selected_lut_name = None;

        if let Some(saved_path) = &saved_lut {
            if let Some(loaded_lut) = self.load_lut_cached(saved_path) {
                self.active_lut = Some(loaded_lut);
                self.lut_path = Some(saved_path.clone());
                // Name is for display only; resolved by path.
                self.lut_lib.selected_lut_name = self
                    .lut_lib
                    .all_luts
                    .iter()
                    .find(|e| &e.path == saved_path)
                    .map(|e| e.name.clone());
            }
        }

        // Preset association: by name from the session, otherwise by exact match.
        self.preset_manager.selected_index = saved_preset.as_ref().and_then(|name| {
            self.preset_manager
                .presets
                .iter()
                .position(|p| &p.name == name)
        });
        if self.preset_manager.selected_index.is_none() {
            self.sync_selected_preset();
        }

        // History of this frame (or an empty one).
        self.history = self
            .session
            .history_map
            .remove(path)
            .unwrap_or_else(History::new);
        // Retouch layer of this frame. Force a re-render only while the tool is
        // active (it needs to load the full-resolution working image for the new
        // frame); otherwise keep the cached preview render.
        let layer = self.session.load_retouch(path);
        self.retouch.set_layer(layer);
        self.retouch_dirty = self.retouch.active;
        self.retouch.session_base = if self.retouch.active {
            Some(self.retouch.layer.clone())
        } else {
            None
        };
        self.retouch.session_hist_len = self.history.len();
        // Crop of this frame (non-destructive — no re-render needed).
        self.crop.crop = self.session.load_crop(path);
        self.crop.dragging = false;
        self.crop.drag = None;
        self.crop.committed = self.crop.crop;
        self.crop.session_baseline = if self.crop.active {
            Some(self.crop.crop)
        } else {
            None
        };
        self.crop.session_hist_len = self.history.len();
        self.crop.angle_base = None;
        self.drag_baseline = None;
        self.drag_preset_dirty = None;
        self.drag_is_default = None;
    }

    /// Removes an image from the session (the file on disk is NOT touched). If the
    /// current frame was removed — switches to the next/previous one.
    pub(crate) fn remove_from_session(&mut self, index: usize, ctx: &egui::Context) {
        if index >= self.session.total() {
            return;
        }

        let path = self.session.image_list.remove(index);

        // Clean up everything tied to the removed path.
        self.session.settings_map.remove(&path);
        self.session.lut_map.remove(&path);
        self.session.preset_map.remove(&path);
        self.session.history_map.remove(&path);
        self.session.retouch_map.remove(&path);
        self.session.crop_map.remove(&path);
        self.thumbnails.remove(&path);
        self.preview_cache.remove(&path);

        // The session became empty — close the screen.
        if self.session.image_list.is_empty() {
            self.close_image();
            self.session = Session::new();
            return;
        }

        if index < self.session.current_index {
            // A frame before the current one was removed: shift the index, the frame itself is unchanged.
            self.session.current_index -= 1;
        } else if index == self.session.current_index {
            // The current one was removed: show the next, or the last if we were at the end.
            if self.session.current_index >= self.session.image_list.len() {
                self.session.current_index = self.session.image_list.len() - 1;
            }
            let new_path = self.session.image_list[self.session.current_index].clone();
            self.apply_session_state(&new_path);
            self.load_new_image(new_path, ctx);
        }

        // The prefetch thread keeps its own copy of the list — after removal it is
        // stale, so restart prefetch on the current list.
        self.restart_prefetch(ctx);

        self.filmstrip_needs_scroll = true;
        ctx.request_repaint();
    }

    /// Restarts the background prefetch on the current image list.
    fn restart_prefetch(&mut self, ctx: &egui::Context) {
        // Stop the old thread and replace the control with a new one.
        self.prefetch.stop.store(true, AtomicOrdering::Relaxed);
        self.prefetch = Arc::new(PrefetchControl::new());
        self.prefetch
            .current
            .store(self.session.current_index, AtomicOrdering::Relaxed);
        let paths = self.session.image_list.clone();
        self.spawn_prefetch_loader(paths, ctx);
    }

    pub(crate) fn load_new_image(&mut self, path: PathBuf, ctx: &egui::Context) {
        // History and preset association are set by the caller
        // (switch_to_image / open_images); we don't touch them here.

        // Pin the frame BEFORE inserting into the cache, otherwise with a tight budget
        // the fresh entry could evict itself.
        self.preview_cache.set_pinned(Some(path.clone()));

        // --- 1. IMAGE PREPARATION (1200px) ---
        // First try the LRU cache — returning to a recent frame without decode+resize.
        let resized_rgb = if let Some(cached) = self.preview_cache.base(&path) {
            cached
        } else {
            let Ok(img) = image::open(&path) else {
                return;
            };
            let (w, h) = img.dimensions();
            let max_dim = 1200.0;
            let scale = (max_dim / w.max(h) as f32).min(1.0);
            let resized = img.resize(
                (w as f32 * scale) as u32,
                (h as f32 * scale) as u32,
                image::imageops::FilterType::Triangle,
            );
            let rgb = resized.to_rgb8();
            // Put a copy into the cache, use the original further.
            self.preview_cache.insert_base(path.clone(), rgb.clone());
            rgb
        };

        let (rw, rh) = resized_rgb.dimensions();
        let rw_u = rw as usize;
        let rh_u = rh as usize;

        // --- 2. APPLICATION DATA UPDATE ---
        self.preview_base = Some(resized_rgb);
        self.image_path = Some(path.clone());
        self.texture = None;
        self.original_texture = None;
        self.is_panning = false;
        self.zoom_scale = 1.0;
        self.pan_offset = egui::Vec2::ZERO;

        // Heal the base before the pipeline (empty layer → byte-identical copy).
        let raw = {
            let base = self.preview_base.as_ref().unwrap();
            self.retouch.healed_base(base).as_raw().to_vec()
        };

        // --- 3. GRAIN GENERATION ---
        // The map is deterministic in size and grain scale: reuse it when both
        // match, otherwise regenerate (e.g. after the Grain Size slider moved).
        self.ensure_grain_map(rw_u, rh_u);

        // --- 4. READY RENDER FROM CACHE OR RECOMPUTE ---
        if let Some(processed) = self
            .preview_cache
            .render(&path, &self.settings, &self.lut_path)
        {
            // Settings/LUT unchanged since the last render — take the ready one.
            self.processed_pixels = processed;
            // The color_buffer (input for spatial) is not stored in the cache: mark it
            // invalid so that when a spatial slider is edited it gets
            // rebuilt from the base frame.
            self.color_buffer = raw;
            self.color_buffer_valid = false;
            self.color_dirty = false;
            self.spatial_dirty = false;
            self.luma_cache_valid = false;
            self.clarity_cache_valid = false;
            self.spatial_base_valid = false;
            self.combined_lut = None;
            self.upload_preview_textures(ctx);
        } else {
            self.color_buffer = raw.clone();
            self.processed_pixels = raw;
            self.color_dirty = true;
            self.spatial_dirty = true;
            self.color_buffer_valid = false;
            self.process_preview(ctx);
        }
    }

    /// A background thread generates thumbnails for the given paths (decoding each
    /// file). The channel is reused for the whole session: adding new files does not
    /// interrupt the thumbnail loading already in progress for the current filmstrip.
    pub(crate) fn spawn_thumbnail_loader(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() {
            return;
        }
        // Create the channel once when the session opens and keep both ends.
        if self.thumb_tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, egui::ColorImage)>();
            self.thumb_tx = Some(tx);
            self.thumb_rx = Some(rx);
        }
        let tx = self
            .thumb_tx
            .clone()
            .expect("thumb_tx created above in this same function");

        // Wake the UI from the background thread, otherwise egui will not repaint while idle.
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for path in paths {
                let Ok(img) = image::open(&path) else {
                    continue;
                };
                // thumbnail — a fast box filter, fits into 160x160 preserving aspect ratio
                let thumb = img.thumbnail(160, 160).to_rgb8();
                let (w, h) = thumb.dimensions();
                let color = egui::ColorImage::from_rgb([w as usize, h as usize], &thumb.into_raw());
                // If the channel is closed (session changed/closed) — stop working
                if tx.send((path, color)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }

    /// Background preview prefetch: decodes frames AHEAD of the current one (a window)
    /// so that paging forward is instant. Does not run far ahead — otherwise
    /// the LRU would evict exactly the frames about to be needed.
    pub(crate) fn spawn_prefetch_loader(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        // sync_channel bounds the queue so megabytes of previews do not accumulate.
        let (tx, rx) = std::sync::mpsc::sync_channel::<(PathBuf, RgbImage)>(4);
        self.prefetch_rx = Some(rx);
        let control = self.prefetch.clone();

        let ctx = ctx.clone();
        std::thread::spawn(move || {
            const AHEAD: usize = 24; // window ahead of the current frame
            let mut i = 1usize; // index 0 is already loaded on the UI thread

            while i < paths.len() {
                if control.stop.load(AtomicOrdering::Relaxed) {
                    break;
                }
                let cur = control.current.load(AtomicOrdering::Relaxed);

                // The user jumped far ahead — catch up with them.
                if i + 2 < cur {
                    i = cur + 1;
                    continue;
                }
                // Do not run too far from the current frame — wait.
                if i > cur + AHEAD {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    continue;
                }

                let Ok(img) = image::open(&paths[i]) else {
                    i += 1;
                    continue;
                };
                let (w, h) = img.dimensions();
                let max_dim = 1200.0;
                let scale = (max_dim / w.max(h) as f32).min(1.0);
                let base = img
                    .resize(
                        (w as f32 * scale) as u32,
                        (h as f32 * scale) as u32,
                        image::imageops::FilterType::Triangle,
                    )
                    .to_rgb8();

                if tx.send((paths[i].clone(), base)).is_err() {
                    break;
                }
                ctx.request_repaint();
                i += 1;
            }
        });
    }

    /// Accepts background previews and puts them into the LRU frame cache.
    pub(crate) fn poll_prefetch(&mut self, ctx: &egui::Context) {
        let mut batch: Vec<(PathBuf, RgbImage)> = Vec::new();
        if let Some(rx) = &self.prefetch_rx {
            while let Ok(item) = rx.try_recv() {
                batch.push(item);
            }
        }
        if batch.is_empty() {
            return;
        }
        for (path, base) in batch {
            // Do not overwrite an already ready render (e.g. of the current frame).
            if !self.preview_cache.contains(&path) {
                self.preview_cache.insert_base(path, base);
            }
        }
        ctx.request_repaint();
    }

    /// Drains background batch export events and updates progress.
    pub(crate) fn poll_export(&mut self, ctx: &egui::Context) {
        let mut finished: Option<(usize, Vec<String>, bool)> = None;
        let mut got_event = false;

        if let Some(rx) = &self.export_rx {
            loop {
                match rx.try_recv() {
                    Ok(super::export::ExportEvent::Progress { done, total, name }) => {
                        self.batch_done = done;
                        self.batch_total = total;
                        self.batch_progress_message =
                            format!("{} Exporting {}/{} — {}", ph::HOURGLASS, done, total, name);
                        got_event = true;
                    }
                    Ok(super::export::ExportEvent::Done {
                        saved,
                        errors,
                        cancelled,
                    }) => {
                        finished = Some((saved, errors, cancelled));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = Some((self.batch_done, Vec::new(), true));
                        break;
                    }
                }
            }
        }

        if let Some((saved, errors, cancelled)) = finished {
            self.export_rx = None;
            self.batch_done = self.batch_total;
            if cancelled {
                self.batch_progress_message =
                    format!("{} Cancelled: {} file(s) saved", ph::PROHIBIT, saved);
                self.notify(
                    ToastKind::Warning,
                    format!("Export cancelled — {} file(s) saved", saved),
                );
            } else if errors.is_empty() {
                self.batch_progress_message =
                    format!("{} Done: {} file(s) saved", ph::CHECK_CIRCLE, saved);
                self.notify(
                    ToastKind::Success,
                    format!("Export complete — {} file(s)", saved),
                );
            } else {
                self.batch_progress_message =
                    format!("{} Saved: {}, errors: {}", ph::WARNING, saved, errors.len());
                self.notify(
                    ToastKind::Warning,
                    format!("Export: {} saved, {} errors", saved, errors.len()),
                );
            }
            got_event = true;
        }

        if got_event {
            ctx.request_repaint();
        }
    }

    /// Drains finished thumbnails from the channel and loads them as textures.
    pub(crate) fn poll_thumbnails(&mut self, ctx: &egui::Context) {
        let mut batch: Vec<(PathBuf, egui::ColorImage)> = Vec::new();
        if let Some(rx) = &self.thumb_rx {
            while let Ok(item) = rx.try_recv() {
                batch.push(item);
            }
        }
        if batch.is_empty() {
            return;
        }
        for (path, img) in batch {
            // The frame could have been removed from the session while the thumbnail was loading.
            if !self.session.image_list.contains(&path) {
                continue;
            }
            let name = format!("thumb:{}", path.display());
            let tex = ctx.load_texture(name, img, egui::TextureOptions::LINEAR);
            self.thumbnails.insert(path, tex);
        }
        ctx.request_repaint();
    }

    // Extract grain_map generation into a separate method
    pub(crate) fn generate_grain_map(w: usize, h: usize, seed: u64, size: f32) -> Vec<f32> {
        crate::pipeline::grain::generate_map(w, h, seed, size)
    }

    /// Regenerates the cached grain map when the frame size or the Grain Size
    /// slider changed. Cheap to call every frame: it usually does nothing.
    pub(crate) fn ensure_grain_map(&mut self, w: usize, h: usize) {
        if self.grain_map_dims == (w, h) && self.grain_map_size == self.settings.grain_size {
            return;
        }
        self.grain_map = Self::generate_grain_map(w, h, 42, self.settings.grain_size);
        self.grain_map_dims = (w, h);
        self.grain_map_size = self.settings.grain_size;
    }
}

/// Whether the file is supported by the application (checked by extension).
pub(crate) fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            crate::ui::SUPPORTED_IMAGE_EXTENSIONS
                .iter()
                .any(|s| *s == ext)
        })
        .unwrap_or(false)
}

/// Sorts paths by creation date (newest first). If `created()` is unavailable
/// (FAT / network drive / Linux) — fall back to `modified()`; if metadata is entirely
/// absent the path goes to the end. On equal dates — natural name order.
pub(crate) fn sort_paths_newest_first(paths: &mut [PathBuf]) {
    paths.sort_by(|a, b| {
        file_timestamp(b)
            .cmp(&file_timestamp(a))
            .then_with(|| natural_cmp(a, b))
    });
}

/// File time for sorting: creation, else modification, else the epoch.
fn file_timestamp(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.created().or_else(|_| m.modified()))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Collects images from a folder (only supported extensions).
/// With `recursive == true` it walks subfolders. The result is sorted in
/// natural order (IMG_2 before IMG_10) so the filmstrip does not "jump".
pub(crate) fn collect_images_from_folder(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_images_into(dir, recursive, &mut result);
    result.sort_by(|a, b| natural_cmp(a, b));
    result
}

fn collect_images_into(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                collect_images_into(&path, recursive, out);
            }
        } else if is_supported_image(&path) {
            out.push(path);
        }
    }
}

/// Compares paths by file name, aware of numbers (natural sort).
fn natural_cmp(a: &Path, b: &Path) -> Ordering {
    let an = a
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let bn = b
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    natural_cmp_str(&an, &bn)
}

fn natural_cmp_str(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    // Compare numbers as a whole, not character by character.
                    let mut na = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() {
                            na.push(c);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    let mut nb = String::new();
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() {
                            nb.push(c);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    let va = na.trim_start_matches('0');
                    let vb = nb.trim_start_matches('0');
                    let av = if va.is_empty() { "0" } else { va };
                    let bv = if vb.is_empty() { "0" } else { vb };
                    match av.len().cmp(&bv.len()).then_with(|| av.cmp(bv)) {
                        Ordering::Equal => {}
                        ord => return ord,
                    }
                } else {
                    match ca.cmp(&cb) {
                        Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        ord => return ord,
                    }
                }
            }
        }
    }
}
