// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The colour mixer body: 8 band chips (Lightroom order) and the three
//! Hue / Saturation / Luminance sliders for the selected band. Drawing and
//! input only — the math lives in `src/pipeline/hsl.rs`.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;
use crate::pipeline::hsl::{BAND_NAMES, HSL_BANDS};
use crate::theme;
use crate::ui::widgets::labeled_slider;

impl TinyLumaApp {
    /// The COLOR MIXER group body: band chips + Hue/Saturation/Luminance.
    /// Returns `(changed, dragging)`.
    pub(crate) fn show_color_mixer(&mut self, ui: &mut egui::Ui) -> (bool, bool) {
        let mut changed = false;
        let mut dragging = false;
        let band = self.mixer_band.min(HSL_BANDS - 1);

        // --- band colour chips (single row) ---
        let gap = 3.0;
        let count = HSL_BANDS;
        let avail = ui.available_width();
        let chip_w = ((avail - gap * (count as f32 - 1.0)) / count as f32).clamp(18.0, 32.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (i, name) in BAND_NAMES.iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(chip_w, 22.0), egui::Sense::click());
                let selected = i == band;
                let painter = ui.painter();
                painter.rect_filled(rect, theme::RADIUS_SM, band_color(i));
                painter.rect_stroke(
                    rect,
                    theme::RADIUS_SM,
                    if selected {
                        egui::Stroke::new(2.0, theme::TEXT)
                    } else {
                        egui::Stroke::new(1.0, theme::SEPARATOR)
                    },
                );
                if resp.clicked() {
                    self.mixer_band = i;
                }
                resp.on_hover_text(*name);
            }
        });

        ui.add_space(4.0);

        // --- selected band name + reset ---
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                egui::RichText::new(BAND_NAMES[band]).size(12.0).color(theme::TEXT),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let any = self.settings.hsl.bands[band].iter().any(|v| v.abs() > 1e-6);
                if ui
                    .add_enabled(
                        !any,
                        egui::Button::new(ph::ARROW_COUNTER_CLOCKWISE).frame(false),
                    )
                    .on_hover_text("Reset this band")
                    .clicked()
                {
                    self.settings.hsl.bands[band] = [0.0; 3];
                    changed = true;
                }
            });
        });

        // --- the three band sliders ---
        let values = &mut self.settings.hsl.bands[band];
        let (c, d) = labeled_slider(ui, "Hue", &mut values[0], -100.0..=100.0, 0.0);
        changed |= c;
        dragging |= d;
        let (c, d) = labeled_slider(ui, "Saturation", &mut values[1], -100.0..=100.0, 0.0);
        changed |= c;
        dragging |= d;
        let (c, d) = labeled_slider(ui, "Luminance", &mut values[2], -100.0..=100.0, 0.0);
        changed |= c;
        dragging |= d;

        (changed, dragging)
    }
}

/// The chip colour of band `i` (an approximate fully-saturated hue swatch).
fn band_color(i: usize) -> egui::Color32 {
    const COLORS: [egui::Color32; HSL_BANDS] = [
        egui::Color32::from_rgb(224, 62, 62),   // Red
        egui::Color32::from_rgb(230, 140, 50),  // Orange
        egui::Color32::from_rgb(222, 200, 60),  // Yellow
        egui::Color32::from_rgb(90, 190, 90),   // Green
        egui::Color32::from_rgb(75, 195, 195),  // Aqua
        egui::Color32::from_rgb(75, 120, 230),  // Blue
        egui::Color32::from_rgb(150, 95, 220),  // Purple
        egui::Color32::from_rgb(220, 90, 180),  // Magenta
    ];
    COLORS[i.min(HSL_BANDS - 1)]
}
