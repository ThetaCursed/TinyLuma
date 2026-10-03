// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings::FilterSettings;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PresetData {
    pub(crate) name: String,
    pub(crate) settings: FilterSettings,
    /// Path to the LUT file (if the preset includes a LUT).
    /// `default` so old presets.json files without this field still load.
    #[serde(default)]
    pub(crate) lut_path: Option<PathBuf>,
}

/// Preset manager (loads/saves `config/presets.json`).
pub(crate) struct PresetManager {
    pub(crate) presets: Vec<PresetData>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) show_new_dialog: bool,
    pub(crate) new_preset_name: String,
    // --- Rename dialog ---
    pub(crate) show_rename_dialog: bool,
    pub(crate) rename_index: Option<usize>,
    pub(crate) rename_name: String,
    pub(crate) rename_focus_requested: bool,
    /// Index of the preset whose delete-confirmation dialog is open.
    pub(crate) delete_confirm_index: Option<usize>,
    pub(crate) file_path: PathBuf,
}

impl PresetManager {
    pub(crate) fn new() -> Self {
        let file_path = PathBuf::from("config/presets.json");
        let mut mgr = Self {
            presets: Vec::new(),
            selected_index: None,
            show_new_dialog: false,
            new_preset_name: String::new(),
            show_rename_dialog: false,
            rename_index: None,
            rename_name: String::new(),
            rename_focus_requested: false,
            delete_confirm_index: None,
            file_path,
        };
        mgr.load();
        mgr
    }

    pub(crate) fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.presets) {
            let _ = std::fs::write(&self.file_path, json);
            println!("💾 Presets saved to {:?}", self.file_path);
        }
    }

    pub(crate) fn load(&mut self) {
        if self.file_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&self.file_path) {
                if let Ok(presets) = serde_json::from_str::<Vec<PresetData>>(&content) {
                    self.presets = presets;
                    println!("📂 Presets loaded: {}", self.presets.len());
                }
            }
        }
    }

    pub(crate) fn add_preset(
        &mut self,
        name: String,
        settings: FilterSettings,
        lut_path: Option<PathBuf>,
    ) {
        self.presets.push(PresetData {
            name,
            settings,
            lut_path,
        });
        self.selected_index = Some(self.presets.len() - 1);
        self.save();
    }

    pub(crate) fn save_current(
        &mut self,
        index: usize,
        settings: FilterSettings,
        lut_path: Option<PathBuf>,
    ) {
        if index < self.presets.len() {
            self.presets[index].settings = settings;
            self.presets[index].lut_path = lut_path;
            self.save();
        }
    }

    /// Whether the name is taken by another preset (to forbid duplicates).
    pub(crate) fn name_exists(&self, name: &str) -> bool {
        self.presets.iter().any(|p| p.name == name)
    }

    /// Whether the name is taken by a preset other than `index` (for renaming).
    pub(crate) fn name_taken_by_other(&self, name: &str, index: usize) -> bool {
        self.presets
            .iter()
            .enumerate()
            .any(|(i, p)| i != index && p.name == name)
    }

    /// Rename a preset (the name must already be validated).
    pub(crate) fn rename_preset(&mut self, index: usize, new_name: String) {
        if let Some(p) = self.presets.get_mut(index) {
            p.name = new_name;
            self.save();
        }
    }

    pub(crate) fn delete_preset(&mut self, index: usize) {
        if index >= self.presets.len() {
            return;
        }
        self.presets.remove(index);
        // Deleting a saved preset does NOT touch the current sliders or LUT:
        // that is separate per-frame state. If the selected preset was deleted,
        // clear the selection (the frame becomes "with its own settings") instead
        // of switching to a neighbor — otherwise the combo box would show one
        // preset while the sliders kept another.
        // If a preset before the selected one was deleted, shift the index so the
        // selection stays on the same preset.
        self.selected_index = match self.selected_index {
            Some(sel) if sel == index => None,
            Some(sel) if sel > index => Some(sel - 1),
            other => other,
        };
        self.save();
    }
}
