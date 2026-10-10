// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The curve editor: a square 1:1 control-point editor for the master and
//! per-channel tone curves. Drawing and input only — the curve math lives in
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
        ui.horizontal(|ui| {
            for (channel, label) in [
                (CurveChannel::Master, "RGB"),
                (CurveChannel::Red, "R"),
                (CurveChannel::Green, "G"),
                (CurveChannel::Blue, "B"),
            ] {
                if ui.selectable_label(ch == channel, label).clicked() {
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

        let side = ui.available_width().clamp(120.0, 220.0);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());

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
        let line_color = channel_color(ch);

        painter.rect_filled(rect, theme::RADIUS_SM, theme::BG_DEEP);
        painter.rect_stroke(
            rect.shrink(0.5),
            theme::RADIUS_SM,
            egui::Stroke::new(1.0, theme::SEPARATOR),
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

        // The curve itself, sampled once per screen column.
        let steps = (rect.width() as usize).max(2);
        let stroke = egui::Stroke::new(1.75, line_color);
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
            painter.circle_filled(center, r, line_color);
            painter.circle_stroke(center, r, egui::Stroke::new(1.0, theme::BG_DEEP));
        }

        (changed, dragging)
    }
}

fn channel_color(ch: CurveChannel) -> egui::Color32 {
    match ch {
        CurveChannel::Master => theme::TEXT,
        CurveChannel::Red => theme::CURVE_RED,
        CurveChannel::Green => theme::CURVE_GREEN,
        CurveChannel::Blue => theme::CURVE_BLUE,
    }
}
