// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use super::filmstrip::{FILMSTRIP_PAD, THUMB_HEIGHT};
use super::notification::ToastKind;
use crate::app::TinyLumaApp;
use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;
use std::path::PathBuf;

impl TinyLumaApp {
    /// Central panel: toolbar, image, filmstrip, bottom buttons, empty screen.
    pub(crate) fn show_center_panel(&mut self, ctx: &egui::Context) {
        // In editing the central area is slightly darker than the panels
        // (canvas): the border is given by the background itself, without separator lines.
        let frame = if self.texture.is_some() {
            egui::Frame::central_panel(&ctx.style()).fill(theme::BG_DEEP)
        } else {
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT)
        };
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            self.process_preview(ctx);

            if self.texture.is_some() {
                // Canvas background — the dot grid of the "infinite canvas" is drawn
                // in draw_image_viewport (in the letterbox, under the frame), so
                // there is no more "sunken" shadow at the edges: the grid and shadows
                // would fight each other.

                // From here on we manage the vertical margins manually:
                // this way the block heights add up exactly and no "floating" gap appears.
                ui.spacing_mut().item_spacing.y = 0.0;

                // Grab the texture data BEFORE the nested closures (otherwise a borrow conflict)
                let tex_id = self.texture.as_ref().unwrap().id();
                let tex_size = self.texture.as_ref().unwrap().size_vec2();
                let orig_tex_id = self.original_texture.as_ref().map(|t| t.id());
                let has_multi = self.session.total() > 1;

                let avail = ui.available_size();

                // Exact block heights (with item_spacing.y == 0 they simply add up).
                const BUTTONS_H: f32 = 36.0; // 6(line) + 2 + button(26) + 2
                let filmstrip_h = if has_multi {
                    // The "tray" with margins above and below for the thumbnail.
                    THUMB_HEIGHT + FILMSTRIP_PAD * 2.0
                } else {
                    0.0
                };
                let image_h = (avail.y - BUTTONS_H - filmstrip_h).max(0.0);

                // --- IMAGE ---
                let image_size = egui::vec2(avail.x, image_h);
                ui.allocate_ui_with_layout(
                    image_size,
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| {
                        ui.set_min_size(image_size);
                        self.draw_image_viewport(ui, ctx, tex_id, tex_size, orig_tex_id);
                    },
                );

                // --- FILMSTRIP (only with multiple images) ---
                if has_multi {
                    let fs_size = egui::vec2(avail.x, filmstrip_h);
                    ui.allocate_ui_with_layout(
                        fs_size,
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.set_min_size(fs_size);
                            // The "tray" under the filmstrip: separates the strip from the image
                            // and serves as the background on which the cards read without
                            // outlines. The corners are square: with rounding, the thumbnails at the edges
                            // (when the strip overflows) stuck out past the rounded corner —
                            // the ScrollArea clip is rectangular. No top divider
                            // — the backing itself acts as the border.
                            //
                            // Horizontally we stretch to the edges of the central panel
                            // (the clip_rect is wider than the content by the panel margins) so the tray
                            // sits flush against the side panels with no gap.
                            let band = ui.max_rect();
                            let clip = ui.clip_rect();
                            let tray = egui::Rect::from_min_max(
                                egui::pos2(clip.left(), band.top()),
                                egui::pos2(clip.right(), band.bottom()),
                            );
                            let painter = ui.painter();
                            painter.rect_filled(tray, 0.0, theme::FILMSTRIP_BG);
                            // No lines above/below: the tray differs from the canvas
                            // by the background itself, not by a stroke.
                            self.show_filmstrip(ui, ctx);
                        },
                    );
                }

                // --- BUTTONS + BOTTOM LINE ---
                let bs = egui::vec2(avail.x, BUTTONS_H);
                ui.allocate_ui_with_layout(bs, egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_min_size(bs);
                    // We do not draw a divider between the canvas and the buttons: the border
                    // is given by the background itself. We keep the padding (with and without a filmstrip)
                    // so the buttons do not jump when the strip appears/disappears.
                    ui.add_space(6.0);
                    self.show_action_buttons(ui, has_multi);
                });
            } else {
                // --- START SCREEN (no open images) ---
                ui.vertical_centered(|ui| {
                    // Estimate the content height for vertical centering.
                    // The window is short — pin to the top (top_space = 0).
                    const CONTENT_H: f32 = 330.0;
                    let top_space = ((ui.available_height() - CONTENT_H) / 2.0).max(0.0);
                    ui.add_space(top_space);

                    // --- App icon tile ---
                    const TILE: f32 = 54.0;
                    let (tile_rect, _) =
                        ui.allocate_exact_size(egui::vec2(TILE, TILE), egui::Sense::hover());
                    {
                        let p = ui.painter();
                        p.rect_filled(tile_rect, theme::RADIUS_LG, theme::BG_SURFACE);
                        p.rect_stroke(
                            tile_rect,
                            theme::RADIUS_LG,
                            egui::Stroke::new(1.0, theme::SEPARATOR),
                        );
                        p.text(
                            tile_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            ph::APERTURE,
                            egui::FontId::proportional(48.0),
                            theme::ACCENT,
                        );
                    }

                    ui.add_space(12.0);

                    // --- Wordmark "TinyLuma": accent on the meaningful part ---
                    {
                        use egui::text::{LayoutJob, TextFormat};
                        let font = egui::FontId::proportional(29.0);
                        let mut job = LayoutJob::default();
                        job.append(
                            "Tiny",
                            0.0,
                            TextFormat {
                                font_id: font.clone(),
                                color: theme::TEXT,
                                ..Default::default()
                            },
                        );
                        job.append(
                            "Luma",
                            0.0,
                            TextFormat {
                                font_id: font,
                                color: theme::ACCENT,
                                ..Default::default()
                            },
                        );
                        ui.label(job);
                    }

                    ui.add_space(5.0);
                    ui.label(
                        egui::RichText::new(crate::ui::APP_TAGLINE)
                            .size(13.0)
                            .color(theme::TEXT_SECONDARY),
                    );

                    ui.add_space(20.0);

                    // --- Call-to-action card ---
                    egui::Frame::group(ui.style())
                        .rounding(theme::RADIUS_LG)
                        .fill(theme::BG_SURFACE)
                        .inner_margin(egui::Margin::symmetric(26.0, 20.0))
                        .show(ui, |ui| {
                            ui.set_width(360.0);

                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new("Give your images the finishing touch")
                                        .size(17.0)
                                        .strong()
                                        .color(theme::TEXT),
                                );
                                ui.add_space(14.0);

                                let btn_size = egui::vec2(158.0, 44.0);
                                let gap = 10.0;
                                let mut open_files = false;
                                let mut open_folder = false;

                                ui.horizontal(|ui| {
                                    let total = btn_size.x * 2.0 + gap;
                                    let pad = ((ui.available_width() - total) / 2.0).max(0.0);
                                    ui.add_space(pad);

                                    if super::widgets::primary_button(
                                        ui,
                                        egui::RichText::new(format!("{} Open Images", ph::IMAGES))
                                            .size(15.0),
                                        btn_size,
                                    )
                                    .clicked()
                                    {
                                        open_files = true;
                                    }

                                    ui.add_space(gap);

                                    if ui
                                        .add(
                                            egui::Button::new(
                                                egui::RichText::new(format!(
                                                    "{} Open Folder",
                                                    ph::FOLDER_OPEN
                                                ))
                                                .size(15.0),
                                            )
                                            .min_size(btn_size),
                                        )
                                        .clicked()
                                    {
                                        open_folder = true;
                                    }

                                    ui.add_space(pad);
                                });

                                // --- Open the selected images ---
                                if open_files {
                                    let mut dialog =
                                        rfd::FileDialog::new().set_title("Open images").add_filter(
                                            "Images",
                                            crate::ui::SUPPORTED_IMAGE_EXTENSIONS,
                                        );
                                    // Start in the remembered folder only if it still exists:
                                    // it could have been deleted/moved between runs.
                                    if self.last_input_dir.is_dir() {
                                        dialog = dialog.set_directory(&self.last_input_dir);
                                    }
                                    let paths: Vec<PathBuf> =
                                        dialog.pick_files().unwrap_or_default();
                                    // The source folder is remembered by open_images() — a single
                                    // point for the dialog and for drag & drop.
                                    if !paths.is_empty() {
                                        self.open_images(paths, ctx);
                                    }
                                }

                                // --- Open all images from a folder ---
                                if open_folder {
                                    let mut dialog =
                                        rfd::FileDialog::new().set_title("Open folder");
                                    if self.last_input_dir.is_dir() {
                                        dialog = dialog.set_directory(&self.last_input_dir);
                                    }
                                    if let Some(folder) = dialog.pick_folder() {
                                        let paths =
                                            crate::app::image_io::collect_images_from_folder(
                                                &folder, false,
                                            );
                                        if paths.is_empty() {
                                            self.notify(
                                                ToastKind::Warning,
                                                "No supported images in this folder",
                                            );
                                        } else {
                                            self.open_images(paths, ctx);
                                        }
                                    }
                                }

                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}   or drop images or a folder anywhere",
                                        ph::DOWNLOAD_SIMPLE
                                    ))
                                    .size(12.0)
                                    .color(theme::TEXT_TERTIARY),
                                );
                            });
                        });

                    // --- Author credit (clickable → GitHub, on one line) ---
                    ui.add_space(16.0);
                    ui.hyperlink_to(
                        egui::RichText::new(format!(
                            "{}  by {}   ·   v{}",
                            ph::GITHUB_LOGO,
                            crate::ui::AUTHOR,
                            crate::ui::APP_VERSION
                        ))
                        .size(12.0),
                        crate::ui::GITHUB_URL,
                    );
                });
            }

            // --- Drag & drop zone highlight ---
            // While the user drags files over the window, we show an accent
            // frame and a hint. Works on the start screen and with an already
            // open session (dropped files are added to the filmstrip).
            let dragging_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
            if dragging_files {
                let rect = ui.max_rect();
                let painter = ui.painter();
                // In hero mode the window is rounded — the overlay follows the shape.
                let rounding = if self.texture.is_none() {
                    theme::RADIUS_LG
                } else {
                    0.0
                };
                painter.rect_filled(rect, rounding, theme::with_alpha(theme::BG_DEEP, 228));
                painter.rect_stroke(
                    rect.shrink(14.0),
                    theme::RADIUS_LG,
                    egui::Stroke::new(2.0, theme::ACCENT),
                );
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}   Drop images or a folder", ph::DOWNLOAD_SIMPLE),
                    egui::FontId::proportional(20.0),
                    theme::TEXT,
                );
            }
        });
    }

    /// Our own window title bar instead of the system one: drag, minimize/maximize/close,
    /// the app name on the left, the file name centered in editing.
    /// Always shown (both in hero and in editing).
    pub(crate) fn show_titlebar(&mut self, ctx: &egui::Context) {
        const H: f32 = 28.0;
        const BTN_W: f32 = 38.0;
        let editing = self.texture.is_some();
        let filename = self
            .image_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());

        egui::TopBottomPanel::top("app_titlebar")
            .exact_height(H)
            .frame(egui::Frame::none())
            // No line under the title bar: it is separated from the canvas by the background itself.
            .show_separator_line(false)
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let close_rect = egui::Rect::from_min_size(
                    egui::pos2(full.right() - BTN_W, full.top()),
                    egui::vec2(BTN_W, H),
                );
                let max_rect = egui::Rect::from_min_size(
                    egui::pos2(close_rect.left() - BTN_W, full.top()),
                    egui::vec2(BTN_W, H),
                );
                // In hero the maximize button is not needed: the window is fixed there, and
                // the minimum is set flush against "close" without extra gap.
                let min_rect = if editing {
                    egui::Rect::from_min_size(
                        egui::pos2(max_rect.left() - BTN_W, full.top()),
                        egui::vec2(BTN_W, H),
                    )
                } else {
                    egui::Rect::from_min_size(
                        egui::pos2(close_rect.left() - BTN_W, full.top()),
                        egui::vec2(BTN_W, H),
                    )
                };

                // Drag — the whole title bar except the buttons.
                let drag_rect =
                    egui::Rect::from_min_max(full.min, egui::pos2(min_rect.left(), full.bottom()));
                let drag = ui.interact(
                    drag_rect,
                    ui.id().with("titlebar_drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if editing && drag.double_clicked() {
                    self.toggle_maximize(ctx);
                }

                // Window buttons on the right: minimize, [maximize/restore], close.
                if window_button(ui, min_rect, ph::MINUS, "min", false) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if editing {
                    // The icon depends on the window state: a square — maximize,
                    // a "double square" — restore the previous size.
                    let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
                    let max_icon = if maximized { ph::COPY } else { ph::SQUARE };
                    if window_button(ui, max_rect, max_icon, "max", false) {
                        self.toggle_maximize(ctx);
                    }
                }
                if window_button(ui, close_rect, ph::X, "close", true) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                // Left — the app; centered (in editing) — the file name.
                let p = ui.painter();
                p.text(
                    egui::pos2(full.left() + 12.0, full.center().y),
                    egui::Align2::LEFT_CENTER,
                    ph::APERTURE,
                    egui::FontId::proportional(16.0),
                    theme::ACCENT,
                );
                p.text(
                    egui::pos2(full.left() + 32.0, full.center().y),
                    egui::Align2::LEFT_CENTER,
                    crate::ui::APP_NAME,
                    egui::FontId::proportional(12.0),
                    theme::TEXT_SECONDARY,
                );
                if editing {
                    if let Some(name) = &filename {
                        // The file name is the main window title: primary color and a bit
                        // larger than the brand on the left. The clip prevents long names from
                        // overlapping the app icon and the window buttons.
                        let clip = egui::Rect::from_min_max(
                            egui::pos2(full.left() + 130.0, full.top()),
                            egui::pos2(min_rect.left() - 12.0, full.bottom()),
                        );
                        p.with_clip_rect(clip).text(
                            full.center(),
                            egui::Align2::CENTER_CENTER,
                            name,
                            egui::FontId::proportional(12.0),
                            theme::TEXT,
                        );
                    }
                }
            });
    }

    /// Toggle window maximize (button or double-click on the title bar).
    fn toggle_maximize(&self, ctx: &egui::Context) {
        let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        ctx.request_repaint();
    }

    /// Resize handles for the borderless window: at the edges and corners. Works only in
    /// windowed mode (in hero the window is a fixed size, in maximized it is not needed).
    pub(crate) fn handle_window_resize(&self, ctx: &egui::Context) {
        const B: f32 = 5.0;
        let vp = ctx.input(|i| i.viewport().clone());
        if vp.maximized == Some(true) || vp.fullscreen == Some(true) {
            return;
        }
        let rect = ctx.screen_rect();
        let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let left = (p.x - rect.left()).abs() <= B;
        let right = (p.x - rect.right()).abs() <= B;
        let top = (p.y - rect.top()).abs() <= B;
        let bottom = (p.y - rect.bottom()).abs() <= B;
        let dir = match (left, right, top, bottom) {
            (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
            (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
            (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
            (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
            (true, _, _, _) => Some(egui::ResizeDirection::West),
            (_, true, _, _) => Some(egui::ResizeDirection::East),
            (_, _, true, _) => Some(egui::ResizeDirection::North),
            (_, _, _, true) => Some(egui::ResizeDirection::South),
            _ => None,
        };
        let Some(dir) = dir else { return };
        ctx.set_cursor_icon(match dir {
            egui::ResizeDirection::North | egui::ResizeDirection::South => {
                egui::CursorIcon::ResizeVertical
            }
            egui::ResizeDirection::East | egui::ResizeDirection::West => {
                egui::CursorIcon::ResizeHorizontal
            }
            egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => {
                egui::CursorIcon::ResizeNeSw
            }
            egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
                egui::CursorIcon::ResizeNwSe
            }
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }

    /// Bottom action panel: Preset to All → Save All, then a muted
    /// Close Session (danger on hover). The primary action (Save) is highlighted
    /// with accent; without edits it is disabled. On a narrow center the row adapts
    /// (hides the link and switches to compact labels).
    fn show_action_buttons(&mut self, ui: &mut egui::Ui, has_multiple: bool) {
        const BTN_H: f32 = 26.0;
        let gap = 10.0f32;

        let modified = self.session.modified_count();
        let busy = self.export_rx.is_some();

        // Adapt to a narrow center: first hide the GitHub link, then
        // switch to compact labels and widths — this way the buttons do not overlap.
        let avail = ui.available_width();
        // The threshold is chosen so the link does not overlap the centered row
        // of buttons: at a 700px center there is spare room between them.
        let show_github = avail >= 700.0;
        let compact = avail < 560.0;
        let (preset_w, save_w, close_w) = if compact {
            (120.0, 115.0, 100.0)
        } else {
            (150.0, 170.0, 130.0)
        };

        // All buttons in the row are separated by the same gap — the row is even and
        // symmetrical; Close stands out only by the ghost style with a stroke.
        let content_w = if has_multiple {
            preset_w + gap + save_w + gap + close_w
        } else {
            save_w + gap + close_w
        };

        // Far-left feedback link, mirroring the author link on the right. A plain
        // hyperlink (same blue as the GitHub link), so it reads as a link, not a
        // button. It only appears when the centered button row leaves enough room
        // on the left, and the leading space is measured from the actual cursor,
        // so the row stays exactly centered regardless of the link width.
        let leading = ((avail - content_w) / 2.0).max(0.0);
        let show_feedback = leading >= 120.0;

        ui.add_space(2.0);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let row_x = ui.cursor().min.x;

            // Left corner: the feedback link.
            if show_feedback {
                ui.hyperlink_to(
                    egui::RichText::new(format!("{}  Send feedback", ph::CHAT_CIRCLE)).size(11.0),
                    crate::ui::FEEDBACK_URL,
                )
                .on_hover_text("Opens a short Google Form — no account needed");
            }

            // Advance to where the centered row must start, compensating for the
            // space the link (and its item spacing) already consumed.
            ui.add_space((row_x + leading - ui.cursor().min.x).max(0.0));

            // --- Preset to All (only with multiple images) ---
            if has_multiple {
                let resp = ui.add_enabled(
                    !busy,
                    egui::Button::new(format!("{} Preset to All", ph::STACK))
                        .min_size(egui::vec2(preset_w, BTN_H)),
                );
                if resp
                    .on_hover_text(
                        "Apply the current preset (plus any slider tweaks) to ALL images, \
                         then fine-tune each frame individually",
                    )
                    .clicked()
                {
                    self.batch_preset_selected_index = self.preset_manager.selected_index;
                    self.batch_use_current_settings = true;
                    self.show_apply_all_dialog = true;
                }
                ui.add_space(gap);
            }

            // --- Save / Save All — the primary (accent) action ---
            let save_label = if has_multiple {
                if compact {
                    format!("{} Save All", ph::FLOPPY_DISK)
                } else {
                    format!("{} Save All ({} modified)", ph::FLOPPY_DISK, modified)
                }
            } else {
                format!("{} Save", ph::FLOPPY_DISK)
            };
            // No edits — nothing to save; during export the actions are blocked.
            let can_save = !busy && (!has_multiple || modified > 0);
            let save_resp = ui
                .add_enabled_ui(can_save, |ui| {
                    super::widgets::primary_button(ui, save_label, egui::vec2(save_w, BTN_H))
                })
                .inner;
            let save_tooltip = if has_multiple {
                if compact {
                    format!("Save all processed images ({} modified)", modified)
                } else {
                    "Save all processed images".to_string()
                }
            } else {
                "Save processed image".to_string()
            };
            if save_resp.on_hover_text(save_tooltip).clicked() {
                self.show_save_dialog = true;
            }

            // --- Close — muted, dangerous on hover ---
            ui.add_space(gap);
            let close_label = if has_multiple && !compact {
                format!("{} Close Session", ph::X)
            } else {
                format!("{} Close", ph::X)
            };
            let close_hint = if modified > 0 {
                "Close all images — unsaved edits will be lost"
            } else {
                "Close all images and clear the screen"
            };
            if super::widgets::ghost_button(
                ui,
                close_label,
                egui::vec2(close_w, BTN_H),
                theme::DANGER,
            )
            .on_hover_text(close_hint)
            .clicked()
            {
                if modified > 0 {
                    self.show_close_confirm = true;
                } else {
                    self.close_session();
                }
            }

            // The author link — on the right, but only if there is room and it
            // does not overlap the buttons.
            if show_github {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    ui.hyperlink_to(
                        egui::RichText::new(format!(
                            "{}  by {}",
                            ph::GITHUB_LOGO,
                            crate::ui::AUTHOR
                        ))
                        .size(11.0),
                        crate::ui::GITHUB_URL,
                    );
                });
            }
        });
        ui.add_space(2.0);
    }

    /// Image viewport: zoom under the cursor, panning with space, splitter.
    fn draw_image_viewport(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tex_id: egui::TextureId,
        tex_size: egui::Vec2,
        orig_tex_id: Option<egui::TextureId>,
    ) {
        let viewport_rect = ui.available_rect_before_wrap();
        let response = ui.interact(
            viewport_rect,
            ui.id().with("image_viewport"),
            egui::Sense::click_and_drag(),
        );
        // painter_at clips drawing to the viewport bounds
        let painter = ui.painter_at(viewport_rect);

        // Floating retouch options bar (sets `retouch_options_rect` for this frame).
        self.show_retouch_options(ctx, viewport_rect);

        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let pointer_over = hover_pos.map_or(false, |p| viewport_rect.contains(p));

        // --- ZOOM UNDER THE CURSOR (wheel / pinch) ---
        if pointer_over {
            let mut zoom_factor = ui.input(|i| i.zoom_delta());
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_y != 0.0 {
                zoom_factor *= (scroll_y * 0.0025).exp();
            }
            if (zoom_factor - 1.0).abs() > f32::EPSILON {
                let old_zoom = self.zoom_scale;
                let new_zoom = (old_zoom * zoom_factor).clamp(0.1, 5.0);
                if (new_zoom - old_zoom).abs() > f32::EPSILON {
                    let center = viewport_rect.center();
                    let anchor = hover_pos.unwrap_or(center);
                    let k = new_zoom / old_zoom;
                    // Keep the point under the cursor in place.
                    self.pan_offset = (anchor - center) - ((anchor - center) - self.pan_offset) * k;
                    self.set_zoom(new_zoom);
                }
            }
        }

        // --- HIT GEOMETRY (before input: classify drags/clicks) ---
        let avail_size = viewport_rect.size();
        let base_scale = (avail_size.x / tex_size.x).min(avail_size.y / tex_size.y);
        let hit_rect = {
            let display_size = tex_size * base_scale * self.zoom_scale;
            let max_pan = max_pan_offset(display_size, avail_size);
            let pan = egui::vec2(
                self.pan_offset.x.clamp(-max_pan.x, max_pan.x),
                self.pan_offset.y.clamp(-max_pan.y, max_pan.y),
            );
            egui::Rect::from_center_size(viewport_rect.center() + pan, display_size)
        };
        // Is the cursor over the image itself (inside the viewport, not the
        // letterbox)? The rendered image can be zoomed/panned beyond the canvas,
        // so `hit_rect` alone is not enough: pixels outside the viewport (over the
        // side panels, the bottom buttons or a dialog) must not count as "over the
        // image", otherwise the OS cursor gets hidden and the custom glyph is
        // clipped away.
        let over_image = pointer_over && hover_pos.map_or(false, |p| hit_rect.contains(p));
        // Does the pointer sit on the before/after splitter handle?
        let over_splitter = orig_tex_id.is_some()
            && !self.show_save_dialog
            && hover_pos.map_or(false, |p| {
                viewport_rect.contains(p) && splitter_grab_zone(hit_rect, self.split_position, p)
            });

        // --- RETOUCH: takes over the left button while the tool is active ---
        let retouch_consumed = self.retouch_pointer(ui, hit_rect, hover_pos);

        // --- PANNING: left-drag anywhere (the splitter keeps priority) ---
        let space_held = ui.input(|i| i.key_down(egui::Key::Space));
        // Classify the gesture once, at the start of the drag.
        if response.drag_started() {
            let drag_origin = response.interact_pointer_pos().or(hover_pos);
            let on_splitter = orig_tex_id.is_some()
                && !self.show_save_dialog
                && !space_held
                && drag_origin.map_or(false, |p| {
                    viewport_rect.contains(p)
                        && splitter_grab_zone(hit_rect, self.split_position, p)
                });
            if on_splitter {
                self.is_dragging_split = true;
                self.is_panning = false;
            } else if retouch_consumed {
                // The retouch tool owns the left drag.
                self.is_dragging_split = false;
                self.is_panning = false;
            } else {
                self.is_panning = true;
            }
        }
        if response.drag_stopped() {
            self.is_panning = false;
        }
        if response.dragged() && self.is_panning {
            // Pan both axes; each is clamped to keep >= 30% of the image visible
            // (see `max_pan_offset`). The cursor is drawn later (custom hand).
            self.pan_offset += response.drag_delta();
        }

        // --- LEFT-CLICK: toggle "fit" <-> 1.5x zoom anchored at the cursor ---
        // First click zooms 50% into the clicked area, the next one (while zoomed)
        // returns the image to fit. The "fit" state is read from the transform, so
        // manual wheel/keyboard zoom is reset the same way.
        if over_image
            && !space_held
            && !self.show_save_dialog
            && !over_splitter
            && !retouch_consumed
            && response.clicked()
        {
            let at_fit = (self.zoom_scale - 1.0).abs() < 0.001
                && self.pan_offset.length_sq() < 0.25;
            if at_fit {
                const CLICK_ZOOM: f32 = 1.5;
                let old_zoom = self.zoom_scale;
                let new_zoom = CLICK_ZOOM.clamp(0.1, 5.0);
                let center = viewport_rect.center();
                let anchor = hover_pos.unwrap_or(center);
                let k = new_zoom / old_zoom;
                // Keep the point under the cursor in place.
                self.pan_offset = (anchor - center) - ((anchor - center) - self.pan_offset) * k;
                self.apply_zoom(new_zoom);
            } else {
                self.apply_zoom(1.0);
            }
        }

        // --- GEOMETRY (after input: re-clamp the pan and build the draw rect) ---
        let display_size = tex_size * base_scale * self.zoom_scale;
        // Both axes may be dragged until at least 30% of the image is still visible.
        let max_pan = max_pan_offset(display_size, avail_size);
        self.pan_offset.x = self.pan_offset.x.clamp(-max_pan.x, max_pan.x);
        self.pan_offset.y = self.pan_offset.y.clamp(-max_pan.y, max_pan.y);
        let rect =
            egui::Rect::from_center_size(viewport_rect.center() + self.pan_offset, display_size);

        // 0. Canvas dot grid — only in the letterbox (no dots are drawn under the
        // frame). The grid node is the frame center, and the step scales with
        // zoom: the background "zooms in/out" in sync with the image and moves
        // with the pan. When zoomed far out the dots get denser — we fade them out smoothly
        // so there is no noise/moire; below MIN_STEP the grid is no longer drawn.
        {
            const BASE_STEP: f32 = 22.0;
            const MIN_STEP: f32 = 10.0;
            const FADE_STEP: f32 = 16.0;
            let dot_step = BASE_STEP * self.zoom_scale;
            let fade = ((dot_step - MIN_STEP) / (FADE_STEP - MIN_STEP)).clamp(0.0, 1.0);
            if fade > 0.0 {
                super::widgets::dot_grid(
                    &painter,
                    viewport_rect,
                    rect,
                    viewport_rect.center() + self.pan_offset,
                    dot_step,
                    1.0,
                    theme::CANVAS_DOT.gamma_multiply(fade),
                );
            }
        }

        // 1. Processed image (AFTER)
        let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(tex_id, rect, full_uv, egui::Color32::WHITE);

        // 2. BEFORE/AFTER split: the original is drawn to the left of the divider.
        //    `\` (or the toolbar button) toggles the split on and off.
        if let Some(orig_id) = orig_tex_id {
            const EDGE_PX: f32 = 6.0; // how close to the edge counts as "handle at the edge"
            const EDGE_GRAB: f32 = 26.0; // grab zone at the edge (wider — easy to catch)
            const HANDLE_GRAB: f32 = 14.0; // grab zone of the handle in the middle
            const FADE: f32 = 0.15; // animation time for appearing/hiding

            let split_x = rect.left() + rect.width() * self.split_position;

            // 3. Original (BEFORE) to the left of the divider
            if self.split_position > 0.0 {
                let left_rect =
                    egui::Rect::from_min_max(rect.min, egui::pos2(split_x, rect.bottom()));
                let uv_rect = egui::Rect::from_min_max(
                    egui::pos2(0.0, 0.0),
                    egui::pos2(self.split_position, 1.0),
                );
                painter.image(orig_id, left_rect, uv_rect, egui::Color32::WHITE);

                // BEFORE badge on the before side — kept for clarity so it is
                // obvious which side is which. Drawn only when it fits without
                // spilling over the divider.
                let galley = painter.layout_no_wrap(
                    "BEFORE".to_owned(),
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
                let pad = egui::vec2(8.0, 4.0);
                let badge_size = galley.size() + pad * 2.0;
                // Top-left of the before region.
                let badge_min = rect.left_top() + egui::vec2(12.0, 12.0);
                if badge_min.x + badge_size.x <= split_x - 8.0 {
                    let badge_rect = egui::Rect::from_min_size(badge_min, badge_size);
                    painter.rect_filled(badge_rect, 4.0, egui::Color32::from_black_alpha(220));
                    painter.galley(badge_min + pad, galley, egui::Color32::WHITE);
                }
            }

            // 4. At the edge the handle hides until the cursor is near (option A).
            let at_left = split_x - rect.left() <= EDGE_PX;
            let at_right = rect.right() - split_x <= EDGE_PX;
            let at_edge = at_left || at_right;

            let pointer_down = ui.input(|i| i.pointer.primary_down());
            // The grab zone is clipped to the visible viewport: when the image is
            // zoomed or panned, `rect` can extend past the canvas under the filmstrip
            // or the bottom buttons, and without this check the splitter could be
            // grabbed there.
            let near_handle = hover_pos.map_or(false, |h| {
                viewport_rect.contains(h)
                    && (h.x - split_x).abs() < HANDLE_GRAB
                    && rect.contains(h)
            });
            // At the edge we catch wider and allow hovering slightly outside the image
            // (in the letterbox), but never outside the viewport.
            let near_edge = at_edge
                && hover_pos.map_or(false, |h| {
                    viewport_rect.contains(h)
                        && ((at_left && (h.x - rect.left()).abs() < EDGE_GRAB)
                            || (at_right && (h.x - rect.right()).abs() < EDGE_GRAB))
                });
            let hover_near = near_handle || near_edge;

            // Show it if it is not at the edge, or a drag is in progress, or the cursor is near.
            let want_visible = !at_edge || self.is_dragging_split || hover_near;
            let alpha = ctx.animate_bool_with_time(
                ui.id().with("split_handle_visible"),
                want_visible,
                FADE,
            );

            // 5. The line and handle are drawn taking opacity into account
            if alpha > 0.01 {
                let a = (alpha * 255.0) as u8;
                let white = theme::with_alpha(theme::TEXT, a);
                let dark = theme::with_alpha(theme::BG_DEEP, a);

                painter.line_segment(
                    [
                        egui::pos2(split_x, rect.top()),
                        egui::pos2(split_x, rect.bottom()),
                    ],
                    (2.5, white),
                );

                let handle_y = rect.center().y;
                painter.circle_filled(egui::pos2(split_x, handle_y), 9.0, white);
                painter.circle_filled(egui::pos2(split_x, handle_y), 5.0, dark);
            }

            // 6. Splitter dragging (except saving and panning)
            if !self.show_save_dialog && !space_held && !self.is_panning && !self.retouch.active {
                if hover_near && !pointer_down {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if pointer_down && (hover_near || self.is_dragging_split) {
                    // Track the pointer only while it stays inside the image viewport:
                    // dragging on past the filmstrip or the bottom buttons must not
                    // keep moving the divider.
                    if let Some(hover) = hover_pos {
                        if viewport_rect.contains(hover) {
                            self.split_position =
                                ((hover.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                        }
                    }
                    self.is_dragging_split = true;
                    ctx.request_repaint();
                }
                if !pointer_down {
                    if self.is_dragging_split {
                        self.save_save_settings();
                    }
                    self.is_dragging_split = false;
                }
            } else {
                self.is_dragging_split = false;
            }
        }

        // Retouch painted-region indication (translucent accent blue). The heal
        // itself only runs when the pointer is released.
        self.draw_retouch_overlay(ctx, &painter, rect);

        // Custom cursor over the photo. Windows has no OS cursors for zoom, and
        // `Grab`/`Grabbing` map to the four-arrow SIZEALL (not a hand), so we hide the
        // system cursor and draw a phosphor glyph under the pointer:
        //   • pan drag   → closed hand;
        //   • Space ready → open hand;
        //   • otherwise   → magnifier "+" at fit / "−" when zoomed in.
        //
        // The glyph is painted on its own top-most layer, clipped to the canvas.
        // The floating tools panel / retouch bar and the modal dialogs live on
        // higher egui layers than the canvas painter; a glyph painted on the
        // canvas would end up *under* them while the OS cursor is hidden, leaving
        // the pointer invisible.
        let cursor_painter = ctx
            .layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("tiny_luma_custom_cursor"),
            ))
            .with_clip_rect(viewport_rect);

        // Floating overlays drawn on top of the canvas: over them we must not
        // replace the OS cursor (the widgets handle their own) and must not heal
        // through them.
        let over_toolbar = hover_pos.map_or(false, |p| {
            self.toolbar_rect.map_or(false, |r| r.expand(2.0).contains(p))
        });
        let over_options = hover_pos.map_or(false, |p| {
            self.retouch_options_rect.map_or(false, |r| r.contains(p))
        });
        let over_tools_panel = hover_pos.map_or(false, |p| {
            self.tools_rect.map_or(false, |r| r.contains(p))
        });
        let over_overlay = over_toolbar || over_options || over_tools_panel;

        // A modal dialog owns the pointer: keep the normal cursor visible so the
        // user can interact with it instead of hiding it under the custom glyph.
        let modal_open = self.show_save_dialog
            || self.show_close_confirm
            || self.show_batch_progress
            || self.show_apply_all_dialog;

        let cursor_glyph = if modal_open || over_overlay {
            None
        } else if self.is_panning {
            // Grabbing takes priority: the drag may leave the image/viewport.
            Some(ph::HAND_GRABBING)
        } else if space_held && pointer_over {
            Some(ph::HAND)
        } else if over_image && !over_splitter {
            Some(if self.zoom_scale <= 1.0 {
                ph::MAGNIFYING_GLASS_PLUS
            } else {
                ph::MAGNIFYING_GLASS_MINUS
            })
        } else {
            None
        };

        if self.retouch.active
            && over_image
            && !over_overlay
            && !space_held
            && !self.is_panning
            && !modal_open
        {
            if let Some(p) = hover_pos {
                let (ww, wh) = self.retouch_work_dims();
                let norm_r = self.retouch.brush.radius_fraction(ww, wh);
                let screen_r = norm_r * hit_rect.width().max(hit_rect.height());
                self.draw_brush_cursor(ctx, &cursor_painter, p, screen_r);
            }
        } else if let (Some(glyph), Some(p)) = (cursor_glyph, hover_pos) {
            ctx.set_cursor_icon(egui::CursorIcon::None);
            draw_cursor_glyph(&cursor_painter, p, glyph);
        }

        // A single toolbar bar at the bottom of the photo: history and before/after on the left,
        // the frame index centered, zoom on the right. It takes no space in the layout.
        self.show_toolbar_overlay(ctx, viewport_rect);
    }
}

/// Window button (minimize/maximize/close). Drawn manually, because the title bar is
/// borderless. "Close" has a red background on hover and a rounded top corner.
fn window_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: &str,
    key: &str,
    is_close: bool,
) -> bool {
    let r = ui.interact(rect, ui.id().with(("win_btn", key)), egui::Sense::click());
    if r.hovered() {
        let rounding = if is_close {
            egui::Rounding {
                ne: theme::RADIUS_LG,
                ..Default::default()
            }
        } else {
            egui::Rounding::ZERO
        };
        ui.painter().rect_filled(
            rect,
            rounding,
            if is_close {
                theme::DANGER
            } else {
                theme::BG_ELEVATED
            },
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(12.0),
        if r.hovered() && is_close {
            egui::Color32::WHITE
        } else {
            theme::TEXT
        },
    );
    r.clicked()
}

/// Maximum pan offset (per axis) for a given rendered image size.
///
/// Both axes use the same rule: the image may be dragged until at least 30% of
/// its size on that axis is still inside the canvas. If the image is zoomed so far
/// that 30% of it no longer fits into the viewport, we fall back to the hard clamp
/// (the image edge cannot leave the viewport).
fn max_pan_offset(display_size: egui::Vec2, avail_size: egui::Vec2) -> egui::Vec2 {
    const MIN_VISIBLE_FRAC: f32 = 0.30;

    let axis = |d: f32, a: f32| {
        let hard = ((d - a) / 2.0).max(0.0);
        // Pan at which exactly `MIN_VISIBLE_FRAC` of the image is still visible.
        let visible = (a + d) / 2.0 - MIN_VISIBLE_FRAC * d;
        hard.max(visible).max(0.0)
    };

    egui::vec2(
        axis(display_size.x, avail_size.x),
        axis(display_size.y, avail_size.y),
    )
}

/// Draws a custom cursor glyph at `p` (screen space) with a thin dark halo in the
/// app's palette, so it stays readable on both light and dark photos.
fn draw_cursor_glyph(painter: &egui::Painter, p: egui::Pos2, glyph: &str) {
    let font = egui::FontId::proportional(22.0);
    const HALO: [(f32, f32); 8] = [
        (-1.0, 0.0),
        (1.0, 0.0),
        (0.0, -1.0),
        (0.0, 1.0),
        (-0.7, -0.7),
        (0.7, -0.7),
        (-0.7, 0.7),
        (0.7, 0.7),
    ];
    let halo = theme::with_alpha(theme::BG_DEEP, 165);
    for (dx, dy) in HALO {
        painter.text(
            p + egui::vec2(dx, dy),
            egui::Align2::CENTER_CENTER,
            glyph,
            font.clone(),
            halo,
        );
    }
    painter.text(
        p + egui::vec2(1.4, 1.4),
        egui::Align2::CENTER_CENTER,
        glyph,
        font.clone(),
        theme::with_alpha(theme::BG_DEEP, 80),
    );
    painter.text(p, egui::Align2::CENTER_CENTER, glyph, font, theme::TEXT);
}

/// Whether `p` (screen space) is close enough to the before/after divider to grab
/// it. Mirrors the grab zones that are used when drawing the splitter handle.
fn splitter_grab_zone(rect: egui::Rect, split_position: f32, p: egui::Pos2) -> bool {
    const EDGE_PX: f32 = 6.0; // how close to the edge counts as "handle at the edge"
    const EDGE_GRAB: f32 = 26.0; // grab zone at the edge (wider — easy to catch)
    const HANDLE_GRAB: f32 = 14.0; // grab zone of the handle in the middle

    let split_x = rect.left() + rect.width() * split_position;
    let at_left = split_x - rect.left() <= EDGE_PX;
    let at_right = rect.right() - split_x <= EDGE_PX;
    let at_edge = at_left || at_right;

    let near_handle = (p.x - split_x).abs() < HANDLE_GRAB && rect.contains(p);
    let near_edge = at_edge
        && ((at_left && (p.x - rect.left()).abs() < EDGE_GRAB)
            || (at_right && (p.x - rect.right()).abs() < EDGE_GRAB));

    near_handle || near_edge
}
