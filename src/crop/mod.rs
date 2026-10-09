// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Pure crop module.
//!
//! Knows nothing about egui, the app state or the file system: it only deals
//! with normalized rectangles, aspect presets and RGB8 buffers. See
//! `docs/CROP.md` for the design and the phase plan.

pub(crate) mod geom;
pub(crate) mod orient;
pub(crate) mod pixel;
pub(crate) mod state;

pub(crate) use geom::{AspectPreset, Handle, NormRect, RATIO_PRESETS};
pub(crate) use orient::Orientation;
pub(crate) use state::{Crop, MAX_ANGLE};
