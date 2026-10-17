// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The COLOR GRADING group body: Midtones centred on top, Shadows and
//! Highlights below it, and a collapsible Global wheel at the bottom. Each
//! wheel has its name above the disc (replaced by a live H/S/L readout while
//! the wheel or its mini Luminance rail is dragged) and a small unlabelled
//! Luminance slider under it, 20 % shorter than the wheel diameter. Below the wheels
//! sit the shared Blending / Balance sliders. Drawing and input only — the math
//! lives in `src/pipeline/grading.rs`.
//!
//! Each wheel is drawn from a single cached hue/saturation texture mapped onto
//! a triangle fan, with a draggable puck. Clicking a wheel selects it;
//! dragging sets hue/saturation; double-clicking resets it.

use eframe::egui;
use egui_phosphor::regular as ph;
use std::f32::consts::{FRAC_PI_2, TAU};

use crate::app::TinyLumaApp;
use crate::pipeline::grading::{GradingWheel, hsv_to_rgb};
use crate::theme;
use crate::ui::widgets::{labeled_slider, rail_slider};

/// Region captions, in the order of [`ColorGrading::wheels`].
const REGIONS: [&str; 4] = ["Shadows", "Midtones", "Highlights", "Global"];

/// The texture-cache id of the shared hue/saturation wheel.
fn wheel_texture_id() -> egui::Id {
    egui::Id::new("tl_color_grading_wheel")
}

impl TinyLumaApp {
    /// The COLOR GRADING group body.
    /// Returns `(changed, dragging)`.
    pub(crate) fn show_color_grading(&mut self, ui: &mut egui::Ui) -> (bool, bool) {
        let mut changed = false;
        let mut dragging = false;

        // Work on a copy so the wheel closures do not borrow `self` mutably.
        let mut grading = self.settings.grading;
        let mut region = self.grading_region.min(REGIONS.len() - 1);

        let texture = wheel_texture(ui.ctx());
        let gap = 6.0;
        let avail = ui.available_width();
        // The group's content rect: the reset-all-wheels button is pinned to its
        // top-right corner, level with the Midtones name.
        let body = ui.available_rect_before_wrap();
        // Midtones on top, the tonal extremes below, and Global at the bottom.
        // At most two wheels per row, so they keep the comfortable 112 px size
        // instead of shrinking to fit three across.
        let side = ((avail - gap) / 2.0).min(112.0);
        // Leading space that centres a row of `n` wheels in the panel.
        let row_pad = |n: usize| {
            let row_w = n as f32 * side + (n.saturating_sub(1)) as f32 * gap;
            ((avail - row_w) * 0.5).max(0.0)
        };

        // Row 1 — Midtones: the zone that carries most of the image.
        ui.horizontal(|ui| {
            ui.add_space(row_pad(1));
            let (c, d, clicked) = grading_wheel(
                ui,
                &texture,
                side,
                Some(REGIONS[1]),
                &mut grading.midtones,
                region == 1,
            );
            if clicked {
                region = 1;
            }
            changed |= c;
            dragging |= d;
        });
        ui.add_space(2.0);

        // Row 2 — Shadows | Highlights: the two ends of the tonal range.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(row_pad(2));
            for i in [0usize, 2] {
                let wheel = if i == 0 {
                    &mut grading.shadows
                } else {
                    &mut grading.highlights
                };
                let (c, d, clicked) =
                    grading_wheel(ui, &texture, side, Some(REGIONS[i]), wheel, region == i);
                if clicked {
                    region = i;
                }
                changed |= c;
                dragging |= d;
            }
        });
        ui.add_space(2.0);

        // Row 3 — Global, behind a spoiler: it is an optional whole-image
        // finish, so it should not cost a full wheel's height unless used.
        // The label brightens when it carries an adjustment, so a collapsed
        // spoiler still tells the user something is set there.
        let global_dirty = !grading.global.is_identity();
        let mut global_name_clicked = false;
        let mut global_header =
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id("grading_global"),
                false,
            )
            .show_header(ui, |ui| {
                global_name_clicked = ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(format!("{} GLOBAL", ph::GLOBE))
                                .size(11.0)
                                .color(if global_dirty {
                                    theme::TEXT
                                } else {
                                    theme::TEXT_TERTIARY
                                }),
                        )
                        .selectable(false)
                        .sense(egui::Sense::click()),
                    )
                    .clicked();
            });
        if global_name_clicked {
            global_header.toggle();
        }
        global_header.body_unindented(|ui| {
            ui.horizontal(|ui| {
                ui.add_space(row_pad(1));
                let (c, d, clicked) = grading_wheel(
                    ui,
                    &texture,
                    side,
                    None,
                    &mut grading.global,
                    region == 3,
                );
                if clicked {
                    region = 3;
                }
                changed |= c;
                dragging |= d;
            });
        });

        // Reset-all-wheels: top-right corner of the group, on the same line as
        // the Midtones name.
        let reset_rect = egui::Rect::from_min_size(
            egui::pos2(body.right() - 20.0, body.top()),
            egui::vec2(20.0, 20.0),
        );
        let mut reset_ui = ui.child_ui_with_id_source(
            reset_rect,
            egui::Layout::top_down(egui::Align::Max),
            "grading_reset",
            None,
        );
        if reset_ui
            .add_enabled(
                !grading.is_identity(),
                egui::Button::new(ph::ARROW_COUNTER_CLOCKWISE).frame(false),
            )
            .on_hover_text("Reset all wheels (keeps Blending/Balance)")
            .clicked()
        {
            grading.reset_wheels();
            changed = true;
        }

        // Shared transition controls.
        ui.add_space(2.0);
        ui.separator();
        ui.add_space(2.0);
        let (c, d) = labeled_slider(ui, "Blending", &mut grading.blending, 0.0..=100.0, 50.0);
        changed |= c;
        dragging |= d;
        let (c, d) = labeled_slider(ui, "Balance", &mut grading.balance, -100.0..=100.0, 0.0);
        changed |= c;
        dragging |= d;

        self.settings.grading = grading;
        self.grading_region = region;
        (changed, dragging)
    }
}

/// Gap between a wheel's name/readout and the disc.
const LABEL_GAP: f32 = 10.0;
/// Height reserved above each wheel: the name/readout row, its top margin and
/// `LABEL_GAP` to the disc.
const LABEL_H: f32 = 26.0;
/// Gap between the wheel and its mini Luminance slider.
const MINI_GAP: f32 = 10.0;
/// Height of the mini Luminance slider (its width is 80 % of the disc).
const MINI_H: f32 = 14.0;
/// Mini slider length relative to the visible wheel diameter.
const MINI_W_FACTOR: f32 = 0.8;

/// Draws and edits one wheel. The name sits above the disc; while the wheel or
/// its mini Luminance rail is dragged the name is replaced by the live H/S/L
/// values. Returns `(changed, dragging, clicked)`.
fn grading_wheel(
    ui: &mut egui::Ui,
    texture: &egui::TextureHandle,
    side: f32,
    label: Option<&str>,
    wheel: &mut GradingWheel,
    selected: bool,
) -> (bool, bool, bool) {
    let mut changed = false;
    let mut dragging = false;
    let mut clicked = false;

    // The cell is taller than the disc so the name/readout has its own row
    // above the wheel and a mini Luminance rail sits below it
    // (Lightroom-style). Only the disc and that rail are interactive, so
    // clicking the label row does not jump the puck.
    let (cell, _) = ui.allocate_exact_size(
        egui::vec2(side, LABEL_H + side + MINI_GAP + MINI_H),
        egui::Sense::hover(),
    );
    let rect = egui::Rect::from_min_size(
        cell.min + egui::vec2(0.0, LABEL_H),
        egui::vec2(side, side),
    );
    let center = rect.center();
    let radius = (side * 0.5 - 2.0).max(2.0);
    let response = ui.interact(
        rect,
        ui.id().with(("grading_wheel", label)),
        egui::Sense::click_and_drag(),
    );

    // --- disc (a textured triangle fan, so the corners stay transparent) ---
    let mut mesh = egui::Mesh::default();
    mesh.texture_id = texture.id();
    mesh.vertices.push(egui::epaint::Vertex {
        pos: center,
        uv: egui::pos2(0.5, 0.5),
        color: egui::Color32::WHITE,
    });
    const SEGMENTS: usize = 64;
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * TAU;
        let (s, c) = a.sin_cos();
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + egui::vec2(radius * c, radius * s),
            uv: egui::pos2(0.5 + 0.5 * c, 0.5 + 0.5 * s),
            color: egui::Color32::WHITE,
        });
    }
    for i in 0..SEGMENTS {
        mesh.indices
            .extend_from_slice(&[0, 1 + i as u32, 2 + i as u32]);
    }

    let painter = ui.painter().clone();
    painter.add(egui::Shape::mesh(mesh));
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected { theme::ACCENT } else { theme::SEPARATOR },
        ),
    );

    // --- input: drag to set hue/saturation, click to select, double-click to reset ---
    if response.double_clicked() {
        *wheel = GradingWheel::default();
        changed = true;
    } else if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let d = pos - center;
        let sat = (d.length() / radius).clamp(0.0, 1.0) * 100.0;
        let hue = d.x.atan2(-d.y).to_degrees().rem_euclid(360.0);
        if (wheel.sat - sat).abs() > 1e-4 || (wheel.hue - hue).abs() > 1e-4 {
            wheel.sat = sat;
            wheel.hue = hue;
            changed = true;
        }
        dragging = true;
        // Dragging a wheel also selects it (accent ring around the disc).
        clicked = true;
    } else if response.clicked() {
        clicked = true;
    }

    // --- puck ---
    let sat = (wheel.sat / 100.0).clamp(0.0, 1.0);
    let ang = wheel.hue.to_radians() - FRAC_PI_2;
    let puck = center + egui::vec2(radius * sat * ang.cos(), radius * sat * ang.sin());
    let [r, g, b] = hsv_to_rgb(wheel.hue, sat.max(0.05), 1.0);
    painter.circle_stroke(
        puck,
        5.5,
        egui::Stroke::new(2.5, egui::Color32::from_black_alpha(140)),
    );
    painter.circle_filled(
        puck,
        4.5,
        egui::Color32::from_rgb(
            (r * 255.0 + 0.5) as u8,
            (g * 255.0 + 0.5) as u8,
            (b * 255.0 + 0.5) as u8,
        ),
    );
    painter.circle_stroke(puck, 4.5, egui::Stroke::new(1.0, egui::Color32::WHITE));

    // --- mini Luminance rail under the wheel, 20 % shorter than the disc ---
    let slider_w = radius * 2.0 * MINI_W_FACTOR;
    let slider_rect = egui::Rect::from_center_size(
        egui::pos2(center.x, rect.bottom() + MINI_GAP + MINI_H * 0.5),
        egui::vec2(slider_w, MINI_H),
    );
    let mut slider_ui = ui.child_ui_with_id_source(
        slider_rect,
        egui::Layout::top_down(egui::Align::Min),
        ("grading_lum", label),
        None,
    );
    slider_ui.spacing_mut().interact_size.y = MINI_H;
    slider_ui.spacing_mut().slider_width = slider_w;
    // Same tinted Luminance rail as the old shared slider: black → the wheel's
    // live hue/saturation at mid → white, so it recolours as the wheel moves.
    let tint_sat = (wheel.sat / 100.0).clamp(0.0, 1.0);
    let lum_track: [egui::Color32; 3] =
        [0.0, 0.5, 1.0].map(|l| theme::hsl_color(wheel.hue, tint_sat, l));
    let (lum_changed, lum_dragging) = rail_slider(
        &mut slider_ui,
        &mut wheel.lum,
        -100.0..=100.0,
        0.0,
        Some(&lum_track),
        None,
        1.0,
    );
    changed |= lum_changed;
    dragging |= lum_dragging;
    if lum_changed || lum_dragging {
        clicked = true;
    }

    // --- name above the wheel, replaced by the live H/S/L values while dragging ---
    let (text, color) = if response.dragged() || lum_dragging {
        (
            format!(
                "H:{} S:{} L:{}",
                (wheel.hue.round() as i32).rem_euclid(360),
                wheel.sat.round() as i32,
                wheel.lum.round() as i32,
            ),
            theme::TEXT,
        )
    } else {
        (
            label.unwrap_or_default().to_owned(),
            if selected {
                theme::TEXT
            } else {
                theme::TEXT_TERTIARY
            },
        )
    };
    if !text.is_empty() {
        painter.text(
            egui::pos2(center.x, rect.top() - LABEL_GAP),
            egui::Align2::CENTER_BOTTOM,
            text,
            egui::FontId::proportional(10.5),
            color,
        );
    }

    response.on_hover_cursor(egui::CursorIcon::PointingHand);

    (changed, dragging, clicked)
}

/// Builds (once per context) and returns the shared hue/saturation wheel
/// texture: hue 0 (red) at the top, increasing clockwise; saturation grows
/// outward.
fn wheel_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = wheel_texture_id();
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return tex;
    }

    const N: usize = 128;
    let mut rgb = vec![0u8; N * N * 3];
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let hue = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
            let sat = (dx * dx + dy * dy).sqrt().min(1.0);
            let [r, g, b] = hsv_to_rgb(hue, sat, 1.0);
            let idx = (y * N + x) * 3;
            rgb[idx] = (r * 255.0 + 0.5) as u8;
            rgb[idx + 1] = (g * 255.0 + 0.5) as u8;
            rgb[idx + 2] = (b * 255.0 + 0.5) as u8;
        }
    }

    let image = egui::ColorImage::from_rgb([N, N], &rgb);
    let tex = ctx.load_texture("tl_color_grading_wheel", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}
