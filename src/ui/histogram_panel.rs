// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The HISTOGRAM panel: a Lightroom-style filled RGB histogram with clipping
//! warning triangles and a toggle, docked directly under the presets.
//!
//! Drawing only — the bins and the clipping model live in `src/app/histogram.rs`
//! and `src/app/clipping.rs`. Reference: RAWmakase's `inspector.rs`
//! (`histogram_ui`), retargeted to TinyLuma's `theme::*` tokens.
//!
//! The bars are drawn from [`TinyLumaApp::histogram_display`], an eased copy of
//! the rendered bins, so moving a slider does not make the plot jump.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::clipping::{self, ClipSide};
use crate::app::TinyLumaApp;
use crate::theme;

/// Height of the histogram window.
const HIST_H: f32 = 90.0;
/// Bars are inset by this much so they stay inside the rounded corners.
const PLOT_INSET: f32 = 5.0;

impl TinyLumaApp {
    pub(crate) fn show_histogram_panel(&mut self, ui: &mut egui::Ui) {
        // Ease the drawn bins toward the freshly rendered ones; keep repainting
        // until they settle so the animation finishes after a drag release.
        if self.ease_histogram(ui.ctx()) {
            ui.ctx().request_repaint();
        }

        ui.add_space(5.0);

        // Header: the label on the left, a both-warnings toggle on the right.
        ui.horizontal(|ui| {
            ui.label(format!("{} HISTOGRAM", ph::CHART_BAR));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let both = self.clipping.both_on();
                let (lo, hi) = self.histogram.clipping();
                let color = if both {
                    theme::ACCENT
                } else {
                    theme::TEXT_TERTIARY
                };
                let resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new(if both { ph::EYE } else { ph::EYE_SLASH })
                            .size(13.0)
                            .color(color),
                    )
                    .frame(false),
                );
                let tip = format!(
                    "Show shadow & highlight clipping\nShadows {:.2}%  ·  Highlights {:.2}%",
                    lo * 100.0,
                    hi * 100.0
                );
                if resp.on_hover_text(tip).clicked() {
                    self.clipping.toggle_both();
                }
            });
        });
        ui.add_space(3.0);

        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), HIST_H),
            egui::Sense::hover(),
        );
        let painter = ui.painter().clone();

        // Rounded window with a subtle rim.
        painter.rect_filled(rect, theme::RADIUS_MD, theme::BG_DEEP);
        painter.rect_stroke(
            rect.shrink(0.5),
            theme::RADIUS_MD,
            egui::Stroke::new(1.0, theme::SEPARATOR),
        );

        let plot = rect.shrink2(egui::vec2(PLOT_INSET, PLOT_INSET));

        // Faint verticals at fifths, as a value reference.
        for i in 1..5 {
            let x = plot.left() + i as f32 / 5.0 * plot.width();
            painter.line_segment(
                [
                    egui::pos2(x, plot.top()),
                    egui::pos2(x, plot.bottom()),
                ],
                egui::Stroke::new(1.0, theme::with_alpha(egui::Color32::WHITE, 8)),
            );
        }

        self.draw_histogram_bars(&painter, plot);

        // Clipping triangles, as Lightroom's: each lit in the colours of the
        // channels clipping at its end, a click toggles its warning, hovering
        // shows it while the pointer stays.
        let triangles = ClipSide::BOTH.map(|side| clip_triangle(rect, side));
        // The panel is disabled while retouch is active (`ui.disable()`); the raw
        // `interact` below does not honour that, so gate it ourselves.
        let interactive = ui.is_enabled();
        let mut hovered = None;
        for (side, (corner, dir, hit)) in ClipSide::BOTH.into_iter().zip(triangles) {
            let left = side == ClipSide::Shadows;
            let response = ui
                .interact(hit, ui.id().with(("clip", left)), egui::Sense::click())
                .on_hover_text(if left {
                    "Show shadow clipping"
                } else {
                    "Show highlight clipping"
                });
            if interactive && response.clicked() {
                self.clipping.toggle(side);
            }
            if interactive && response.hovered() {
                hovered = Some(side);
            }
            let hovered_here = interactive && response.hovered();

            let on = self.clipping.is_on(side);
            let channels = clipping::clipped_channels(&self.histogram, side);
            let color = theme::clip_indicator(channels).unwrap_or(if on || hovered_here {
                theme::HIST_TRIANGLE_ACTIVE
            } else {
                theme::HIST_TRIANGLE_IDLE
            });
            let stroke = if on {
                egui::Stroke::new(1.0, theme::TEXT)
            } else {
                egui::Stroke::NONE
            };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    corner,
                    corner + egui::vec2(9.0 * dir, 0.0),
                    corner + egui::vec2(0.0, 7.0),
                ],
                color,
                stroke,
            ));
        }
        self.clipping.set_hover(hovered);
    }

    /// Lerps the drawn bins toward the rendered ones with a short time constant,
    /// so the histogram glides instead of jumping while a slider moves. Returns
    /// `true` while it is still moving (the caller should request another frame).
    ///
    /// A different image snaps, so opening a frame never morphs from the
    /// previous frame's histogram.
    fn ease_histogram(&mut self, ctx: &egui::Context) -> bool {
        let target = self.histogram.bins;
        if self.histogram_display_path != self.image_path {
            for (c, row) in target.iter().enumerate() {
                for (i, &v) in row.iter().enumerate() {
                    self.histogram_display[c][i] = v as f32;
                }
            }
            self.histogram_display_path = self.image_path.clone();
            return false;
        }

        // Frame-rate independent easing (~50 ms time constant).
        let dt = ctx.input(|i| i.stable_dt);
        let alpha = (dt / 0.05).clamp(0.05, 1.0);
        let mut moving = false;
        for (c, row) in target.iter().enumerate() {
            for (i, &tv) in row.iter().enumerate() {
                let d = self.histogram_display[c][i];
                let t = tv as f32;
                if (t - d).abs() > 0.5 {
                    moving = true;
                }
                self.histogram_display[c][i] = d + (t - d) * alpha;
            }
        }
        moving
    }

    /// Filled channel bars whose overlaps mix to yellow / cyan / magenta / grey,
    /// with a light 1-2-1 smoothing and a scale that ignores the clipped end bins.
    fn draw_histogram_bars(&self, painter: &egui::Painter, plot: egui::Rect) {
        let h = self.histogram_display;
        let smooth = |c: usize, i: usize| {
            let at = |j: isize| h[c][j.clamp(0, 255) as usize];
            let i = i as isize;
            (at(i - 1) + 2.0 * at(i) + at(i + 1)) / 4.0
        };
        let max = (2..254usize)
            .flat_map(|i| (0..3usize).map(move |c| (c, i)))
            .map(|(c, i)| smooth(c, i))
            .fold(1.0f32, f32::max);

        let bar = plot.width() / 256.0;
        let colors = [theme::HIST_RED, theme::HIST_GREEN, theme::HIST_BLUE];
        let pair = |a: usize, b: usize| match (a.min(b), a.max(b)) {
            (0, 1) => theme::HIST_OVERLAP_RG,
            (1, 2) => theme::HIST_OVERLAP_GB,
            _ => theme::HIST_OVERLAP_RB,
        };

        for i in 0..256 {
            let mut v = [0.0f32; 3];
            for (c, val) in v.iter_mut().enumerate() {
                *val = (smooth(c, i) / max).sqrt().min(1.0) * plot.height();
            }
            // Draw the smallest channel first so the overlaps read as mixes.
            let mut order = [0usize, 1, 2];
            order.sort_by(|a, b| v[*a].total_cmp(&v[*b]));

            let x = plot.left() + i as f32 * bar;
            let segment = |from: f32, to: f32, color: egui::Color32| {
                if to > from {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(x, plot.bottom() - to),
                            egui::pos2(x + bar + 0.5, plot.bottom() - from),
                        ),
                        egui::Rounding::ZERO,
                        color,
                    );
                }
            };
            segment(0.0, v[order[0]], theme::HIST_BASE);
            segment(v[order[0]], v[order[1]], pair(order[1], order[2]));
            segment(v[order[1]], v[order[2]], colors[order[2]]);
        }
    }
}

/// A histogram corner's clipping triangle: its corner point, which way it
/// points, and its hit rectangle. Inset so it sits inside the rounded window.
fn clip_triangle(rect: egui::Rect, side: ClipSide) -> (egui::Pos2, f32, egui::Rect) {
    let (corner, dir) = match side {
        ClipSide::Shadows => (rect.left_top() + egui::vec2(5.0, 5.0), 1.0),
        ClipSide::Highlights => (rect.right_top() + egui::vec2(-5.0, 5.0), -1.0),
    };
    let hit = egui::Rect::from_center_size(
        corner + egui::vec2(4.5 * dir, 3.0),
        egui::vec2(18.0, 18.0),
    );
    (corner, dir, hit)
}
