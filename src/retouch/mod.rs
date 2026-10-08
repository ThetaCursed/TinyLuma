// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Pure retouch layer.
//!
//! This module knows nothing about egui, the app state or the file system: it
//! only takes pixel buffers (f32) and masks and returns buffers. That keeps
//! every algorithm unit-testable on tiny synthetic images.
//!
//! See `docs/RETOUCH.md` for the design.

pub(crate) mod brush;
pub(crate) mod inpaint;
pub(crate) mod poisson;
pub(crate) mod raster;
pub(crate) mod spot;

pub(crate) use brush::BrushSettings;
#[allow(unused_imports)]
pub(crate) use spot::{RetouchLayer, Spot, SpotKind};
