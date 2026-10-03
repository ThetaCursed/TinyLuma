// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;

// ============================================================
// Filmstrip settings — tweak these numbers to taste.
// ============================================================
/// Thumbnail height in the filmstrip (px). The width is computed from the aspect.
pub(crate) const THUMB_HEIGHT: f32 = 120.0;
/// Vertical margin of the filmstrip "tray" above and below the thumbnails (px).
pub(crate) const FILMSTRIP_PAD: f32 = 6.0;
/// Thumbnail corner radius. A thumbnail is a card, so we take
/// RADIUS_LG: the filmstrip looks part of the overall design, not "glued on".
pub(crate) const THUMB_ROUNDING: f32 = theme::RADIUS_LG;
/// Stroke width of the current (selected) thumbnail. A thin accent line
/// reads neater than a thick frame and does not fight with the image.
pub(crate) const THUMB_STROKE_CURRENT: f32 = 2.0;
/// Stroke width on hover over an unselected thumbnail — a thin hairline.
pub(crate) const THUMB_STROKE_NORMAL: f32 = 1.0;

/// Diameter of the round "remove from session" button.
const CLOSE_DIAMETER: f32 = 18.0;
const CLOSE_RADIUS: f32 = CLOSE_DIAMETER * 0.5;
/// Offset of the button center from the top and right edges of the thumbnail. Chosen so
/// that the circle lies entirely inside the rounded corner and is not cut by it.
const CLOSE_INSET: f32 = 14.0;
/// Size of the cross icon (Phosphor), px. An icon instead of hand-made lines —
/// the same visual language as the panel buttons.
const CLOSE_ICON_SIZE: f32 = 9.0;

impl TinyLumaApp {
    /// Horizontal thumbnail filmstrip. Shown only when there is >1 image.
    /// Clicking a thumbnail switches the image; the current one is outlined,
    /// modified ones are marked with a dot.
    pub(crate) fn show_filmstrip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let thumb_h = THUMB_HEIGHT;

        // Total filmstrip width: needed to center the thumbnails when
        // they fit entirely in the window. When overflowing — normal left-aligned scrolling.
        let count = self.session.image_list.len();
        let mut content_w = 0.0f32;
        for path in &self.session.image_list {
            content_w += match self.thumbnails.get(path) {
                Some(t) => {
                    let s = t.size_vec2();
                    (s.x / s.y) * thumb_h
                }
                None => thumb_h,
            };
        }
        if count > 1 {
            content_w += 8.0 * (count - 1) as f32;
        }
        let side_pad = ((ui.available_width() - content_w) / 2.0).max(0.0);

        let mut clicked: Option<usize> = None;
        let mut remove_index: Option<usize> = None;
        // Auto-scroll to the current image is performed once after switching,
        // so as not to interfere with the user scrolling the filmstrip manually.
        let mut do_scroll = self.filmstrip_needs_scroll;

        // The mouse wheel over the filmstrip scrolls it horizontally: a ScrollArea
        // has only one active direction, and egui can redirect the wheel delta there.
        let prev_only_dir = ui.style().always_scroll_the_only_direction;
        ui.style_mut().always_scroll_the_only_direction = true;

        // A neat scrollbar: thin and it does NOT balloon on hover
        // (for the floating style the width = lerp(floating_width ..= bar_width),
        //  so we make them equal). It still takes up zero space.
        let prev_scroll = ui.style().spacing.scroll;
        let mut scroll_style = prev_scroll;
        scroll_style.floating = true;
        scroll_style.floating_width = 4.0;
        scroll_style.bar_width = 4.0;
        ui.style_mut().spacing.scroll = scroll_style;

        // Breathe vertically inside the "tray": zero out item_spacing.y so
        // add_space gives exactly this top margin, with no extra gap.
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(FILMSTRIP_PAD);

        egui::ScrollArea::horizontal()
            .id_source("filmstrip_scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.add_space(side_pad);

                    for index in 0..self.session.image_list.len() {
                        let path = &self.session.image_list[index];
                        let is_current = index == self.session.current_index;
                        let is_modified = self.session.is_modified(path);
                        let texture = self.thumbnails.get(path);

                        // Size from the thumbnail aspect (or a square while there is none).
                        let size = match texture {
                            Some(t) => {
                                let s = t.size_vec2();
                                egui::vec2((s.x / s.y) * thumb_h, thumb_h)
                            }
                            None => egui::vec2(thumb_h, thumb_h),
                        };

                        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                        let painter = ui.painter_at(rect);

                        // Placeholder background (while the thumbnail is not ready) — with rounding,
                        // so the corners are not "cut off" when the frame appears. Lighter than
                        // the dark filmstrip "tray", otherwise the card would be invisible.
                        painter.rect_filled(rect, THUMB_ROUNDING, theme::BG_SURFACE);

                        // Image with rounded corners. `painter.image` can
                        // only draw a rectangle, so we use a
                        // textured RectShape — it supports rounding.
                        if let Some(t) = texture {
                            painter.add(egui::Shape::Rect(egui::epaint::RectShape {
                                rect,
                                rounding: egui::Rounding::same(THUMB_ROUNDING),
                                fill: egui::Color32::WHITE,
                                stroke: egui::Stroke::NONE,
                                blur_width: 0.0,
                                fill_texture_id: t.id(),
                                uv: egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                            }));
                        }

                        // Frame: accent only on the current one. The others have no
                        // frame (the dark "tray" and the gap separate the cards themselves),
                        // and on hover a thin light one appears — for feedback.
                        // Draw slightly INSIDE the edge (`shrink`): the painter is clipped to
                        // `rect`, and a stroke exactly on the boundary would lose its outer
                        // half — the line would look flat and sloppy.
                        if is_current {
                            const INSET: f32 = 1.0;
                            painter.rect_stroke(
                                rect.shrink(INSET),
                                egui::Rounding::same(THUMB_ROUNDING - INSET),
                                egui::Stroke::new(THUMB_STROKE_CURRENT, theme::ACCENT),
                            );
                        } else if resp.hovered() {
                            const INSET: f32 = 0.5;
                            painter.rect_stroke(
                                rect.shrink(INSET),
                                egui::Rounding::same(THUMB_ROUNDING - INSET),
                                egui::Stroke::new(THUMB_STROKE_NORMAL, theme::BORDER),
                            );
                        }

                        // "Modified" indicator: an accent dot in the LEFT top
                        // corner (the right one is taken by the remove cross).
                        if is_modified {
                            super::widgets::modified_dot(
                                &painter,
                                rect.left_top() + egui::vec2(9.0, 9.0),
                            );
                        }

                        // The "remove from session" cross zone (top-right corner).
                        // We do NOT create a separate widget: it competed for hover
                        // with the thumbnail — the cross flickered and could not be clicked. Instead
                        // we check the click position on the thumbnail itself.
                        let close_center =
                            egui::pos2(rect.right() - CLOSE_INSET, rect.top() + CLOSE_INSET);
                        let close_rect = egui::Rect::from_center_size(
                            close_center,
                            egui::vec2(CLOSE_DIAMETER, CLOSE_DIAMETER),
                        );
                        let hover_pos = ui.input(|i| i.pointer.hover_pos());
                        let pointer_over_close =
                            hover_pos.map_or(false, |p| close_rect.contains(p));

                        // The cross appears when hovering over the thumbnail. It lies
                        // ON TOP of the accent frame: the almost opaque backing
                        // hides it underneath, and a thin light rim separates
                        // the button from the image. This way the cross reads as a separate
                        // element, not as a "hole" in the selection frame.
                        if resp.hovered() {
                            let danger = pointer_over_close;
                            let fill = if danger {
                                theme::DANGER
                            } else {
                                theme::OVERLAY_DARK
                            };
                            let rim = if danger {
                                theme::with_alpha(egui::Color32::WHITE, 35)
                            } else {
                                theme::OVERLAY_RIM
                            };
                            let glyph = if danger {
                                egui::Color32::WHITE
                            } else {
                                theme::TEXT
                            };

                            painter.circle_filled(close_center, CLOSE_RADIUS, fill);
                            painter.circle_stroke(
                                close_center,
                                CLOSE_RADIUS,
                                egui::Stroke::new(1.0, rim),
                            );
                            painter.text(
                                close_center,
                                egui::Align2::CENTER_CENTER,
                                ph::X,
                                egui::FontId::proportional(CLOSE_ICON_SIZE),
                                glyph,
                            );
                        }

                        if resp.clicked() {
                            // A click in the cross zone — remove, otherwise open the frame.
                            let in_close = hover_pos.map_or(false, |p| close_rect.contains(p));
                            if in_close {
                                remove_index = Some(index);
                            } else {
                                clicked = Some(index);
                            }
                        }
                        if let Some(name) = path.file_name() {
                            resp.on_hover_text(name.to_string_lossy());
                        }

                        // Auto-scroll to the current one.
                        if is_current && do_scroll {
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                            do_scroll = false;
                        }
                    }
                });
            });

        ui.style_mut().always_scroll_the_only_direction = prev_only_dir;
        ui.style_mut().spacing.scroll = prev_scroll;

        self.filmstrip_needs_scroll = do_scroll;

        // Removal from the session takes priority over frame switching.
        if let Some(index) = remove_index {
            self.remove_from_session(index, ctx);
            return;
        }

        if let Some(index) = clicked {
            if index != self.session.current_index {
                self.switch_to_image(index, ctx);
            }
        }
    }
}
