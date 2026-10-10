// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use std::path::PathBuf;

use crate::crop::Crop;
use crate::retouch::RetouchLayer;
use crate::settings::FilterSettings;

/// Snapshot of the entire editable state (sliders + selected LUT + preset + retouch + crop).
#[derive(Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) settings: FilterSettings,
    /// The LUT is restored by path (the Lut3D itself is reloaded from disk).
    pub(crate) lut_path: Option<PathBuf>,
    /// Name of the associated preset (for correct undo/redo of the selection).
    pub(crate) preset_name: Option<String>,
    /// The retouch layer (heal spots). Cheap to clone — a few spots per image.
    pub(crate) retouch: RetouchLayer,
    /// The crop frame + preset (a plain `Copy` value).
    pub(crate) crop: Crop,
}

/// Undo/redo stack over snapshots.
pub(crate) struct History {
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    limit: usize,
}

impl History {
    pub(crate) fn new() -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            limit: 100,
        }
    }

    /// Full reset (e.g. when opening/switching an image).
    pub(crate) fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    /// Number of recorded "before" states.
    pub(crate) fn len(&self) -> usize {
        self.past.len()
    }

    /// Drops every recorded state beyond `len` and clears the redo branch.
    /// Used when a whole tool session is cancelled (`Esc`).
    pub(crate) fn truncate(&mut self, len: usize) {
        self.past.truncate(len);
        self.future.clear();
    }

    /// Record the state that existed BEFORE a change.
    /// Any new record truncates the redo branch.
    pub(crate) fn push(&mut self, before: Snapshot) {
        self.past.push(before);
        self.future.clear();
        if self.past.len() > self.limit {
            self.past.remove(0);
        }
    }

    pub(crate) fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let prev = self.past.pop()?;
        self.future.push(current);
        Some(prev)
    }

    pub(crate) fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.future.pop()?;
        self.past.push(current);
        Some(next)
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crop::{AspectPreset, Crop};

    fn snap(crop: Crop) -> Snapshot {
        Snapshot {
            settings: FilterSettings::default(),
            lut_path: None,
            preset_name: None,
            retouch: RetouchLayer::default(),
            crop,
        }
    }

    fn cropped() -> Crop {
        let mut c = Crop::default();
        c.set_preset(AspectPreset::Fixed(3, 4), 832.0 / 1248.0);
        c
    }

    #[test]
    fn undo_restores_the_crop() {
        let mut h = History::new();
        // Gesture: identity → cropped. The baseline (identity) is pushed.
        h.push(snap(Crop::default()));
        let restored = h.undo(snap(cropped())).expect("undo available");
        assert!(restored.crop.is_identity(), "crop must return to the full frame");
    }

    #[test]
    fn redo_reapplies_the_crop() {
        let mut h = History::new();
        h.push(snap(Crop::default()));
        let undone = h.undo(snap(cropped())).unwrap();
        assert!(undone.crop.is_identity());
        let redone = h.redo(snap(Crop::default())).unwrap();
        assert!(!redone.crop.is_identity());
        assert_eq!(redone.crop, cropped());
    }

    #[test]
    fn truncate_drops_a_session_and_its_redo() {
        let mut h = History::new();
        h.push(snap(Crop::default()));
        let baseline_len = h.len();
        // Simulate a tool session that recorded two entries.
        h.push(snap(cropped()));
        h.push(snap(Crop::default()));
        assert_eq!(h.len(), baseline_len + 2);
        // Cancelling the session drops them and clears any redo branch.
        let _ = h.undo(snap(cropped()));
        assert!(h.can_redo());
        h.truncate(baseline_len);
        assert_eq!(h.len(), baseline_len);
        assert!(!h.can_redo());
    }
}
