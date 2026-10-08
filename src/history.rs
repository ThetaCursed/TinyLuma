// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use std::path::PathBuf;

use crate::retouch::RetouchLayer;
use crate::settings::FilterSettings;

/// Snapshot of the entire editable state (sliders + selected LUT + preset + retouch).
#[derive(Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) settings: FilterSettings,
    /// The LUT is restored by path (the Lut3D itself is reloaded from disk).
    pub(crate) lut_path: Option<PathBuf>,
    /// Name of the associated preset (for correct undo/redo of the selection).
    pub(crate) preset_name: Option<String>,
    /// The retouch layer (heal spots). Cheap to clone — a few spots per image.
    pub(crate) retouch: RetouchLayer,
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
