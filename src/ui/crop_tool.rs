// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Crop tool UI: the crop view on the canvas (image at fit + dark surround +
//! thirds grid + handles), its pointer handling, and the floating options bar.
//!
//! All geometry lives in `src/crop/`; this file only maps normalized rectangles
//! to screen space and back.

use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::crop::CropDrag;
use crate::app::TinyLumaApp;
use crate::crop::orient::rotate_point;
use crate::crop::{geom, Handle, NormRect, Orientation};
use crate::theme;
use super::notification::ToastKind;

/// All eight handles, in draw/hit order.
const HANDLES: [Handle; 8] = [
    Handle::Nw,
    Handle::Ne,
    Handle::Sw,
    Handle::Se,
    Handle::N,
    Handle::S,
    Handle::W,
    Handle::E,
];

/// Grab radius of a handle, in screen pixels.
const GRAB: f32 = 13.0;

/// The crop view fits the image into this fraction of the free area, so the
/// frame keeps a margin and every handle is easy to catch.
const CROP_FIT: f32 = 0.90;
/// Space kept clear for the floating bars (options bar on top, toolbar at the
/// bottom) so the crop handles never sit under them.
const CROP_BAR_INSET: f32 = 56.0;
/// Size of an icon button in the crop options bar — a comfortable click target.
const BAR_BTN: egui::Vec2 = egui::vec2(28.0, 24.0);

/// A grouped icon button in the crop options bar: framed (so it has a hover
/// highlight) and sized for easy clicking.
fn bar_icon_button(ui: &mut egui::Ui, icon: &str, enabled: bool, tooltip: &str) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(icon).size(14.0)).min_size(BAR_BTN),
    )
    .on_hover_text(tooltip)
    .clicked()
}

/// A thin vertical divider between logical groups of the options bar.
fn bar_separator(ui: &mut egui::Ui) {
    ui.add_space(7.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 20.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top()..=rect.bottom(),
        egui::Stroke::new(1.0, theme::DIVIDER),
    );
    ui.add_space(7.0);
}

impl TinyLumaApp {
    /// Draws the whole crop view: the image at fit, the crop overlay and the
    /// floating bars. Replaces the normal image viewport while the tool is on.
    pub(crate) fn draw_crop_view(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tex_id: egui::TextureId,
        tex_size: egui::Vec2,
    ) {
        let viewport_rect = ui.available_rect_before_wrap();
        // Register the canvas region so egui routes pointer input here (the crop
        // gesture itself is read from the raw pointer state below).
        let _response = ui.interact(
            viewport_rect,
            ui.id().with("crop_viewport"),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(viewport_rect);

        self.show_crop_options(ctx, viewport_rect);

        // Crop tool shows the whole image (rotated on the GPU).
        let (tex_size, visible, orientation, angle) = self.crop_display_state(tex_size);

        // Image at fit (zoom/pan were reset on entering the tool). Fit at 90% of
        // the area between the top options bar and the bottom toolbar, so the
        // frame keeps a margin and its handles stay clear of the bars.
        let top = viewport_rect.top() + CROP_BAR_INSET;
        let bottom = (viewport_rect.bottom() - CROP_BAR_INSET).max(top);
        let area = egui::Rect::from_min_max(
            egui::pos2(viewport_rect.left(), top),
            egui::pos2(viewport_rect.right(), bottom),
        );
        let fit = egui::vec2(area.width() * CROP_FIT, area.height() * CROP_FIT);
        let base_scale = (fit.x / tex_size.x).min(fit.y / tex_size.y).max(0.0);

        // Zoom/pan are shared with the normal viewport (`zoom_scale` /
        // `pan_offset`). Entering the tool resets them, so the image still snaps
        // to the fit position; from there the mouse wheel zooms under the cursor.
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let over_overlay = hover_pos.map_or(false, |p| {
            self.crop_options_rect.map_or(false, |r| r.contains(p))
                || self.toolbar_rect.map_or(false, |r| r.expand(2.0).contains(p))
                || self.tools_rect.map_or(false, |r| r.contains(p))
        });
        let pointer_over = hover_pos.map_or(false, |p| viewport_rect.contains(p));
        if pointer_over && !over_overlay {
            let mut zoom_factor = ui.input(|i| i.zoom_delta());
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_y != 0.0 {
                zoom_factor *= (scroll_y * 0.0025).exp();
            }
            if (zoom_factor - 1.0).abs() > f32::EPSILON {
                let old_zoom = self.zoom_scale;
                let new_zoom = (old_zoom * zoom_factor).clamp(0.1, 5.0);
                if (new_zoom - old_zoom).abs() > f32::EPSILON {
                    let center = area.center();
                    let anchor = hover_pos.unwrap_or(center);
                    let k = new_zoom / old_zoom;
                    // Keep the point under the cursor in place.
                    self.pan_offset = (anchor - center) - ((anchor - center) - self.pan_offset) * k;
                    self.set_zoom(new_zoom);
                }
            }
        }

        let display_size = tex_size * base_scale * self.zoom_scale;
        let max_pan = super::center_panel::max_pan_offset(display_size, area.size());
        self.pan_offset.x = self.pan_offset.x.clamp(-max_pan.x, max_pan.x);
        self.pan_offset.y = self.pan_offset.y.clamp(-max_pan.y, max_pan.y);
        let image_rect =
            egui::Rect::from_center_size(area.center() + self.pan_offset, display_size);

        // Canvas dot grid on the letterbox (the "infinite canvas" background),
        // drawn under the image — same look as the normal viewport, scaling and
        // panning with the zoom so the background feels attached to the photo.
        {
            const BASE_STEP: f32 = 22.0;
            const MIN_STEP: f32 = 10.0;
            const FADE_STEP: f32 = 16.0;
            let dot_step = BASE_STEP * self.zoom_scale;
            let fade = ((dot_step - MIN_STEP) / (FADE_STEP - MIN_STEP)).clamp(0.0, 1.0);
            if fade > 0.0 {
                super::widgets::dot_grid(
                    &painter,
                    viewport_rect,
                    image_rect,
                    area.center() + self.pan_offset,
                    dot_step,
                    1.0,
                    theme::CANVAS_DOT.gamma_multiply(fade),
                );
            }
        }

        draw_transformed(
            &painter,
            tex_id,
            image_rect,
            visible,
            orientation,
            angle,
            self.crop_image_ratio(),
            Some(image_rect),
        );

        self.draw_crop_overlay(&painter, image_rect);

        self.crop_canvas_input(ui, hover_pos, image_rect, over_overlay);

        // The bottom toolbar stays available (undo/redo, frame nav).
        self.show_toolbar_overlay(ctx, viewport_rect);
    }

    /// Screen rect of the crop frame for the current image rect.
    fn crop_screen_rect(&self, image_rect: egui::Rect) -> egui::Rect {
        let r = self.crop.crop.rect;
        egui::Rect::from_min_max(
            to_screen(image_rect, (r.x, r.y)),
            to_screen(image_rect, (r.x + r.w, r.y + r.h)),
        )
    }

    /// Dark surround, thirds grid, frame border and handles.
    fn draw_crop_overlay(&self, painter: &egui::Painter, image_rect: egui::Rect) {
        let crop = self.crop_screen_rect(image_rect);
        let dim = egui::Color32::from_black_alpha(150);

        // Dim everything outside the frame (four bands, so the frame stays clear).
        let top = egui::Rect::from_min_max(
            image_rect.min,
            egui::pos2(image_rect.right(), crop.top()),
        );
        let bottom = egui::Rect::from_min_max(
            egui::pos2(image_rect.left(), crop.bottom()),
            image_rect.max,
        );
        let left = egui::Rect::from_min_max(
            egui::pos2(image_rect.left(), crop.top()),
            egui::pos2(crop.left(), crop.bottom()),
        );
        let right = egui::Rect::from_min_max(
            egui::pos2(crop.right(), crop.top()),
            egui::pos2(image_rect.right(), crop.bottom()),
        );
        for band in [top, bottom, left, right] {
            if band.width() > 0.0 && band.height() > 0.0 {
                painter.rect_filled(band, egui::Rounding::ZERO, dim);
            }
        }

        // Rule-of-thirds grid inside the frame.
        let grid = egui::Color32::from_white_alpha(70);
        for k in 1..3 {
            let t = k as f32 / 3.0;
            let x = egui::lerp(crop.left()..=crop.right(), t);
            let y = egui::lerp(crop.top()..=crop.bottom(), t);
            painter.vline(x, crop.top()..=crop.bottom(), egui::Stroke::new(1.0, grid));
            painter.hline(crop.left()..=crop.right(), y, egui::Stroke::new(1.0, grid));
        }

        // Frame border — a dark outline under a light line so it reads on any photo.
        painter.rect_stroke(
            crop,
            egui::Rounding::ZERO,
            egui::Stroke::new(2.5, egui::Color32::from_black_alpha(120)),
        );
        painter.rect_stroke(
            crop,
            egui::Rounding::ZERO,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        );

        // Handles.
        for h in HANDLES {
            let p = handle_pos(crop, h);
            painter.circle_filled(p, 5.0, egui::Color32::from_black_alpha(140));
            painter.circle_filled(p, 3.5, egui::Color32::WHITE);
        }
    }

    /// Pointer handling for the crop view: pan (space+drag or middle mouse) and
    /// the crop-frame drag gesture.
    ///
    /// Uses the raw pointer state (`primary_pressed/down/released`) rather than
    /// egui's drag events — the same proven pattern as the retouch tool. That
    /// way the gesture always ends (and its history entry is committed) even if
    /// the release happens over the options bar or the toolbar.
    fn crop_canvas_input(
        &mut self,
        ui: &mut egui::Ui,
        hover_pos: Option<egui::Pos2>,
        image_rect: egui::Rect,
        over_overlay: bool,
    ) {
        // Guard against a collapsed viewport (minimized window): no mapping possible.
        if image_rect.width() < 1.0 || image_rect.height() < 1.0 {
            return;
        }
        let crop = self.crop_screen_rect(image_rect);

        let (pressed, down, released, middle_pressed, middle_down, space_held) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.button_pressed(egui::PointerButton::Middle),
                i.pointer.button_down(egui::PointerButton::Middle),
                i.key_down(egui::Key::Space),
            )
        });
        let pointer = ui.input(|i| i.pointer.interact_pos()).or(hover_pos);
        let pointer_delta = ui.input(|i| i.pointer.delta());

        // --- PAN (space + left drag, or middle-button drag) ---
        let pan_held = middle_down || (space_held && down);
        if !self.is_panning
            && !self.crop.dragging
            && !over_overlay
            && (middle_pressed || (pressed && space_held))
        {
            self.is_panning = true;
        }
        if self.is_panning {
            self.pan_offset += pointer_delta;
            ui.ctx().request_repaint();
            if !pan_held {
                self.is_panning = false;
            }
        }

        if pressed && !over_overlay && !self.is_panning && !space_held {
            if let Some(p) = pointer {
                if image_rect.expand(GRAB).contains(p) {
                    let n = to_norm(image_rect, p);
                    self.crop_begin_gesture();
                    self.crop.drag = Some(match hit_handle(crop, p) {
                        Some(handle) => CropDrag::Resize {
                            handle,
                            start: self.crop.crop.rect,
                        },
                        None => CropDrag::Move {
                            start: self.crop.crop.rect,
                            grab: n,
                        },
                    });
                }
            }
        }

        if down && !self.is_panning {
            if let (Some(drag), Some(p)) = (self.crop.drag, pointer) {
                let n = to_norm(image_rect, p);
                let ar = self.crop.crop.norm_ratio(self.crop_image_ratio());
                self.crop.crop.rect = match drag {
                    CropDrag::Resize { handle, start } => {
                        geom::resize_rect(start, handle, n, ar)
                    }
                    CropDrag::Move { start, grab } => {
                        geom::move_rect(start, n.0 - grab.0, n.1 - grab.1)
                    }
                };
                ui.ctx().request_repaint();
            }
        }

        // Commit on release — or if the button is no longer down but the gesture
        // is still marked as active (e.g. the release was swallowed elsewhere).
        if released || (!down && self.crop.dragging) {
            self.crop_end_gesture();
        }

        // Cursor feedback: panning / pan-ready / resize handles.
        if self.is_panning {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if space_held && !over_overlay {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if !self.crop.dragging && !over_overlay {
            if let Some(p) = hover_pos {
                if let Some(h) = hit_handle(crop, p) {
                    let icon = match h {
                        Handle::Nw | Handle::Se => egui::CursorIcon::ResizeNwSe,
                        Handle::Ne | Handle::Sw => egui::CursorIcon::ResizeNeSw,
                        Handle::N | Handle::S => egui::CursorIcon::ResizeVertical,
                        Handle::W | Handle::E => egui::CursorIcon::ResizeHorizontal,
                    };
                    ui.ctx().set_cursor_icon(icon);
                }
            }
        }
    }

    /// Floating crop options bar (top-center of the canvas), grouped into two
    /// rows: framing (ratio, size, actions) and transform (rotate/flip) +
    /// AI utilities (snap ×64, trim).
    pub(crate) fn show_crop_options(&mut self, ctx: &egui::Context, viewport_rect: egui::Rect) {
        if !self.crop.active {
            self.crop_options_rect = None;
            return;
        }
        let offset_x = viewport_rect.center().x - ctx.screen_rect().center().x;

        egui::Area::new(egui::Id::new("crop_options"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(offset_x, viewport_rect.top() + 14.0),
            )
            .show(ctx, |ui| {
                let bg = ui.painter().add(egui::Shape::Noop);
                let content = ui.vertical(|ui| {
                    // ===== Row 1: framing =====
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(ph::CROP)
                                .size(15.0)
                                .color(theme::ACCENT),
                        );
                        ui.label(egui::RichText::new("Crop").size(12.0).color(theme::TEXT));
                        bar_separator(ui);

                        // Aspect ratio (both orientations live in the list).
                        egui::ComboBox::from_id_source("crop_ratio")
                            .selected_text(self.crop.crop.preset.label())
                            .width(84.0)
                            .show_ui(ui, |ui| {
                                for &preset in crate::crop::RATIO_PRESETS {
                                    if ui
                                        .selectable_label(
                                            self.crop.crop.preset == preset,
                                            preset.label(),
                                        )
                                        .clicked()
                                    {
                                        self.crop_set_preset(preset);
                                    }
                                }
                            });

                        // Output size (fixed width, no jitter while dragging).
                        let (iw, ih) = self.crop_image_size();
                        let (_, _, pw, ph) = self.crop.crop.rect.pixel_rect(iw, ih);
                        ui.add_sized(
                            egui::vec2(96.0, BAR_BTN.y),
                            egui::Label::new(
                                egui::RichText::new(format!("{pw} × {ph} px"))
                                    .size(12.0)
                                    .monospace()
                                    .color(theme::TEXT_SECONDARY),
                            )
                            .selectable(false),
                        );
                        bar_separator(ui);

                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("{} Reset", ph::ARROWS_CLOCKWISE))
                                        .size(12.0),
                                )
                                .min_size(egui::vec2(72.0, BAR_BTN.y)),
                            )
                            .on_hover_text("Reset all crop edits (frame, rotation, flip, straighten)")
                            .clicked()
                        {
                            self.crop_reset();
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("{} Done", ph::CHECK)).size(12.0),
                                )
                                .min_size(egui::vec2(66.0, BAR_BTN.y))
                                .fill(theme::ACCENT),
                            )
                            .on_hover_text("Leave the crop tool (Esc)")
                            .clicked()
                        {
                            self.crop_toggle();
                        }
                    });

                    ui.add_space(4.0);

                    // ===== Row 2: transform + AI utilities =====
                    ui.horizontal(|ui| {
                        // Transform — the crop frame follows the content.
                        if bar_icon_button(
                            ui,
                            ph::ARROW_COUNTER_CLOCKWISE,
                            true,
                            "Rotate 90° left",
                        ) {
                            self.crop_rotate_ccw();
                        }
                        if bar_icon_button(ui, ph::ARROW_CLOCKWISE, true, "Rotate 90° right") {
                            self.crop_rotate_cw();
                        }
                        if bar_icon_button(ui, ph::FLIP_HORIZONTAL, true, "Flip horizontally") {
                            self.crop_flip_h();
                        }
                        if bar_icon_button(ui, ph::FLIP_VERTICAL, true, "Flip vertically") {
                            self.crop_flip_v();
                        }
                        bar_separator(ui);

                        // Straighten: a manual degree slider + auto-detect.
                        ui.label(
                            egui::RichText::new("Straighten")
                                .size(12.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.spacing_mut().slider_width = 130.0;
                        let mut angle = self.crop.crop.angle;
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut angle,
                                    -crate::crop::MAX_ANGLE..=crate::crop::MAX_ANGLE,
                                )
                                .show_value(false)
                                .trailing_fill(true),
                            )
                            .on_hover_text("Straighten by degrees (the frame auto-fits)")
                            .changed()
                        {
                            self.crop_set_angle(angle);
                        }
                        ui.add_sized(
                            egui::vec2(48.0, BAR_BTN.y),
                            egui::Label::new(
                                egui::RichText::new(format!("{:+.1}°", self.crop.crop.angle))
                                    .size(11.0)
                                    .monospace()
                                    .color(theme::TEXT_SECONDARY),
                            )
                            .selectable(false),
                        );
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("Auto").size(12.0))
                                    .min_size(egui::vec2(50.0, BAR_BTN.y)),
                            )
                            .on_hover_text("Auto-detect the tilt and level it")
                            .clicked()
                        {
                            let a = self.crop_auto_straighten();
                            if a.abs() < 0.05 {
                                self.notify(ToastKind::Info, "Already level");
                            }
                        }
                    });
                });

                let rect = content.response.rect.expand2(egui::vec2(12.0, 8.0));
                self.crop_options_rect = Some(rect);
                let rounding = egui::Rounding::same(theme::RADIUS_MD);
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
}

/// Maps a normalized point to screen space.
fn to_screen(image_rect: egui::Rect, n: (f32, f32)) -> egui::Pos2 {
    egui::pos2(
        image_rect.left() + n.0 * image_rect.width(),
        image_rect.top() + n.1 * image_rect.height(),
    )
}

/// Maps a screen point to normalized image space (not clamped — the geometry
/// layer clamps the result).
fn to_norm(image_rect: egui::Rect, p: egui::Pos2) -> (f32, f32) {
    (
        (p.x - image_rect.left()) / image_rect.width(),
        (p.y - image_rect.top()) / image_rect.height(),
    )
}

/// Screen position of a handle.
fn handle_pos(rect: egui::Rect, h: Handle) -> egui::Pos2 {
    let cx = rect.center().x;
    let cy = rect.center().y;
    match h {
        Handle::N => egui::pos2(cx, rect.top()),
        Handle::S => egui::pos2(cx, rect.bottom()),
        Handle::W => egui::pos2(rect.left(), cy),
        Handle::E => egui::pos2(rect.right(), cy),
        Handle::Nw => rect.min,
        Handle::Ne => egui::pos2(rect.right(), rect.top()),
        Handle::Sw => egui::pos2(rect.left(), rect.bottom()),
        Handle::Se => rect.max,
    }
}

/// The handle nearest to `p`, if within `GRAB` pixels.
fn hit_handle(rect: egui::Rect, p: egui::Pos2) -> Option<Handle> {
    let mut best: Option<(f32, Handle)> = None;
    for h in HANDLES {
        let d = (handle_pos(rect, h) - p).length();
        if d < GRAB && best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, h));
        }
    }
    best.map(|(_, h)| h)
}

/// Draws the image into `screen`, applying the orientation and the straighten
/// angle as a rotated quad mesh (GPU only — no CPU resample, so the straighten
/// slider stays smooth). `visible` is the region of the *rotated* image space
/// that maps onto `screen`; `aspect` is the oriented image's `width / height`,
/// so the rotation happens in physical space (rotating the normalized unit
/// square instead would shear a non-square image). `clip` optionally limits the
/// painted area.
pub(crate) fn draw_transformed(
    painter: &egui::Painter,
    tex_id: egui::TextureId,
    screen: egui::Rect,
    visible: NormRect,
    orientation: Orientation,
    angle: f32,
    aspect: f32,
    clip: Option<egui::Rect>,
) {
    if screen.width() <= 0.0 || screen.height() <= 0.0 || visible.w <= 0.0 || visible.h <= 0.0 {
        return;
    }
    // Corners of the oriented image, before the fine rotation.
    let corners = [
        (0.0f32, 0.0f32),
        (1.0f32, 0.0f32),
        (1.0f32, 1.0f32),
        (0.0f32, 1.0f32),
    ];
    let mut mesh = egui::Mesh::with_texture(tex_id);
    for (x, y) in corners {
        let (rx, ry) = rotate_point(x, y, angle, aspect);
        let sx = screen.left() + (rx - visible.x) / visible.w * screen.width();
        let sy = screen.top() + (ry - visible.y) / visible.h * screen.height();
        let (u, v) = orientation.to_texture(x, y);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(sx, sy),
            uv: egui::pos2(u, v),
            color: egui::Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);

    let painter = match clip {
        Some(c) => painter.with_clip_rect(c.intersect(painter.clip_rect())),
        None => painter.clone(),
    };
    painter.add(egui::Shape::mesh(mesh));
}
