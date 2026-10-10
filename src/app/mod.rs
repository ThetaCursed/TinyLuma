// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use eframe::egui;
use image::RgbImage;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::history::{History, Snapshot};
use crate::lut::Lut3D;
use crate::pipeline::curve::CurveChannel;
use crate::lut_library::LutLibrary;
use crate::presets::PresetManager;
use crate::session::Session;
use crate::settings::{FilterSettings, SaveFormat};
use crate::ui::notification::Notification;

pub(crate) mod export;
pub(crate) mod image_io;
mod processing;
pub(crate) mod histogram;
pub(crate) mod clipping;
pub(crate) mod crop;
pub(crate) mod retouch;
#[cfg(test)]
mod perf;

/// Main application state.
pub(crate) struct TinyLumaApp {
    pub(crate) settings: FilterSettings,
    pub(crate) image_path: Option<PathBuf>,
    pub(crate) preview_base: Option<RgbImage>,
    pub(crate) color_buffer: Vec<u8>, // color_pass result (input for spatial_pass)
    pub(crate) color_buffer_valid: bool, // is color_buffer up to date? (false when loaded from the render cache)
    /// dehaze result (input for spatial), when dehaze is active.
    pub(crate) dehazed_buffer: Vec<u8>,
    pub(crate) dehaze_cache_valid: bool,
    /// dehaze value for which `dehazed_buffer` was computed.
    pub(crate) dehaze_applied: f32,
    /// Whether `dehazed_buffer` was computed in fast mode (drag). On release
    /// this forces the map to be recomputed at full quality.
    pub(crate) dehaze_cached_fast: bool,
    pub(crate) processed_pixels: Vec<u8>, // final result for display (final_buffer)
    /// Output histogram of the current render (the HISTOGRAM panel).
    pub(crate) histogram: histogram::Histogram,
    /// Eased copy of the histogram bins that the panel actually draws. It is
    /// lerped toward the rendered target every frame so moving a slider does not
    /// make the plot jump; `histogram_display_path` makes a new image snap
    /// instead of morphing from the previous one.
    pub(crate) histogram_display: [[f32; 256]; 3],
    pub(crate) histogram_display_path: Option<PathBuf>,
    /// Clipping-warning view state (Histogram panel triangles + header toggle).
    pub(crate) clipping: clipping::ClippingView,
    /// The clipping overlay actually baked into the display texture, so a toggle
    /// re-uploads only when it changes.
    pub(crate) clip_overlay_applied: clipping::ClipOverlay,
    pub(crate) luma_cache: Vec<f32>,      // luma cache for spatial_pass (avoids reallocation)
    pub(crate) luma_cache_valid: bool,    // flag: is the luma_cache up to date?
    pub(crate) clarity_cache: Vec<f32>,   // blurred-map cache for Clarity (bilateral base)
    pub(crate) clarity_cache_sw: usize,   // clarity map width (downsampled)
    pub(crate) clarity_cache_sh: usize,   // clarity map height (downsampled)
    pub(crate) clarity_cache_valid: bool, // flag: is the clarity cache up to date?
    /// Precomputed spatial-pass data (Texture/Sharpen), depends only on the input.
    /// Reused until color/dehaze/frame size changes.
    pub(crate) spatial_base: processing::SpatialBase,
    pub(crate) spatial_base_valid: bool,
    pub(crate) texture: Option<egui::TextureHandle>,
    pub(crate) color_dirty: bool,
    pub(crate) spatial_dirty: bool,
    /// Retouch (spot heal) state: tool, layer, caches. See `app/retouch.rs`.
    pub(crate) retouch: retouch::RetouchState,
    /// Crop state: value + tool. See `app/crop.rs`.
    pub(crate) crop: crop::CropState,
    /// A retouch change requires re-running the heal + the pipeline.
    pub(crate) retouch_dirty: bool,
    /// Screen rect of the retouch options bar (set while drawing it), so canvas
    /// input does not heal through the sliders.
    pub(crate) retouch_options_rect: Option<egui::Rect>,
    /// Screen rect of the crop options bar (set while drawing it).
    pub(crate) crop_options_rect: Option<egui::Rect>,
    /// Screen rect of the floating tools panel (heal + before/after).
    pub(crate) tools_rect: Option<egui::Rect>,
    pub(crate) zoom_scale: f32,
    pub(crate) pan_offset: egui::Vec2, // image offset from the center (in screen pixels)
    /// Thumbnail cache for the filmstrip (keyed by file path).
    pub(crate) thumbnails: HashMap<PathBuf, egui::TextureHandle>,
    /// Receiver for background thumbnail generation.
    pub(crate) thumb_rx: Option<std::sync::mpsc::Receiver<(PathBuf, egui::ColorImage)>>,
    /// Sender for background thumbnail generation. Kept separately from the receiver
    /// so that when files are ADDED to an open session only the new thumbnails
    /// are loaded, without interrupting the one already running (shared channel for the whole session).
    pub(crate) thumb_tx: Option<std::sync::mpsc::Sender<(PathBuf, egui::ColorImage)>>,
    /// Receiver for background preview prefetch (frames following the current one).
    pub(crate) prefetch_rx: Option<std::sync::mpsc::Receiver<(PathBuf, RgbImage)>>,
    /// Background prefetch control (current index + stop signal).
    pub(crate) prefetch: Arc<image_io::PrefetchControl>,
    /// Whether the filmstrip should scroll to the current image (once after switching).
    pub(crate) filmstrip_needs_scroll: bool,
    pub(crate) split_position: f32,
    /// Expansion state of the left-panel groups: [LIGHT, COLOR, DETAILS, EFFECTS].
    pub(crate) open_groups: [bool; 4],
    /// Open/closed state of the CURVES group (its own field, see `SaveSettings`).
    pub(crate) open_curve_group: bool,
    /// Open/closed state of the COLOR MIXER group (its own field, so extending
    /// the four-entry `open_groups` array cannot break old configs).
    pub(crate) open_mixer_group: bool,
    /// The colour-mixer band being edited (0..8). UI-only, never serialized.
    pub(crate) mixer_band: usize,
    /// Tone-curve editor state: the channel being edited and, while dragging,
    /// the index of the grabbed control point.
    pub(crate) curve_channel: CurveChannel,
    pub(crate) curve_drag: Option<usize>,
    pub(crate) original_texture: Option<egui::TextureHandle>,
    pub(crate) is_dragging_split: bool,
    /// True while a left-drag is panning the image (classified at drag start).
    pub(crate) is_panning: bool,
    /// Screen rect of the floating toolbar pill (set while drawing it). Used to avoid
    /// replacing the cursor with the custom magnifier over the toolbar.
    pub(crate) toolbar_rect: Option<egui::Rect>,
    pub(crate) active_lut: Option<Arc<Lut3D>>,
    pub(crate) lut_path: Option<PathBuf>,
    /// Cache of parsed LUTs keyed by path (so .cube is not read/parsed on
    /// every frame switch and 3-MB data is not cloned).
    pub(crate) lut_cache: HashMap<PathBuf, Arc<Lut3D>>,
    pub(crate) combined_lut: Option<Lut3D>, // baked slider settings + external LUT (33³ during drag, 64³ otherwise)
    pub(crate) grain_map: Vec<f32>,
    /// Size for which `grain_map` was generated (for reuse).
    pub(crate) grain_map_dims: (usize, usize),
    /// Grain-size slider value `grain_map` was generated for (for reuse).
    pub(crate) grain_map_size: f32,
    /// LRU cache of decoded previews — instant frame switching.
    pub(crate) preview_cache: image_io::PreviewCache,
    pub(crate) drag_active: bool,
    pub(crate) was_dragging: bool, // whether a drag happened in the previous frame (for release detection)
    pub(crate) history: History,   // undo/redo stack
    pub(crate) drag_baseline: Option<Snapshot>, // state BEFORE the current slider movement started
    /// "Is the preset dirty" at the moment the gesture started. While the slider
    /// is being dragged, the unsaved-changes indicators take their value from here
    /// rather than from the live settings: otherwise, during fast movement the value
    /// momentarily matches the preset/default and the dot and Save button flicker.
    pub(crate) drag_preset_dirty: Option<bool>,
    /// "All sliders at default" at the moment the gesture started — so the Reset all
    /// button does not flicker while the value passes through the default during drag.
    pub(crate) drag_is_default: Option<bool>,
    pub(crate) full_render_pending: bool, // a full render is required after release
    pub(crate) first_frame: bool,
    /// Wait one frame before maximizing the window: first apply the window state,
    /// and only maximize on the next frame.
    pub(crate) pending_maximize: bool,
    /// Whether the previous frame was in editing mode (a frame is open).
    /// On a change of this flag the window is maximized/restored.
    pub(crate) was_editing: bool,
    pub(crate) last_render_time: Instant,
    pub(crate) lut_lib: LutLibrary,
    /// Search query in the LUT library (filter by name/category).
    pub(crate) lut_search: String,
    /// Show only favorites in the library (filter by ★).
    /// The separate "Favorites" section was removed: every time a star was toggled
    /// it changed height and reflowed the categories below.
    pub(crate) lut_favorites_only: bool,
    /// Names of expanded LUT library categories (saved to config).
    pub(crate) open_lut_categories: BTreeSet<String>,
    pub(crate) favorites_dirty: bool,
    pub(crate) last_favorite_toggle: Instant,
    pub(crate) last_input_dir: PathBuf,
    pub(crate) last_output_dir: PathBuf,
    pub(crate) save_settings_path: PathBuf,
    /// Stack of toast notifications (bottom-right corner).
    pub(crate) notifications: Vec<Notification>,
    pub(crate) show_save_dialog: bool,
    /// Confirmation of closing the session when there are unsaved changes.
    pub(crate) show_close_confirm: bool,
    pub(crate) save_format: SaveFormat,
    pub(crate) save_quality: u8,
    /// Carry the source PNG metadata into the saved PNG.
    pub(crate) embed_png_metadata: bool,
    pub(crate) preset_manager: PresetManager,
    pub(crate) session: Session,
    pub(crate) show_batch_progress: bool,
    pub(crate) batch_progress_message: String,
    pub(crate) batch_total: usize,
    pub(crate) batch_done: usize,
    /// Event receiver for background batch export.
    pub(crate) export_rx: Option<std::sync::mpsc::Receiver<export::ExportEvent>>,
    /// Cancellation flag for the current background export.
    pub(crate) export_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// "Apply settings to all images" dialog (mutates session).
    pub(crate) show_apply_all_dialog: bool,
    pub(crate) batch_preset_selected_index: Option<usize>,
    /// Custom postfix for batch export (Save All).
    pub(crate) batch_postfix: String,
    pub(crate) batch_use_current_settings: bool,
    /// Application mode: true — only to untouched frames.
    pub(crate) batch_apply_only_untouched: bool,
}

impl TinyLumaApp {
    /// Current editable state as a snapshot.
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            settings: self.settings,
            lut_path: self.lut_path.clone(),
            preset_name: self.current_preset_name(),
            retouch: self.retouch.layer.clone(),
            crop: self.crop.crop,
        }
    }

    /// Name of the preset selected in the list (even with unsaved changes).
    pub(crate) fn current_preset_name(&self) -> Option<String> {
        self.preset_manager
            .selected_index
            .and_then(|i| self.preset_manager.presets.get(i))
            .map(|p| p.name.clone())
    }

    /// Loads a LUT from the cache by path; on a miss reads the file and remembers it.
    /// Previously `.cube` was parsed (allocating a `Vec` per line) on every
    /// frame switch, undo/redo and preset application — that caused the lag.
    pub(crate) fn load_lut_cached(&mut self, path: &Path) -> Option<Arc<Lut3D>> {
        if let Some(lut) = self.lut_cache.get(path) {
            return Some(lut.clone());
        }
        let lut = Arc::new(Lut3D::load_from_file(path)?);
        // Cap the cache so dozens of 3-MB LUTs are not kept in memory.
        if self.lut_cache.len() >= 32 {
            self.lut_cache.clear();
        }
        self.lut_cache.insert(path.to_path_buf(), lut.clone());
        Some(lut)
    }

    /// Opens a `.cube` picker dialog and immediately applies the chosen LUT.
    /// Returns `true` if a LUT was loaded (so the settings changed).
    /// Needed so file loading is available even when a LUT is already applied.
    pub(crate) fn load_lut_from_dialog(&mut self) -> bool {
        let path = match rfd::FileDialog::new().add_filter("LUT", &["cube"]).pick_file() {
            Some(p) => p,
            None => return false,
        };
        let lut = match self.load_lut_cached(&path) {
            Some(l) => l,
            None => return false,
        };
        self.active_lut = Some(lut);
        self.lut_path = Some(path);
        // File loaded manually — nothing is selected in the library.
        self.lut_lib.selected_lut_name = None;
        self.settings.lut_intensity = 100.0;
        true
    }

    /// Restore state from a snapshot (reloading the LUT by path).
    pub(crate) fn restore(&mut self, snap: Snapshot) {
        self.settings = snap.settings;
        // First reset the current LUT, then set the new one if loading succeeds.
        self.active_lut = None;
        self.lut_path = None;
        self.lut_lib.selected_lut_name = None;

        if let Some(path) = &snap.lut_path {
            if let Some(lut) = self.load_lut_cached(path) {
                self.lut_path = Some(path.clone());
                self.active_lut = Some(lut);
                self.lut_lib.selected_lut_name = self
                    .lut_lib
                    .all_luts
                    .iter()
                    .find(|e| &e.path == path)
                    .map(|e| e.name.clone());
            }
        }
        self.combined_lut = None;
        // Restore the retouch layer and invalidate the heal + render caches.
        self.retouch.set_layer(snap.retouch);
        self.retouch_dirty = true;
        // An undo/redo during an active session restarts it from the restored
        // state, so a later `Esc` reverts to (and never past) this point.
        self.retouch.session_base = if self.retouch.active {
            Some(self.retouch.layer.clone())
        } else {
            None
        };
        self.retouch.session_hist_len = self.history.len();
        // Restore the crop (display-only, no re-render needed). Any in-progress
        // crop gesture is dropped so it cannot overwrite the restored value, and
        // `committed` follows so the frame-end commit does not re-record it.
        self.crop.crop = snap.crop;
        self.crop.dragging = false;
        self.crop.drag = None;
        self.crop.committed = self.crop.crop;
        // After an undo/redo the session keeps going from the restored value:
        // the next crop change starts a new single-entry operation.
        self.crop.session_baseline = if self.crop.active {
            Some(self.crop.crop)
        } else {
            None
        };
        self.crop.session_hist_len = self.history.len();
        self.crop.angle_base = None;
        if let Some(path) = &self.image_path {
            self.preview_cache.drop_render(path);
            let layer = self.retouch.layer.clone();
            self.session.save_retouch(path, layer);
            self.session.save_crop(path, self.crop.crop);
        }
        self.color_dirty = true;
        self.spatial_dirty = true;
        self.full_render_pending = true;

        // Restore the preset association from the snapshot.
        self.preset_manager.selected_index = snap.preset_name.as_ref().and_then(|name| {
            self.preset_manager
                .presets
                .iter()
                .position(|p| &p.name == name)
        });

        // Keep the session up to date so the modified counter is correct.
        if let Some(path) = self.image_path.clone() {
            let preset_name = self.current_preset_name();
            self.session.save_current_settings(
                &path,
                self.settings,
                self.lut_path.clone(),
                preset_name,
            );
        }
    }

    /// Applies a preset. Returns a warning if the preset's LUT could not be
    /// loaded (file missing/unavailable) — then the LUT is forcibly cleared.
    pub(crate) fn apply_preset(&mut self, index: usize) -> Option<String> {
        // Clone what is needed so we don't hold a borrow of preset_manager.
        let Some((settings, lut_path, name)) = self
            .preset_manager
            .presets
            .get(index)
            .map(|p| (p.settings, p.lut_path.clone(), p.name.clone()))
        else {
            return None;
        };

        self.settings = settings;

        // The LUT is now addressed by path. If the path exists and the file loads — apply it,
        // otherwise clear EXPLICITLY so the previous LUT does not remain.
        let loaded_lut = lut_path
            .as_ref()
            .and_then(|p| self.load_lut_cached(p).map(|lut| (p.clone(), lut)));

        let warning = if let Some((path, lut)) = loaded_lut {
            self.active_lut = Some(lut);
            // Display name — if the path is present in the library.
            self.lut_lib.selected_lut_name = self
                .lut_lib
                .all_luts
                .iter()
                .find(|e| e.path == path)
                .map(|e| e.name.clone());
            self.lut_path = Some(path);
            None
        } else {
            self.active_lut = None;
            self.lut_path = None;
            self.lut_lib.selected_lut_name = None;
            lut_path.as_ref().map(|p| {
                let file = p
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().into_owned());
                format!("Preset \"{}\": LUT not found ({file})", name)
            })
        };

        self.combined_lut = None;
        self.preset_manager.selected_index = Some(index);
        println!("🎯 Preset applied: {}", name);
        warning
    }

    /// Syncs the selection in the preset list with the current state:
    /// if the settings and LUT exactly match some preset — select it,
    /// otherwise clear the selection. Needed so that when returning to a frame with
    /// an already applied preset it is shown as selected again and the
    /// unsaved-changes indication works correctly.
    pub(crate) fn sync_selected_preset(&mut self) {
        self.preset_manager.selected_index = self
            .preset_manager
            .presets
            .iter()
            .position(|p| p.settings == self.settings && p.lut_path == self.lut_path);
    }

    /// Applies settings (and LUT) to all images of the session as per-file
    /// state, WITHOUT touching files on disk. The user can then fine-tune
    /// each frame with the sliders and export everything via "Save All".
    ///
    /// `only_untouched` — skip frames that already have edits.
    /// Returns (number of frames affected, warning).
    pub(crate) fn apply_to_all(
        &mut self,
        settings: FilterSettings,
        lut_path: Option<PathBuf>,
        preset_name: Option<String>,
        only_untouched: bool,
    ) -> (usize, Option<String>) {
        // Validate the LUT once: a bad path is better discarded, otherwise frames
        // would be marked as modified but render as an identity pass.
        let mut warning = None;
        let lut_path = match lut_path {
            Some(p) => {
                if self.load_lut_cached(&p).is_some() {
                    Some(p)
                } else {
                    let file = p
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.to_string_lossy().into_owned());
                    warning = Some(format!("LUT not found ({file}) — applied without LUT"));
                    None
                }
            }
            None => None,
        };

        let default = FilterSettings::default();
        let mut applied = 0usize;
        let mut current_applied = false;

        // Clone the list of paths: we mutate self.session inside the body.
        for path in self.session.image_list.clone() {
            let is_current = self.image_path.as_ref() == Some(&path);

            // "Before" state: for the current frame — live, for the rest — from the map.
            let (old_settings, old_lut, old_preset) = if is_current {
                (
                    self.settings,
                    self.lut_path.clone(),
                    self.current_preset_name(),
                )
            } else {
                self.session.load_settings(&path)
            };
            let old_layer = if is_current {
                self.retouch.layer.clone()
            } else {
                self.session.load_retouch(&path)
            };

            // A frame is "touched" if it has edits (not default, a LUT, or heal spots).
            let touched = old_settings != default || old_lut.is_some() || !old_layer.is_empty();
            if only_untouched && touched {
                continue;
            }

            // Nothing changes — don't create empty history entries.
            if old_settings == settings && old_lut == lut_path && old_preset == preset_name {
                continue;
            }

            let before = Snapshot {
                settings: old_settings,
                lut_path: old_lut,
                preset_name: old_preset,
                retouch: old_layer,
                crop: self.session.load_crop(&path),
            };
            // Put the "before" snapshot into this frame's history so that undo/redo
            // works even on frames the user has not switched to yet.
            if is_current {
                self.history.push(before);
            } else {
                self.session
                    .history_map
                    .entry(path.clone())
                    .or_insert_with(History::new)
                    .push(before);
            }

            self.session.save_current_settings(
                &path,
                settings,
                lut_path.clone(),
                preset_name.clone(),
            );

            if is_current {
                current_applied = true;
            }
            applied += 1;
        }

        // If anything was applied — cached renders are stale. Keep the decodes (base):
        // they are still valid and reused when returning.
        if applied > 0 {
            self.preview_cache.drop_renders();
        }

        // Keep the current frame's live state in step with the applied preset
        // so the preview and sliders immediately reflect the result.
        if current_applied {
            self.settings = settings;
            self.active_lut = None;
            self.lut_path = None;
            self.lut_lib.selected_lut_name = None;
            if let Some(p) = &lut_path {
                if let Some(lut) = self.load_lut_cached(p) {
                    self.lut_path = Some(p.clone());
                    self.active_lut = Some(lut);
                    self.lut_lib.selected_lut_name = self
                        .lut_lib
                        .all_luts
                        .iter()
                        .find(|e| &e.path == p)
                        .map(|e| e.name.clone());
                }
            }
            self.preset_manager.selected_index = preset_name.as_ref().and_then(|name| {
                self.preset_manager
                    .presets
                    .iter()
                    .position(|p| &p.name == name)
            });
            self.combined_lut = None;
            self.color_dirty = true;
            self.spatial_dirty = true;
            self.full_render_pending = true;
        }

        (applied, warning)
    }

    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Unified theme: color tokens + Visuals setup (Apple dark).
        crate::theme::apply(&cc.egui_ctx);

        // Phosphor icon font: registered as a fallback for
        // proportional text so icons can be embedded
        // directly in strings next to labels (e.g. "{} Save").
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let mut lut_lib = LutLibrary::new();
        // Automatic scan of the "luts" folder at startup
        // Create the config folder if it does not exist
        let _ = std::fs::create_dir_all("config");

        lut_lib.scan_folder(PathBuf::from("luts"));
        // Path for saving favorites
        lut_lib.favorites_path = Some(PathBuf::from("config/favorites.json"));
        lut_lib.load_favorites();

        let saved = Self::load_save_settings();

        Self {
            settings: FilterSettings::default(),
            image_path: None,
            preview_base: None,
            color_buffer: Vec::new(),
            color_buffer_valid: false,
            dehazed_buffer: Vec::new(),
            dehaze_cache_valid: false,
            dehaze_applied: f32::NAN,
            dehaze_cached_fast: false,
            processed_pixels: Vec::new(),
            histogram: histogram::Histogram::EMPTY,
            histogram_display: [[0.0; 256]; 3],
            histogram_display_path: None,
            clipping: clipping::ClippingView::default(),
            clip_overlay_applied: clipping::ClipOverlay::NONE,
            luma_cache: Vec::new(),
            luma_cache_valid: false,
            clarity_cache: Vec::new(),
            clarity_cache_sw: 0,
            clarity_cache_sh: 0,
            clarity_cache_valid: false,
            spatial_base: processing::SpatialBase::default(),
            spatial_base_valid: false,
            texture: None,
            color_dirty: false,
            spatial_dirty: false,
            retouch: {
                // Start with the brush the user is comfortable with (persisted in
                // the config), not the factory default.
                let mut retouch = retouch::RetouchState::default();
                retouch.brush.size = saved.brush_size;
                retouch.brush.hardness = saved.brush_hardness;
                retouch
            },
            retouch_dirty: false,
            crop: crop::CropState::default(),
            retouch_options_rect: None,
            crop_options_rect: None,
            tools_rect: None,
            zoom_scale: 1.0,
            pan_offset: egui::Vec2::ZERO,
            thumbnails: HashMap::new(),
            thumb_rx: None,
            thumb_tx: None,
            prefetch_rx: None,
            prefetch: Arc::new(image_io::PrefetchControl::new()),
            filmstrip_needs_scroll: false,
            original_texture: None,
            is_dragging_split: false,
            is_panning: false,
            toolbar_rect: None,
            active_lut: None,
            lut_path: None,
            lut_cache: HashMap::new(),
            combined_lut: None,
            grain_map: Vec::new(),
            grain_map_dims: (0, 0),
            grain_map_size: crate::settings::default_grain_size(),
            // The preview cache budget is set in bytes (base + render), so
            // memory does not grow linearly with the number of open frames. At 1200px
            // 256 MiB is ~30–40 previews around the current one.
            preview_cache: image_io::PreviewCache::new(image_io::PREVIEW_BUDGET_BYTES),
            drag_active: false,
            was_dragging: false,
            history: History::new(),
            drag_baseline: None,
            drag_preset_dirty: None,
            drag_is_default: None,
            full_render_pending: false,
            first_frame: true,
            pending_maximize: false,
            was_editing: false,
            last_render_time: Instant::now(),
            lut_lib,
            lut_search: String::new(),
            lut_favorites_only: saved.lut_favorites_only,
            open_lut_categories: saved.open_lut_categories,
            favorites_dirty: false,
            last_favorite_toggle: Instant::now(),
            save_settings_path: PathBuf::from("config/save_settings.json"),
            notifications: Vec::new(),
            show_save_dialog: false,
            show_close_confirm: false,
            last_input_dir: saved
                .last_input_dir
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_default(),
            last_output_dir: saved
                .last_output_dir
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_default(),
            split_position: saved.split_position,
            open_groups: saved.open_groups,
            open_curve_group: saved.open_curve_group,
            open_mixer_group: saved.open_mixer_group,
            mixer_band: 0,
            curve_channel: CurveChannel::Master,
            curve_drag: None,
            save_format: saved.format,
            save_quality: saved.quality,
            embed_png_metadata: saved.embed_png_metadata,
            preset_manager: PresetManager::new(),
            session: Session::new(),
            show_batch_progress: false,
            batch_progress_message: String::new(),
            batch_total: 0,
            batch_done: 0,
            export_rx: None,
            export_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            show_apply_all_dialog: false,
            batch_preset_selected_index: None,
            batch_postfix: saved.postfix,
            batch_use_current_settings: true,
            batch_apply_only_untouched: false,
        }
    }
}
