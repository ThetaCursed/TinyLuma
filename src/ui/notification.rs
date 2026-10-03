// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;

use crate::app::TinyLumaApp;

/// Notification type — defines the icon, accent color and lifetime.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Success,
    Error,
    Warning,
    #[allow(dead_code)]
    Info,
}

impl ToastKind {
    fn style(self) -> (&'static str, egui::Color32) {
        match self {
            ToastKind::Success => (ph::CHECK_CIRCLE, theme::SUCCESS),
            ToastKind::Error => (ph::X_CIRCLE, theme::DANGER),
            ToastKind::Warning => (ph::WARNING, theme::WARNING),
            ToastKind::Info => (ph::INFO, theme::ACCENT),
        }
    }

    fn duration(self) -> f32 {
        match self {
            // An error is read for longer — so it gets more time.
            ToastKind::Error => 2.5,
            ToastKind::Warning => 2.0,
            ToastKind::Success | ToastKind::Info => 1.5,
        }
    }
}

/// A single notification in the stack. The lifetime accumulates in `age` (rather than
/// an absolute timestamp) so the timer can be "paused" by hovering.
pub(crate) struct Notification {
    text: String,
    kind: ToastKind,
    age: f32,
    duration: f32,
    /// Hover state from the previous frame: while the cursor is on the card — don't tick.
    /// Starts as `true`, i.e. we assume the cursor "was already" on the toast: this
    /// suppresses arming the pause on the first frame, when the cursor stayed where
    /// the click happened. A real entry (after a frame with no hover) will trigger.
    hovered: bool,
    /// Whether the hover pause is "armed". It is armed only when the cursor enters
    /// the card, not when it was already there at the moment of appearance.
    armed: bool,
}

impl Notification {
    fn new(kind: ToastKind, text: String) -> Self {
        Self {
            text,
            kind,
            age: 0.0,
            duration: kind.duration(),
            hovered: true,
            armed: false,
        }
    }
}

const FADE_IN: f32 = 0.18;
const FADE_OUT: f32 = 0.25;
const MAX_VISIBLE: usize = 4;

/// Adds a notification. Identical ones (type + text) are not duplicated: an already
/// visible one is simply "shaken" if it has started to fade.
pub(crate) fn push(list: &mut Vec<Notification>, kind: ToastKind, text: String) {
    if let Some(existing) = list.iter_mut().find(|n| n.kind == kind && n.text == text) {
        if existing.age > existing.duration - FADE_OUT {
            existing.age = 0.0;
        }
        return;
    }
    list.push(Notification::new(kind, text));
    while list.len() > MAX_VISIBLE {
        list.remove(0);
    }
}

/// Draws the notification stack centered on top of the whole interface: that way
/// they cannot be missed (in a corner the appearance could be "missed").
pub(crate) fn show(ctx: &egui::Context, list: &mut Vec<Notification>) {
    if list.is_empty() {
        return;
    }

    // Clamp dt so the animation does not "jump" after an idle period.
    let dt = ctx.input(|i| i.stable_dt).min(0.1);

    egui::Area::new(egui::Id::new("notifications"))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            // Center the cards relative to each other — a neat stack.
            ui.vertical_centered(|ui| {
                for n in list.iter_mut() {
                    // Pause — only on deliberate hover (armed). A cursor
                    // left where the click happened does not keep the toast on screen.
                    if !(n.hovered && n.armed) {
                        n.age += dt;
                    }
                    let resp = draw_card(ui, n, fade_alpha(n));
                    let hovered_now = resp.hovered();
                    // Cursor entering the card (was not hovered → hovered) arms the pause.
                    if hovered_now && !n.hovered {
                        n.armed = true;
                    }
                    n.hovered = hovered_now;
                }
            });
        });

    // Remove the expired ones. The hover pause only delays removal, it does not cancel it.
    list.retain(|n| n.age < n.duration + FADE_OUT + 0.01);

    // As long as there is something to show — keep animating.
    if !list.is_empty() {
        ctx.request_repaint();
    }
}

fn fade_alpha(n: &Notification) -> f32 {
    if n.age < FADE_IN {
        (n.age / FADE_IN).clamp(0.0, 1.0)
    } else if n.age > n.duration {
        ((n.duration + FADE_OUT - n.age) / FADE_OUT).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// A single notification card. Returns the response for hover tracking.
fn draw_card(ui: &mut egui::Ui, n: &Notification, alpha: f32) -> egui::Response {
    let (icon, accent) = n.kind.style();

    let bg = theme::BG_ELEVATED;
    let border = theme::BORDER;

    ui.scope(|ui| {
        ui.set_opacity(alpha);
        egui::Frame::none()
            .fill(bg)
            .stroke(egui::Stroke::new(1.0, border))
            .rounding(egui::Rounding::same(9.0))
            .shadow(egui::epaint::Shadow {
                offset: egui::vec2(0.0, 5.0),
                blur: 16.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(110),
            })
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Colored "medallion" with the icon.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 10.0, accent.gamma_multiply(0.18));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icon,
                        egui::FontId::proportional(11.0),
                        accent,
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&n.text).size(13.0).color(theme::TEXT),
                        )
                        .wrap(),
                    );
                });
            })
            .response
    })
    .inner
}

impl TinyLumaApp {
    /// Show a notification of the given type.
    pub(crate) fn notify(&mut self, kind: ToastKind, text: impl Into<String>) {
        push(&mut self.notifications, kind, text.into());
    }

    /// Draw the notification stack (called once per frame on top of everything).
    pub(crate) fn show_notifications(&mut self, ctx: &egui::Context) {
        show(ctx, &mut self.notifications);
    }
}
