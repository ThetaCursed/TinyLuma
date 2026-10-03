// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use std::collections::HashSet;
use std::path::PathBuf;

/// A single LUT file in the library.
#[derive(Clone)]
pub(crate) struct LutEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) category: String,
}

impl LutEntry {
    /// Stable key for favorites: category + name.
    /// Distinguishes same-named LUTs from different folders and does not depend
    /// on the path. '\u{1}' is a separator that never appears in file names.
    pub(crate) fn favorite_key(&self) -> String {
        format!("{}\u{1}{}", self.category, self.name)
    }
}

/// Library of LUT files from the `luts/` folder, with favorites.
pub(crate) struct LutLibrary {
    pub(crate) all_luts: Vec<LutEntry>,
    pub(crate) favorites: HashSet<String>, // keys of favorite LUTs (category\x01name)
    pub(crate) selected_lut_name: Option<String>,
    pub(crate) favorites_path: Option<PathBuf>,
}

impl LutLibrary {
    pub(crate) fn new() -> Self {
        Self {
            all_luts: Vec::new(),
            favorites: HashSet::new(),
            selected_lut_name: None,
            favorites_path: None,
        }
    }

    pub(crate) fn save_favorites(&self) {
        if let Some(path) = &self.favorites_path {
            if let Ok(json) = serde_json::to_string_pretty(&self.favorites) {
                let _ = std::fs::write(path, json);
                println!("💾 Favorites saved to {:?}", path);
            }
        }
    }

    pub(crate) fn load_favorites(&mut self) {
        if let Some(path) = &self.favorites_path {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if let Ok(favs) = serde_json::from_str::<HashSet<String>>(&content) {
                        self.favorites = self.migrate_favorites(favs);
                        println!(
                            "📂 Favorites loaded from {:?}: {} filters",
                            path,
                            self.favorites.len()
                        );
                    }
                }
            }
        }
    }

    /// Migrates the old format (plain names) to the new one (category\x01name).
    /// Called after `scan_folder`, so the LUTs are already known.
    fn migrate_favorites(&self, raw: HashSet<String>) -> HashSet<String> {
        let mut out = HashSet::new();
        for key in raw {
            // Already a new-format key — keep it as is.
            if key.contains('\u{1}') {
                out.insert(key);
                continue;
            }
            // Old format (a plain name): mark all same-named LUTs (as before).
            let mut matched = false;
            for lut in &self.all_luts {
                if lut.name == key {
                    out.insert(lut.favorite_key());
                    matched = true;
                }
            }
            // The LUT is not present right now — keep the entry so the favorite
            // is not lost.
            if !matched {
                out.insert(key);
            }
        }
        out
    }

    // Subfolders are scanned as categories.
    pub(crate) fn scan_folder(&mut self, root: PathBuf) {
        self.all_luts.clear();

        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();

                if path.is_dir() {
                    // This is a folder — its name becomes the category name.
                    let category_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Unknown".to_string());

                    // Scan the files inside this folder.
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub_entry in sub_entries.flatten() {
                            let sub_path = sub_entry.path();
                            if sub_path.extension().and_then(|s| s.to_str()) == Some("cube") {
                                self.all_luts.push(LutEntry {
                                    name: sub_path
                                        .file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                    path: sub_path,
                                    category: category_name.clone(),
                                });
                            }
                        }
                    }
                } else if path.extension().and_then(|s| s.to_str()) == Some("cube") {
                    // Files that sit directly in the root of the chosen folder.
                    self.all_luts.push(LutEntry {
                        name: path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        path,
                        category: "General (Root)".to_string(),
                    });
                }
            }
        }
        // Sort: by category first, then by name within a category.
        self.all_luts
            .sort_by(|a, b| match a.category.cmp(&b.category) {
                std::cmp::Ordering::Equal => a.name.cmp(&b.name),
                other => other,
            });
    }
}
