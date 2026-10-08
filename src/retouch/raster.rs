// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The only u8↔f32 glue for retouch.
//!
//! Keeping the index math in one place avoids copy-pasted pixel addressing
//! across the algorithms.

use image::RgbImage;

/// A rectangle in image pixels (top-left + size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) w: usize,
    pub(crate) h: usize,
}

/// A rectangle of interleaved f32 channels (row-major): `data[(y*w + x)*ch + c]`.
pub(crate) struct FloatRegion {
    pub(crate) rect: Rect,
    pub(crate) ch: usize,
    pub(crate) data: Vec<f32>,
}

impl FloatRegion {
    /// Reads `rect` out of an 8-bit RGB image and normalizes to `0.0..=1.0`.
    pub(crate) fn from_rgb8(img: &RgbImage, rect: Rect) -> Self {
        let mut data = Vec::with_capacity(rect.w * rect.h * 3);
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let p = img.get_pixel(x as u32, y as u32).0;
                data.push(p[0] as f32 / 255.0);
                data.push(p[1] as f32 / 255.0);
                data.push(p[2] as f32 / 255.0);
            }
        }
        Self { rect, ch: 3, data }
    }

    #[inline]
    pub(crate) fn idx(&self, x: usize, y: usize) -> usize {
        (y * self.rect.w + x) * self.ch
    }
}

impl FloatRegion {
    /// Writes the region back into the image, clamped and rounded to u8.
    pub(crate) fn write_rgb8(&self, img: &mut RgbImage) {
        for y in 0..self.rect.h {
            for x in 0..self.rect.w {
                let i = self.idx(x, y);
                let px = img.get_pixel_mut((self.rect.x + x) as u32, (self.rect.y + y) as u32);
                for c in 0..3 {
                    px.0[c] = (self.data[i + c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }
}
