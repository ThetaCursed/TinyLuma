// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use crate::app::export::{run_export, ExportJob};
use crate::app::TinyLumaApp;
use crate::settings::SaveFormat;
use crate::theme;
use crate::ui::notification::ToastKind;
use eframe::egui;
use egui_phosphor::regular as ph;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

impl TinyLumaApp {
    /// Modal windows: save, batch preset, batch progress.
    pub(crate) fn show_dialogs(&mut self, ctx: &egui::Context) {
        // The window title draws its separator line with the
        // `widgets.noninteractive.bg_stroke` color ([`theme::SEPARATOR`]), which on
        // the window background is almost invisible. While the dialogs are shown we swap it
        // for a lighter [`theme::DIVIDER`] and restore it at the end.
        let prev_window_sep = ctx.style().visuals.widgets.noninteractive.bg_stroke;
        ctx.style_mut(|s| {
            s.visuals.widgets.noninteractive.bg_stroke =
                egui::Stroke::new(1.0, theme::DIVIDER);
        });
        // Save dialog with format and quality selection
        if self.show_save_dialog {
            let mut save_clicked = false;
            let has_multiple = self.session.total() > 1;
            let modified = self.session.modified_count();

            let title = if has_multiple {
                format!("{} Batch Export ({} files)", ph::FLOPPY_DISK, modified)
            } else {
                format!("{} Save As", ph::FLOPPY_DISK)
            };
            let window_height = if has_multiple { 445.0 } else { 295.0 };

            egui::Window::new(&title)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size([320.0, window_height])
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add_space(10.0);

                    // Format selection
                    ui.label(egui::RichText::new("File format:").strong());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        // When the format changes we NO LONGER touch quality: the lossy
                        // formats (JPEG and WebP) share it (90 by default) and it is kept
                        // until the user moves the slider themselves. PNG ignores quality.
                        let mut format_changed = false;
                        format_changed |= ui
                            .radio_value(&mut self.save_format, SaveFormat::Png, "PNG")
                            .clicked();
                        format_changed |= ui
                            .radio_value(&mut self.save_format, SaveFormat::Jpg, "JPEG")
                            .clicked();
                        format_changed |= ui
                            .radio_value(&mut self.save_format, SaveFormat::WebP, "WebP")
                            .clicked();
                        if format_changed {
                            self.save_save_settings();
                        }
                    });

                    ui.add_space(16.0);

                    // Quality — for the lossy formats (JPEG and WebP). PNG is lossless,
                    // so no slider is shown for it; the value is shared between JPEG and
                    // WebP and defaults to 90.
                    match self.save_format {
                        SaveFormat::Jpg | SaveFormat::WebP => {
                            ui.label(egui::RichText::new("Quality:").strong());
                            ui.add_space(4.0);
                            let mut quality_f32 = self.save_quality as f32;
                            if ui
                                .add(
                                    egui::Slider::new(&mut quality_f32, 1.0..=100.0)
                                        .show_value(true)
                                        .trailing_fill(true),
                                )
                                .changed()
                            {
                                self.save_quality = quality_f32.round() as u8;
                                self.save_save_settings();
                            }
                        }
                        SaveFormat::Png => {
                            ui.add_space(4.0);
                            ui.weak("PNG — lossless compression (quality 100%)");
                        }
                    }

                    // Metadata carry-over (PNG only): text/EXIF/ICC/private chunks.
                    if matches!(self.save_format, SaveFormat::Png) {
                        ui.add_space(10.0);
                        if ui
                            .checkbox(
                                &mut self.embed_png_metadata,
                                "Preserve PNG metadata (workflow, etc.)",
                            )
                            .on_hover_text(
                                "Copy all metadata chunks from the original PNG \
                                 (ComfyUI workflow, EXIF, ICC, …)",
                            )
                            .changed()
                        {
                            self.save_save_settings();
                        }
                    }

                    // For batch we show extra information
                    if has_multiple {
                        ui.add_space(12.0);

                        let ext = match self.save_format {
                            SaveFormat::Png => "png",
                            SaveFormat::Jpg => "jpg",
                            SaveFormat::WebP => "webp",
                        };
                        ui.horizontal(|ui| {
                            ui.label("Postfix:");
                            ui.add_space(4.0);
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.batch_postfix)
                                    .hint_text("_edited")
                                    .desired_width(110.0),
                            );
                            if resp.changed() {
                                self.save_save_settings();
                            }
                            ui.label(
                                egui::RichText::new(format!(
                                    "image{}.{}",
                                    &self.batch_postfix, ext
                                ))
                                .size(11.0)
                                .color(theme::TEXT_SECONDARY),
                            );
                        });

                        // The divider separates the export settings (format,
                        // quality, postfix) from the final summary.
                        ui.add_space(12.0);
                        crate::ui::widgets::divider(ui);
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} Will save: {} / {} images",
                                ph::IMAGES,
                                modified,
                                self.session.total()
                            ))
                            .size(12.0),
                        );

                        if modified == 0 {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} No processed images!\nAdjust sliders or apply a preset.",
                                    ph::WARNING
                                ))
                                .size(11.0)
                                .color(theme::WARNING),
                            );
                        }

                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("You will be asked to choose an output folder.")
                                .size(11.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                    }

                    ui.add_space(20.0);

                    // Save / Cancel buttons
                    ui.horizontal(|ui| {
                        let spacing = (ui.available_width() - 190.0) / 2.0;
                        ui.add_space(spacing.max(0.0));

                        let btn_label = if has_multiple {
                            format!("{} Export {} files", ph::FLOPPY_DISK, modified)
                        } else {
                            format!("{} Save", ph::FLOPPY_DISK)
                        };
                        let save_enabled = !has_multiple || modified > 0;

                        if ui
                            .add_enabled(
                                save_enabled,
                                egui::Button::new(&btn_label)
                                    .min_size(egui::vec2(130.0, 28.0))
                                    .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            save_clicked = true;
                        }

                        ui.add_space(10.0);

                        if ui
                            .add(egui::Button::new("Cancel").min_size(egui::vec2(80.0, 28.0)))
                            .clicked()
                        {
                            self.show_save_dialog = false;
                        }
                    });
                });

            if save_clicked {
                if has_multiple {
                    // ===== BATCH EXPORT (background thread) =====
                    if self.export_rx.is_some() {
                        // An export is already running — ignore the repeated start.
                        self.notify(ToastKind::Warning, "Export already running");
                    } else {
                        let mut dialog =
                            rfd::FileDialog::new().set_title("Choose an output folder...");
                        // Start in the remembered folder only if it still exists.
                        if self.last_output_dir.is_dir() {
                            dialog = dialog.set_directory(&self.last_output_dir);
                        }
                        if let Some(output_dir) = dialog.pick_folder() {
                            // Save the current settings before the batch.
                            if let Some(path) = &self.image_path {
                                let preset_name = self.current_preset_name();
                                self.session.save_current_settings(
                                    path,
                                    self.settings,
                                    self.lut_path.clone(),
                                    preset_name,
                                );
                            }

                            let items = self.collect_modified_items();
                            if items.is_empty() {
                                self.notify(ToastKind::Warning, "Nothing to export");
                            } else {
                                let total = items.len();
                                let job = ExportJob {
                                    items,
                                    output_dir: output_dir.clone(),
                                    format: self.save_format,
                                    quality: self.save_quality,
                                    postfix: self.batch_postfix.clone(),
                                    embed_png_metadata: self.embed_png_metadata,
                                };
                                let (tx, rx) = std::sync::mpsc::channel();
                                self.export_rx = Some(rx);
                                self.export_cancel = Arc::new(AtomicBool::new(false));
                                let cancel = self.export_cancel.clone();
                                std::thread::spawn(move || run_export(job, tx, cancel));

                                self.last_output_dir = output_dir;
                                self.save_save_settings();

                                self.show_batch_progress = true;
                                self.batch_total = total;
                                self.batch_done = 0;
                                self.batch_progress_message =
                                    format!("{} Exporting 0/{}...", ph::HOURGLASS, total);
                                ctx.request_repaint();
                            }
                        }
                    }
                } else {
                    // ===== SINGLE SAVE =====
                    let original_path = self.image_path.as_ref();
                    let stem = original_path
                        .and_then(|p| p.file_stem())
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "image".to_string());

                    let ext = match self.save_format {
                        SaveFormat::Png => "png",
                        SaveFormat::Jpg => "jpg",
                        SaveFormat::WebP => "webp",
                    };
                    let default_name = format!("{}_edited.{}", stem, ext);

                    let filter_name = match self.save_format {
                        SaveFormat::Png => "PNG",
                        SaveFormat::Jpg => "JPEG",
                        SaveFormat::WebP => "WebP",
                    };

                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Save as...")
                        .add_filter(filter_name, &[ext]);

                    // Set the directory and the name SEPARATELY: on Windows `set_file_name` with a full
                    // path does not switch the folder, it puts the whole path
                    // into the name field. Take the folder only if it still exists — it could have
                    // been deleted/renamed between runs.
                    let save_dir = if self.last_output_dir.is_dir() {
                        self.last_output_dir.clone()
                    } else if let Some(path) = &self.image_path {
                        path.parent().map(|p| p.to_path_buf()).unwrap_or_default()
                    } else {
                        PathBuf::new()
                    };
                    if save_dir.is_dir() {
                        dialog = dialog.set_directory(&save_dir);
                    }
                    dialog = dialog.set_file_name(&default_name);

                    if matches!(self.save_format, SaveFormat::Jpg | SaveFormat::WebP) {
                        dialog =
                            dialog.add_filter("All images", crate::ui::SUPPORTED_IMAGE_EXTENSIONS);
                    }

                    if let Some(save_path) = dialog.save_file() {
                        let success = self.save_current_preview(
                            &save_path,
                            self.save_format,
                            self.save_quality,
                        );
                        if success {
                            // Remember the export folder
                            if let Some(parent) = save_path.parent() {
                                self.last_output_dir = parent.to_path_buf();
                                self.save_save_settings();
                            }
                            self.notify(ToastKind::Success, "Image saved");
                            println!("Image saved: {:?}", save_path);
                        } else {
                            self.notify(ToastKind::Error, "Failed to save");
                        }
                    }
                }
                self.show_save_dialog = false;
            }
        }

        // ===== APPLY TO ALL DIALOG =====
        if self.show_apply_all_dialog && self.session.total() > 1 {
            let mut apply_clicked = false;
            let total = self.session.total();

            // The window and the full-width rows share the same width —
            // so no empty field is left at the right edge.
            const DIALOG_W: f32 = 380.0;

            egui::Window::new(format!("{} Apply Preset to All Images", ph::STACK))
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .min_width(DIALOG_W)
                .max_width(DIALOG_W)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add_space(4.0);

                    // What exactly will go to all frames — shown right in the label.
                    let current_preset = self.current_preset_name();
                    let current_dirty = self.selected_preset_dirty();
                    let current_label = match &current_preset {
                        Some(name) if current_dirty => {
                            format!("Current: \"{}\" + adjusted sliders", name)
                        }
                        Some(name) => format!("Current preset: \"{}\"", name),
                        None => "Current settings (sliders)".to_string(),
                    };

                    // --- 1. SETTINGS SOURCE ---
                    ui.label(
                        egui::RichText::new(format!("{} Settings to apply:", ph::TARGET))
                            .strong()
                            .size(14.0),
                    );
                    ui.add_space(4.0);

                    if crate::ui::widgets::option_row(
                        ui,
                        self.batch_use_current_settings,
                        current_label,
                        "Preset selected on the left panel, plus any unsaved slider changes",
                    )
                    .clicked()
                    {
                        self.batch_use_current_settings = true;
                        self.batch_preset_selected_index = None;
                    }

                    if crate::ui::widgets::option_row(
                        ui,
                        !self.batch_use_current_settings,
                        "Choose a preset from the list",
                        "Choose a previously saved preset",
                    )
                    .clicked()
                    {
                        self.batch_use_current_settings = false;
                    }

                    if !self.batch_use_current_settings {
                        ui.add_space(4.0);
                        let preset_names: Vec<String> = self
                            .preset_manager
                            .presets
                            .iter()
                            .map(|p| p.name.clone())
                            .collect();

                        // The same selector as in the left panel: identical look and
                        // a list without an inner scrollbar.
                        let choice = crate::ui::widgets::preset_dropdown(
                            ui,
                            "batch_preset_selector",
                            &preset_names,
                            self.batch_preset_selected_index,
                            false,
                            false,
                        );
                        if let Some(i) = choice.index {
                            self.batch_preset_selected_index = Some(i);
                        }

                        if preset_names.is_empty() {
                            ui.add_space(4.0);
                            ui.weak("Save a preset first via the left panel");
                        }
                    }

                    // --- 2. APPLICATION MODE ---
                    // No divider needed: the two choice groups are peers,
                    // and the bold headings already separate them from each other.
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("{} Apply to:", ph::PUZZLE_PIECE))
                            .strong()
                            .size(14.0),
                    );
                    ui.add_space(4.0);

                    if crate::ui::widgets::option_row(
                        ui,
                        !self.batch_apply_only_untouched,
                        "All images",
                        "Overwrite every image, including already edited ones",
                    )
                    .clicked()
                    {
                        self.batch_apply_only_untouched = false;
                    }

                    if crate::ui::widgets::option_row(
                        ui,
                        self.batch_apply_only_untouched,
                        "Only untouched images",
                        "Skip images that already have unsaved edits",
                    )
                    .clicked()
                    {
                        self.batch_apply_only_untouched = true;
                    }

                    // --- 3. INFO ---
                    ui.add_space(6.0);
                    crate::ui::widgets::divider(ui);
                    ui.add_space(6.0);

                    let modified = self.session.modified_count();
                    let affected = if self.batch_apply_only_untouched {
                        total.saturating_sub(modified)
                    } else {
                        total
                    };

                    // Summary: what exactly will be applied (preset / sliders / LUT).
                    let applying = if self.batch_use_current_settings {
                        let base = match &current_preset {
                            Some(name) if current_dirty => {
                                format!("\"{}\" + slider tweaks", name)
                            }
                            Some(name) => format!("preset \"{}\"", name),
                            None => "current sliders".to_string(),
                        };
                        if self.lut_path.is_some() {
                            format!("{} + LUT", base)
                        } else {
                            base
                        }
                    } else if let Some(idx) = self.batch_preset_selected_index {
                        self.preset_manager
                            .presets
                            .get(idx)
                            .map(|p| {
                                if p.lut_path.is_some() {
                                    format!("preset \"{}\" + LUT", p.name)
                                } else {
                                    format!("preset \"{}\"", p.name)
                                }
                            })
                            .unwrap_or_else(|| "no preset selected".to_string())
                    } else {
                        "no preset selected".to_string()
                    };

                    ui.label(
                        egui::RichText::new(format!("{} Applying: {}", ph::PALETTE, applying))
                            .size(12.0)
                            .strong()
                            .color(theme::ACCENT),
                    );
                    ui.add_space(4.0);

                    ui.label(
                        egui::RichText::new(format!(
                            "{} Images: {} total, {} already edited",
                            ph::CHART_BAR,
                            total,
                            modified
                        ))
                        .size(12.0),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} Will apply to {} image(s)",
                            ph::ARROW_RIGHT,
                            affected
                        ))
                        .size(12.0)
                        .color(theme::ACCENT),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Then tweak individual frames and use Save All")
                            .size(11.0)
                            .color(theme::TEXT_SECONDARY),
                    );

                    // --- 4. BUTTONS ---
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        const APPLY_W: f32 = 170.0;
                        const CANCEL_W: f32 = 80.0;
                        const BTN_GAP: f32 = 10.0;
                        // Center by the actual width of the buttons, not "by eye",
                        // so the left and right margins are equal.
                        let spacing = ((ui.available_width() - (APPLY_W + CANCEL_W + BTN_GAP))
                            / 2.0)
                            .max(0.0);
                        ui.add_space(spacing);

                        let has_source = self.batch_use_current_settings
                            || self.batch_preset_selected_index.is_some();
                        let can_apply = has_source && affected > 0;

                        if ui
                            .add_enabled(
                                can_apply,
                                egui::Button::new(format!("{} Apply to All", ph::ROCKET))
                                    .min_size(egui::vec2(APPLY_W, 28.0))
                                    .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            apply_clicked = true;
                        }

                        ui.add_space(BTN_GAP);

                        if ui
                            .add(egui::Button::new("Cancel").min_size(egui::vec2(CANCEL_W, 28.0)))
                            .clicked()
                        {
                            self.show_apply_all_dialog = false;
                        }
                    });
                });

            if apply_clicked {
                // Determine the source: settings + LUT path + preset name.
                let (settings, lut_path, preset_name) = if self.batch_use_current_settings {
                    // Associate the preset name only if the current settings
                    // exactly match the selected preset.
                    let name = if !self.selected_preset_dirty() {
                        self.current_preset_name()
                    } else {
                        None
                    };
                    (self.settings, self.lut_path.clone(), name)
                } else if let Some(idx) = self.batch_preset_selected_index {
                    self.preset_manager
                        .presets
                        .get(idx)
                        .map(|p| (p.settings, p.lut_path.clone(), Some(p.name.clone())))
                        .unwrap_or((self.settings, self.lut_path.clone(), None))
                } else {
                    (self.settings, self.lut_path.clone(), None)
                };

                self.show_apply_all_dialog = false;

                let (applied, warning) = self.apply_to_all(
                    settings,
                    lut_path,
                    preset_name,
                    self.batch_apply_only_untouched,
                );

                match warning {
                    Some(w) => self.notify(ToastKind::Warning, w),
                    None => self.notify(
                        ToastKind::Success,
                        format!("Applied to {} image(s)", applied),
                    ),
                }
            }
        }

        // ===== BATCH PROGRESS WINDOW =====
        // Show only while the export is actually running; on completion we close
        // the window ourselves — the toast reports the result, no extra OK button needed.
        if self.show_batch_progress {
            if self.export_rx.is_some() {
                egui::Window::new(format!("{} Batch Export", ph::HOURGLASS))
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .collapsible(false)
                    .resizable(false)
                    // Fix only the width; the height follows the content,
                    // so no emptiness remains below and the button is exactly centered.
                    .min_width(400.0)
                    .max_width(400.0)
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(4.0);

                            let progress = if self.batch_total > 0 {
                                (self.batch_done as f32 / self.batch_total as f32).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };

                            ui.add(
                                egui::ProgressBar::new(progress)
                                    .show_percentage()
                                    .desired_width(340.0)
                                    .fill(theme::ACCENT),
                            );

                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new(&self.batch_progress_message)
                                    .size(13.0)
                                    .color(theme::TEXT),
                            );

                            ui.add_space(14.0);
                            if ui
                                .add(
                                    egui::Button::new(format!("{} Cancel", ph::PROHIBIT))
                                        .min_size(egui::vec2(120.0, 28.0)),
                                )
                                .clicked()
                            {
                                self.export_cancel
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                                self.batch_progress_message =
                                    format!("{} Cancelling...", ph::PROHIBIT);
                            }
                            ui.add_space(4.0);
                        });
                    });

                // While the export runs — periodically wake the UI to drain the progress.
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            } else {
                // The export finished (success/error/cancel) — close the window.
                self.show_batch_progress = false;
            }
        }

        // ===== CLOSE SESSION CONFIRMATION =====
        // Ask only when there is something to lose: otherwise closing should be instant.
        if self.show_close_confirm {
            let modified = self.session.modified_count();
            let mut confirm = false;
            let mut cancel = false;

            egui::Window::new(format!("{} Close session?", ph::WARNING))
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .min_width(340.0)
                .max_width(340.0)
                .show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("{} image(s) have unsaved edits.", modified))
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Closing the session will discard them.")
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                    );

                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        const CONFIRM_W: f32 = 160.0;
                        const CANCEL_W: f32 = 80.0;
                        let spacing =
                            ((ui.available_width() - (CONFIRM_W + CANCEL_W + 10.0)) / 2.0).max(0.0);
                        ui.add_space(spacing);

                        if ui
                            .add(
                                egui::Button::new(format!("{} Close anyway", ph::X))
                                    .min_size(egui::vec2(CONFIRM_W, 28.0))
                                    .fill(theme::DANGER),
                            )
                            .clicked()
                        {
                            confirm = true;
                        }

                        ui.add_space(10.0);
                        if ui
                            .add(egui::Button::new("Cancel").min_size(egui::vec2(CANCEL_W, 28.0)))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });

            if confirm {
                self.show_close_confirm = false;
                self.close_session();
            }
            if cancel {
                self.show_close_confirm = false;
            }
        }

        // Restore the original separator color after drawing the dialogs.
        ctx.style_mut(|s| {
            s.visuals.widgets.noninteractive.bg_stroke = prev_window_sep;
        });
    }
}
