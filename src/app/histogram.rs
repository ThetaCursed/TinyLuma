// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! Output histogram of the rendered preview and the clipping counts that feed
//! the warning indicators.
//!
//! Pure: it only reads an interleaved RGB8 buffer, so it is unit-testable and
//! knows nothing about egui or the pipeline. References: RAWmakase's
//! `rendered.rs` (the model and thresholds) and lightcraft's
//! `raster/histogram.rs` (the sampling stride).

/// A channel at or above this value counts as clipped in the highlights, and at
/// or below [`SHADOW_CLIP`] in the shadows. These are rendered output bytes
/// (encoded sRGB), so the display profile never changes what counts as clipped.
pub(crate) const HIGHLIGHT_CLIP: u8 = 254;
/// See [`HIGHLIGHT_CLIP`].
pub(crate) const SHADOW_CLIP: u8 = 1;

/// Per channel, how many (sampled) pixels clip at each end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Clipped {
    pub(crate) shadows: [u32; 3],
    pub(crate) highlights: [u32; 3],
}

/// 256 bins per channel of the rendered output, plus the clipped counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Histogram {
    pub(crate) bins: [[u32; 256]; 3],
    pub(crate) clipped: Clipped,
    /// Number of pixels counted (after sampling).
    pub(crate) total: u32,
}

impl Histogram {
    pub(crate) const EMPTY: Self = Self {
        bins: [[0; 256]; 3],
        clipped: Clipped {
            shadows: [0; 3],
            highlights: [0; 3],
        },
        total: 0,
    };

    /// Counts an interleaved RGB8 buffer. To stay cheap on large previews the
    /// image is sampled with a stride that keeps the count near 1 MP; a preview
    /// is ≤1200 px on the long side, so the stride is usually 1.
    pub(crate) fn of_rgb8(rgb: &[u8]) -> Self {
        let pixels = rgb.len() / 3;
        let mut h = Self::EMPTY;
        if pixels == 0 {
            return h;
        }
        let step = ((pixels as f64 / 1_000_000.0).sqrt().ceil() as usize).max(1);
        for (i, p) in rgb.chunks_exact(3).enumerate() {
            if step > 1 && i % step != 0 {
                continue;
            }
            for (c, &v) in p.iter().enumerate() {
                h.bins[c][v as usize] += 1;
                if v >= HIGHLIGHT_CLIP {
                    h.clipped.highlights[c] += 1;
                }
                if v <= SHADOW_CLIP {
                    h.clipped.shadows[c] += 1;
                }
            }
            h.total += 1;
        }
        h
    }

    /// Fraction of the counted pixels clipped at black / white, the worst
    /// channel in each case (`0..1`). This drives the warning triangles.
    pub(crate) fn clipping(&self) -> (f32, f32) {
        if self.total == 0 {
            return (0.0, 0.0);
        }
        let t = self.total as f32;
        let lo = self.clipped.shadows.iter().copied().max().unwrap_or(0) as f32 / t;
        let hi = self
            .clipped
            .highlights
            .iter()
            .copied()
            .max()
            .unwrap_or(0) as f32
            / t;
        (lo, hi)
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bins_and_clipping() {
        // black, white, mid, near-white
        let rgb = [
            0, 0, 0, //
            255, 255, 255, //
            128, 128, 128, //
            254, 254, 254,
        ];
        let h = Histogram::of_rgb8(&rgb);
        assert_eq!(h.total, 4);
        assert_eq!(h.bins[0][0], 1);
        assert_eq!(h.bins[0][128], 1);
        assert_eq!(h.bins[0][255], 1);
        // 0 clips shadows; 254 and 255 clip highlights.
        assert_eq!(h.clipped.shadows, [1, 1, 1]);
        assert_eq!(h.clipped.highlights, [2, 2, 2]);
        let (lo, hi) = h.clipping();
        assert!((lo - 0.25).abs() < 1e-6, "{lo}");
        assert!((hi - 0.5).abs() < 1e-6, "{hi}");
    }

    #[test]
    fn empty_buffer_is_empty() {
        let h = Histogram::of_rgb8(&[]);
        assert_eq!(h.total, 0);
        assert_eq!(h.clipping(), (0.0, 0.0));
    }

    #[test]
    fn samples_large_images() {
        let rgb = vec![128u8; 1500 * 1500 * 3];
        let h = Histogram::of_rgb8(&rgb);
        assert!(h.total > 0);
        // 2.25 MP → stride 2 → about a quarter of the pixels counted.
        assert!(h.total < 1500 * 1500, "{}", h.total);
        assert_eq!(h.bins[1][128], h.total);
    }
}
