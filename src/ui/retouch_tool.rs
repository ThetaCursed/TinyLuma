// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Spot Healing Brush UI: tool options bar, pointer handling and the brush
//! cursor. No pixel math and no algorithm details live here.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;
use crate::theme;

impl TinyLumaApp {
    /// Retouch keyboard shortcuts: `[` / `]` change the size, `Shift+[` /
    /// `Shift+]` the hardness, `Esc` leaves the tool.
    pub(crate) fn handle_retouch_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.retouch.active || ctx.wants_keyboard_input() {
            return;
        }
        let (open, close) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::OpenBracket),
                i.key_pressed(egui::Key::CloseBracket),
            )
        });
        if open || close {
            let shift = ctx.input(|i| i.modifiers.shift);
            if shift {
                let d = if open { -0.05 } else { 0.05 };
                self.retouch.brush.hardness = (self.retouch.brush.hardness + d).clamp(0.0, 1.0);
            } else {
                let f = if open { 1.0 / 1.15 } else { 1.15 };
                self.retouch.brush.size = (self.retouch.brush.size * f).clamp(4.0, 400.0);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.retouch.stroke_active {
                self.retouch_cancel_stroke();
            }
            if self.retouch.active {
                self.retouch_toggle();
            }
        }
    }

    /// Floating options bar over the canvas while the tool is active.
    pub(crate) fn show_retouch_options(&mut self, ctx: &egui::Context, viewport_rect: egui::Rect) {
        if !self.retouch.active {
            self.retouch_options_rect = None;
            return;
        }
        // Sliders grow on a roomy canvas and shrink on a narrow one. Both use the
        // SAME width so the two rails stay visually aligned.
        let slider_w = if viewport_rect.width() > 780.0 {
            120.0
        } else {
            92.0
        };
        // Hotkey hints only when there is horizontal room for them.
        let show_hints = viewport_rect.width() > 700.0;
        // Center on the canvas (the side panels are symmetric, but this stays
        // correct even if they are not).
        let offset_x = viewport_rect.center().x - ctx.screen_rect().center().x;

        egui::Area::new(egui::Id::new("retouch_options"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(offset_x, viewport_rect.top() + 14.0),
            )
            .show(ctx, |ui| {
                // Reserve the background shape and fill it only after the content
                // is laid out, so the plaque hugs the controls exactly (otherwise
                // there is a long empty tail after the last hotkey hint).
                let bg = ui.painter().add(egui::Shape::Noop);
                let content = ui.horizontal(|child| {
                    child.label(
                        egui::RichText::new(ph::FIRST_AID)
                            .size(15.0)
                            .color(theme::ACCENT),
                    );
                    child.label(egui::RichText::new("Heal").size(12.0).color(theme::TEXT));
                    child.add_space(12.0);

                    // --- Size: number + slider ---
                    child.label(
                        egui::RichText::new("Size")
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                    );
                    child.add(
                        egui::DragValue::new(&mut self.retouch.brush.size)
                            .speed(1.0)
                            .range(4.0..=400.0)
                            .suffix("px"),
                    );
                    // Wide slider + accent fill (same look as the main sliders).
                    child.spacing_mut().slider_width = slider_w;
                    child.add(
                        egui::Slider::new(&mut self.retouch.brush.size, 4.0..=400.0)
                            .show_value(false)
                            .trailing_fill(true),
                    );

                    child.add_space(12.0);

                    // --- Hardness: number (%) + slider ---
                    child.label(
                        egui::RichText::new("Hardness")
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                    );
                    let mut hardness_pct = (self.retouch.brush.hardness * 100.0).round();
                    let hard_resp = child.add(
                        egui::DragValue::new(&mut hardness_pct)
                            .speed(1.0)
                            .range(0.0..=100.0)
                            .suffix("%"),
                    );
                    if hard_resp.changed() {
                        self.retouch.brush.hardness = (hardness_pct / 100.0).clamp(0.0, 1.0);
                    }
                    child.spacing_mut().slider_width = slider_w;
                    child.add(
                        egui::Slider::new(&mut self.retouch.brush.hardness, 0.0..=1.0)
                            .show_value(false)
                            .trailing_fill(true),
                    );

                    if show_hints {
                        // Separator between the sliders and the hotkey hints.
                        child.add_space(12.0);
                        v_separator(child);
                        child.add_space(12.0);

                        // Size is what people actually use; hardness hotkeys still
                        // work but are not advertised.
                        hint_group(child, "Size", &["[", "]"]);
                        child.add_space(12.0);
                        hint_group(child, "Exit", &["Esc"]);
                    }
                });

                let rect = content.response.rect.expand2(egui::vec2(12.0, 7.0));
                self.retouch_options_rect = Some(rect);
                let rounding = egui::Rounding::same(rect.height() * 0.5);
                ui.painter().set(
                    bg,
                    egui::Shape::Vec(vec![
                        egui::Shape::rect_filled(rect, rounding, theme::NAV_BG),
                        egui::Shape::rect_stroke(
                            rect.shrink(0.5),
                            rounding,
                            egui::Stroke::new(1.0, theme::NAV_BORDER),
                        ),
                    ]),
                );
            });
    }

    /// Handles retouch pointer input over the image. Returns `true` when the
    /// tool consumed the pointer, so pan/click handling must be skipped.
    pub(crate) fn retouch_pointer(
        &mut self,
        ui: &mut egui::Ui,
        hit_rect: egui::Rect,
        hover_pos: Option<egui::Pos2>,
    ) -> bool {
        if !self.retouch.active || hit_rect.width() <= 0.0 || hit_rect.height() <= 0.0 {
            return false;
        }
        let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
        let primary_down = ui.input(|i| i.pointer.primary_down());
        let primary_released = ui.input(|i| i.pointer.primary_released());
        let secondary_pressed = ui.input(|i| i.pointer.secondary_pressed());
        // Right click cancels the pending stroke (nothing has been applied yet).
        if secondary_pressed && self.retouch.stroke_active {
            self.retouch_cancel_stroke();
            return true;
        }
        // Space+drag is reserved for panning: end the current gesture cleanly.
        let space_held = ui.input(|i| i.key_down(egui::Key::Space));
        if self.retouch.stroke_active && space_held {
            self.retouch_commit_stroke();
        }

        let over = hover_pos.map_or(false, |p| hit_rect.contains(p));
        let over_bar = hover_pos.map_or(false, |p| {
            self.retouch_options_rect.map_or(false, |r| r.contains(p))
        });
        let over_toolbar = hover_pos.map_or(false, |p| {
            self.toolbar_rect.map_or(false, |r| r.expand(2.0).contains(p))
        });
        let over_tools = hover_pos.map_or(false, |p| {
            self.tools_rect.map_or(false, |r| r.contains(p))
        });

        let to_norm = |p: egui::Pos2| -> [f32; 2] {
            [
                ((p.x - hit_rect.left()) / hit_rect.width()).clamp(0.0, 1.0),
                ((p.y - hit_rect.top()) / hit_rect.height()).clamp(0.0, 1.0),
            ]
        };

        if !space_held && primary_pressed && over && !over_bar && !over_toolbar && !over_tools {
            if let Some(p) = hover_pos {
                self.retouch_begin_stroke(to_norm(p));
            }
            return true;
        }
        if !space_held && self.retouch.stroke_active && primary_down {
            if let Some(p) = hover_pos {
                if over && !over_bar && !over_toolbar && !over_tools {
                    self.retouch_extend_stroke(to_norm(p));
                }
            }
            ui.ctx().request_repaint();
            return true;
        }
        if primary_released && self.retouch.stroke_active {
            self.retouch_commit_stroke();
            return true;
        }
        false
    }

    /// Draws the translucent accent-blue indication of the region painted during
    /// the current gesture. The heal itself only runs on release.
    ///
    /// The indication is a single coverage texture (union of dabs), so overlapping
    /// brush circles never stack up into a darker band.
    pub(crate) fn draw_retouch_overlay(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        image_rect: egui::Rect,
    ) {
        if !self.retouch.stroke_active || self.retouch.pending_path.is_empty() {
            return;
        }
        self.refresh_overlay_texture(ctx);

        let (Some(tex), Some(area)) = (self.retouch.overlay_tex.as_ref(), self.retouch.overlay_area)
        else {
            return;
        };
        let screen = egui::Rect::from_min_max(
            egui::pos2(
                image_rect.left() + area[0] * image_rect.width(),
                image_rect.top() + area[1] * image_rect.height(),
            ),
            egui::pos2(
                image_rect.left() + area[2] * image_rect.width(),
                image_rect.top() + area[3] * image_rect.height(),
            ),
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter
            .with_clip_rect(image_rect)
            .image(tex.id(), screen, uv, egui::Color32::WHITE);
    }

    /// Rebuilds the indication texture when the pending path changed.
    fn refresh_overlay_texture(&mut self, ctx: &egui::Context) {
        if !self.retouch.overlay_dirty {
            return;
        }
        self.retouch.overlay_dirty = false;

        let Some(_) = self.preview_base.as_ref() else {
            self.retouch.overlay_tex = None;
            return;
        };
        let (ww, wh) = self.retouch_work_dims();
        // The indication is a soft hint — bound its resolution so large working
        // images stay cheap. `scale` maps working pixels to this grid.
        const OVERLAY_GRID: f32 = 1024.0;
        let scale = (OVERLAY_GRID / ww.max(wh) as f32).min(1.0);
        let gw = ((ww as f32 * scale).round() as usize).max(1);
        let gh = ((wh as f32 * scale).round() as usize).max(1);
        let Some(cov) = self
            .retouch
            .brush
            .stroke_coverage(&self.retouch.pending_path, gw, gh, scale)
        else {
            self.retouch.overlay_tex = None;
            return;
        };

        let accent = theme::ACCENT;
        let max_alpha = 110.0f32;
        let pixels: Vec<egui::Color32> = cov
            .data
            .iter()
            .map(|&c| {
                let a = (c.clamp(0.0, 1.0) * max_alpha) as u8;
                egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), a)
            })
            .collect();
        let image = egui::ColorImage {
            size: [cov.w, cov.h],
            pixels,
        };

        if let Some(tex) = &mut self.retouch.overlay_tex {
            tex.set(image, egui::TextureOptions::LINEAR);
        } else {
            self.retouch.overlay_tex = Some(ctx.load_texture(
                "retouch_overlay",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        self.retouch.overlay_area = Some(cov.area);
    }

    /// Draws the circular brush cursor at the pointer. `radius` is already in
    /// screen pixels (derived from the normalized brush radius so it matches the
    /// healed area at any display resolution).
    pub(crate) fn draw_brush_cursor(
        &self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        center: egui::Pos2,
        radius: f32,
    ) {
        ctx.set_cursor_icon(egui::CursorIcon::None);
        let r = radius.max(1.5);
        // Dark halo then a light ring, so it reads on light and dark photos.
        painter.circle_stroke(center, r, egui::Stroke::new(2.5, theme::with_alpha(theme::BG_DEEP, 180)));
        painter.circle_stroke(center, r, egui::Stroke::new(1.0, theme::TEXT));
        // A small center dot helps precise placement.
        painter.circle_filled(center, 1.2, theme::TEXT);
    }
}

/// A short vertical separator between the sliders and the hotkey hints.
fn v_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 18.0), egui::Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        rect.top()..=rect.bottom(),
        egui::Stroke::new(1.0, theme::DIVIDER),
    );
}

/// A small caption followed by its keycaps, e.g. `Size [ ]`.
fn hint_group(ui: &mut egui::Ui, label: &str, keys: &[&str]) {
    ui.label(
        egui::RichText::new(label)
            .size(12.0)
            .color(theme::TEXT_SECONDARY),
    );
    for k in keys {
        keycap(ui, k);
    }
}

/// Draws a small keyboard-key badge ("keycap") so a hotkey hint reads as a
/// shortcut rather than as a label.
fn keycap(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::monospace(12.0),
        theme::TEXT,
    );
    let pad = egui::vec2(6.0, 3.0);
    let size = galley.size() + pad * 2.0;
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::RADIUS_SM, theme::BG_ELEVATED);
    ui.painter().rect_stroke(
        rect,
        theme::RADIUS_SM,
        egui::Stroke::new(1.0, theme::NAV_BORDER),
    );
    ui.painter().galley(rect.min + pad, galley, theme::TEXT);
}
