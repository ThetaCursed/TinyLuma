// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The colour mixer body: 8 band dots (Lightroom order) and the three
//! Hue / Saturation / Luminance sliders for the selected band. Drawing and
//! input only — the math lives in `src/pipeline/hsl.rs`.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;
use crate::pipeline::hsl::{BAND_NAMES, HSL_BANDS, band_center_hue};
use crate::theme;
use crate::ui::widgets::labeled_slider_gradient;

impl TinyLumaApp {
    /// The COLOR MIXER group body: band dots + Hue/Saturation/Luminance.
    /// Returns `(changed, dragging)`.
    pub(crate) fn show_color_mixer(&mut self, ui: &mut egui::Ui) -> (bool, bool) {
        let mut changed = false;
        let mut dragging = false;
        let band = self.mixer_band.min(HSL_BANDS - 1);

        // --- selected band name + reset (above the band dots) ---
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                egui::RichText::new(BAND_NAMES[band]).size(12.0).color(theme::TEXT),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let any = !self.settings.hsl.is_identity();
                if ui
                    .add_enabled(
                        any,
                        egui::Button::new(ph::ARROW_COUNTER_CLOCKWISE).frame(false),
                    )
                    .on_hover_text("Reset all color mixer bands")
                    .clicked()
                {
                    self.settings.hsl.bands = [[0.0; 3]; HSL_BANDS];
                    changed = true;
                }
            });
        });
        ui.add_space(6.0);

        // --- band colour dots (single row) ---
        let count = HSL_BANDS;
        let avail = ui.available_width();
        // Eight circles with even gaps, spanning the panel like the old chips.
        let diameter = 24.0;
        let gap = ((avail - diameter * count as f32) / (count as f32 - 1.0))
            .clamp(2.0, 12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (i, name) in BAND_NAMES.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(diameter, diameter),
                    egui::Sense::click(),
                );
                let selected = i == band;
                let painter = ui.painter();
                let center = rect.center();
                let r = diameter * 0.5;
                painter.circle_filled(center, r, band_color(i));
                let stroke = if selected {
                    egui::Stroke::new(2.0, theme::TEXT)
                } else {
                    egui::Stroke::new(1.0, theme::SEPARATOR)
                };
                // Draw the ring inside the disc so it stays within its slot.
                painter.circle_stroke(center, r - stroke.width * 0.5, stroke);
                if resp.clicked() {
                    self.mixer_band = i;
                }
                resp.on_hover_text(*name);
            }
        });

        ui.add_space(4.0);

        // --- the three band sliders (coloured rails) ---
        // The gradients follow `pipeline::hsl`'s own scales so the colour under
        // the handle is the actual output hue: the Hue rail spans ±30° (our
        // `HUE_SCALE` is 0.5 rad ≈ 28.65°), and Saturation/Luminance re-tint
        // live as the Hue/Saturation sliders move. Palette (`hsl`, 50%) matches
        // RapidRAW's mixer rails.
        let band_hue = band_center_hue(band);
        const HUE_SCALE_DEG: f32 = 28.65;

        let values = &mut self.settings.hsl.bands[band];

        let hue_track: [egui::Color32; 9] = std::array::from_fn(|i| {
            let t = i as f32 / 8.0;
            theme::hsl_color(band_hue - 30.0 + t * 60.0, 0.5, 0.5)
        });
        let (c, d) = labeled_slider_gradient(
            ui,
            "Hue",
            &mut values[0],
            -100.0..=100.0,
            0.0,
            &hue_track,
        );
        changed |= c;
        dragging |= d;

        let effective_hue = band_hue + values[0] * HUE_SCALE_DEG / 100.0;

        // Saturation: neutral → vivid band colour (RapidRAW `sat-slider-*`).
        let sat_track: [egui::Color32; 3] =
            std::array::from_fn(|i| theme::hsl_color(effective_hue, i as f32 / 2.0, 0.5));
        let (c, d) = labeled_slider_gradient(
            ui,
            "Saturation",
            &mut values[1],
            -100.0..=100.0,
            0.0,
            &sat_track,
        );
        changed |= c;
        dragging |= d;

        // Luminance: black → tinted mid → white at the live saturation
        // (RapidRAW `lum-slider-*`).
        let effective_sat = ((values[1] + 100.0) / 200.0).clamp(0.0, 1.0);
        let lum_track: [egui::Color32; 3] =
            [0.0, 0.5, 1.0].map(|l| theme::hsl_color(effective_hue, effective_sat, l));
        let (c, d) = labeled_slider_gradient(
            ui,
            "Luminance",
            &mut values[2],
            -100.0..=100.0,
            0.0,
            &lum_track,
        );
        changed |= c;
        dragging |= d;

        (changed, dragging)
    }
}

/// The dot colour of band `i` (an approximate fully-saturated hue swatch).
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
