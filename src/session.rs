// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use std::collections::HashMap;
use std::path::PathBuf;

use crate::history::History;
use crate::settings::FilterSettings;

/// A session: the image list plus per-file settings.
pub(crate) struct Session {
    pub(crate) image_list: Vec<PathBuf>,
    pub(crate) current_index: usize,
    pub(crate) settings_map: HashMap<PathBuf, FilterSettings>,
    /// A LUT is addressed by file PATH (not by name): names can collide, while
    /// a path is unique and matches how a preset stores its LUT.
    pub(crate) lut_map: HashMap<PathBuf, Option<PathBuf>>,
    /// Name of the preset a frame is associated with (even if the settings were
    /// later changed without saving). It is kept separate from the settings so
    /// that returning to a frame shows "preset + unsaved changes".
    pub(crate) preset_map: HashMap<PathBuf, Option<String>>,
    /// A separate undo/redo stack per image: switching frames must not wipe the
    /// edit history of the neighboring frames.
    pub(crate) history_map: HashMap<PathBuf, History>,
}

impl Session {
    pub(crate) fn new() -> Self {
        Self {
            image_list: Vec::new(),
            current_index: 0,
            settings_map: HashMap::new(),
            lut_map: HashMap::new(),
            preset_map: HashMap::new(),
            history_map: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.image_list.is_empty()
    }

    pub(crate) fn total(&self) -> usize {
        self.image_list.len()
    }

    #[allow(dead_code)]
    pub(crate) fn current_path(&self) -> Option<&PathBuf> {
        self.image_list.get(self.current_index)
    }

    pub(crate) fn modified_count(&self) -> usize {
        self.image_list
            .iter()
            .filter(|path| self.is_modified(path))
            .count()
    }

    /// Whether a specific image is modified (settings differ from default or a
    /// LUT is selected).
    pub(crate) fn is_modified(&self, path: &PathBuf) -> bool {
        let default = FilterSettings::default();
        match self.settings_map.get(path) {
            Some(s) if s != &default => true,
            _ => self.lut_map.get(path).and_then(|o| o.as_ref()).is_some(),
        }
    }

    pub(crate) fn save_current_settings(
        &mut self,
        path: &PathBuf,
        settings: FilterSettings,
        lut: Option<PathBuf>,
        preset: Option<String>,
    ) {
        self.settings_map.insert(path.clone(), settings);
        self.lut_map.insert(path.clone(), lut);
        self.preset_map.insert(path.clone(), preset);
    }

    pub(crate) fn load_settings(
        &self,
        path: &PathBuf,
    ) -> (FilterSettings, Option<PathBuf>, Option<String>) {
        let settings = self.settings_map.get(path).copied().unwrap_or_default();
        let lut = self.lut_map.get(path).and_then(|o| o.clone());
        let preset = self.preset_map.get(path).and_then(|o| o.clone());
        (settings, lut, preset)
    }
}
