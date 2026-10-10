// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Spot Healing Brush UI: tool options bar, pointer handling and the brush
//! cursor. No pixel math and no algorithm details live here.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;
use crate::retouch::BrushSettings;
use crate::theme;

/// Size of the `Reset` / `Done` action buttons in the retouch options bar.
/// Same height as the crop bar's action buttons so the two tools feel alike.
const ACTION_BTN: egui::Vec2 = egui::vec2(78.0, 24.0);

impl TinyLumaApp {
    /// Retouch keyboard shortcuts: `[` / `]` change the size, `Shift+[` /
    /// `Shift+]` the hardness. `Enter` keeps the heals and leaves the tool
    /// (keyboard form of `Done`); `Esc` discards every heal made in this session
    /// and leaves.
    pub(crate) fn handle_retouch_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.retouch.active
            || ctx.wants_keyboard_input()
            || self.modal_open()
            || ctx.memory(|m| m.any_popup_open())
        {
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
        // `Esc` cancels (the in-progress gesture and the whole session), `Enter`
        // applies (commits the pending stroke) and leaves.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.retouch_cancel();
        } else if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.retouch_toggle();
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
        // Defaults restored on a handle double-click.
        let defaults = BrushSettings::default();
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
                // Fixed numeric-field width. With the icon fallback font the
                // value text grows by 2px at the 2->3 digit boundary
                // (`99px` -> `100px`), which would widen the whole (centered)
                // bar; while dragging the Size slider that re-centres the bar
                // under the cursor and the panel jitters back and forth. A wider
                // `interact_size.x` keeps both DragValues a constant width.
                ui.spacing_mut().interact_size.x = 48.0;
                // Reserve the background shape and fill it only after the content
                // is laid out, so the plaque hugs the controls exactly.
                let bg = ui.painter().add(egui::Shape::Noop);
                // A plain `horizontal` row is only `interact_size.y` (18px) tall,
                // so the taller buttons make it grow and the short widgets end up
                // ~3px high. Allocate the row at the button height instead, while
                // leaving `interact_size.y` at its default so the sliders keep
                // their normal (18px) handle size.
                let mut row_size = ui.available_size_before_wrap();
                row_size.y = ACTION_BTN.y;
                let content = ui.allocate_ui_with_layout(
                    row_size,
                    egui::Layout::left_to_right(egui::Align::Center),
                    |child| {
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
                    // Double-clicking the handle snaps back to the default size.
                    reset_slider(
                        child,
                        &mut self.retouch.brush.size,
                        4.0..=400.0,
                        defaults.size,
                        slider_w,
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
                    reset_slider(
                        child,
                        &mut self.retouch.brush.hardness,
                        0.0..=1.0,
                        defaults.hardness,
                        slider_w,
                    );

                    // Actions, matching the crop bar: `Reset` clears every heal
                    // on this image (one undo step), `Done` leaves the tool. The
                    // `[`/`]`/`Esc` hotkeys still work, they are just no longer
                    // advertised with keycaps.
                    child.add_space(12.0);
                    v_separator(child);
                    child.add_space(12.0);

                    // Both buttons share one width so they read as a pair.
                    if child
                        .add_enabled(
                            !self.retouch.layer.is_empty(),
                            egui::Button::new(
                                egui::RichText::new(format!(
                                    "{} Reset",
                                    ph::ARROWS_CLOCKWISE
                                ))
                                .size(12.0),
                            )
                            .min_size(ACTION_BTN),
                        )
                        .on_hover_text("Reset all heal edits on this image")
                        .clicked()
                    {
                        self.retouch_reset();
                    }
                    if child
                        .add_sized(
                            ACTION_BTN,
                            egui::Button::new(
                                egui::RichText::new(format!("{} Done", ph::CHECK)).size(12.0),
                            )
                            .fill(theme::ACCENT),
                        )
                        .on_hover_text("Keep the heals and leave (Enter). Esc discards them.")
                        .clicked()
                    {
                        self.retouch_toggle();
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

/// A slider (no value text, accent trailing fill) that snaps back to `default`
/// when its **handle** is double-clicked.
///
/// `Slider` senses drags only, so click sensing is mixed in with
/// `Response::interact`; the pointer must additionally fall on the handle
/// circle, not just anywhere on the rail.
fn reset_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    width: f32,
) {
    let min = *range.start();
    let max = *range.end();
    ui.spacing_mut().slider_width = width;
    let response = ui.add(
        egui::Slider::new(value, range)
            .show_value(false)
            .trailing_fill(true),
    );
    // `interact` builds a new `Response`, but the slider already wrote the value
    // in place, so only double-click detection is needed here.
    let response = response.interact(egui::Sense::click());
    if response.double_clicked() {
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            // Mirror egui's slider handle geometry (see slider.rs): the handle
            // centre sits on the shrunk track at the value's position.
            let handle_r = response.rect.height() / 2.5;
            let track = response.rect.x_range().shrink(handle_r);
            let t = if max > min {
                ((*value - min) / (max - min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let center = egui::pos2(egui::lerp(track, t), response.rect.center().y);
            if p.distance(center) <= handle_r + 4.0 {
                *value = default;
            }
        }
    }
}

/// A short vertical separator between the sliders and the action buttons.
fn v_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 18.0), egui::Sense::hover());
    let x = rect.center().x;
    ui.painter().vline(
        x,
        rect.top()..=rect.bottom(),
        egui::Stroke::new(1.0, theme::DIVIDER),
    );
}
