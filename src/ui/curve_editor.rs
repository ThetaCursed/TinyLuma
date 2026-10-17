// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The curve editor: a square 1:1 control-point editor for the master and
//! per-channel tone curves, with the live image histogram behind the grid
//! (Lightroom-style). Drawing and input only — the curve math lives in
//! `src/pipeline/curve.rs`.
//!
//! A left click on empty space adds a point, a drag moves one, a right click
//! removes an interior point. The master curve uses the neutral text colour;
//! the per-channel curves use their channel colour.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;
use crate::pipeline::curve::CurveChannel;
use crate::theme;

/// How close (screen px) the pointer must be to grab a control point.
const GRAB_RADIUS: f32 = 11.0;

impl TinyLumaApp {
    /// The CURVES group body: channel tabs, reset, and the curve editor.
    /// Returns `(changed, dragging)`.
    pub(crate) fn show_curve_editor(&mut self, ui: &mut egui::Ui) -> (bool, bool) {
        let mut changed = false;
        let ch = self.curve_channel;

        // --- channel tabs + reset ---
        // The chips are always visible (a tinted fill and border, not just on
        // hover) and centred over the square editor. The reset sits at the far
        // right of the same row.
        const CHIP_GAP: f32 = 4.0;
        let avail = ui.available_width();
        let chips_w = 34.0 + 26.0 * 3.0 + CHIP_GAP * 3.0;
        let pad = ((avail - chips_w) * 0.5).max(0.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CHIP_GAP;
            ui.add_space(pad);
            for (channel, label) in [
                (CurveChannel::Master, "RGB"),
                (CurveChannel::Red, "R"),
                (CurveChannel::Green, "G"),
                (CurveChannel::Blue, "B"),
            ] {
                if channel_chip(ui, channel, label, ch == channel) {
                    self.curve_channel = channel;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let is_identity = self.settings.curves.curve(ch).is_identity();
                if ui
                    .add_enabled(
                        !is_identity,
                        egui::Button::new(ph::ARROW_COUNTER_CLOCKWISE).frame(false),
                    )
                    .on_hover_text("Reset this channel")
                    .clicked()
                {
                    self.settings.curves.curve_mut(ch).reset();
                    self.curve_drag = None;
                    changed = true;
                }
            });
        });
        ui.add_space(4.0);

        let avail = ui.available_width();
        let side = avail.clamp(120.0, 220.0);
        // Centre the square editor in the group; without this the leftover
        // width all ends up on the right and the plot looks shifted left.
        let pad = ((avail - side) * 0.5).max(0.0);
        let (rect, response) = ui
            .horizontal(|ui| {
                ui.add_space(pad);
                ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag())
            })
            .inner;

        let to_screen = |p: [f32; 2]| {
            egui::pos2(
                rect.left() + p[0] * rect.width(),
                rect.bottom() - p[1] * rect.height(),
            )
        };
        let from_screen = |pos: egui::Pos2| {
            [
                ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0),
            ]
        };
        let grab = |pts: &[[f32; 2]], pos: egui::Pos2| -> Option<usize> {
            let mut best: Option<(usize, f32)> = None;
            for (i, p) in pts.iter().enumerate() {
                let d = to_screen(*p).distance(pos);
                if d <= GRAB_RADIUS && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
            best.map(|(i, _)| i)
        };

        // Hit-test against the pre-interaction curve.
        let before = *self.settings.curves.curve(ch);
        let pointer = response.interact_pointer_pos();

        // --- input ---
        if response.drag_started()
            && let Some(pos) = pointer
        {
            if let Some(i) = grab(before.points(), pos) {
                self.curve_drag = Some(i);
            } else {
                let i = self.settings.curves.curve_mut(ch).insert(from_screen(pos));
                self.curve_drag = Some(i);
                changed = true;
            }
        }
        if response.dragged()
            && let (Some(i), Some(pos)) = (self.curve_drag, pointer)
        {
            let moved = self.settings.curves.curve_mut(ch);
            moved.move_point(i, from_screen(pos));
            if *moved != before {
                changed = true;
            }
        }
        if response.drag_stopped() {
            self.curve_drag = None;
        }
        if response.clicked()
            && let Some(pos) = pointer
            && grab(before.points(), pos).is_none()
        {
            self.settings.curves.curve_mut(ch).insert(from_screen(pos));
            changed = true;
        }
        if response.secondary_clicked()
            && let Some(pos) = pointer
            && let Some(i) = grab(before.points(), pos)
            && self.settings.curves.curve_mut(ch).remove(i)
        {
            self.curve_drag = None;
            changed = true;
        }
        let dragging = self.curve_drag.is_some();

        // --- drawing (after input, so a live drag shows immediately) ---
        let curve = *self.settings.curves.curve(ch);
        let painter = ui.painter();

        painter.rect_filled(rect, theme::RADIUS_SM, theme::BG_DEEP);
        painter.rect_stroke(
            rect.shrink(0.5),
            theme::RADIUS_SM,
            egui::Stroke::new(1.0, theme::SEPARATOR),
        );

        // Histogram behind the curve (Lightroom-style): the selected channel's
        // bins on R/G/B, a combined RGB envelope on the master tab. It is the
        // live rendered histogram, so it follows the curve as it is edited. The
        // backdrop carries the channel colour; the curve itself stays white.
        let hist_color = channel_color(ch);
        paint_histogram(
            painter,
            rect.shrink(1.0),
            &self.histogram_display,
            ch,
            theme::with_alpha(
                hist_color,
                if matches!(ch, CurveChannel::Master) {
                    26
                } else {
                    34
                },
            ),
        );

        // Quarter grid and a faint identity diagonal.
        let grid = theme::with_alpha(egui::Color32::WHITE, 10);
        for i in 1..4 {
            let t = i as f32 / 4.0;
            let x = rect.left() + t * rect.width();
            let y = rect.bottom() - t * rect.height();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0, grid),
            );
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, grid),
            );
        }
        painter.line_segment(
            [rect.left_bottom(), rect.right_top()],
            egui::Stroke::new(1.0, theme::with_alpha(egui::Color32::WHITE, 16)),
        );

        // The curve itself, sampled once per screen column. It is always white,
        // so it reads clearly over any channel-tinted histogram; the channel is
        // identified by the backdrop and the tabs.
        let steps = (rect.width() as usize).max(2);
        let stroke = egui::Stroke::new(1.75, theme::TEXT);
        let mut prev = to_screen([0.0, curve.evaluate(0.0)]);
        for i in 1..=steps {
            let x = i as f32 / steps as f32;
            let cur = to_screen([x, curve.evaluate(x)]);
            painter.line_segment([prev, cur], stroke);
            prev = cur;
        }

        // Control points (the grabbed or hovered one is a little larger).
        let hovered = pointer.and_then(|p| grab(curve.points(), p));
        for (i, p) in curve.points().iter().enumerate() {
            let center = to_screen(*p);
            let active = self.curve_drag == Some(i) || hovered == Some(i);
            let r = if active { 5.0 } else { 4.0 };
            painter.circle_filled(center, r, theme::TEXT);
            painter.circle_stroke(center, r, egui::Stroke::new(1.0, theme::BG_DEEP));
        }

        (changed, dragging)
    }
}

/// The histogram tint of `ch`: the channel colour, neutral for the master.
fn channel_color(ch: CurveChannel) -> egui::Color32 {
    match ch {
        CurveChannel::Master => theme::TEXT,
        CurveChannel::Red => theme::CURVE_RED,
        CurveChannel::Green => theme::CURVE_GREEN,
        CurveChannel::Blue => theme::CURVE_BLUE,
    }
}

/// A channel selector chip: always visible, tinted with the channel colour
/// (neutral grey for the master). Returns `true` when clicked.
fn channel_chip(
    ui: &mut egui::Ui,
    channel: CurveChannel,
    label: &str,
    selected: bool,
) -> bool {
    let master = matches!(channel, CurveChannel::Master);
    let tint = if master {
        theme::TEXT_SECONDARY
    } else {
        channel_color(channel)
    };
    let width = if master { 34.0 } else { 26.0 };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::click());
    let hovered = response.hovered();

    let (fill, stroke, text) = if selected {
        (
            theme::with_alpha(tint, 80),
            egui::Stroke::new(1.5, tint),
            if master {
                theme::BG_DEEP
            } else {
                theme::TEXT
            },
        )
    } else {
        (
            theme::with_alpha(tint, if hovered { 44 } else { 24 }),
            egui::Stroke::new(
                1.0,
                theme::with_alpha(tint, if hovered { 180 } else { 120 }),
            ),
            if master { theme::TEXT } else { tint },
        )
    };

    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS_SM, fill);
    painter.rect_stroke(rect, theme::RADIUS_SM, stroke);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        text,
    );

    response.clicked()
}

/// Paints the live image histogram behind the curve, Lightroom-style: the
/// selected channel's bins on the R/G/B tabs and a combined envelope (the max
/// of the three) on the master tab, where a true luminance histogram is not
/// available. The scale ignores the clipped end bins and a light 1-2-1
/// smoothing keeps the backdrop calm, matching the HISTOGRAM panel.
fn paint_histogram(
    painter: &egui::Painter,
    rect: egui::Rect,
    bins: &[[f32; 256]; 3],
    ch: CurveChannel,
    color: egui::Color32,
) {
    let at = |i: usize| match ch {
        CurveChannel::Master => bins[0][i].max(bins[1][i]).max(bins[2][i]),
        CurveChannel::Red => bins[0][i],
        CurveChannel::Green => bins[1][i],
        CurveChannel::Blue => bins[2][i],
    };
    let smooth = |i: usize| {
        let j = i as isize;
        let sample = |k: isize| at(k.clamp(0, 255) as usize);
        (sample(j - 1) + 2.0 * sample(j) + sample(j + 1)) * 0.25
    };
    let max = (2..254).map(smooth).fold(1.0f32, f32::max);
    if max <= 1.0 {
        // No image loaded / an empty histogram: don't paint a flat floor.
        return;
    }

    const STEPS: usize = 128;
    let mut mesh = egui::Mesh::default();
    let mut prev: Option<(f32, f32)> = None;
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let bin = (t * 255.0).round() as usize;
        let level = (smooth(bin) / max).sqrt().min(1.0);
        let x = rect.left() + t * rect.width();
        let top = rect.bottom() - level * rect.height() * 0.9;
        if let Some((px, ptop)) = prev {
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(egui::pos2(px, rect.bottom()), color);
            mesh.colored_vertex(egui::pos2(px, ptop), color);
            mesh.colored_vertex(egui::pos2(x, top), color);
            mesh.colored_vertex(egui::pos2(x, rect.bottom()), color);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        prev = Some((x, top));
    }
    painter.add(egui::Shape::mesh(mesh));
}
