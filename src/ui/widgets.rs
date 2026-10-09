// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;

/// Accent dot indicator "there are changes" (same look as the filmstrip).
/// Drawn centered on the given point.
pub(crate) fn modified_dot(painter: &egui::Painter, center: egui::Pos2) {
    // Dark halo: separates the dot from arbitrary thumbnail content,
    // thanks to it the blue reads on both a bright sky and a dark frame.
    painter.circle_filled(center, 5.8, theme::HALO_DARK);
    // The accent circle — noticeably larger than before.
    painter.circle_filled(center, 4.5, theme::ACCENT);
    // A thin light highlight on the edge: the dot looks more three-dimensional and "switches on"
    // in peripheral vision without pulling attention to itself.
    painter.circle_stroke(
        center,
        4.1,
        egui::Stroke::new(1.0, theme::with_alpha(egui::Color32::WHITE, 150)),
    );
}

/// The result of a choice in [`preset_dropdown`].
#[derive(Default)]
pub(crate) struct DropdownChoice {
    /// The "None" item was chosen (only if `include_none`).
    pub(crate) none: bool,
    /// A preset was chosen by index.
    pub(crate) index: Option<usize>,
}

/// Shared preset dropdown selector: a 24px-tall button (easy to hit)
/// and a list without an inner scrollbar — the whole list is visible at once. The same look
/// in the left panel and in the "Apply to All" dialog.
///
/// `dirty` draws the accent dot "there are unsaved edits" and the Save hint.
/// The caller applies the choice itself — the widget only reports what
/// was clicked in this frame.
pub(crate) fn preset_dropdown(
    ui: &mut egui::Ui,
    id_salt: &str,
    names: &[String],
    selected_index: Option<usize>,
    include_none: bool,
    dirty: bool,
) -> DropdownChoice {
    let id = egui::Id::new(id_salt);
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 24.0),
        egui::Sense::click(),
    );

    let visuals = ui.style().interact(&resp);
    ui.painter()
        .rect(rect, theme::RADIUS_SM, visuals.weak_bg_fill, visuals.bg_stroke);

    let selected_name = selected_index
        .and_then(|i| names.get(i))
        .cloned()
        .unwrap_or_default();
    let empty = selected_name.is_empty();
    let display = if empty {
        "Select a preset...".to_string()
    } else {
        selected_name
    };
    const TEXT_PAD: f32 = 8.0;
    const CHEVRON_W: f32 = 18.0;
    let text_max_w = (rect.width() - TEXT_PAD - CHEVRON_W).max(0.0);
    let mut job = egui::text::LayoutJob::single_section(
        display,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color: if empty {
                theme::TEXT_SECONDARY
            } else {
                visuals.text_color()
            },
            ..Default::default()
        },
    );
    job.wrap.max_width = text_max_w;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts(|f| f.layout_job(job));
    let text_w = galley.size().x;
    ui.painter().galley(
        egui::pos2(
            rect.left() + TEXT_PAD,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        visuals.text_color(),
    );
    ui.painter().text(
        egui::pos2(rect.right() - TEXT_PAD, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        ph::CARET_DOWN,
        egui::FontId::proportional(12.0),
        theme::TEXT_SECONDARY,
    );

    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }

    if dirty {
        // The dot is drawn as a shape (not a glyph) — the font does not "eat" it.
        // The position is right after the name, clamped before the chevron.
        let max_x = rect.right() - TEXT_PAD - CHEVRON_W;
        let dot_x = (rect.left() + TEXT_PAD + text_w + 8.0).min(max_x);
        modified_dot(ui.painter(), egui::pos2(dot_x, rect.center().y));
        let _ = resp.clone().on_hover_text(format!(
            "Unsaved changes — press {} Save",
            ph::FLOPPY_DISK
        ));
    }

    let mut choice = DropdownChoice::default();
    egui::popup_below_widget(
        ui,
        id,
        &resp,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(rect.width());
            // We do not wrap long names — we widen the popup.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let mut close = false;

            // "None" — clear the selection (only where that makes sense).
            if include_none && !names.is_empty() {
                if ui.selectable_label(selected_index.is_none(), "None").clicked() {
                    choice.none = true;
                    close = true;
                }
                ui.separator();
            }
            for (i, name) in names.iter().enumerate() {
                if ui.selectable_label(selected_index == Some(i), name).clicked() {
                    choice.index = Some(i);
                    close = true;
                }
            }
            if names.is_empty() {
                ui.weak("No saved presets");
            }
            if close {
                ui.memory_mut(|m| m.close_popup());
            }
        },
    );
    choice
}

/// A thin divider for windows/dialogs.
///
/// The window background (`BG_ELEVATED`) is almost the same brightness as `SEPARATOR`,
/// so the built-in `ui.separator()` is practically invisible in dialogs.
/// We draw a barely visible light line: it only hints at the section
/// boundary and does not fight with the content.
pub(crate) fn divider(ui: &mut egui::Ui) {
    let sep = theme::with_alpha(egui::Color32::WHITE, 10);
    ui.scope(|ui| {
        ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, sep);
        ui.separator();
    });
}

/// Floating pill plaque: a dark translucent backing with a light rim.
/// A unified look for floating elements (toolbar, frame-index plaque). The ends
/// are fully rounded (radius = half the height): this way floating surfaces
/// read as one class and differ from sunken buttons with [theme::RADIUS_SM].
pub(crate) fn plaque(painter: &egui::Painter, rect: egui::Rect) {
    let rounding = egui::Rounding::same(rect.height() * 0.5);
    painter.rect_filled(rect, rounding, theme::NAV_BG);
    painter.rect_stroke(
        rect.shrink(0.5),
        rounding,
        egui::Stroke::new(1.0, theme::NAV_BORDER),
    );
}

/// Dot grid of the "infinite canvas".
///
/// The dots form a lattice with step `step` and a node at `origin`. `origin` is the
/// frame center (`viewport_center + pan_offset`), and `step` scales together
/// with zoom: this way the background "zooms in/out" in sync with the image and
/// pans along with it. Dots inside `skip` (the image
/// rectangle) are not drawn — under the frame they are hidden anyway, and we don't waste
/// work on them.
pub(crate) fn dot_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    skip: egui::Rect,
    origin: egui::Pos2,
    step: f32,
    radius: f32,
    color: egui::Color32,
) {
    if step <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    // The frame fully covers the canvas — there will be no dots at all, so don't build the grid.
    if skip.contains_rect(rect) {
        return;
    }

    // The first lattice point that falls inside `rect` (nodes lie at
    // `origin + n * step`).
    let start_x = origin.x + ((rect.left() - origin.x) / step).ceil() * step;
    let start_y = origin.y + ((rect.top() - origin.y) / step).ceil() * step;
    let size = egui::vec2(radius * 2.0, radius * 2.0);

    let mut mesh = egui::Mesh::default();
    let mut y = start_y;
    while y <= rect.bottom() {
        let mut x = start_x;
        while x <= rect.right() {
            let c = egui::pos2(x, y);
            if !skip.contains(c) {
                mesh.add_colored_rect(egui::Rect::from_center_size(c, size), color);
            }
            x += step;
        }
        y += step;
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Fills the rectangle with a horizontal gradient: `left` on the left, `right` on the right.
/// Used for a soft transition between the panel and the canvas instead of lines.
#[allow(dead_code)]
pub(crate) fn gradient_h(
    painter: &egui::Painter,
    rect: egui::Rect,
    left: egui::Color32,
    right: egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Fills the rectangle with a vertical gradient: `top` at the top, `bottom` at the bottom.
#[allow(dead_code)]
pub(crate) fn gradient_v(
    painter: &egui::Painter,
    rect: egui::Rect,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Full-width radio option row for dialogs.
///
/// Unlike `ui.radio`, the backing and the click extend across the full available width,
/// so no empty field is left at the right edge of the window, and long labels
/// are neatly clipped at the row boundary.
/// Returns a `Response`: a click anywhere on the row toggles the option.
pub(crate) fn option_row(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl Into<String>,
    hover: &str,
) -> egui::Response {
    let height = 26.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let hovered = response.hovered();

    // No fill on hover: the selected option is marked only by
    // the radio circle, and hover by the ring color. The row stays calm.
    // The radio circle on the left — like a system control.
    let dot = egui::pos2(rect.left() + 13.0, rect.center().y);
    let ring = if selected {
        theme::ACCENT
    } else if hovered {
        theme::TEXT_SECONDARY
    } else {
        theme::TEXT_TERTIARY
    };
    ui.painter()
        .circle_stroke(dot, 7.0, egui::Stroke::new(1.5, ring));
    if selected {
        ui.painter().circle_filled(dot, 3.6, theme::ACCENT);
    }

    // The text is clipped at the row boundary, so long preset names
    // do not overflow the window.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.into(), font, theme::TEXT);
    let pos = egui::pos2(rect.left() + 30.0, rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(rect)
        .galley(pos, galley, theme::TEXT);

    response.on_hover_text(hover)
}

/// Accent (primary) CTA button. Unlike `Button::fill`, it repaints
/// the theme's semantic accent colors in ALL states (normal/hover/press),
/// so the button looks like the primary action and responds vividly to the cursor.
pub(crate) fn primary_button(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    min_size: egui::Vec2,
) -> egui::Response {
    let white = egui::Stroke::new(1.0, egui::Color32::WHITE);
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;
        widgets.inactive.weak_bg_fill = theme::ACCENT;
        widgets.inactive.fg_stroke = white;
        widgets.hovered.weak_bg_fill = theme::ACCENT_HOVER;
        widgets.hovered.bg_stroke = egui::Stroke::NONE;
        widgets.hovered.fg_stroke = white;
        widgets.active.weak_bg_fill = theme::ACCENT_ACTIVE;
        widgets.active.bg_stroke = egui::Stroke::NONE;
        widgets.active.fg_stroke = white;
        ui.add(egui::Button::new(text).min_size(min_size))
    })
    .inner
}

/// "Ghost" secondary button: transparent background, but with a thin stroke
/// so it reads as a button and does not "hang" in the air. On hover the stroke and
/// text are tinted with `hover_color`, and a light backing appears. Needed for
/// actions that must not compete with the primary ones — e.g. "Close Session".
pub(crate) fn ghost_button(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    min_size: egui::Vec2,
    hover_color: egui::Color32,
) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        w.inactive.bg_stroke = egui::Stroke::new(1.0, theme::BORDER);
        w.inactive.fg_stroke = egui::Stroke::new(1.0, theme::TEXT_SECONDARY);
        w.hovered.weak_bg_fill = theme::with_alpha(hover_color, 28);
        w.hovered.bg_stroke = egui::Stroke::new(1.0, hover_color);
        w.hovered.fg_stroke = egui::Stroke::new(1.0, hover_color);
        w.active.weak_bg_fill = theme::with_alpha(hover_color, 56);
        w.active.bg_stroke = egui::Stroke::new(1.0, hover_color);
        w.active.fg_stroke = egui::Stroke::new(1.0, hover_color);
        ui.add(egui::Button::new(text).min_size(min_size))
    })
    .inner
}

/// A noticeable secondary button: the fill of a normal button plus an accent
/// (e.g. warning) highlight of the stroke and text on hover. It differs from
/// `primary_button` only in style, but it is laid out exactly at the given size
/// from the left edge, and the label inside is centered (egui takes text alignment
/// from the layout, so inside the child ui we enable a centering layout — otherwise
/// the text is pinned to the left edge along with the button).
pub(crate) fn subtle_button(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    min_size: egui::Vec2,
    hover_color: egui::Color32,
) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.weak_bg_fill = theme::BG_SURFACE;
        w.inactive.bg_stroke = egui::Stroke::new(1.0, theme::BORDER);
        w.inactive.fg_stroke = egui::Stroke::new(1.0, theme::TEXT);
        w.hovered.weak_bg_fill = theme::with_alpha(hover_color, 26);
        w.hovered.bg_stroke = egui::Stroke::new(1.0, hover_color);
        w.hovered.fg_stroke = egui::Stroke::new(1.0, hover_color);
        w.active.weak_bg_fill = theme::with_alpha(hover_color, 48);
        w.active.bg_stroke = egui::Stroke::new(1.0, hover_color);
        w.active.fg_stroke = egui::Stroke::new(1.0, hover_color);
        ui.allocate_ui_with_layout(
            min_size,
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                // The inner ui is exactly the button width with a centering layout:
                // this way the button sits on the left and its label is centered.
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add(egui::Button::new(text).min_size(min_size))
                })
                .inner
            },
        )
        .inner
    })
    .inner
}

/// Full-width accent CTA card: an icon in a circle on the left plus a label.
/// More noticeable than a normal button and works as a single tile, but without a nested
/// group frame. The content (icon + label) is centered as a single group, and
/// the card edges match the panel groups. Returns a `Response` (a click on
/// the whole card).
pub(crate) fn icon_card_button(
    ui: &mut egui::Ui,
    icon: &str,
    text: &str,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = response.hovered();
    let rounding = theme::RADIUS_SM;

    // Backing and stroke: calm at rest, accent on hover.
    let (bg, border, circle_bg, glyph) = if hovered {
        (theme::with_alpha(theme::ACCENT, 42), theme::ACCENT, theme::ACCENT, egui::Color32::WHITE)
    } else {
        (
            theme::with_alpha(theme::ACCENT, 16),
            theme::with_alpha(theme::ACCENT, 80),
            theme::with_alpha(theme::ACCENT, 52),
            theme::TEXT,
        )
    };

    let p = ui.painter();
    p.rect_filled(rect, rounding, bg);
    p.rect_stroke(rect.shrink(0.5), rounding, egui::Stroke::new(1.0, border));

    // Icon in a circle + label — a single group, centered in the card.
    // The circle diameter is chosen to fit the short card
    // (height like the "Reset all settings" button).
    let galley = p.layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.0), theme::TEXT);
    let circle_r = 9.0;
    let gap = 9.0;
    let content_w = circle_r * 2.0 + gap + galley.size().x;
    let content_left = rect.center().x - content_w * 0.5;

    let circle_c = egui::pos2(content_left + circle_r, rect.center().y);
    p.circle_filled(circle_c, circle_r, circle_bg);
    p.text(
        circle_c,
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(11.0),
        glyph,
    );

    // The label — right after the circle, vertically centered.
    let text_x = content_left + circle_r * 2.0 + gap;
    p.galley(
        egui::pos2(text_x, rect.center().y - galley.size().y * 0.5),
        galley,
        theme::TEXT,
    );

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Slider with a label, a reset button and an input field.
/// Returns `(changed, is_dragging)`.
pub(crate) fn labeled_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
) -> (bool, bool) {
    labeled_slider_impl(ui, label, value, range, default, None)
}

/// Same as [`labeled_slider`], but the rail is painted as a colour gradient.
/// Used for the bipolar white-balance axes (temperature/tint), where the track
/// colour is meaningful. `stops` are evenly spaced from `range.start()` to
/// `range.end()`.
pub(crate) fn labeled_slider_gradient(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    stops: &[egui::Color32],
) -> (bool, bool) {
    labeled_slider_impl(ui, label, value, range, default, Some(stops))
}

fn labeled_slider_impl(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    gradient: Option<&[egui::Color32]>,
) -> (bool, bool) {
    let mut changed = false;
    let mut is_dragging = false;
    let right_padding = 3.0; // Right padding so it does not touch the scrollbar

    ui.vertical(|ui| {
        // --- TOP ROW ---
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(
                egui::RichText::new(label).size(12.0).color(theme::TEXT),
            ));

            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
            let widget_rect = egui::Rect::from_min_size(
                ui.min_rect().min,
                egui::vec2(ui.available_width(), 35.0),
            );
            let is_hovered = pointer_pos.map_or(false, |p| widget_rect.contains(p));

            if is_hovered && *value != default {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(ph::ARROW_COUNTER_CLOCKWISE).size(10.0),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Reset")
                    .clicked()
                {
                    *value = default;
                    changed = true;
                }
            }

            // Align the DragValue to the right with a small padding
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Add padding BEFORE the DragValue (since we are in right_to_left, this will be the far right corner)
                ui.add_space(right_padding);

                let drag = ui.add(egui::DragValue::new(value).speed(0.1).max_decimals(2));
                if drag.changed() {
                    *value = value.clamp(*range.start(), *range.end());
                    changed = true;
                }
                if drag.dragged() {
                    is_dragging = true;
                }
            });
        });

        // --- BOTTOM ROW (Slider) ---
        ui.scope(|ui| {
            // Subtract the padding from the total slider width
            ui.spacing_mut().slider_width = ui.available_width() - right_padding;

            match gradient {
                Some(stops) => {
                    let (c, d) = axis_slider(ui, value, range, default, stops);
                    changed |= c;
                    is_dragging |= d;
                }
                None => {
                    let slider_resp = ui.add(
                        egui::Slider::new(value, range)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    // Double-click (left or right) on the slider resets it to its default
                    // value. The Slider only senses drags (not clicks), so `Response::clicked`
                    // never fires — we read the double-click straight from the pointer state
                    // and make sure the cursor is over this slider.
                    let double_clicked = ui.input(|i| {
                        i.pointer.button_double_clicked(egui::PointerButton::Primary)
                            || i.pointer.button_double_clicked(egui::PointerButton::Secondary)
                    });
                    if double_clicked && slider_resp.hovered() && *value != default {
                        *value = default;
                        changed = true;
                    }
                    if slider_resp.changed() {
                        changed = true;
                    }
                    if slider_resp.dragged() {
                        is_dragging = true;
                    }
                }
            }
        });
    });

    ui.add_space(6.0);
    (changed, is_dragging)
}

/// Horizontal slider whose rail is a colour gradient. Click/drag sets the value;
/// double-click resets to `default`; the handle matches the stock egui slider so
/// it blends with the rest of the panel. Returns `(changed, is_dragging)`.
fn axis_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    stops: &[egui::Color32],
) -> (bool, bool) {
    let min = *range.start();
    let max = *range.end();
    let span = (max - min).abs();

    let height = ui.spacing().interact_size.y;
    let width = ui.spacing().slider_width;
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());

    let rail_radius = (ui.spacing().slider_rail_height * 0.5).max(1.0);
    let rail = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.center().y - rail_radius),
        egui::pos2(rect.right(), rect.center().y + rail_radius),
    );
    // The handle cannot reach past its own radius, exactly like egui's slider.
    let handle_radius = rect.height() / 2.5;
    let track = rail.x_range().shrink(handle_radius);

    let mut changed = false;
    if let Some(p) = response.interact_pointer_pos() {
        let t = ((p.x - track.min) / (track.max - track.min)).clamp(0.0, 1.0);
        let new = min + t * span;
        if new != *value {
            *value = new;
            changed = true;
        }
    }
    // Double-click (left or right) resets to the default, like the stock sliders.
    let double_clicked = ui.input(|i| {
        i.pointer.button_double_clicked(egui::PointerButton::Primary)
            || i.pointer.button_double_clicked(egui::PointerButton::Secondary)
    });
    if double_clicked && response.hovered() && *value != default {
        *value = default;
        changed = true;
    }
    if changed {
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        paint_gradient_rail(ui.painter(), rail, stops);

        let t = if span > 0.0 {
            ((*value - min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let center = egui::pos2(egui::lerp(track.min..=track.max, t), rect.center().y);
        ui.painter().add(egui::epaint::CircleShape {
            center,
            radius: handle_radius + visuals.expansion,
            fill: visuals.bg_fill,
            stroke: visuals.fg_stroke,
        });
    }

    // Keyboard: arrows nudge by 1 (the DragValue handles typing/precision).
    if response.has_focus() {
        let step = ui.input(|i| {
            let up = i.num_presses(egui::Key::ArrowUp) + i.num_presses(egui::Key::ArrowRight);
            let down = i.num_presses(egui::Key::ArrowDown) + i.num_presses(egui::Key::ArrowLeft);
            up as f32 - down as f32
        });
        if step != 0.0 {
            *value = (*value + step).clamp(min, max);
            changed = true;
            response.mark_changed();
        }
    }

    let is_dragging = response.dragged();

    (changed, is_dragging)
}

/// Paints a horizontal colour-gradient capsule matching egui's rounded rail:
/// the two round end caps are drawn first, then the gradient mesh covers only
/// the straight middle section, so the ends stay fully rounded (no square corners).
fn paint_gradient_rail(painter: &egui::Painter, rail: egui::Rect, stops: &[egui::Color32]) {
    if stops.len() < 2 || rail.width() <= 0.0 {
        return;
    }
    let n = stops.len();
    let r = rail.height() * 0.5;

    // Round caps at both ends (same radius as the stock rail's rounding).
    painter.circle_filled(egui::pos2(rail.left() + r, rail.center().y), r, stops[0]);
    painter.circle_filled(
        egui::pos2(rail.right() - r, rail.center().y),
        r,
        stops[n - 1],
    );

    // Gradient on the straight section between the cap centers, so the mesh is
    // tangent to the caps and never pokes past them.
    let inner = egui::Rect::from_min_max(
        egui::pos2(rail.left() + r, rail.top()),
        egui::pos2((rail.right() - r).max(rail.left() + r), rail.bottom()),
    );
    if inner.width() <= 0.0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let xs: Vec<f32> = (0..n)
        .map(|i| egui::lerp(inner.x_range(), i as f32 / (n - 1) as f32))
        .collect();
    for i in 0..n - 1 {
        let (x0, x1) = (xs[i], xs[i + 1]);
        let (c0, c1) = (stops[i], stops[i + 1]);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x0, inner.top()), c0);
        mesh.colored_vertex(egui::pos2(x1, inner.top()), c1);
        mesh.colored_vertex(egui::pos2(x1, inner.bottom()), c1);
        mesh.colored_vertex(egui::pos2(x0, inner.bottom()), c0);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
}
