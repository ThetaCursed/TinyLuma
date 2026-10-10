// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The PRESETS block: selector, management actions and the rename/delete
//! dialogs. It lives in the right panel, above the LUT library.
//!
//! Applying a preset changes BOTH color and spatial settings, so the block
//! reports `(color_changed, spatial_changed)`; the caller feeds those into the
//! same dirty/history machinery as the sliders.

use eframe::egui;
use egui_phosphor::regular as ph;

use super::notification::ToastKind;
use crate::app::TinyLumaApp;
use crate::settings::FilterSettings;
use crate::theme;

impl TinyLumaApp {
    /// The PRESETS block: selector, management actions, rename/delete dialogs.
    /// Returns `(color_changed, spatial_changed)`.
    pub(crate) fn show_presets_panel(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
    ) -> (bool, bool) {
        let mut color_changed = false;
        let mut spatial_changed = false;

        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - super::SCROLL_RESERVE).max(0.0));

            // ==========================================
            // PRESETS
            // ==========================================
            ui.add_space(5.0);
            ui.label(format!("{} PRESETS", ph::FOLDER));
            ui.group(|ui| {
                ui.set_width(ui.available_width());

                let preset_names: Vec<String> = self
                    .preset_manager
                    .presets
                    .iter()
                    .map(|p| p.name.clone())
                    .collect();

                // Whether the current preset differs from the saved one.
                let preset_dirty = self.selected_preset_dirty();

                let can_sel = self.preset_manager.selected_index.is_some();

                // Shared dropdown selector: 24px collapsed and
                // a list without an inner scrollbar (see widgets::preset_dropdown).
                let choice = super::widgets::preset_dropdown(
                    ui,
                    "preset_selector",
                    &preset_names,
                    self.preset_manager.selected_index,
                    true,
                    preset_dirty,
                );
                if choice.none {
                    // "None" — clear the selection and work freely.
                    self.preset_manager.selected_index = None;
                }
                if let Some(i) = choice.index {
                    if let Some(warn) = self.apply_preset(i) {
                        self.notify(ToastKind::Warning, warn);
                    }
                    color_changed = true;
                    spatial_changed = true;
                }

                if preset_names.is_empty() {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new("No presets yet — create one with Save As")
                            .size(11.0)
                            .color(theme::TEXT_SECONDARY),
                    );
                }

                // Button sizes are shared by both rows.
                let btn_h = 24.0;
                let gap = 5.0;

                // --- PRESET MANAGEMENT (top): icons with tooltips ---
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let icon_w = ((ui.available_width() - gap * 2.0) / 3.0).max(32.0);

                    // Revert — roll back unsaved changes to the preset.
                    if ui
                        .add_enabled(
                            can_sel && preset_dirty,
                            egui::Button::new(
                                egui::RichText::new(ph::ARROW_U_UP_LEFT).size(14.0),
                            )
                            .min_size(egui::vec2(icon_w, btn_h)),
                        )
                        .on_hover_text("Discard unsaved changes and restore the preset")
                        .clicked()
                    {
                        let idx = self.preset_manager.selected_index.unwrap();
                        if let Some(warn) = self.apply_preset(idx) {
                            self.notify(ToastKind::Warning, warn);
                        }
                        color_changed = true;
                        spatial_changed = true;
                    }

                    // Rename.
                    if ui
                        .add_enabled(
                            can_sel,
                            egui::Button::new(
                                egui::RichText::new(ph::PENCIL_SIMPLE).size(14.0),
                            )
                            .min_size(egui::vec2(icon_w, btn_h)),
                        )
                        .on_hover_text("Rename the selected preset")
                        .clicked()
                    {
                        let idx = self.preset_manager.selected_index.unwrap();
                        if let Some(p) = self.preset_manager.presets.get(idx) {
                            self.preset_manager.rename_name = p.name.clone();
                        }
                        self.preset_manager.rename_index = Some(idx);
                        self.preset_manager.show_rename_dialog = true;
                        self.preset_manager.rename_focus_requested = true;
                    }

                    // Delete — with red hover and confirmation.
                    let del_resp = ui
                        .scope(|ui| {
                            let vis = &mut ui.style_mut().visuals.widgets;
                            vis.hovered.weak_bg_fill = theme::with_alpha(theme::DANGER, 30);
                            vis.hovered.bg_stroke = egui::Stroke::NONE;
                            vis.hovered.fg_stroke = egui::Stroke::new(1.0, theme::DANGER);
                            vis.active.weak_bg_fill = theme::with_alpha(theme::DANGER, 55);
                            vis.active.fg_stroke = egui::Stroke::new(1.0, theme::DANGER);
                            ui.add_enabled(
                                can_sel,
                                egui::Button::new(
                                    egui::RichText::new(ph::TRASH_SIMPLE).size(14.0),
                                )
                                .min_size(egui::vec2(icon_w, btn_h)),
                            )
                        })
                        .inner;
                    if del_resp.on_hover_text("Delete this preset").clicked() {
                        self.preset_manager.delete_confirm_index =
                            self.preset_manager.selected_index;
                    }
                });

                // --- SAVING (bottom): primary actions ---
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let w = ((ui.available_width() - gap) / 2.0).max(60.0);

                    // Save is active only when there is something to save; with
                    // edits present we highlight it as the primary action.
                    let save_enabled = can_sel && preset_dirty;
                    let save_label = format!("{} Save", ph::FLOPPY_DISK);
                    let save_resp = if save_enabled {
                        super::widgets::primary_button(
                            ui,
                            save_label,
                            egui::vec2(w, btn_h),
                        )
                    } else {
                        ui.add_enabled(
                            false,
                            egui::Button::new(save_label).min_size(egui::vec2(w, btn_h)),
                        )
                    };
                    if save_resp
                        .on_hover_text(if preset_dirty {
                            "Save the current settings into the selected preset"
                        } else {
                            "The preset is already up to date"
                        })
                        .clicked()
                    {
                        let idx = self.preset_manager.selected_index.unwrap();
                        let lut_path = self.lut_path.clone();
                        self.preset_manager
                            .save_current(idx, self.settings, lut_path);
                        self.notify(ToastKind::Success, "Preset saved");
                    }

                    if ui
                        .add(
                            egui::Button::new(format!("{} Save As", ph::FILE_PLUS))
                                .min_size(egui::vec2(w, btn_h)),
                        )
                        .on_hover_text("Create a new preset from the current settings")
                        .clicked()
                    {
                        self.preset_manager.show_new_dialog = true;
                        self.preset_manager.new_preset_name.clear();
                    }
                });
            });

            // New preset dialog (saves the current settings)
            if self.preset_manager.show_new_dialog {
                let mut created = false;
                egui::Window::new(format!("{} New Preset", ph::FILE_PLUS))
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .fixed_size([280.0, 165.0])
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.add_space(15.0);
                        ui.label("Enter preset name:");
                        ui.add_space(5.0);
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut self.preset_manager.new_preset_name,
                            )
                            .hint_text("Name...")
                            .desired_width(240.0),
                        );

                        let trimmed =
                            self.preset_manager.new_preset_name.trim().to_string();
                        let is_dup = self.preset_manager.name_exists(&trimmed);
                        let can_create = !trimmed.is_empty() && !is_dup;

                        ui.add_space(6.0);
                        if is_dup {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} A preset with this name already exists",
                                    ph::WARNING
                                ))
                                .size(11.0)
                                .color(theme::WARNING),
                            );
                        } else {
                            ui.add_space(14.0); // keep the height stable
                        }

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let spacing = (ui.available_width() - 180.0) / 2.0;
                            ui.add_space(spacing.max(0.0));

                            let mut create =
                                egui::Button::new(format!("{} Create", ph::CHECK))
                                    .min_size(egui::vec2(100.0, 26.0));
                            if can_create {
                                create = create.fill(theme::ACCENT);
                            }
                            if ui.add_enabled(can_create, create).clicked() {
                                let lut_path = self.lut_path.clone();
                                self.preset_manager.add_preset(
                                    trimmed.clone(),
                                    self.settings,
                                    lut_path,
                                );
                                self.notify(ToastKind::Success, "Preset created");
                                created = true;
                            }
                            ui.add_space(10.0);
                            if ui
                                .add(
                                    egui::Button::new("Cancel")
                                        .min_size(egui::vec2(70.0, 26.0)),
                                )
                                .clicked()
                            {
                                self.preset_manager.show_new_dialog = false;
                            }
                        });
                    });
                if created {
                    self.preset_manager.show_new_dialog = false;
                }
                ctx.request_repaint();
            }

            // Rename preset dialog (opened by double-clicking the name).
            if self.preset_manager.show_rename_dialog {
                let mut renamed = false;
                let mut cancel = false;

                egui::Window::new(format!("{} Rename Preset", ph::PENCIL_SIMPLE))
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .fixed_size([280.0, 165.0])
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.add_space(15.0);
                        ui.label("New name:");
                        ui.add_space(5.0);

                        let edit = ui.add(
                            egui::TextEdit::singleline(
                                &mut self.preset_manager.rename_name,
                            )
                            .hint_text("Name...")
                            .desired_width(240.0),
                        );
                        if self.preset_manager.rename_focus_requested {
                            edit.request_focus();
                            self.preset_manager.rename_focus_requested = false;
                        }

                        let idx = self.preset_manager.rename_index.unwrap_or(0);
                        let trimmed = self.preset_manager.rename_name.trim().to_string();
                        let is_dup = self.preset_manager.name_taken_by_other(&trimmed, idx);
                        let can_rename = !trimmed.is_empty() && !is_dup;

                        // Enter — confirm, Escape — cancel.
                        if can_rename
                            && edit.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            self.preset_manager.rename_preset(idx, trimmed.clone());
                            self.notify(ToastKind::Success, "Preset renamed");
                            renamed = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        }

                        ui.add_space(6.0);
                        if is_dup {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} A preset with this name already exists",
                                    ph::WARNING
                                ))
                                .size(11.0)
                                .color(theme::WARNING),
                            );
                        } else {
                            ui.add_space(14.0);
                        }

                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let spacing = (ui.available_width() - 180.0) / 2.0;
                            ui.add_space(spacing.max(0.0));

                            let mut ok = egui::Button::new(format!("{} Rename", ph::CHECK))
                                .min_size(egui::vec2(100.0, 26.0));
                            if can_rename {
                                ok = ok.fill(theme::ACCENT);
                            }
                            if ui.add_enabled(can_rename, ok).clicked() {
                                self.preset_manager.rename_preset(idx, trimmed.clone());
                                self.notify(ToastKind::Success, "Preset renamed");
                                renamed = true;
                            }
                            ui.add_space(10.0);
                            if ui
                                .add(
                                    egui::Button::new("Cancel")
                                        .min_size(egui::vec2(70.0, 26.0)),
                                )
                                .clicked()
                            {
                                cancel = true;
                            }
                        });
                    });

                if renamed || cancel {
                    self.preset_manager.show_rename_dialog = false;
                    self.preset_manager.rename_index = None;
                }
                ctx.request_repaint();
            }

            // Preset deletion confirmation (no undo — we ask).
            if let Some(idx) = self.preset_manager.delete_confirm_index {
                let name = self
                    .preset_manager
                    .presets
                    .get(idx)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let mut confirm = false;
                let mut cancel = false;

                egui::Window::new(format!("{} Delete preset?", ph::TRASH_SIMPLE))
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .fixed_size([300.0, 150.0])
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.add_space(14.0);
                        ui.label(
                            egui::RichText::new(format!("Delete \"{}\"?", name))
                                .size(13.0),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("This cannot be undone.")
                                .size(11.0)
                                .color(theme::TEXT_SECONDARY),
                        );

                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            let spacing = (ui.available_width() - 180.0) / 2.0;
                            ui.add_space(spacing.max(0.0));
                            if ui
                                .add(
                                    egui::Button::new(format!(
                                        "{} Delete",
                                        ph::TRASH_SIMPLE
                                    ))
                                    .min_size(egui::vec2(100.0, 26.0))
                                    .fill(theme::DANGER),
                                )
                                .clicked()
                            {
                                confirm = true;
                            }
                            ui.add_space(10.0);
                            if ui
                                .add(
                                    egui::Button::new("Cancel")
                                        .min_size(egui::vec2(70.0, 26.0)),
                                )
                                .clicked()
                            {
                                cancel = true;
                            }
                        });
                    });

                if confirm {
                    // A preset can only be deleted by selecting it, and selecting
                    // immediately applies the preset to the frame. So if the frame is still
                    // EQUAL to the deleted preset (the user changed nothing
                    // after applying), we reset the sliders and LUT: the preset
                    // was applied only because it was selected for deletion.
                    // If the frame has already been edited on top of the preset — the edits
                    // are kept, they belong to the user, not to the preset.
                    let matches_preset = self
                        .preset_manager
                        .presets
                        .get(idx)
                        .map(|p| p.settings == self.settings && p.lut_path == self.lut_path)
                        .unwrap_or(false);

                    self.preset_manager.delete_preset(idx);
                    self.preset_manager.delete_confirm_index = None;

                    if matches_preset {
                        self.settings = FilterSettings::default();
                        self.active_lut = None;
                        self.lut_path = None;
                        self.lut_lib.selected_lut_name = None;
                        self.combined_lut = None;
                        color_changed = true;
                        spatial_changed = true;
                    }

                    self.notify(
                        ToastKind::Success,
                        format!("Preset \"{}\" deleted", name),
                    );
                }
                if cancel {
                    self.preset_manager.delete_confirm_index = None;
                }
            }
        });

        (color_changed, spatial_changed)
    }
}
