// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use crate::theme;
use eframe::egui;
use egui_phosphor::regular as ph;
use std::path::PathBuf;
use std::time::Instant;

use crate::app::TinyLumaApp;
use crate::lut_library::LutEntry;

impl TinyLumaApp {
    /// Right panel: active LUT and library. Returns true on change.
    /// `visible == false` — the panel slides off the edge (on the screen without a frame).
    pub(crate) fn draw_lut_panel(&mut self, ctx: &egui::Context, visible: bool) -> bool {
        let mut changed = false;
        // The set of expanded categories changed — the config needs writing.
        let mut lut_categories_changed = false;

        egui::SidePanel::right("lut_library_panel")
            .resizable(false)
            .exact_width(super::PANEL_W)
            // See the left panel: the border is given by the background, not a line.
            .show_separator_line(false)
            .show_animated(ctx, visible, |ui| {
                // A neat scrollbar: thin and it does NOT balloon on hover
                // (for the floating style the width = lerp(floating_width ..= bar_width),
                //  so we make them equal). Same as on the left panel.
                // The scrollbar is at the right edge of the panel: group→handle gap = 8px,
                // handle→panel edge = 8px.
                let s = &mut ui.spacing_mut().scroll;
                s.bar_width = 4.0;
                s.floating_width = 4.0;
                s.floating_allocated_width = super::SCROLL_RESERVE;
                s.bar_outer_margin = 0.0;

                // ===== ACTIVE LUT — pinned at the bottom =====
                // The library above scrolls, while the status and strength of the selected LUT
                // are always visible. As on the left panel, we remove the footer's
                // horizontal padding and constrain the group by the scrollbar
                // reserve — then its width matches the library chips.
                egui::TopBottomPanel::bottom("lut_active_footer")
                    .frame(
                        egui::Frame::side_top_panel(&ctx.style())
                            .inner_margin(egui::Margin::symmetric(0.0, 2.0)),
                    )
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        ui.add_space(8.0);
                        ui.label(format!("{} ACTIVE LUT", ph::FILM_STRIP));
                        ui.add_space(6.0);

                        if self.active_lut.is_some() {
                            ui.group(|ui| {
                                // The footer does not scroll, so the scrollbar reserve
                                // is not needed here: we stretch the group to the very edge of the panel
                                // so the left and right paddings match (8px each).
                                ui.set_width(ui.available_width().max(40.0));
                                ui.spacing_mut().item_spacing.y = 4.0;

                                // The LUT name is the slider label; the value is on the right, the ✖ next to the slider.
                                let display_name =
                                    if let Some(name) = &self.lut_lib.selected_lut_name {
                                        name.clone()
                                    } else if let Some(path) = &self.lut_path {
                                        path.file_stem()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned()
                                    } else {
                                        String::new()
                                    };

                                let (c, d, rm) = lut_strength_slider(
                                    ui,
                                    &display_name,
                                    &mut self.settings.lut_intensity,
                                );
                                if c {
                                    changed = true;
                                }
                                if d {
                                    self.drag_active = true;
                                }
                                if rm {
                                    self.active_lut = None;
                                    self.lut_path = None;
                                    self.lut_lib.selected_lut_name = None;
                                    changed = true;
                                }
                            });
                        } else {
                            // Empty state: status + accent CTA card.
                            // Loading from a file is the only one here (there is none in the header),
                            // so we present it prominently. No group needed — the card
                            // is itself the frame, without nesting.
                            ui.label(
                                egui::RichText::new("No LUT applied")
                                    .size(11.5)
                                    .color(theme::TEXT_TERTIARY),
                            );
                            ui.add_space(6.0);
                            // The footer does not scroll — the scrollbar reserve is not needed,
                            // otherwise the right padding is larger than the left.
                            let w = ui.available_width().max(40.0);
                            if super::widgets::icon_card_button(
                                ui,
                                ph::FOLDER_OPEN,
                                "Load LUT…",
                                egui::vec2(w, super::FOOTER_BTN_H),
                            )
                            .clicked()
                                && self.load_lut_from_dialog()
                            {
                                changed = true;
                            }
                        }

                        ui.add_space(8.0);
                    });

                // ===== LIBRARY =====
                ui.add_space(5.0);

                // Header: title + library actions. We show the .cube load button
                // only when a LUT is already applied: in the empty state it is offered
                // by the large button in the footer, so it is not duplicated here.
                ui.horizontal(|ui| {
                    ui.label(format!("{} LIBRARY", ph::BOOKS));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(ph::ARROWS_CLOCKWISE)
                            .on_hover_text("Refresh file list")
                            .clicked()
                        {
                            self.lut_lib.scan_folder(PathBuf::from("luts"));
                        }
                        if self.active_lut.is_some()
                            && ui
                                .button(ph::FOLDER_OPEN)
                                .on_hover_text("Load LUT from a .cube file…")
                                .clicked()
                            && self.load_lut_from_dialog()
                        {
                            changed = true;
                        }
                    });
                });

                ui.add_space(6.0);

                // Search by name/category + "favorites only" filter.
                // The field and the star button are fixed above the list and do not scroll away.
                // The ★ filter replaces the former duplicate "Favorites"
                // section: in normal mode the category list is stable and is not
                // reflowed when stars are toggled.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    const STAR_BTN_W: f32 = 26.0;
                    let clear_w = if self.lut_search.is_empty() {
                        0.0
                    } else {
                        20.0
                    };
                    let field_w =
                        (ui.available_width() - clear_w - STAR_BTN_W - 4.0).max(40.0);
                    let resp = ui.add_sized(
                        [field_w, 22.0],
                        egui::TextEdit::singleline(&mut self.lut_search)
                            .hint_text(format!("{} Search LUTs…", ph::MAGNIFYING_GLASS)),
                    );
                    // Esc clears the query, as in familiar search boxes.
                    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.lut_search.clear();
                    }
                    if !self.lut_search.is_empty()
                        && ui
                            .add(
                                egui::Button::new(ph::X)
                                    .frame(false)
                                    .min_size(egui::vec2(18.0, 22.0)),
                            )
                            .on_hover_text("Clear search")
                            .clicked()
                    {
                        self.lut_search.clear();
                    }

                    // "Favorites only" toggle: in the on state
                    // an accent backing — like a selected LUT chip.
                    let favorites_only = self.lut_favorites_only;
                    let star_color = if favorites_only {
                        theme::ACCENT
                    } else {
                        theme::TEXT_SECONDARY
                    };
                    let mut star_btn = egui::Button::new(
                        egui::RichText::new(ph::STAR).size(13.0).color(star_color),
                    )
                    .frame(favorites_only)
                    .min_size(egui::vec2(STAR_BTN_W, 22.0));
                    if favorites_only {
                        star_btn = star_btn.fill(theme::with_alpha(theme::ACCENT, 45));
                    }
                    if ui
                        .add(star_btn)
                        .on_hover_text("Show favorites only")
                        .clicked()
                    {
                        self.lut_favorites_only = !favorites_only;
                        // Remember the mode in the config (unlike the search
                        // query — that one is session-only).
                        self.save_save_settings();
                        ui.ctx().request_repaint();
                    }
                });

                ui.add_space(6.0);

                let all_luts_copy = self.lut_lib.all_luts.clone();
                let favorites_copy = self.lut_lib.favorites.clone();
                let query = self.lut_search.trim().to_lowercase();
                let query_display = self.lut_search.trim().to_string();

                let favorites_only = self.lut_favorites_only;

                egui::ScrollArea::vertical()
                    .id_source("lut_scroll")
                    .show(ui, |ui| {
                        // --- SEARCH MODE: flat list of matches ---
                        // The ★ filter applies here too: we search only among favorites.
                        if !query.is_empty() {
                            let matches: Vec<LutEntry> = all_luts_copy
                                .iter()
                                .filter(|l| {
                                    (!favorites_only
                                        || favorites_copy.contains(&l.favorite_key()))
                                        && (l.name.to_lowercase().contains(&query)
                                            || l.category.to_lowercase().contains(&query))
                                })
                                .cloned()
                                .collect();

                            if matches.is_empty() {
                                ui.add_space(20.0);
                                ui.weak(format!("Nothing matches “{}”", query_display));
                            } else {
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(format!("{} result(s)", matches.len()))
                                        .size(11.0)
                                        .color(theme::TEXT_TERTIARY),
                                );
                                if self.draw_lut_chips(ui, &matches) {
                                    changed = true;
                                }
                            }
                            return;
                        }

                        // --- "FAVORITES ONLY" MODE: flat grid ---
                        // Without section headers: toggling stars changes
                        // only the tail of the list, rather than reflowing everything.
                        if favorites_only {
                            let fav_luts: Vec<LutEntry> = all_luts_copy
                                .iter()
                                .filter(|l| favorites_copy.contains(&l.favorite_key()))
                                .cloned()
                                .collect();

                            ui.add_space(2.0);
                            if fav_luts.is_empty() {
                                ui.add_space(20.0);
                                ui.weak(format!(
                                    "No favorites yet — click {} on a LUT",
                                    ph::STAR
                                ));
                            } else {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} {} favorite(s)",
                                        ph::STAR,
                                        fav_luts.len()
                                    ))
                                    .size(11.0)
                                    .color(theme::TEXT_TERTIARY),
                                );
                                if self.draw_lut_chips(ui, &fav_luts) {
                                    changed = true;
                                }
                            }
                            return;
                        }

                        // --- NORMAL MODE: categories only ---
                        // The "Favorites" section is gone here: favorites are available
                        // via the ★ filter, so toggling stars does not change
                        // the list height and categories do not jump.
                        let mut categories = Vec::new();
                        for lut in &all_luts_copy {
                            if !categories.contains(&lut.category) {
                                categories.push(lut.category.clone());
                            }
                        }

                        for cat in categories {
                            ui.add_space(2.0);
                            // The category expansion state is stored in the config. We use
                            // CollapsingState (as for the left-panel groups) to
                            // set the state from the saved set.
                            //
                            // Note: unlike the old `CollapsingHeader`, `show_header` only
                            // wires up the little triangle for clicks, not the whole row.
                            // We therefore make the category name clickable ourselves and
                            // toggle the state explicitly — otherwise clicking the name
                            // (the way it used to work) no longer expands the group.
                            let was_open = self.open_lut_categories.contains(&cat);
                            let mut name_clicked = false;
                            let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                ui.make_persistent_id(("lut_category", &cat)),
                                was_open,
                            )
                            .show_header(ui, |ui| {
                                name_clicked = ui
                                    .add(
                                        egui::Label::new(egui::RichText::new(&cat).strong())
                                            // `selectable(false)` keeps egui from turning the
                                            // label into selectable text: that is what made the
                                            // cursor an I-beam on hover and swallowed the click.
                                            .selectable(false)
                                            .sense(egui::Sense::click()),
                                    )
                                    .clicked();
                            });
                            if name_clicked {
                                header.toggle();
                            }
                            let is_open = header.is_open();
                            // `.body()` (not `.body_unindented()`) restores the old
                            // `CollapsingHeader::show` look: the body is indented and egui
                            // draws the faint vertical guide line down the left side of the
                            // group (visuals.indent_has_left_vline). It also nudges the LUT
                            // chips to the right. `.body_unindented()` suppressed both.
                            header.body(|ui| {
                                // Inside an expanded folder — a grid of compact chips.
                                let cat_luts: Vec<LutEntry> = all_luts_copy
                                    .iter()
                                    .filter(|l| l.category == cat)
                                    .cloned()
                                    .collect();
                                if self.draw_lut_chips(ui, &cat_luts) {
                                    changed = true;
                                }
                            });
                            if is_open != self.open_lut_categories.contains(&cat) {
                                if is_open {
                                    self.open_lut_categories.insert(cat.clone());
                                } else {
                                    self.open_lut_categories.remove(&cat);
                                }
                                lut_categories_changed = true;
                            }
                        }

                        if all_luts_copy.is_empty() {
                            ui.add_space(20.0);
                            ui.weak("Folder 'luts' not found or empty");
                        }
                    });

                // Write the category state immediately (like the left panel's open_groups).
                if lut_categories_changed {
                    self.save_save_settings();
                }
            });

        changed
    }

    /// Grid of LUT chips, two per row. The column width is fixed — half the
    /// available width, not fitted to the content: a large chip and the
    /// favorite star are noticeably easier to hit with the mouse, and long names
    /// get more room. Used in expanded categories, favorites
    /// and search results.
    fn draw_lut_chips(&mut self, ui: &mut egui::Ui, luts: &[LutEntry]) -> bool {
        if luts.is_empty() {
            return false;
        }
        let mut changed = false;
        const GAP: f32 = 6.0;
        let avail = ui.available_width().max(40.0);
        // Exactly two columns. −0.5px is a safeguard against fractional error,
        // otherwise egui may wrap the second chip onto a new line.
        let chip_w = ((avail - GAP) * 0.5 - 0.5).max(64.0);

        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.horizontal_wrapped(|ui| {
                for lut in luts {
                    if self.draw_lut_chip(ui, lut, egui::vec2(chip_w, 28.0)) {
                        changed = true;
                    }
                }
            });
        });
        changed
    }

    /// Compact LUT chip. A click on the chip selects the LUT, a click on the star on the left
    /// (shown on hover) adds/removes it from favorites.
    fn draw_lut_chip(&mut self, ui: &mut egui::Ui, lut: &LutEntry, size: egui::Vec2) -> bool {
        let mut changed = false;
        let is_selected = self.lut_lib.selected_lut_name.as_ref() == Some(&lut.name);
        let fav_key = lut.favorite_key();
        let is_fav = self.lut_lib.favorites.contains(&fav_key);

        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
        let hovered = resp.hovered();

        // Chip background and frame: selected — accent, hover — elevated background.
        let (bg, stroke) = if is_selected {
            (
                theme::with_alpha(theme::ACCENT, 80),
                egui::Stroke::new(1.0, theme::ACCENT),
            )
        } else if hovered {
            (theme::BG_ELEVATED, egui::Stroke::new(1.0, theme::BORDER))
        } else {
            (theme::BG_SURFACE, egui::Stroke::new(1.0, theme::SEPARATOR))
        };
        ui.painter().rect(rect, theme::RADIUS_SM, bg, stroke);

        // The star zone on the left (a click on it toggles favorite). The space is reserved
        // always so the text does not jitter when the star appears on hover.
        // The zone is deliberately wider than the glyph itself: in a half-panel-wide chip, the
        // left edge is enough to hit the favorite without aiming.
        const LEFT_PAD: f32 = 6.0;
        const STAR_W: f32 = 17.0;
        const GAP: f32 = 4.0;
        let star_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(LEFT_PAD, 0.0),
            egui::vec2(STAR_W, rect.height()),
        );
        // We always show the star on favorites, and on others — on hover,
        // so the action is discoverable. A click on the star zone toggles
        // the favorite (previously this only worked on right-click).
        let star_area = star_rect.expand(2.0);
        let star_hovered = hovered && ui.rect_contains_pointer(star_area);
        let hit_star = resp.clicked()
            && resp
                .interact_pointer_pos()
                .map_or(false, |p| star_area.contains(p));
        if is_fav || hovered {
            // Favorites — a white star, always visible; on others it is dim
            // and appears only on hover, and brightens when hovering the star itself.
            let color = if is_fav || star_hovered {
                theme::TEXT
            } else {
                theme::TEXT_TERTIARY
            };
            ui.painter().text(
                star_rect.center(),
                egui::Align2::CENTER_CENTER,
                ph::STAR,
                egui::FontId::proportional(12.0),
                color,
            );
        }

        // The LUT name — centered in the chip, the star stays on the left. The width is constrained
        // by the zone to the right of the star, so a long name does not run into it, and a
        // short one (e.g. "C-01") sits exactly centered.
        let text_min_left = star_rect.right() + GAP;
        let text_right = rect.right() - 9.0;
        let text_w = (text_right - text_min_left).max(0.0);
        let color = if is_selected {
            theme::TEXT
        } else {
            theme::TEXT_SECONDARY
        };
        let mut job = egui::text::LayoutJob::single_section(
            lut.name.clone(),
            egui::TextFormat {
                font_id: egui::FontId::proportional(12.5),
                color,
                ..Default::default()
            },
        );
        job.wrap.max_width = text_w;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let galley = ui.fonts(|f| f.layout_job(job));
        let text_left = (rect.center().x - galley.size().x * 0.5).max(text_min_left);
        let pos = egui::pos2(text_left, rect.center().y - galley.size().y * 0.5);
        ui.painter().galley(pos, galley, color);

        let resp = resp.on_hover_text(format!(
            "{}\nClick {} (or right-click) to {} favorite",
            lut.name,
            ph::STAR,
            if is_fav { "remove from" } else { "add to" }
        ));

        // The favorite is toggled by clicking the star, and also by right-clicking
        // anywhere on the chip. A left click on the rest selects the LUT.
        if hit_star || resp.secondary_clicked() {
            if is_fav {
                self.lut_lib.favorites.remove(&fav_key);
            } else {
                self.lut_lib.favorites.insert(fav_key.clone());
            }
            self.favorites_dirty = true;
            self.last_favorite_toggle = Instant::now();
            // The star is drawn BEFORE the click is handled, so we request a repaint —
            // otherwise it would visually toggle only on the next frame.
            ui.ctx().request_repaint();
        } else if resp.clicked() {
            if let Some(loaded_lut) = self.load_lut_cached(&lut.path) {
                self.active_lut = Some(loaded_lut);
                self.lut_lib.selected_lut_name = Some(lut.name.clone());
                self.lut_path = Some(lut.path.clone());
                // If the intensity was 0, set it to 100 when selecting a new one.
                if self.settings.lut_intensity == 0.0 {
                    self.settings.lut_intensity = 100.0;
                }
                changed = true;
            }
        }

        changed
    }
}

/// LUT strength slider, where the label is the active LUT's name.
/// Top row: name + ✖ next to it, value on the right. Bottom row: full-width slider.
/// Returns `(changed, is_dragging, remove_clicked)`.
fn lut_strength_slider(ui: &mut egui::Ui, name: &str, value: &mut f32) -> (bool, bool, bool) {
    let mut changed = false;
    let mut is_dragging = false;
    let mut remove = false;
    let right_padding = 3.0; // Don't touch the scrollbar, as in labeled_slider.

    // --- TOP ROW: name + ✖ on the left, value on the right ---
    ui.horizontal(|ui| {
        const DRAG_W: f32 = 56.0;
        const BTN_W: f32 = 18.0;

        // Fixed-width left block: the name (with clipping) and the remove button.
        let left_w = (ui.available_width() - DRAG_W - 8.0).max(40.0);
        ui.allocate_ui_with_layout(
            egui::vec2(left_w, 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let name_w = (ui.available_width() - BTN_W - 4.0).max(20.0);

                // Explicitly constrain the name width so the Label does not spread
                // across the center and push out the ✖ (unlike add_sized).
                ui.scope(|ui| {
                    ui.set_max_width(name_w);
                    ui.add(
                        egui::Label::new(egui::RichText::new(name).size(12.0).color(theme::TEXT))
                            .truncate(),
                    )
                    .on_hover_text(name);
                });

                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(ph::X).size(11.0))
                            .frame(false)
                            .min_size(egui::vec2(BTN_W, 16.0)),
                    )
                    .on_hover_text("Remove LUT")
                    .clicked()
                {
                    remove = true;
                }
            },
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(right_padding);
            let drag = ui.add(egui::DragValue::new(value).speed(0.1).max_decimals(2));
            if drag.changed() {
                *value = value.clamp(0.0, 100.0);
                changed = true;
            }
            if drag.dragged() {
                is_dragging = true;
            }
        });
    });

    // --- BOTTOM ROW: full-width slider ---
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width() - right_padding;
        let slider_resp = ui.add(
            egui::Slider::new(value, 0.0..=100.0)
                .show_value(false)
                .trailing_fill(true),
        );
        if slider_resp.changed() {
            changed = true;
        }
        if slider_resp.dragged() {
            is_dragging = true;
        }
    });

    ui.add_space(6.0);
    (changed, is_dragging, remove)
}
