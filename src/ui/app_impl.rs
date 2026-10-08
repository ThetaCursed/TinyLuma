// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use eframe::egui;
use std::path::PathBuf;

use crate::app::TinyLumaApp;
use crate::settings::FilterSettings;
use crate::theme;
use crate::ui::notification::ToastKind;

impl eframe::App for TinyLumaApp {
    /// Transparent window background in hero mode (we draw the rounded corners ourselves),
    /// the normal dark one — when a frame is open.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.texture.is_none() {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        } else {
            theme::BG_WINDOW.to_normalized_gamma_f32()
        }
    }

    /// Single save point on exit: flush the settings to disk
    /// (including remembered folders) and the LUT favorites. Otherwise edits from the last
    /// 500ms (favorites debounce) or changes without an explicit action could be
    /// lost on close.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.lut_lib.save_favorites();
        self.save_save_settings();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Reset drag_active at the start of every frame — it will be set to true
        // if at least one slider is active
        self.drag_active = false;

        // Drag-drop: we support multiple files AND folders.
        let dropped_paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });

        if !dropped_paths.is_empty() {
            // A folder was dropped — expand it into a list of supported images.
            let mut files: Vec<PathBuf> = Vec::new();
            for path in dropped_paths {
                if path.is_dir() {
                    files.extend(crate::app::image_io::collect_images_from_folder(
                        &path, false,
                    ));
                } else {
                    files.push(path);
                }
            }
            // Remove duplicates (e.g. a file and its folder dropped together).
            let mut seen = std::collections::HashSet::new();
            files.retain(|p| seen.insert(p.clone()));

            if files.is_empty() {
                self.notify(ToastKind::Warning, "No supported images found");
            } else {
                // If a session is already open — ADD the frames instead of replacing it:
                // the already open images stay in the filmstrip.
                self.add_images(files, ctx);
            }
        }

        // Keyboard navigation over the session (we don't interfere with text-field input)
        if self.session.total() > 1 && self.texture.is_some() && !ctx.wants_keyboard_input() {
            let left = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
            let right = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight));
            if left && self.session.current_index > 0 {
                self.switch_to_image(self.session.current_index - 1, ctx);
            }
            if right && self.session.current_index + 1 < self.session.total() {
                self.switch_to_image(self.session.current_index + 1, ctx);
            }

            // Delete/Backspace — remove the current frame from the session (without deleting the file).
            let delete = ctx.input(|i| i.key_pressed(egui::Key::Delete))
                || ctx.input(|i| i.key_pressed(egui::Key::Backspace));
            if delete {
                self.remove_from_session(self.session.current_index, ctx);
            }
        }

        // On the first frame center the compact window on the monitor.
        if self.first_frame {
            self.first_frame = false;
            let vp = ctx.input(|i| i.viewport().clone());
            if let (Some(monitor), Some(outer)) = (vp.monitor_size, vp.outer_rect) {
                let offset = (monitor - outer.size()) * 0.5;
                let pos = egui::pos2(offset.x.max(0.0), offset.y.max(0.0));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
        }

        // --- Window mode ---
        // The window is always borderless (we draw our own title bar and buttons). While no frame
        // is open — a compact rounded hero window; opening a frame maximizes it.
        let editing = self.texture.is_some();
        if editing != self.was_editing {
            self.was_editing = editing;
            if editing {
                // Entered editing: maximize the window on the NEXT frame and
                // raise the minimum — otherwise the center shrinks and elements overlap.
                self.pending_maximize = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                    crate::ui::EDIT_MIN_W,
                    crate::ui::EDIT_MIN_H,
                )));
                // Set the "normal" size BEFORE maximizing: then restore down
                // returns a comfortable window, not the hero size (otherwise the layout briefly
                // shrinks and the buttons shift until the first manual resize).
                let vp = ctx.input(|i| i.viewport().clone());
                let mut w = crate::ui::EDIT_DEFAULT_W;
                let mut h = crate::ui::EDIT_DEFAULT_H;
                if let Some(mon) = vp.monitor_size {
                    w = w.min(mon.x);
                    h = h.min(mon.y);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
            } else {
                // Returned to hero: restore the window (if it was minimized),
                // un-maximize and return to the compact size.
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                    crate::ui::HERO_W,
                    crate::ui::HERO_H,
                )));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    crate::ui::HERO_W,
                    crate::ui::HERO_H,
                )));
            }
            ctx.request_repaint();
        }
        if self.pending_maximize && editing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            self.pending_maximize = false;
            ctx.request_repaint();
        }

        // Resize from the window edges (a borderless window needs its own handles).
        if editing {
            self.handle_window_resize(ctx);
        }

        // In hero mode the panels must not fill the corners — we draw them rounded.
        let hero = !editing;
        let panel_fill = if hero {
            egui::Color32::TRANSPARENT
        } else {
            theme::BG_WINDOW
        };
        ctx.style_mut(|style| style.visuals.panel_fill = panel_fill);
        if hero {
            let screen = ctx.screen_rect();
            let p = ctx.layer_painter(egui::LayerId::background());
            p.rect_filled(screen, theme::RADIUS_LG, theme::BG_WINDOW);
            p.rect_stroke(
                screen,
                theme::RADIUS_LG,
                egui::Stroke::new(1.0, theme::DIVIDER),
            );
        }

        // Our own window title bar (drag + minimize/maximize/close).
        self.show_titlebar(ctx);

        // --- UNDO / REDO and ZOOM (hotkeys, before the frame snapshot) ---
        self.handle_shortcuts(ctx);
        // Retouch hotkeys ([ / ] size, Shift+[ / Shift+] hardness, Esc).
        self.handle_retouch_shortcuts(ctx);

        // Drain the finished thumbnails from the background thread (filmstrip)
        self.poll_thumbnails(ctx);
        self.poll_prefetch(ctx);
        self.poll_export(ctx);

        // State at the start of the frame (AFTER undo/redo) — the base for the history entry.
        // IMPORTANT: taken BEFORE the panels, otherwise the initial frame of a slider drag
        // already contains the first change.
        let frame_start = self.snapshot();

        // Debounced favorites save (wait 500ms after the last toggle).
        // IMPORTANT: without request_repaint_after the timer would fire only on the next
        // user action (eframe sleeps when there is no input), and edits could be lost
        // on close/crash. So we explicitly schedule a frame for the remaining time.
        const FAVORITES_DEBOUNCE: std::time::Duration =
            std::time::Duration::from_millis(500);
        if self.favorites_dirty {
            let elapsed = self.last_favorite_toggle.elapsed();
            if elapsed >= FAVORITES_DEBOUNCE {
                self.lut_lib.save_favorites();
                self.favorites_dirty = false;
            } else {
                ctx.request_repaint_after(FAVORITES_DEBOUNCE - elapsed);
            }
        }

        // The side panels exist only when a frame is open: on the start
        // screen only the central "Get Started" card remains.
        let panels_visible = self.texture.is_some();

        // Left panel (presets + sliders)
        let (mut color_changed, spatial_changed) = self.show_left_panel(ctx, panels_visible);

        // Right panel (LUT). Called BEFORE CentralPanel — this matters for centering.
        if self.draw_lut_panel(ctx, panels_visible) {
            color_changed = true;
        }
        if color_changed {
            self.color_dirty = true;
        }
        if spatial_changed {
            self.spatial_dirty = true;
        }

        // Sync the current settings with the session ONLY when the gesture is finished
        // (or it is an atomic change: preset, Reset all, LUT). While a slider
        // is being dragged — we don't touch the session, otherwise the "modified" counters/dots
        // are recomputed every frame and flicker during fast movement.
        // The session snapshot is updated below, in the gesture-completion block.
        if (color_changed || spatial_changed) && self.image_path.is_some() && !self.drag_active {
            if let Some(path) = self.image_path.as_ref() {
                let preset_name = self.current_preset_name();
                self.session.save_current_settings(
                    path,
                    self.settings,
                    self.lut_path.clone(),
                    preset_name,
                );
            }
        }

        // --- HISTORY: coalescing slider gestures ---
        // Start of movement: remember the state BEFORE the first change, and also
        // the preset "dirtiness" at that moment — this is what the indicators show until
        // the end of the gesture.
        if self.drag_active && !self.was_dragging {
            self.drag_baseline = Some(frame_start.clone());
            self.drag_preset_dirty =
                Some(self.preset_dirty_with(&frame_start.settings, &frame_start.lut_path));
            self.drag_is_default = Some(frame_start.settings == FilterSettings::default());
        }

        // Slider release detection: we commit ONE entry for the whole gesture,
        // not a hundred for the frames of dragging. Here we also write the final
        // settings to the session and unfreeze the dirty indicators.
        if self.was_dragging && !self.drag_active {
            if let Some(base) = self.drag_baseline.take() {
                if base != self.snapshot() {
                    self.history.push(base);
                }
            }
            self.drag_preset_dirty = None;
            self.drag_is_default = None;
            if let Some(path) = self.image_path.clone() {
                let preset_name = self.current_preset_name();
                self.session.save_current_settings(
                    &path,
                    self.settings,
                    self.lut_path.clone(),
                    preset_name,
                );
            }
            self.color_dirty = true;
            self.spatial_dirty = true;
            self.full_render_pending = true;
        }

        // Atomic changes without drag: preset, Reset all, LUT selection, delete preset.
        if (color_changed || spatial_changed) && !self.drag_active && !self.was_dragging {
            if frame_start != self.snapshot() {
                self.history.push(frame_start);
            }
        }

        self.was_dragging = self.drag_active;

        // Central panel with the image
        self.show_center_panel(ctx);

        // Modal windows (save, batch)
        self.show_dialogs(ctx);

        // Notifications — on top of the whole interface.
        self.show_notifications(ctx);

        if self.color_dirty || self.spatial_dirty {
            ctx.request_repaint();
        }
    }
}

impl TinyLumaApp {
    /// Sets a new zoom level without touching the pan. Used by wheel/pinch, where the
    /// point under the cursor must stay put and the user's framing is preserved.
    pub(crate) fn set_zoom(&mut self, new_zoom: f32) {
        self.zoom_scale = new_zoom.clamp(0.1, 5.0);
    }

    /// Zoom via the +/- buttons, Fit, hotkeys or the image click toggle: zooming out
    /// smoothly pulls the pan back toward the center, so the 30% overscroll fades out
    /// as the image approaches "fit" instead of snapping there. At fit or below the
    /// image is exactly centered. Zooming in keeps the position intact.
    pub(crate) fn apply_zoom(&mut self, new_zoom: f32) {
        let old_zoom = self.zoom_scale;
        let new_zoom = new_zoom.clamp(0.1, 5.0);

        // Only when zooming out: taper the pan to zero over the last stretch before
        // fit (from FIT_TAPER down to 1.0x).
        if new_zoom < old_zoom {
            const FIT_TAPER: f32 = 1.5;
            let f = ((new_zoom - 1.0) / (FIT_TAPER - 1.0)).clamp(0.0, 1.0);
            self.pan_offset *= f;
        }

        self.zoom_scale = new_zoom;
        if new_zoom <= 1.0 {
            self.pan_offset = egui::Vec2::ZERO;
        }
    }

    /// Undo/redo and zoom hotkeys. They do not fire in a text field.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }

        use egui::{Key, KeyboardShortcut, Modifiers};

        let undo = ctx
            .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)));
        let redo = ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::SHIFT,
                Key::Z,
            ))
        }) || ctx
            .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)));

        if undo {
            if let Some(s) = self.history.undo(self.snapshot()) {
                self.restore(s);
            }
        } else if redo {
            if let Some(s) = self.history.redo(self.snapshot()) {
                self.restore(s);
            }
        }

        // --- Zoom ---
        let zoom_in = ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Plus))
        }) || ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals))
        });
        let zoom_out = ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus))
        });
        let zoom_reset = ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0))
        });

        if self.texture.is_some() {
            let zoom = self.zoom_scale;
            if zoom_in {
                self.apply_zoom(zoom * 1.25);
            }
            if zoom_out {
                self.apply_zoom(zoom / 1.25);
            }
        }
        if zoom_reset {
            self.apply_zoom(1.0);
        }

        // --- BEFORE / AFTER split: `\` (as in Lightroom) toggles the
        // before/after divider on and off, exactly like the toolbar button.
        if self.texture.is_some()
            && self.original_texture.is_some()
            && ctx.input(|i| i.key_pressed(Key::Backslash))
        {
            self.split_position = if self.split_position > 0.0 { 0.0 } else { 0.5 };
            self.save_save_settings();
        }
    }
}
