// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! The only u8↔f32 glue for retouch.
//!
//! The algorithms (averaging, Poisson blending, patch voting) are physically
//! meaningful in **linear light**, but the base image is 8-bit **sRGB**. The
//! conversion happens here, at the single boundary, so nothing below this file
//! has to care: `from_rgb8` decodes sRGB → linear, and `write_rgb8` encodes
//! linear → sRGB again.
//!
//! Keeping the index math in one place avoids copy-pasted pixel addressing
//! across the algorithms.

use image::RgbImage;

/// sRGB → linear light (IEC 61966-2-1 piecewise curve).
#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light → sRGB (inverse of [`srgb_to_linear`]).
#[inline]
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

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
    /// Reads `rect` out of an 8-bit sRGB image into **linear light** `0.0..=1.0`.
    pub(crate) fn from_rgb8(img: &RgbImage, rect: Rect) -> Self {
        let mut data = Vec::with_capacity(rect.w * rect.h * 3);
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let p = img.get_pixel(x as u32, y as u32).0;
                for c in 0..3 {
                    data.push(srgb_to_linear(p[c] as f32 / 255.0));
                }
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
    /// Encodes the region from linear light back to 8-bit sRGB.
    pub(crate) fn write_rgb8(&self, img: &mut RgbImage) {
        for y in 0..self.rect.h {
            for x in 0..self.rect.w {
                let i = self.idx(x, y);
                let px = img.get_pixel_mut((self.rect.x + x) as u32, (self.rect.y + y) as u32);
                for c in 0..3 {
                    let v = linear_to_srgb(self.data[i + c].clamp(0.0, 1.0));
                    px.0[c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sRGB round-trip is (near) lossless for every 8-bit input.
    #[test]
    fn srgb_round_trip_is_lossless() {
        for v in 0u16..=255 {
            let s = v as f32 / 255.0;
            let back = linear_to_srgb(srgb_to_linear(s));
            let q = (back * 255.0 + 0.5) as u16;
            assert_eq!(q, v, "sRGB round-trip broke at {v}");
        }
    }

    /// Reading then writing the same region must reproduce the source bytes.
    #[test]
    fn region_round_trip_is_identity() {
        let (w, h) = (16u32, 9u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(x, y, image::Rgb([(x * 17) as u8, (y * 29) as u8, (x * y) as u8]));
            }
        }
        let rect = Rect { x: 0, y: 0, w: w as usize, h: h as usize };
        let region = FloatRegion::from_rgb8(&img, rect);
        let mut out = img.clone();
        region.write_rgb8(&mut out);
        assert_eq!(out.into_raw(), img.into_raw());
    }
}
