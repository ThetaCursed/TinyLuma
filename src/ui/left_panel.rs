// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use super::notification::ToastKind;
use super::widgets::labeled_slider;
use crate::app::TinyLumaApp;
use crate::settings::FilterSettings;
use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;
use std::path::PathBuf;

impl TinyLumaApp {
    /// Left panel: presets and sliders. Returns (color_changed, spatial_changed).
    /// `visible == false` — the panel slides off the edge (on the screen without a frame
    /// we show only "Get Started").
    pub(crate) fn show_left_panel(&mut self, ctx: &egui::Context, visible: bool) -> (bool, bool) {
        let mut color_changed = false;
        let mut spatial_changed = false;
        // Whether the collapsible group state changed (needs saving to config)
        let mut groups_changed = false;

        // 1. LEFT PANEL (width shared with the right one — for symmetry)
        egui::SidePanel::left("controls")
            .resizable(false)
            .exact_width(super::PANEL_W)
            // We do not draw the separator line at the panel edge: the border is given by the
            // background difference between the panel and the central area.
            .show_separator_line(false)
            .show_animated(ctx, visible, |ui| {
                // The scrollbar is floating. There is no scroll on the left, so the content
                // starts at the panel's regular margin; on the right the margin adds up
                // with SCROLL_RESERVE (reserve for the handle).
                //   group→handle gap = SCROLL_RESERVE - bar_outer_margin - bar_width
                //   handle→panel edge = bar_outer_margin + 8
                // With 12 - 0 - 4 = 8 and 0 + 8 = 8 — both are 8px.
                // floating_width == bar_width so the handle does not balloon on hover.
                let s = &mut ui.spacing_mut().scroll;
                s.bar_width = 4.0;
                s.floating_width = 4.0;
                s.floating_allocated_width = super::SCROLL_RESERVE;
                s.bar_outer_margin = 0.0;

                // The line is drawn by the last settings group itself (at the bottom); a separate
                // footer line would give a doubled divider before the button.
                // We remove the footer's horizontal padding: otherwise the button of the already
                // centered content drifts right. We constrain the button width
                // to the group width (the scrollbar reserve on the right).
                egui::TopBottomPanel::bottom("controls_reset_footer")
                    .frame(
                        egui::Frame::side_top_panel(&ctx.style())
                            .inner_margin(egui::Margin::symmetric(0.0, 2.0)),
                    )
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        ui.add_space(8.0);

                        let is_default = self.settings_are_default();
                        let reset_hint = if is_default {
                            "All sliders are already at their default values"
                        } else {
                            "Reset all sliders to defaults (the active LUT is preserved)"
                        };

                        let resp = ui
                            .add_enabled_ui(!is_default, |ui| {
                                super::widgets::subtle_button(
                                    ui,
                                    format!("{} Reset all settings", ph::ARROWS_CLOCKWISE),
                                    egui::vec2(
                                        ui.available_width() - super::SCROLL_RESERVE,
                                        super::FOOTER_BTN_H,
                                    ),
                                    theme::WARNING,
                                )
                            })
                            .inner;

                        if resp.on_hover_text(reset_hint).clicked() {
                            // Preserve the LUT when resetting the slider settings
                            let active_lut_backup = self.active_lut.clone();
                            let lut_path_backup = self.lut_path.clone();
                            let lut_lib_selection = self.lut_lib.selected_lut_name.clone();

                            self.settings = FilterSettings::default();

                            self.active_lut = active_lut_backup;
                            self.lut_path = lut_path_backup;
                            self.lut_lib.selected_lut_name = lut_lib_selection;

                            color_changed = true;
                            spatial_changed = true;
                            self.notify(ToastKind::Success, "Settings reset");
                        }

                        ui.add_space(8.0);
                    });

                // Presets are pinned at the top and do not scroll with the sliders.
                // We reduce the width by SCROLL_RESERVE so the group matches the
                // width of the scrollable groups below (where space is reserved
                // for the floating scrollbar).
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

                // Everything below the presets scrolls: the collapsible slider
                // groups. No divider needed — the preset group has its own
                // frame, which separates the fixed header from the list.
                ui.add_space(6.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // The column for the collapse arrow is narrower (by default ~18-25px)
                    ui.spacing_mut().indent = 14.0;

                    // ==========================================
                    // SLIDERS (collapsible groups with remembered state)
                    // ==========================================

                    // --- LIGHT ---
                    //
                    // `show_header` only makes the little triangle clickable, and `ui.label`
                    // is selectable text by default (I-beam cursor). Make the name clickable
                    // and non-selectable so a click on it also collapses/expands the group.
                    let mut light_name_clicked = false;
                    let mut light_header =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("group_light"),
                            self.open_groups[0],
                        )
                        .show_header(ui, |ui| {
                            light_name_clicked = ui
                                .add(
                                    egui::Label::new(format!("{} LIGHT", ph::SUN))
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked();
                        });
                    if light_name_clicked {
                        light_header.toggle();
                    }
                    let light_open = light_header.is_open();
                    light_header.body_unindented(|ui| {
                        ui.group(|ui| {
                            let def = FilterSettings::default();
                            let (c, d) = labeled_slider(
                                ui,
                                "Exposure",
                                &mut self.settings.exposure,
                                -5.0..=5.0,
                                def.exposure,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Contrast",
                                &mut self.settings.contrast,
                                -100.0..=100.0,
                                def.contrast,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Highlights",
                                &mut self.settings.highlights,
                                -100.0..=100.0,
                                def.highlights,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Shadows",
                                &mut self.settings.shadows,
                                -100.0..=100.0,
                                def.shadows,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Whites",
                                &mut self.settings.whites,
                                -100.0..=100.0,
                                def.whites,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Blacks",
                                &mut self.settings.blacks,
                                -100.0..=100.0,
                                def.blacks,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                        });
                    });
                    if light_open != self.open_groups[0] {
                        self.open_groups[0] = light_open;
                        groups_changed = true;
                    }

                    ui.add_space(5.0);

                    // --- COLOR ---
                    let mut color_name_clicked = false;
                    let mut color_header =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("group_color"),
                            self.open_groups[1],
                        )
                        .show_header(ui, |ui| {
                            color_name_clicked = ui
                                .add(
                                    egui::Label::new(format!("{} COLOR", ph::PALETTE))
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked();
                        });
                    if color_name_clicked {
                        color_header.toggle();
                    }
                    let color_open = color_header.is_open();
                    color_header.body_unindented(|ui| {
                        ui.group(|ui| {
                            let def = FilterSettings::default();
                            let (c, d) = labeled_slider(
                                ui,
                                "Temp",
                                &mut self.settings.temp,
                                -100.0..=100.0,
                                def.temp,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Tint",
                                &mut self.settings.tint,
                                -100.0..=100.0,
                                def.tint,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Vibrance",
                                &mut self.settings.vibrance,
                                -100.0..=100.0,
                                def.vibrance,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Saturation",
                                &mut self.settings.saturation,
                                -100.0..=100.0,
                                def.saturation,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                        });
                    });
                    if color_open != self.open_groups[1] {
                        self.open_groups[1] = color_open;
                        groups_changed = true;
                    }

                    ui.add_space(5.0);

                    // --- DETAILS ---
                    let mut details_name_clicked = false;
                    let mut details_header =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("group_details"),
                            self.open_groups[2],
                        )
                        .show_header(ui, |ui| {
                            details_name_clicked = ui
                                .add(
                                    egui::Label::new(format!("{} DETAILS", ph::CROSSHAIR))
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked();
                        });
                    if details_name_clicked {
                        details_header.toggle();
                    }
                    let details_open = details_header.is_open();
                    details_header.body_unindented(|ui| {
                        ui.group(|ui| {
                            let def = FilterSettings::default();
                            let (c, d) = labeled_slider(
                                ui,
                                "Texture",
                                &mut self.settings.texture,
                                -100.0..=100.0,
                                def.texture,
                            );
                            if c {
                                spatial_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Clarity",
                                &mut self.settings.clarity,
                                -100.0..=100.0,
                                def.clarity,
                            );
                            if c {
                                spatial_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Sharpening",
                                &mut self.settings.sharpen,
                                0.0..=150.0,
                                def.sharpen,
                            );
                            if c {
                                spatial_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                        });
                    });
                    if details_open != self.open_groups[2] {
                        self.open_groups[2] = details_open;
                        groups_changed = true;
                    }

                    ui.add_space(5.0);

                    // --- EFFECTS ---
                    let mut effects_name_clicked = false;
                    let mut effects_header =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("group_effects"),
                            self.open_groups[3],
                        )
                        .show_header(ui, |ui| {
                            effects_name_clicked = ui
                                .add(
                                    egui::Label::new(format!("{} EFFECTS", ph::SPARKLE))
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked();
                        });
                    if effects_name_clicked {
                        effects_header.toggle();
                    }
                    let effects_open = effects_header.is_open();
                    effects_header.body_unindented(|ui| {
                        ui.group(|ui| {
                            let def = FilterSettings::default();
                            let (c, d) = labeled_slider(
                                ui,
                                "Dehaze",
                                &mut self.settings.dehaze,
                                -100.0..=100.0,
                                def.dehaze,
                            );
                            if c {
                                spatial_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider(
                                ui,
                                "Grain",
                                &mut self.settings.grain,
                                0.0..=100.0,
                                def.grain,
                            );
                            if c {
                                spatial_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                        });
                    });
                    if effects_open != self.open_groups[3] {
                        self.open_groups[3] = effects_open;
                        groups_changed = true;
                    }

                    ui.add_space(10.0);
                });
            });

        if groups_changed {
            self.save_save_settings();
        }

        (color_changed, spatial_changed)
    }

    /// Whether the settings are at default, for indication purposes. While a slider
    /// is being dragged, we return the value captured at the start of the gesture: otherwise the
    /// Reset all button flickers when the value passes through the default.
    pub(crate) fn settings_are_default(&self) -> bool {
        if self.drag_active || self.was_dragging {
            if let Some(frozen) = self.drag_is_default {
                return frozen;
            }
        }
        self.settings == FilterSettings::default()
    }

    /// Whether the current settings/LUT have unsaved differences from the selected preset.
    /// While a slider is being dragged, we return the value captured at the start of the gesture
    /// (`drag_preset_dirty`): the live settings change every frame at that time, and
    /// during fast movement the comparison momentarily matches the preset — the dot and the
    /// Save button would flicker.
    pub(crate) fn selected_preset_dirty(&self) -> bool {
        if self.drag_active || self.was_dragging {
            if let Some(frozen) = self.drag_preset_dirty {
                return frozen;
            }
        }
        self.preset_dirty_with(&self.settings, &self.lut_path)
    }

    /// Compares the given settings/LUT with the selected preset.
    pub(crate) fn preset_dirty_with(
        &self,
        settings: &FilterSettings,
        lut_path: &Option<PathBuf>,
    ) -> bool {
        let Some(idx) = self.preset_manager.selected_index else {
            return false;
        };
        let Some(preset) = self.preset_manager.presets.get(idx) else {
            return false;
        };
        *settings != preset.settings || *lut_path != preset.lut_path
    }
}
