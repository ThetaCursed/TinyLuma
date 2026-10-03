// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;

/// Icon-button sizes (undo/redo, before/split).
const ICON_W: f32 = 26.0;
/// Zoom buttons: "−", the "Fit"/% label, "+".
const ZOOM_BTN_W: f32 = 22.0;
const ZOOM_LABEL_W: f32 = 44.0;
/// Height of the zoom-number plaque — less than the row so it does not collide with the pill.
const ZOOM_LABEL_H: f32 = 16.0;
/// Frame navigation arrows ‹ / ›.
const ARROW_W: f32 = 12.0;
/// Gap between buttons within one pair.
const INNER_GAP: f32 = 2.0;
/// Free space on each side of the divider.
const DIVIDER_PAD: f32 = 9.0;
/// Inner padding of the bar.
const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 5.0;
/// Margin of the bar from the bottom edge of the viewport.
const BOTTOM_MARGIN: f32 = 8.0;
/// Margins of the before/after plaque from the canvas's top-left corner.
const EDGE_MARGIN: f32 = 12.0;
const TOP_MARGIN: f32 = 12.0;

impl TinyLumaApp {
    /// A single toolbar bar at the bottom of the photo, centered:
    ///
    /// ```text
    /// [⟲][⟳] [👁][⇄] │ [‹] N / M [›] │ [−][Fit][+]
    /// ```
    ///
    /// Left of the frame index are history and before/after, right is zoom; the groups
    /// are separated by vertical lines. A floating overlay: it takes no space
    /// in the layout, so the image gets the full height.
    /// `Middle` layer: above the canvas (Background), but below the dialog windows.
    pub(crate) fn show_toolbar_overlay(
        &mut self,
        ctx: &egui::Context,
        viewport_rect: egui::Rect,
    ) {
        let row_h = ctx.style().spacing.interact_size.y.max(24.0);
        let total = self.session.total();
        let has_index = total > 1;

        // The index width follows the widest string (total/total) so it
        // does not jitter when going 9/230 → 10/230.
        let index_w = if has_index {
            let widest = format!("{} / {}", total, total);
            let text_w = ctx.fonts(|f| {
                f.layout_no_wrap(
                    widest,
                    egui::FontId::proportional(12.0),
                    egui::Color32::PLACEHOLDER,
                )
                .size()
                .x
            });
            (text_w + 5.0).max(36.0)
        } else {
            0.0
        };

        // --- Geometry ---
        // Make all three groups the same width: the dividers then sit
        // symmetrically, and the index automatically ends up centered in the bar.
        let divider_w = 1.0 + DIVIDER_PAD * 2.0;
        let hist_w = ICON_W * 2.0 + INNER_GAP;
        let index_group_w = if has_index {
            ARROW_W * 2.0 + index_w + INNER_GAP * 2.0
        } else {
            0.0
        };
        let zoom_w = ZOOM_BTN_W * 2.0 + ZOOM_LABEL_W + INNER_GAP * 2.0;
        // The width of one cell follows the widest group.
        let group_w = hist_w.max(index_group_w).max(zoom_w);
        let group_count = if has_index { 3.0 } else { 2.0 };
        let divider_count = group_count - 1.0;

        let bar_w = group_w * group_count + divider_w * divider_count + PAD_X * 2.0;
        let bar_h = row_h + PAD_Y * 2.0;

        // The cells are equal in width → the center of the middle (index) cell coincides with
        // the center of the bar, so centering the bar on the panel is enough.
        let bar_left = viewport_rect.center().x - bar_w * 0.5;

        let pos = egui::pos2(
            bar_left,
            viewport_rect.bottom() - BOTTOM_MARGIN - bar_h,
        );

        egui::Area::new(egui::Id::new("toolbar_overlay"))
            .order(egui::Order::Middle)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_min_width(bar_w);
                ui.set_max_width(bar_w);

                let full = ui.available_rect_before_wrap();
                let pill_rect = egui::Rect::from_min_size(full.min, egui::vec2(bar_w, bar_h));
                super::widgets::plaque(ui.painter(), pill_rect);

                let top = full.top() + PAD_Y;
                let base = full.left() + PAD_X;

                let btn = |x: f32, w: f32| {
                    egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(w, row_h))
                };

                // 1. History (undo/redo) — content centered in its cell.
                let mut gx = base;
                let hx = gx + (group_w - hist_w) * 0.5;
                let undo_rect = btn(hx, ICON_W);
                let redo_rect = btn(hx + ICON_W + INNER_GAP, ICON_W);
                gx += group_w;

                // Divider before the index.
                let sep1_x = gx + DIVIDER_PAD + 0.5;
                gx += divider_w;

                // 2. Frame index ‹ N / M › — centered in its cell.
                let mut prev_rect = None;
                let mut index_rect = None;
                let mut next_rect = None;
                if has_index {
                    let ix = gx + (group_w - index_group_w) * 0.5;
                    let prev = btn(ix, ARROW_W);
                    let index =
                        btn(ix + ARROW_W + INNER_GAP, index_w);
                    let next = btn(
                        ix + ARROW_W + INNER_GAP + index_w + INNER_GAP,
                        ARROW_W,
                    );
                    prev_rect = Some(prev);
                    index_rect = Some(index);
                    next_rect = Some(next);
                    gx += group_w;

                    // Divider after the index.
                    let sep2_x = gx + DIVIDER_PAD + 0.5;
                    gx += divider_w;
                    draw_toolbar_divider(ui, sep2_x, top, row_h);
                }

                draw_toolbar_divider(ui, sep1_x, top, row_h);

                // 3. Zoom (− Fit +) — centered in its cell.
                let zx = gx + (group_w - zoom_w) * 0.5;
                let minus_rect = btn(zx, ZOOM_BTN_W);
                // The number plaque — lower than the row and centered on it.
                let label_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        zx + ZOOM_BTN_W + INNER_GAP,
                        top + (row_h - ZOOM_LABEL_H) * 0.5,
                    ),
                    egui::vec2(ZOOM_LABEL_W, ZOOM_LABEL_H),
                );
                let plus_rect = btn(
                    zx + ZOOM_BTN_W + INNER_GAP + ZOOM_LABEL_W + INNER_GAP,
                    ZOOM_BTN_W,
                );

                // --- Undo / Redo ---
                let can_undo = self.history.can_undo();
                let undo_resp = ui.put(
                    undo_rect,
                    icon_button(ph::ARROW_U_UP_LEFT, 18.0, can_undo, false),
                );
                if can_undo && undo_resp.on_hover_text("Undo (Ctrl+Z)").clicked() {
                    if let Some(s) = self.history.undo(self.snapshot()) {
                        self.restore(s);
                    }
                }

                let can_redo = self.history.can_redo();
                let redo_resp = ui.put(
                    redo_rect,
                    icon_button(ph::ARROW_U_UP_RIGHT, 18.0, can_redo, false),
                );
                if can_redo && redo_resp.on_hover_text("Redo (Ctrl+Shift+Z)").clicked() {
                    if let Some(s) = self.history.redo(self.snapshot()) {
                        self.restore(s);
                    }
                }

                // --- Frame navigation (index) ---
                if let (Some(prev_rect), Some(index_rect), Some(next_rect)) =
                    (prev_rect, index_rect, next_rect)
                {
                    let can_prev = self.session.current_index > 0;
                    let prev = ui.put(
                        prev_rect,
                        icon_button(ph::CARET_LEFT, 12.0, can_prev, false),
                    );
                    if can_prev && prev.on_hover_text("Previous image (←)").clicked() {
                        self.switch_to_image(self.session.current_index - 1, ctx);
                    }

                    ui.put(
                        index_rect,
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{} / {}",
                                self.session.current_index + 1,
                                total
                            ))
                            .size(12.0)
                            .color(theme::TEXT),
                        )
                        .selectable(false),
                    );

                    let can_next = self.session.current_index + 1 < total;
                    let next = ui.put(
                        next_rect,
                        icon_button(ph::CARET_RIGHT, 12.0, can_next, false),
                    );
                    if can_next && next.on_hover_text("Next image (→)").clicked() {
                        self.switch_to_image(self.session.current_index + 1, ctx);
                    }
                }

                // --- ZOOM (− [Fit] +) ---
                let zoom = self.zoom_scale;
                if ui
                    .put(
                        minus_rect,
                        egui::Button::new(egui::RichText::new(ph::MINUS).size(15.0))
                            .frame(false),
                    )
                    .on_hover_text("Zoom out (Ctrl+-)")
                    .clicked()
                {
                    self.zoom_scale = (zoom / 1.25).clamp(0.1, 5.0);
                }

                let zoom_label = if (zoom - 1.0).abs() < 0.001 {
                    "Fit".to_string()
                } else {
                    format!("{:.0}%", zoom * 100.0)
                };

                // frame(true) — makes "Fit" look like a neat plaque button.
                if ui
                    .put(
                        label_rect,
                        egui::Button::new(
                            egui::RichText::new(zoom_label).size(11.0).monospace(),
                        )
                        .frame(true),
                    )
                    .on_hover_text("Reset zoom to fit (Ctrl+0)")
                    .clicked()
                {
                    self.zoom_scale = 1.0;
                }

                if ui
                    .put(
                        plus_rect,
                        egui::Button::new(egui::RichText::new("+").size(15.0)).frame(false),
                    )
                    .on_hover_text("Zoom in (Ctrl+=)")
                    .clicked()
                {
                    self.zoom_scale = (zoom * 1.25).clamp(0.1, 5.0);
                }
            });

        // --- Before/after (split): plaque in the canvas's top-left corner ---
        // Moved out of the bottom bar so history/index/zoom line up neatly.
        // There is no separate "show original" button — the split plays its role.
        let cmp_w = ICON_W + PAD_X * 2.0;
        let cmp_h = row_h + PAD_Y * 2.0;
        let cmp_pos = egui::pos2(
            viewport_rect.left() + EDGE_MARGIN,
            viewport_rect.top() + TOP_MARGIN,
        );
        egui::Area::new(egui::Id::new("compare_overlay"))
            .order(egui::Order::Middle)
            .fixed_pos(cmp_pos)
            .show(ctx, |ui| {
                ui.set_min_width(cmp_w);
                ui.set_max_width(cmp_w);

                let full = ui.available_rect_before_wrap();
                let pill_rect = egui::Rect::from_min_size(full.min, egui::vec2(cmp_w, cmp_h));
                super::widgets::plaque(ui.painter(), pill_rect);

                let top = full.top() + PAD_Y;
                let split_rect = egui::Rect::from_min_size(
                    egui::pos2(full.left() + PAD_X, top),
                    egui::vec2(ICON_W, row_h),
                );

                let can_compare = self.original_texture.is_some();

                let split_active = self.split_position > 0.0;
                let split_resp = ui
                    .put(
                        split_rect,
                        icon_button(
                            ph::ARROWS_LEFT_RIGHT,
                            15.0,
                            can_compare,
                            false,
                        ),
                    )
                    .on_hover_text("Toggle before/after split");
                if can_compare && split_resp.clicked() {
                    self.split_position = if split_active { 0.0 } else { 0.5 };
                    self.save_save_settings();
                }
            });
    }
}

/// Vertical divider line inside the bar.
fn draw_toolbar_divider(ui: &egui::Ui, x: f32, top: f32, row_h: f32) {
    let v_pad = (row_h - 14.0) * 0.5;
    ui.painter().vline(
        x,
        (top + v_pad)..=(top + row_h - v_pad),
        egui::Stroke::new(1.0, theme::DIVIDER),
    );
}

pub(crate) fn icon_button(
    icon: &str,
    size: f32,
    enabled: bool,
    framed: bool,
) -> egui::Button<'static> {
    let text = if enabled {
        egui::RichText::new(icon).size(size)
    } else {
        egui::RichText::new(icon)
            .size(size)
            .color(theme::TEXT_DISABLED)
    };
    let button = egui::Button::new(text).frame(framed);
    if enabled {
        button
    } else {
        button.sense(egui::Sense::hover())
    }
}

