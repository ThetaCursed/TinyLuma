// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Clipping warnings, as Lightroom's histogram triangles: the shadow and
//! highlight warnings toggle independently, hovering a triangle shows its
//! warning while the pointer stays there. View state only, never saved with the
//! edit. Reference: RAWmakase's `app/clipping.rs`.

use super::histogram::{HIGHLIGHT_CLIP, Histogram, SHADOW_CLIP};

/// One end of the histogram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipSide {
    Shadows,
    Highlights,
}

impl ClipSide {
    pub(crate) const BOTH: [Self; 2] = [Self::Shadows, Self::Highlights];
}

/// Which warnings are painted over the shown photo: clipped highlights red
/// where any channel clips, clipped shadows blue where all three do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClipOverlay {
    pub(crate) shadows: bool,
    pub(crate) highlights: bool,
}

impl ClipOverlay {
    pub(crate) const NONE: Self = Self {
        shadows: false,
        highlights: false,
    };
    pub(crate) const HIGHLIGHT_COLOR: [u8; 3] = [255, 40, 40];
    pub(crate) const SHADOW_COLOR: [u8; 3] = [40, 80, 255];

    pub(crate) fn any(self) -> bool {
        self.shadows || self.highlights
    }

    /// The warning colour shown instead of this rendered pixel, if any.
    pub(crate) fn color(self, pixel: [u8; 3]) -> Option<[u8; 3]> {
        if self.highlights && pixel.iter().any(|v| *v >= HIGHLIGHT_CLIP) {
            Some(Self::HIGHLIGHT_COLOR)
        } else if self.shadows && pixel.iter().all(|v| *v <= SHADOW_CLIP) {
            Some(Self::SHADOW_COLOR)
        } else {
            None
        }
    }

    /// Paints the warnings into an interleaved RGB8 buffer.
    pub(crate) fn paint(self, rgb: &mut [u8]) {
        if !self.any() {
            return;
        }
        for p in rgb.chunks_exact_mut(3) {
            if let Some(c) = self.color([p[0], p[1], p[2]]) {
                p[0] = c[0];
                p[1] = c[1];
                p[2] = c[2];
            }
        }
    }
}

/// Lightroom-style toggle / hover state for the two warnings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClippingView {
    /// Turned on by a triangle's click or the header toggle.
    on: ClipOverlay,
    /// The triangle under the pointer, shown only while it stays there.
    hover: Option<ClipSide>,
}

impl ClippingView {
    pub(crate) fn is_on(&self, side: ClipSide) -> bool {
        match side {
            ClipSide::Shadows => self.on.shadows,
            ClipSide::Highlights => self.on.highlights,
        }
    }

    /// Both warnings are on, as the header toggle shows.
    pub(crate) fn both_on(&self) -> bool {
        self.on.shadows && self.on.highlights
    }

    /// A triangle's click.
    pub(crate) fn toggle(&mut self, side: ClipSide) {
        let on = !self.is_on(side);
        match side {
            ClipSide::Shadows => self.on.shadows = on,
            ClipSide::Highlights => self.on.highlights = on,
        }
    }

    /// The header toggle: both on when either is off, otherwise both off.
    pub(crate) fn toggle_both(&mut self) {
        let on = !self.both_on();
        self.on = ClipOverlay {
            shadows: on,
            highlights: on,
        };
    }

    pub(crate) fn set_hover(&mut self, side: Option<ClipSide>) {
        self.hover = side;
    }

    /// Off, as leaving the editor leaves it.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// The warnings painted over the photo: those turned on and the hovered one.
    pub(crate) fn overlay(&self) -> ClipOverlay {
        let hovered = |side| self.hover == Some(side);
        ClipOverlay {
            shadows: self.on.shadows || hovered(ClipSide::Shadows),
            highlights: self.on.highlights || hovered(ClipSide::Highlights),
        }
    }
}

/// A channel clips visibly when more than this share of the pixels does.
pub(crate) const CLIPPED_SHARE: f32 = 0.001;

/// Which channels clip at `side`: red, green, blue.
pub(crate) fn clipped_channels(histogram: &Histogram, side: ClipSide) -> [bool; 3] {
    let counts = match side {
        ClipSide::Shadows => histogram.clipped.shadows,
        ClipSide::Highlights => histogram.clipped.highlights,
    };
    let total = histogram.total.max(1) as f32;
    counts.map(|n| n as f32 / total > CLIPPED_SHARE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_is_independent_per_side() {
        let mut view = ClippingView::default();
        view.toggle(ClipSide::Shadows);
        assert_eq!(
            view.overlay(),
            ClipOverlay {
                shadows: true,
                highlights: false
            }
        );
        view.toggle(ClipSide::Highlights);
        view.toggle(ClipSide::Shadows);
        assert_eq!(
            view.overlay(),
            ClipOverlay {
                shadows: false,
                highlights: true
            }
        );
    }

    #[test]
    fn toggle_both_turns_both_on_when_either_is_off() {
        let mut view = ClippingView::default();
        view.toggle_both();
        assert!(view.both_on());
        view.toggle_both();
        assert_eq!(view.overlay(), ClipOverlay::NONE);
        // One on: the header toggle turns the other on too, not this one off.
        view.toggle(ClipSide::Highlights);
        view.toggle_both();
        assert!(view.both_on());
    }

    #[test]
    fn hovering_shows_a_warning_only_while_hovered() {
        let mut view = ClippingView::default();
        view.set_hover(Some(ClipSide::Highlights));
        assert_eq!(
            view.overlay(),
            ClipOverlay {
                shadows: false,
                highlights: true
            }
        );
        assert!(!view.is_on(ClipSide::Highlights));
        view.set_hover(None);
        assert_eq!(view.overlay(), ClipOverlay::NONE);
        // Hovering a warning that is on changes nothing, and leaving keeps it on.
        view.toggle(ClipSide::Shadows);
        view.set_hover(Some(ClipSide::Shadows));
        view.set_hover(None);
        assert!(view.overlay().shadows);
    }

    #[test]
    fn paint_marks_only_its_own_end() {
        let mut rgb = [
            255, 200, 200, // red channel clips → highlight
            0, 0, 0, // all channels clip → shadow
            0, 128, 0, // only green clip → nothing
        ];
        ClipOverlay {
            highlights: true,
            ..ClipOverlay::NONE
        }
        .paint(&mut rgb);
        assert_eq!(&rgb[0..3], &ClipOverlay::HIGHLIGHT_COLOR);
        assert_eq!(&rgb[3..6], &[0, 0, 0]);

        let mut rgb = [255, 200, 200, 0, 0, 0, 0, 128, 0];
        ClipOverlay {
            shadows: true,
            ..ClipOverlay::NONE
        }
        .paint(&mut rgb);
        assert_eq!(&rgb[0..3], &[255, 200, 200]);
        assert_eq!(&rgb[3..6], &ClipOverlay::SHADOW_COLOR);
        assert_eq!(&rgb[6..9], &[0, 128, 0]);
    }

    #[test]
    fn clipped_channels_respects_the_visible_share() {
        let mut h = Histogram::EMPTY;
        h.total = 10_000;
        // 50 of 10,000 = 0.5% is over the 0.1% share; 5 (0.05%) is under.
        h.clipped.highlights = [50, 50, 5];
        h.clipped.shadows = [0, 0, 5];
        assert_eq!(
            clipped_channels(&h, ClipSide::Highlights),
            [true, true, false]
        );
        assert_eq!(clipped_channels(&h, ClipSide::Shadows), [false, false, false]);
    }
}
