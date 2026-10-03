// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Image-processing passes (tonal and color operations).
//!
//! This module holds the "pure" per-pixel functions that are later baked into
//! the application's combined 3D LUT (`run_color_pass`/`bake_combined_lut`).

pub(crate) mod color;
pub(crate) mod dehaze;
pub(crate) mod grain;
pub(crate) mod light;
