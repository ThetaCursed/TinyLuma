// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use super::notification::ToastKind;
use super::widgets::{labeled_slider, labeled_slider_gradient};
use crate::app::TinyLumaApp;
use crate::settings::FilterSettings;
use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;
use std::path::PathBuf;

impl TinyLumaApp {
    /// Left panel: histogram and the adjustment sliders.
    /// Returns (color_changed, spatial_changed).
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
            // Mirror the floating-scrollbar reserve (right) with an equal inset on the
            // left, so the groups get symmetric gutters and read centered in the panel.
            .frame(
                egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin {
                    left: 8.0 + super::SCROLL_RESERVE,
                    right: 8.0,
                    top: 2.0,
                    bottom: 2.0,
                }),
            )
            .show_animated(ctx, visible, |ui| {
                // The scrollbar is floating, so the content is narrowed on the right by
                // SCROLL_RESERVE (the handle itself is 4px, with 8px on either side).
                // The SidePanel frame adds the same SCROLL_RESERVE on the LEFT (see
                // above), so the group gutters match on both sides.
                //   group→handle gap = SCROLL_RESERVE - bar_outer_margin - bar_width
                //   handle→panel edge = bar_outer_margin + 8
                // With 12 - 0 - 4 = 8 and 0 + 8 = 8 — both are 8px.
                // floating_width == bar_width so the handle does not balloon on hover.
                let s = &mut ui.spacing_mut().scroll;
                s.bar_width = 4.0;
                s.floating_width = 4.0;
                s.floating_allocated_width = super::SCROLL_RESERVE;
                s.bar_outer_margin = 0.0;

                // Retouch mode shows the unadjusted base, so the adjustments are
                // visually disabled (they still apply outside the tool).
                if self.retouch.active {
                    ui.disable();
                }

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

                // Histogram — pinned at the top of the panel, so it is
                // always visible while the slider groups below it scroll.
                ui.scope(|ui| {
                    ui.set_max_width((ui.available_width() - super::SCROLL_RESERVE).max(0.0));
                    self.show_histogram_panel(ui);
                });

                // Everything below the histogram scrolls: the collapsible slider
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
                            let (c, d) = labeled_slider_gradient(
                                ui,
                                "Temp",
                                &mut self.settings.temp,
                                -100.0..=100.0,
                                def.temp,
                                &theme::TEMP_TRACK,
                            );
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                            let (c, d) = labeled_slider_gradient(
                                ui,
                                "Tint",
                                &mut self.settings.tint,
                                -100.0..=100.0,
                                def.tint,
                                &theme::TINT_TRACK,
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

                    // --- COLOR MIXER ---
                    let mut mixer_name_clicked = false;
                    let mut mixer_header =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("group_mixer"),
                            self.open_mixer_group,
                        )
                        .show_header(ui, |ui| {
                            mixer_name_clicked = ui
                                .add(
                                    egui::Label::new(format!("{} COLOR MIXER", ph::SWATCHES))
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked();
                        });
                    if mixer_name_clicked {
                        mixer_header.toggle();
                    }
                    let mixer_open = mixer_header.is_open();
                    mixer_header.body_unindented(|ui| {
                        ui.group(|ui| {
                            let (c, d) = self.show_color_mixer(ui);
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                        });
                    });
                    if mixer_open != self.open_mixer_group {
                        self.open_mixer_group = mixer_open;
                        groups_changed = true;
                    }

                    ui.add_space(5.0);

                    // --- CURVES ---
                    let mut curve_name_clicked = false;
                    let mut curve_header =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("group_curves"),
                            self.open_curve_group,
                        )
                        .show_header(ui, |ui| {
                            curve_name_clicked = ui
                                .add(
                                    egui::Label::new(format!("{} CURVES", ph::BEZIER_CURVE))
                                        .selectable(false)
                                        .sense(egui::Sense::click()),
                                )
                                .clicked();
                        });
                    if curve_name_clicked {
                        curve_header.toggle();
                    }
                    let curve_open = curve_header.is_open();
                    curve_header.body_unindented(|ui| {
                        ui.group(|ui| {
                            let (c, d) = self.show_curve_editor(ui);
                            if c {
                                color_changed = true;
                            }
                            if d {
                                self.drag_active = true;
                            }
                        });
                    });
                    if curve_open != self.open_curve_group {
                        self.open_curve_group = curve_open;
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
                            let (c, d) = labeled_slider(
                                ui,
                                "Grain Size",
                                &mut self.settings.grain_size,
                                0.0..=100.0,
                                def.grain_size,
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
