// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use eframe::egui;
use image::RgbImage;
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

use super::TinyLumaApp;
use crate::color::oklab::{linear_to_srgb, rgb_to_oklab, srgb_u8_linear_table};
use crate::lut::Lut3D;
use crate::pipeline::color::{ColorSettings, WhiteBalance, apply_chroma, gamut_map_linear};
use crate::pipeline::light::{
    LightSettings, apply_light, contrast_from_slider, exposure_from_slider, unit_from_slider,
};
use crate::settings::FilterSettings;

/// Reference resolution for the detail group (matches the preview cap, 1200px).
///
/// The Sharpen/Texture/Clarity radii are normalized to it, so the preview
/// (≤1200px on the long side) and the full-size export produce the same
/// relative effect.
const DETAIL_WORK_MAX: usize = 1200;
/// Downsample factor for Clarity at the reference resolution.
const CLARITY_SCALE: usize = 8;

/// RGB texture base at working resolution (interleaved storage).
pub(crate) struct TextureBase {
    pub(crate) data: Vec<f32>,
    pub(crate) sw: usize,
    pub(crate) sh: usize,
}

/// Precomputations for the spatial pass, depending only on the input (color/luma) and
/// resolution, but NOT on the Texture/Clarity/Sharpen/Grain slider values.
/// Cached between frames so `gaussian_blur` and the Texture base are not computed
/// again on every slider move.
#[derive(Default)]
pub(crate) struct SpatialBase {
    pub(crate) sharpen_blur: Option<Vec<f32>>,
    pub(crate) texture_base: Option<TextureBase>,
}

impl TinyLumaApp {
    /// Pass 1: color. All edits are per-pixel and baked into the combined LUT.
    ///
    /// Order: sRGB→linear → white balance (Bradford) → Oklab → Light →
    /// Saturation/Vibrance → soft gamut → linear→sRGB → external LUT.
    pub(crate) fn run_color_pass(
        settings: &FilterSettings,
        active_lut: Option<&Lut3D>,
        buffer: &mut [u8],
    ) {
        // Precompute the parameters and the table once for the whole buffer.
        let lin_lut = srgb_u8_linear_table();

        let wb = WhiteBalance::new(settings.temp, settings.tint);

        let light = LightSettings {
            exposure: exposure_from_slider(settings.exposure),
            contrast: contrast_from_slider(settings.contrast),
            highlights: unit_from_slider(settings.highlights),
            shadows: unit_from_slider(settings.shadows),
            whites: unit_from_slider(settings.whites),
            blacks: unit_from_slider(settings.blacks),
        };

        let color = ColorSettings {
            saturation: unit_from_slider(settings.saturation),
            vibrance: unit_from_slider(settings.vibrance),
        };

        let wb_identity = wb.is_identity();
        let light_identity = light.is_identity();
        let chroma_identity = color.saturation.abs() < 1e-6 && color.vibrance.abs() < 1e-6;

        let lut_intensity = settings.lut_intensity / 100.0;

        buffer.par_chunks_mut(3).for_each(|pixel| {
            let mut lin = [
                lin_lut[pixel[0] as usize],
                lin_lut[pixel[1] as usize],
                lin_lut[pixel[2] as usize],
            ];

            if !wb_identity {
                wb.apply_linear(&mut lin);
            }

            let mut p = rgb_to_oklab(lin[0], lin[1], lin[2]);

            if !light_identity {
                apply_light(&mut p, &light);
            }
            if !chroma_identity {
                apply_chroma(&mut p, &color);
            }

            let out = gamut_map_linear(p[0], p[1], p[2]);
            let mut r = linear_to_srgb(out[0]);
            let mut g = linear_to_srgb(out[1]);
            let mut b = linear_to_srgb(out[2]);

            if let Some(lut) = active_lut {
                if lut_intensity > 0.0 {
                    let mapped = lut.apply(r, g, b);
                    r = r * (1.0 - lut_intensity) + mapped[0] * lut_intensity;
                    g = g * (1.0 - lut_intensity) + mapped[1] * lut_intensity;
                    b = b * (1.0 - lut_intensity) + mapped[2] * lut_intensity;
                }
            }

            pixel[0] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            pixel[1] = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            pixel[2] = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        });
    }

    // ==========================================
    // BAKE: 64³ COMBINED LUT (all sliders + external .cube)
    // ==========================================
    pub(crate) fn bake_combined_lut(&self, size: usize) -> Option<Lut3D> {
        Some(Self::bake_lut(&self.settings, self.active_lut.as_deref(), size))
    }

    /// Bakes the combined `size³` LUT (all sliders + external `.cube`).
    ///
    /// Single entry point for the preview (`bake_combined_lut`), export
    /// (`bake_temp_lut`) and benchmarks: all the math goes through
    /// `run_color_pass` over an identity grid.
    pub(crate) fn bake_lut(
        settings: &FilterSettings,
        active_lut: Option<&Lut3D>,
        size: usize,
    ) -> Lut3D {
        let total = size * size * size;
        let mut buffer = vec![0u8; total * 3];

        // Fill the identity grid: the pixel color = its coordinate in the LUT.
        for iz in 0..size {
            for iy in 0..size {
                for ix in 0..size {
                    let idx = (ix + iy * size + iz * size * size) * 3;
                    buffer[idx] = (ix as f32 / (size - 1) as f32 * 255.0) as u8;
                    buffer[idx + 1] = (iy as f32 / (size - 1) as f32 * 255.0) as u8;
                    buffer[idx + 2] = (iz as f32 / (size - 1) as f32 * 255.0) as u8;
                }
            }
        }

        // All the slider math + the external LUT.
        Self::run_color_pass(settings, active_lut, &mut buffer);

        // Convert u8 → f32 Lut3D.
        let mut data = Vec::with_capacity(total);
        for chunk in buffer.chunks_exact(3) {
            data.push([
                chunk[0] as f32 / 255.0,
                chunk[1] as f32 / 255.0,
                chunk[2] as f32 / 255.0,
            ]);
        }

        Lut3D { size, data }
    }

    // ==========================================
    // PASS 2: SPATIAL EFFECTS
    // (Texture, Clarity, Sharpen, Dehaze)
    // ==========================================
    /// Precomputations for the spatial pass (the bilateral Texture base and the luma
    /// blur for Sharpen). Depend ONLY on the input and resolution, but NOT on the
    /// slider values — so they can be cached and reused.
    pub(crate) fn compute_spatial_base(
        settings: &FilterSettings,
        color_buffer: &[u8],
        luma_map: &[f32],
        width: usize,
        height: usize,
    ) -> SpatialBase {
        let detail_scale = Self::detail_scale(width, height);
        let n_texture = settings.texture / 100.0;
        let n_sharpen = settings.sharpen / 100.0;

        // Sharpen: luma blur (separable Gaussian) with a fractional radius
        // proportional to the resolution, so the effect is scale-invariant.
        let sharpen_blur = if n_sharpen > 0.0 {
            Some(Self::gaussian_blur_map(
                luma_map,
                width,
                height,
                detail_scale,
            ))
        } else {
            None
        };

        // Texture: base (bilateral blur) at reduced resolution.
        let texture_base = if n_texture != 0.0 {
            Some(Self::compute_texture_base(
                color_buffer,
                luma_map,
                width,
                height,
                detail_scale,
            ))
        } else {
            None
        };

        SpatialBase {
            sharpen_blur,
            texture_base,
        }
    }

    /// Pass 2 (spatial) that computes its own precomputations.
    /// For one-off calls (export, tests). In the interactive preview
    /// use `run_spatial_pass_with_base` with the cache.
    pub(crate) fn run_spatial_pass(
        settings: &FilterSettings,
        color_buffer: &[u8],
        output: &mut [u8],
        grain_map: &[f32],
        luma_map: &[f32],
        clarity_base: &[f32],
        clarity_sw: usize,
        clarity_sh: usize,
        width: usize,
        height: usize,
    ) {
        let base = Self::compute_spatial_base(settings, color_buffer, luma_map, width, height);
        Self::run_spatial_pass_with_base(
            settings,
            color_buffer,
            output,
            grain_map,
            clarity_base,
            clarity_sw,
            clarity_sh,
            &base,
            width,
            height,
        );
    }

    /// Pass 2 (spatial) on top of the already-prepared `spatial_base` precomputations.
    pub(crate) fn run_spatial_pass_with_base(
        settings: &FilterSettings,
        color_buffer: &[u8],
        output: &mut [u8],
        grain_map: &[f32],
        clarity_base: &[f32],
        clarity_sw: usize,
        clarity_sh: usize,
        spatial_base: &SpatialBase,
        width: usize,
        height: usize,
    ) {
        let s = *settings;

        let n_texture = s.texture / 100.0; // -1.0 .. 1.0
        let n_clarity = s.clarity / 100.0; // -1.0 .. 1.0
        let n_sharpen = s.sharpen / 100.0; // 0.0 .. 1.5

        // Read from color_buffer (stable color_pass result) — WITHOUT clone!
        let color_snapshot: &[u8] = color_buffer;

        output
            .par_chunks_mut(3)
            .enumerate()
            .for_each(|(idx, pixel)| {
                let x = idx % width;
                let y = idx / width;

                let center_idx = idx * 3;
                let mut r = color_snapshot[center_idx] as f32 / 255.0;
                let mut g = color_snapshot[center_idx + 1] as f32 / 255.0;
                let mut b = color_snapshot[center_idx + 2] as f32 / 255.0;

                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;

                // TEXTURE
                if let Some(tex) = &spatial_base.texture_base {
                    // The base was computed at reduced resolution — sample bilinearly.
                    let blurred = Self::sample_texture_base(tex, x, y, width, height);

                    // Soft blend: original + (original − base) * strength.
                    let strength = n_texture * 2.0;
                    r = (r + (r - blurred[0]) * strength).clamp(0.0, 1.0);
                    g = (g + (g - blurred[1]) * strength).clamp(0.0, 1.0);
                    b = (b + (b - blurred[2]) * strength).clamp(0.0, 1.0);
                }

                // ---- CLARITY PRO (Logarithmic Local Contrast) ----
                if n_clarity != 0.0 && !clarity_base.is_empty() {
                    // Take the CURRENT luma (including the already applied Texture)
                    // so the gain is computed from the current state.
                    // Cost ~5 flops/pixel — practically free.
                    // We do not recompute the base (`clarity_base`): that is expensive, and the base is
                    // a low-frequency reference.
                    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    let scale = Self::clarity_scale(width, height);
                    let fx = x as f32 / scale as f32;
                    let fy = y as f32 / scale as f32;

                    let x0 = (fx as usize).min(clarity_sw - 1);
                    let y0 = (fy as usize).min(clarity_sh - 1);
                    let x1 = (x0 + 1).min(clarity_sw - 1);
                    let y1 = (y0 + 1).min(clarity_sh - 1);
                    let wx = fx - x0 as f32;
                    let wy = fy - y0 as f32;

                    let b00 = clarity_base[y0 * clarity_sw + x0];
                    let b10 = clarity_base[y0 * clarity_sw + x1];
                    let b01 = clarity_base[y1 * clarity_sw + x0];
                    let b11 = clarity_base[y1 * clarity_sw + x1];
                    let base_luma = b00 * (1.0 - wx) * (1.0 - wy)
                        + b10 * wx * (1.0 - wy)
                        + b01 * (1.0 - wx) * wy
                        + b11 * wx * wy;

                    let amount = n_clarity;
                    let diff = luma - base_luma;

                    // (1) Soft clipping of the diff
                    let k = 10.0;
                    let safe_diff = diff / (1.0 + diff.abs() * k);

                    // (2) Local-contrast mask (anti-halo)
                    let local_contrast = diff.abs();
                    let edge_mask = 1.0 / (1.0 + local_contrast * 6.0);

                    let punch = 1.0;
                    let mut final_luma = luma + safe_diff * amount * punch * edge_mask;
                    final_luma = final_luma.clamp(0.0, 1.0);

                    let gain = final_luma / luma.max(0.001);
                    r = (r * gain).clamp(0.0, 1.0);
                    g = (g * gain).clamp(0.0, 1.0);
                    b = (b * gain).clamp(0.0, 1.0);
                }

                // ==========================================
                // 📸 SHARPENING (Unsharp Mask)
                // ==========================================
                // Sharpen goes last among the details (after Texture/Clarity),
                // as in darktable/Lightroom: local contrast must not
                // amplify sharpening halos. The high-pass is taken from the source luma
                // (`sharpen_blur` is precomputed), and the brightness ratio
                // is applied to the final RGB.
                if let Some(blur) = &spatial_base.sharpen_blur {
                    let high_pass = luma - blur[idx];

                    let max_lighten = 0.20;
                    let max_darken = 0.12;

                    let limited_hp = if high_pass > 0.0 {
                        high_pass.min(max_lighten)
                    } else {
                        high_pass.max(-max_darken)
                    };

                    let strength = n_sharpen * 4.0;
                    let luma_delta = limited_hp * strength;

                    if luma > 0.001 {
                        let ratio = (luma + luma_delta) / luma;
                        r = (r * ratio).clamp(0.0, 1.0);
                        g = (g * ratio).clamp(0.0, 1.0);
                        b = (b * ratio).clamp(0.0, 1.0);
                    }
                }

                // ==========================================
                // 🎞 FILM GRAIN (darktable grain.c: simplex + paper LUT)
                // ==========================================
                if s.grain > 0.0 {
                    if let Some(&noise) = grain_map.get(idx) {
                        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                        let delta =
                            crate::pipeline::grain::grain_delta(noise, s.grain / 100.0, luma);
                        // Monochrome lightness grain: the same shift in all channels.
                        r = (r + delta).clamp(0.0, 1.0);
                        g = (g + delta).clamp(0.0, 1.0);
                        b = (b + delta).clamp(0.0, 1.0);
                    }
                }

                pixel[0] = (r * 255.0).clamp(0.0, 255.0) as u8;
                pixel[1] = (g * 255.0).clamp(0.0, 255.0) as u8;
                pixel[2] = (b * 255.0).clamp(0.0, 255.0) as u8;
            });
    }

    // ==========================================
    // SAVING

    pub(crate) fn bake_temp_lut(
        settings: &FilterSettings,
        lut_path: &Option<PathBuf>,
    ) -> Option<Lut3D> {
        // Load the LUT from a file if a path is given.
        let active_lut = lut_path
            .as_ref()
            .and_then(|path| Lut3D::load_from_file(path));
        Some(Self::bake_lut(settings, active_lut.as_ref(), 64))
    }

    // ==========================================
    // SHARED HELPERS FOR PREVIEW AND EXPORT
    // ==========================================

    /// Scale of the detail group relative to the reference (preview cap).
    ///
    /// `1.0` for frames ≤ `DETAIL_WORK_MAX` on the long side, then grows
    /// linearly.
    fn detail_scale(width: usize, height: usize) -> f32 {
        (width.max(height) as f32 / DETAIL_WORK_MAX as f32).max(1.0)
    }

    /// Downsample factor for Clarity taking resolution into account.
    fn clarity_scale(width: usize, height: usize) -> usize {
        (CLARITY_SCALE as f32 * Self::detail_scale(width, height))
            .round()
            .max(1.0) as usize
    }

    /// Normalized 1D Gaussian kernel, half-width `radius`, σ = 0.85·radius.
    /// With `radius = 1` this is the classic 1-2-1 kernel.
    fn gaussian_kernel(radius: usize) -> Vec<f32> {
        let radius = radius.max(1);
        let sigma = radius as f32 * 0.85;
        let mut kernel = vec![0.0f32; 2 * radius + 1];
        let mut ksum = 0.0f32;
        for (i, k) in kernel.iter_mut().enumerate() {
            let d = i as f32 - radius as f32;
            *k = (-(d * d) / (2.0 * sigma * sigma)).exp();
            ksum += *k;
        }
        for k in &mut kernel {
            *k /= ksum;
        }
        kernel
    }

    /// Separable convolution of a single-channel map with a 1D `kernel`
    /// (its half-width is `(kernel.len() - 1) / 2`).
    fn convolve_separable(src: &[f32], w: usize, h: usize, kernel: &[f32]) -> Vec<f32> {
        let radius = (kernel.len() - 1) / 2;
        let mut tmp = vec![0.0f32; w * h];
        tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let base = y * w;
            for (x, out) in row.iter_mut().enumerate() {
                let mut acc = 0.0f32;
                for (k, &kv) in kernel.iter().enumerate() {
                    let sx = (x as isize + k as isize - radius as isize).clamp(0, w as isize - 1)
                        as usize;
                    acc += src[base + sx] * kv;
                }
                *out = acc;
            }
        });

        let mut out = vec![0.0f32; w * h];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = 0.0f32;
                for (k, &kv) in kernel.iter().enumerate() {
                    let sy = (y as isize + k as isize - radius as isize).clamp(0, h as isize - 1)
                        as usize;
                    acc += tmp[sy * w + x] * kv;
                }
                *o = acc;
            }
        });
        out
    }

    /// Separable Gaussian blur of a single-channel map with a **fractional**
    /// radius that scales with resolution.
    ///
    /// At integer radii the weights are exactly the legacy kernel (`radius = 1`
    /// → 1-2-1). Between integers the two neighbouring integer kernels are
    /// blended *before* the convolution (valid because convolution is linear),
    /// so the blur grows smoothly instead of in integer steps — this removed the
    /// weak-sharpen dip in the 1.2–1.8 MP range — while still costing a single
    /// separable pass.
    fn gaussian_blur_map(src: &[f32], w: usize, h: usize, radius: f32) -> Vec<f32> {
        let r0 = radius.floor().max(1.0) as usize;
        let frac = (radius - r0 as f32).clamp(0.0, 1.0);
        if frac < 1e-4 {
            return Self::convolve_separable(src, w, h, &Self::gaussian_kernel(r0));
        }
        let k0 = Self::gaussian_kernel(r0);
        let k1 = Self::gaussian_kernel(r0 + 1);
        let half = r0 + 1;
        let mut kernel = vec![0.0f32; 2 * half + 1];
        for (i, k) in kernel.iter_mut().enumerate() {
            let d = i as isize - half as isize;
            let v0 = if d.unsigned_abs() <= r0 {
                k0[(d + r0 as isize) as usize]
            } else {
                0.0
            };
            *k = v0 * (1.0 - frac) + k1[i] * frac;
        }
        Self::convolve_separable(src, w, h, &kernel)
    }

    /// Bilinear sampling of the RGB texture base.
    fn sample_texture_base(base: &TextureBase, x: usize, y: usize, w: usize, h: usize) -> [f32; 3] {
        let (sw, sh) = (base.sw, base.sh);
        let fx = (x as f32 + 0.5) * sw as f32 / w as f32 - 0.5;
        let fy = (y as f32 + 0.5) * sh as f32 / h as f32 - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let xi = |v: f32| (v as isize).clamp(0, sw as isize - 1) as usize;
        let yi = |v: f32| (v as isize).clamp(0, sh as isize - 1) as usize;
        let (xa, xb) = (xi(x0), xi(x0 + 1.0));
        let (ya, yb) = (yi(y0), yi(y0 + 1.0));
        let mut out = [0.0f32; 3];
        for c in 0..3 {
            let v00 = base.data[(ya * sw + xa) * 3 + c];
            let v10 = base.data[(ya * sw + xb) * 3 + c];
            let v01 = base.data[(yb * sw + xa) * 3 + c];
            let v11 = base.data[(yb * sw + xb) * 3 + c];
            let top = v00 * (1.0 - tx) + v10 * tx;
            let bot = v01 * (1.0 - tx) + v11 * tx;
            out[c] = top * (1.0 - ty) + bot * ty;
        }
        out
    }

    /// Texture base: bilateral blur at a reduced (relative to
    /// fixed) resolution.
    ///
    /// This way the effect is the same in preview and export, and the cost does not grow with
    /// resolution. With `scale = 1` (preview) it is exactly the former full-res
    /// bilateral filter of radius 2.
    fn compute_texture_base(
        color: &[u8],
        luma: &[f32],
        w: usize,
        h: usize,
        scale: f32,
    ) -> TextureBase {
        let sw = ((w as f32 / scale).round() as usize).max(1);
        let sh = ((h as f32 / scale).round() as usize).max(1);

        // Box-downsample luma and RGB into the working resolution.
        let sx = w as f32 / sw as f32;
        let sy = h as f32 / sh as f32;
        let mut small_luma = vec![0.0f32; sw * sh];
        let mut small_rgb = vec![0.0f32; sw * sh * 3];
        small_luma
            .par_iter_mut()
            .zip(small_rgb.par_chunks_mut(3))
            .enumerate()
            .for_each(|(i, (lo, rgb))| {
                let dx = i % sw;
                let dy = i / sw;
                let x0 = (dx as f32 * sx) as usize;
                let x1 = (((dx + 1) as f32 * sx) as usize).clamp(x0 + 1, w);
                let y0 = (dy as f32 * sy) as usize;
                let y1 = (((dy + 1) as f32 * sy) as usize).clamp(y0 + 1, h);
                let mut la = 0.0f32;
                let mut acc = [0.0f32; 3];
                let mut cnt = 0.0f32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        la += luma[y * w + x];
                        let b = (y * w + x) * 3;
                        acc[0] += color[b] as f32 / 255.0;
                        acc[1] += color[b + 1] as f32 / 255.0;
                        acc[2] += color[b + 2] as f32 / 255.0;
                        cnt += 1.0;
                    }
                }
                *lo = la / cnt;
                rgb[0] = acc[0] / cnt;
                rgb[1] = acc[1] / cnt;
                rgb[2] = acc[2] / cnt;
            });

        // Bilateral blur on the working grid (radius 2, as before).
        let r = 2isize;
        let mut weights = [0.0f32; 25];
        for dy in -r..=r {
            for dx in -r..=r {
                let dist_sq = (dx * dx + dy * dy) as f32;
                weights[((dy + r) * 5 + (dx + r)) as usize] = (-dist_sq / 2.0).exp();
            }
        }
        let mut out = vec![0.0f32; sw * sh * 3];
        out.par_chunks_mut(3).enumerate().for_each(|(i, o)| {
            let cx = i % sw;
            let cy = i / sw;
            let l_c = small_luma[i];
            let mut acc = [0.0f32; 3];
            let mut wsum = 0.0f32;
            for dy in -r..=r {
                let ny = (cy as isize + dy).clamp(0, sh as isize - 1) as usize;
                for dx in -r..=r {
                    let nx = (cx as isize + dx).clamp(0, sw as isize - 1) as usize;
                    let ni = ny * sw + nx;
                    let diff = (l_c - small_luma[ni]).abs();
                    let range_w = 1.0 / (1.0 + diff * diff * 40.0);
                    let wgt = weights[((dy + r) * 5 + (dx + r)) as usize] * range_w;
                    acc[0] += small_rgb[ni * 3] * wgt;
                    acc[1] += small_rgb[ni * 3 + 1] * wgt;
                    acc[2] += small_rgb[ni * 3 + 2] * wgt;
                    wsum += wgt;
                }
            }
            o[0] = acc[0] / wsum;
            o[1] = acc[1] / wsum;
            o[2] = acc[2] / wsum;
        });

        TextureBase {
            data: out,
            sw,
            sh,
        }
    }

    /// Luma map from an RGB buffer.
    pub(crate) fn compute_luma_map(buffer: &[u8]) -> Vec<f32> {
        buffer
            .par_chunks(3)
            .map(|p| {
                0.2126 * (p[0] as f32 / 255.0)
                    + 0.7152 * (p[1] as f32 / 255.0)
                    + 0.0722 * (p[2] as f32 / 255.0)
            })
            .collect()
    }

    /// Downsample + bilateral blur of lightness for Clarity.
    /// Returns (map, width, height).
    pub(crate) fn compute_clarity_cache(
        luma_map: &[f32],
        w: usize,
        h: usize,
    ) -> (Vec<f32>, usize, usize) {
        // The downsample factor is normalized to resolution (see `clarity_scale`)
        // so the Clarity base has the same frequency in preview and export.
        let scale = Self::clarity_scale(w, h);
        let sw = w / scale + 1;
        let sh = h / scale + 1;

        // 1. Downsample luma_map → small_luma
        let mut small_luma = vec![0.0f32; sw * sh];
        for y in 0..sh {
            for x in 0..sw {
                let mut sum = 0.0f32;
                let mut count = 0.0f32;
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = (x * scale + dx).min(w - 1);
                        let py = (y * scale + dy).min(h - 1);
                        sum += luma_map[py * w + px];
                        count += 1.0;
                    }
                }
                small_luma[y * sw + x] = sum / count;
            }
        }

        // 2. Bilateral blur small_luma → clarity_cache
        let mut clarity_cache = vec![0.0f32; sw * sh];
        let r = 2isize;
        clarity_cache
            .par_iter_mut()
            .enumerate()
            .for_each(|(idx, out)| {
                let cx = (idx % sw) as isize;
                let cy = (idx / sw) as isize;
                let center_luma = small_luma
                    [cy.min(sh as isize - 1) as usize * sw + cx.min(sw as isize - 1) as usize];

                let mut sum = 0.0f32;
                let mut count = 0.0f32;
                for dy in -r..=r {
                    for dx in -r..=r {
                        let nx = (cx + dx).clamp(0, sw as isize - 1) as usize;
                        let ny = (cy + dy).clamp(0, sh as isize - 1) as usize;
                        let neighbor_luma = small_luma[ny * sw + nx];

                        let spatial_w = 1.0 / (1.0 + (dx * dx + dy * dy) as f32 * 0.5);
                        let luma_diff = (center_luma - neighbor_luma).abs();
                        let range_w = 1.0 / (1.0 + luma_diff * luma_diff * 250.0);
                        let w = spatial_w * range_w;
                        sum += neighbor_luma * w;
                        count += w;
                    }
                }
                *out = sum / count;
            });
        (clarity_cache, sw, sh)
    }

    /// Full offline render of an arbitrary image: the color pass
    /// (sliders + external LUT) + the spatial pass (Texture, Clarity,
    /// Sharpening, Grain). Used by export so that a single save
    /// and batch processing produce the SAME result at full resolution.
    pub(crate) fn render_full_image(
        img: &RgbImage,
        settings: &FilterSettings,
        lut_path: &Option<PathBuf>,
    ) -> Vec<u8> {
        let w = img.width() as usize;
        let h = img.height() as usize;
        let mut color_buffer = img.as_raw().clone();

        // ---- Pass 1: color (through the baked LUT) ----
        if let Some(lut) = Self::bake_temp_lut(settings, lut_path) {
            color_buffer.par_chunks_mut(3).for_each(|pixel| {
                let r = pixel[0] as f32 / 255.0;
                let g = pixel[1] as f32 / 255.0;
                let b = pixel[2] as f32 / 255.0;
                let mapped = lut.apply(r, g, b);
                pixel[0] = (mapped[0] * 255.0).clamp(0.0, 255.0) as u8;
                pixel[1] = (mapped[1] * 255.0).clamp(0.0, 255.0) as u8;
                pixel[2] = (mapped[2] * 255.0).clamp(0.0, 255.0) as u8;
            });
        }

        // ---- Pass 2: dehaze (spatial, Dark Channel Prior) ----
        if settings.dehaze != 0.0 {
            let mut dh = vec![0u8; color_buffer.len()];
            crate::pipeline::dehaze::apply(
                &color_buffer,
                &mut dh,
                w,
                h,
                settings.dehaze / 100.0,
            );
            color_buffer = dh;
        }

        let needs_spatial = settings.texture != 0.0
            || settings.clarity != 0.0
            || settings.sharpen > 0.0
            || settings.grain != 0.0;

        if !needs_spatial {
            return color_buffer;
        }

        // ---- Pass 3: spatial effects ----
        let luma_map = Self::compute_luma_map(&color_buffer);
        let (clarity_cache, csw, csh) = if settings.clarity != 0.0 {
            Self::compute_clarity_cache(&luma_map, w, h)
        } else {
            (Vec::new(), 0, 0)
        };
        let grain_map = Self::generate_grain_map(w, h, 42);

        let mut output = vec![0u8; color_buffer.len()];
        Self::run_spatial_pass(
            settings,
            &color_buffer,
            &mut output,
            &grain_map,
            &luma_map,
            &clarity_cache,
            csw,
            csh,
            w,
            h,
        );
        output
    }

    pub(crate) fn process_preview(&mut self, ctx: &egui::Context) {
        let needs_reprocess = self.color_dirty || self.spatial_dirty;
        if !needs_reprocess && !self.full_render_pending {
            return;
        }

        // ==========================================
        // DEBOUNCE: during drag do not render more often than 60fps
        // If full_render_pending — do not debounce (a render is needed after release)
        // ==========================================
        if !self.full_render_pending {
            let now = Instant::now();
            if self.drag_active
                && (now - self.last_render_time) < std::time::Duration::from_millis(16)
            {
                ctx.request_repaint();
                return;
            }
            self.last_render_time = now;
        }
        self.full_render_pending = false;

        let base = match &self.preview_base {
            Some(b) => b,
            None => return,
        };

        let w = base.width() as usize;
        let h = base.height() as usize;
        let base_raw = base.as_raw().to_vec();

        let needs_spatial = self.settings.texture != 0.0
            || self.settings.clarity != 0.0
            || self.settings.sharpen > 0.0
            || self.settings.grain > 0.0;

        // During a COLOR slider drag the spatial pass is not recomputed:
        // this saves ~22 ms/frame, the full render happens on release.
        let skip_spatial_for_drag = self.drag_active && self.color_dirty;
        let needs_spatial = needs_spatial && !skip_spatial_for_drag;

        // ---- Pass 1: Color (through the baked LUT) ----
        // `!color_buffer_valid` — the ready render was taken from the cache, but the color_buffer
        // for subsequent spatial edits is not yet assembled.
        if self.color_dirty || !self.color_buffer_valid {
            // Fast path: default sliders + no LUT = an identity pass.
            // We do not bake an identity LUT — that is a noticeable saving on every frame.
            if self.settings == FilterSettings::default() && self.active_lut.is_none() {
                self.color_buffer.copy_from_slice(&base_raw);
                self.combined_lut = None;
            } else {
                // The preview is always baked at 33³: versus 64³ the mean error is ~1/255 for
                // both, and 64³ only wins in the worst case of clipping —
                // while costing ~18 ms on release. The export stays at 64³
                // (`bake_temp_lut`), so files do not lose quality.
                const PREVIEW_LUT_SIZE: usize = 33;
                self.combined_lut = self.bake_combined_lut(PREVIEW_LUT_SIZE);

                // Apply the LUT to all pixels (trilinear interpolation)
                self.color_buffer.copy_from_slice(&base_raw);
                if let Some(lut) = &self.combined_lut {
                    self.color_buffer.par_chunks_mut(3).for_each(|pixel| {
                        let r = pixel[0] as f32 / 255.0;
                        let g = pixel[1] as f32 / 255.0;
                        let b = pixel[2] as f32 / 255.0;
                        let mapped = lut.apply(r, g, b);
                        pixel[0] = (mapped[0] * 255.0).clamp(0.0, 255.0) as u8;
                        pixel[1] = (mapped[1] * 255.0).clamp(0.0, 255.0) as u8;
                        pixel[2] = (mapped[2] * 255.0).clamp(0.0, 255.0) as u8;
                    });
                } else {
                    // fallback (rare)
                    Self::run_color_pass(
                        &self.settings,
                        self.active_lut.as_deref(),
                        &mut self.color_buffer,
                    );
                }
            }
            self.color_buffer_valid = true;
            self.color_dirty = false;
            self.spatial_dirty = true;
            self.luma_cache_valid = false;
            self.clarity_cache_valid = false;
            self.dehaze_cache_valid = false;
            self.spatial_base_valid = false;
        }

        let has_dehaze = self.settings.dehaze != 0.0;
        let has_dehaze_active = has_dehaze && !skip_spatial_for_drag;
        // During drag the haze map is computed in fast mode, on release in
        // the full one. A mode change also requires a recompute (otherwise on release
        // the fast result would remain).
        let dehaze_fast = self.drag_active;
        let dehaze_changed = self.dehaze_applied != self.settings.dehaze
            || !self.dehaze_cache_valid
            || self.dehaze_cached_fast != dehaze_fast;

        // ---- Dehaze (spatial, Dark Channel Prior) ----
        if has_dehaze_active && dehaze_changed {
            let n = self.color_buffer.len();
            if self.dehazed_buffer.len() != n {
                self.dehazed_buffer.resize(n, 0);
            }
            if dehaze_fast {
                crate::pipeline::dehaze::apply_fast(
                    &self.color_buffer,
                    &mut self.dehazed_buffer,
                    w,
                    h,
                    self.settings.dehaze / 100.0,
                );
            } else {
                crate::pipeline::dehaze::apply(
                    &self.color_buffer,
                    &mut self.dehazed_buffer,
                    w,
                    h,
                    self.settings.dehaze / 100.0,
                );
            }
            self.dehaze_cache_valid = true;
            self.dehaze_applied = self.settings.dehaze;
            self.dehaze_cached_fast = dehaze_fast;
            // The spatial input changed, but during drag the Details precomputations
            // (luma/clarity/texture/sharpen) are NOT recomputed — they catch up
            // on release (where `color_dirty` already resets the caches). The spatial
            // pass itself still runs: the effects are visible, their base just
            // lags during the drag.
            if !dehaze_fast {
                self.luma_cache_valid = false;
                self.clarity_cache_valid = false;
                self.spatial_base_valid = false;
            }
        } else if !has_dehaze_active && self.dehaze_applied != self.settings.dehaze {
            // Dehaze is off (or skipped during drag): the spatial input = color_buffer.
            self.dehaze_applied = self.settings.dehaze;
            self.luma_cache_valid = false;
            self.clarity_cache_valid = false;
            self.spatial_base_valid = false;
        }

        // ---- Recompute luma_cache (only when the spatial input changed) ----
        if self.spatial_dirty && needs_spatial && !self.luma_cache_valid {
            let input: &[u8] = if has_dehaze_active {
                &self.dehazed_buffer
            } else {
                &self.color_buffer
            };
            self.luma_cache = input
                .par_chunks(3)
                .map(|p| {
                    0.2126 * (p[0] as f32 / 255.0)
                        + 0.7152 * (p[1] as f32 / 255.0)
                        + 0.0722 * (p[2] as f32 / 255.0)
                })
                .collect();
            self.luma_cache_valid = true;
        }

        // ---- Recompute clarity_cache (depends on luma_cache, also only on color change) ----
        if self.spatial_dirty
            && needs_spatial
            && self.settings.clarity != 0.0
            && !self.clarity_cache_valid
            && self.luma_cache_valid
        {
            let (cache, sw, sh) = Self::compute_clarity_cache(&self.luma_cache, w, h);
            self.clarity_cache = cache;
            self.clarity_cache_sw = sw;
            self.clarity_cache_sh = sh;
            self.clarity_cache_valid = true;
        }

        // ---- Recompute spatial_base (Texture/Sharpen) — also only on input change.
        // If the effect was off when the base was computed and is now on —
        // compute it: `None` in the cache with a non-zero strength means "needs computing".
        if self.spatial_dirty
            && needs_spatial
            && self.luma_cache_valid
            && (!self.spatial_base_valid
                || (self.settings.texture != 0.0 && self.spatial_base.texture_base.is_none())
                || (self.settings.sharpen > 0.0 && self.spatial_base.sharpen_blur.is_none()))
        {
            let input: &[u8] = if has_dehaze_active {
                &self.dehazed_buffer
            } else {
                &self.color_buffer
            };
            self.spatial_base =
                Self::compute_spatial_base(&self.settings, input, &self.luma_cache, w, h);
            self.spatial_base_valid = true;
        }

        // ---- Pass: Spatial effects ----
        if self.spatial_dirty {
            let input: &[u8] = if has_dehaze_active {
                &self.dehazed_buffer
            } else {
                &self.color_buffer
            };
            if needs_spatial {
                Self::run_spatial_pass_with_base(
                    &self.settings,
                    input,
                    &mut self.processed_pixels,
                    &self.grain_map,
                    &self.clarity_cache,
                    self.clarity_cache_sw,
                    self.clarity_cache_sh,
                    &self.spatial_base,
                    w,
                    h,
                );
            } else {
                if self.processed_pixels.len() != input.len() {
                    self.processed_pixels.resize(input.len(), 0);
                }
                self.processed_pixels.copy_from_slice(input);
            }
            self.spatial_dirty = false;
        }

        // GPU upload
        self.upload_preview_textures(ctx);

        // Store the ready render in the cache (final only, not during drag).
        // The identity pass (default + no LUT) is not cached: the result equals base,
        // and a separate Vec would only take up budget. On return such a
        // frame is recomputed with two copies (cheap).
        let is_identity = self.settings == FilterSettings::default() && self.lut_path.is_none();
        if !self.drag_active && !is_identity {
            if let Some(path) = self.image_path.clone() {
                self.preview_cache.store_render(
                    &path,
                    self.settings,
                    self.lut_path.clone(),
                    self.processed_pixels.clone(),
                );
            }
        }
    }

    /// Uploads `processed_pixels` and the base frame into GPU textures.
    pub(crate) fn upload_preview_textures(&mut self, ctx: &egui::Context) {
        let (w, h) = match self.preview_base.as_ref() {
            Some(b) => (b.width() as usize, b.height() as usize),
            None => return,
        };

        let color_image = egui::ColorImage::from_rgb([w, h], &self.processed_pixels);
        if let Some(tex) = &mut self.texture {
            tex.set(color_image, egui::TextureOptions::LINEAR);
        } else {
            self.texture =
                Some(ctx.load_texture("preview", color_image, egui::TextureOptions::LINEAR));
        }

        let orig_image = {
            let base = self.preview_base.as_ref().unwrap();
            egui::ColorImage::from_rgb([w, h], base.as_raw())
        };
        if let Some(tex) = &mut self.original_texture {
            tex.set(orig_image, egui::TextureOptions::LINEAR);
        } else {
            self.original_texture =
                Some(ctx.load_texture("original", orig_image, egui::TextureOptions::LINEAR));
        }
    }
}

// ==========================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_roundtrip() {
        let mut buf: Vec<u8> = (0u8..=255).flat_map(|v| [v, 255 - v, v / 2]).collect();
        let orig = buf.clone();
        TinyLumaApp::run_color_pass(&FilterSettings::default(), None, &mut buf);
        for (a, b) in buf.iter().zip(orig.iter()) {
            assert!((*a as i32 - *b as i32).abs() <= 1, "changed {a} vs {b}");
        }
    }

    #[test]
    fn exposure_brightens() {
        let mut buf = vec![64u8, 64, 64];
        let mut s = FilterSettings::default();
        s.exposure = 1.0;
        TinyLumaApp::run_color_pass(&s, None, &mut buf);
        assert!(buf[0] > 64, "not brightened: {}", buf[0]);
    }

    #[test]
    fn warm_shifts_neutral_to_red() {
        let mut buf = vec![128u8, 128, 128];
        let mut s = FilterSettings::default();
        s.temp = 80.0;
        TinyLumaApp::run_color_pass(&s, None, &mut buf);
        assert!(buf[0] > buf[2], "not warm: {buf:?}");
    }

    #[test]
    fn saturation_minus_one_is_grey() {
        let mut buf = vec![200u8, 60, 20];
        let mut s = FilterSettings::default();
        s.saturation = -100.0;
        TinyLumaApp::run_color_pass(&s, None, &mut buf);
        let (r, g, b) = (buf[0] as i32, buf[1] as i32, buf[2] as i32);
        assert!((r - g).abs() <= 1 && (g - b).abs() <= 1, "not grey: {buf:?}");
    }

    #[test]
    fn highlights_plus_brightens_brights() {
        let mut buf = vec![230u8, 230, 230];
        let mut s = FilterSettings::default();
        s.highlights = 60.0;
        TinyLumaApp::run_color_pass(&s, None, &mut buf);
        assert!(buf[0] > 230, "highlights+ should brighten: {}", buf[0]);
    }

    #[test]
    fn highlights_minus_recovers_brights() {
        let mut buf = vec![230u8, 230, 230];
        let mut s = FilterSettings::default();
        s.highlights = -60.0;
        TinyLumaApp::run_color_pass(&s, None, &mut buf);
        assert!(buf[0] < 230, "highlights- should recover: {}", buf[0]);
    }

    #[test]
    fn detail_scale_is_one_for_preview_sizes() {
        assert!((TinyLumaApp::detail_scale(1200, 800) - 1.0).abs() < 1e-6);
        assert!((TinyLumaApp::detail_scale(800, 600) - 1.0).abs() < 1e-6);
        assert!((TinyLumaApp::detail_scale(6000, 4000) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn clarity_scale_tracks_resolution() {
        assert_eq!(TinyLumaApp::clarity_scale(1200, 800), 8);
        assert_eq!(TinyLumaApp::clarity_scale(6000, 4000), 40);
    }

    #[test]
    fn gaussian_blur_radius1_is_binomial() {
        // With radius = 1 the weights must match the 1-2-1 kernel.
        let (w, h) = (5usize, 1usize);
        let mut src = vec![0.0f32; w * h];
        src[2] = 1.0;
        let out = TinyLumaApp::gaussian_blur_map(&src, w, h, 1.0);
        assert!((out[1] - 0.25).abs() < 5e-3, "left {}", out[1]);
        assert!((out[2] - 0.5).abs() < 5e-3, "center {}", out[2]);
        assert!((out[3] - 0.25).abs() < 5e-3, "right {}", out[3]);
    }

    #[test]
    fn gaussian_blur_fractional_radius_interpolates() {
        // A fractional radius must spread strictly between the two neighbouring
        // integer radii — the old `round()` made it stick to one of them, which
        // caused the weak-sharpen dip around 1.2–1.8 MP.
        let (w, h) = (9usize, 1usize);
        let mut src = vec![0.0f32; w * h];
        src[4] = 1.0;
        let spread = |b: &[f32]| -> f32 {
            b.iter()
                .enumerate()
                .map(|(i, v)| (i as f32 - 4.0).abs() * v)
                .sum()
        };
        let b1 = TinyLumaApp::gaussian_blur_map(&src, w, h, 1.0);
        let b2 = TinyLumaApp::gaussian_blur_map(&src, w, h, 2.0);
        let bf = TinyLumaApp::gaussian_blur_map(&src, w, h, 1.5);
        let (s1, sf, s2) = (spread(&b1), spread(&bf), spread(&b2));
        assert!(s1 < sf && sf < s2, "spread {s1} < {sf} < {s2}");
    }

    #[test]
    fn texture_base_is_constant_on_flat_image() {
        // On a flat frame the texture base equals the color itself at any scale.
        let (w, h) = (16usize, 16usize);
        let color: Vec<u8> = (0..w * h).flat_map(|_| [80u8, 120, 160]).collect();
        let luma: Vec<f32> = (0..w * h)
            .map(|_| 0.2126 * 80.0 / 255.0 + 0.7152 * 120.0 / 255.0 + 0.0722 * 160.0 / 255.0)
            .collect();
        for &scale in &[1.0f32, 2.5] {
            let base = TinyLumaApp::compute_texture_base(&color, &luma, w, h, scale);
            let expected = [80.0 / 255.0, 120.0 / 255.0, 160.0 / 255.0];
            for c in 0..3 {
                assert!(
                    (base.data[c] - expected[c]).abs() < 1e-3,
                    "scale {scale} ch {c}: {} vs {}",
                    base.data[c],
                    expected[c]
                );
            }
        }
    }

    /// The cached spatial precomputations do not change the result: the pass with a ready
    /// `SpatialBase` is byte-identical to the pass that computes the base itself.
    #[test]
    fn spatial_base_cache_is_bit_identical() {
        let (w, h) = (128usize, 96usize);
        let color: Vec<u8> = (0..w * h * 3).map(|i| ((i * 7 + i / 11) % 256) as u8).collect();
        let luma = TinyLumaApp::compute_luma_map(&color);
        let (clarity, sw, sh) = TinyLumaApp::compute_clarity_cache(&luma, w, h);
        let grain = vec![0.0f32; w * h];

        let mut s = FilterSettings::default();
        s.texture = 40.0;
        s.clarity = 30.0;
        s.sharpen = 60.0;
        s.grain = 20.0;

        let mut direct = vec![0u8; w * h * 3];
        TinyLumaApp::run_spatial_pass(
            &s, &color, &mut direct, &grain, &luma, &clarity, sw, sh, w, h,
        );

        let base = TinyLumaApp::compute_spatial_base(&s, &color, &luma, w, h);
        let mut cached = vec![0u8; w * h * 3];
        TinyLumaApp::run_spatial_pass_with_base(
            &s, &color, &mut cached, &grain, &clarity, sw, sh, &base, w, h,
        );

        assert_eq!(direct, cached, "cached spatial diverged from direct");
    }

    /// A base captured at one effect strength works for any other strength:
    /// the Texture/Sharpen precomputations do not depend on the slider values.
    #[test]
    fn spatial_base_is_strength_independent() {
        let (w, h) = (96usize, 64usize);
        let color: Vec<u8> = (0..w * h * 3).map(|i| ((i * 13 + 5) % 256) as u8).collect();
        let luma = TinyLumaApp::compute_luma_map(&color);
        let (clarity, sw, sh) = TinyLumaApp::compute_clarity_cache(&luma, w, h);
        let grain = vec![0.0f32; w * h];

        // The base is captured at one set of strengths and then reused at others.
        let mut base_settings = FilterSettings::default();
        base_settings.texture = 50.0;
        base_settings.sharpen = 50.0;
        let base = TinyLumaApp::compute_spatial_base(&base_settings, &color, &luma, w, h);

        for (tex, sharp) in [(10.0f32, 10.0f32), (80.0, 120.0), (-40.0, 0.0), (0.0, 90.0)] {
            let mut s = FilterSettings::default();
            s.texture = tex;
            s.clarity = 25.0;
            s.sharpen = sharp;
            s.grain = 15.0;

            let mut direct = vec![0u8; w * h * 3];
            TinyLumaApp::run_spatial_pass(
                &s, &color, &mut direct, &grain, &luma, &clarity, sw, sh, w, h,
            );

            let mut cached = vec![0u8; w * h * 3];
            TinyLumaApp::run_spatial_pass_with_base(
                &s, &color, &mut cached, &grain, &clarity, sw, sh, &base, w, h,
            );

            assert_eq!(direct, cached, "tex={tex} sharp={sharp}");
        }
    }

    /// Spatial pass budget (Sharpen+Texture+Clarity) at different
    /// resolutions — a check that radius normalization did not kill performance.
    /// `cargo test --release -- --ignored bench_spatial_pass --nocapture`
    #[test]
    #[ignore]
    fn bench_spatial_pass() {
        use std::time::Instant;
        for (w, h) in [(1200usize, 800usize), (4000, 3000)] {
            let n = w * h;
            let color: Vec<u8> = (0..n * 3).map(|i| ((i * 7 + i / 13) % 256) as u8).collect();
            let luma = TinyLumaApp::compute_luma_map(&color);
            let (clarity, csw, csh) = TinyLumaApp::compute_clarity_cache(&luma, w, h);
            let grain = vec![0.0f32; n];
            let mut out = vec![0u8; n * 3];
            let mut s = FilterSettings::default();
            s.texture = 50.0;
            s.clarity = 50.0;
            s.sharpen = 50.0;
            let t = Instant::now();
            TinyLumaApp::run_spatial_pass(
                &s, &color, &mut out, &grain, &luma, &clarity, csw, csh, w, h,
            );
            println!(
                "spatial {}x{}: {:.1} ms",
                w,
                h,
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// LUT bake budget: how long `run_color_pass` takes on the identity grid
    /// with aggressive edits (many out-of-gamut pixels through gamut_map).
    /// Run `cargo test --release -- --ignored bench_bake_lut --nocapture`.
    #[test]
    #[ignore]
    fn bench_bake_lut() {
        use std::time::Instant;
        let mut s = FilterSettings::default();
        s.saturation = 60.0;
        s.vibrance = 40.0;
        s.temp = 30.0;
        s.contrast = 40.0;
        for size in [33usize, 64] {
            let total = size * size * size;
            let mut buf = vec![0u8; total * 3];
            for iz in 0..size {
                for iy in 0..size {
                    for ix in 0..size {
                        let idx = (ix + iy * size + iz * size * size) * 3;
                        buf[idx] = (ix as f32 / (size - 1) as f32 * 255.0) as u8;
                        buf[idx + 1] = (iy as f32 / (size - 1) as f32 * 255.0) as u8;
                        buf[idx + 2] = (iz as f32 / (size - 1) as f32 * 255.0) as u8;
                    }
                }
            }
            let t = Instant::now();
            TinyLumaApp::run_color_pass(&s, None, &mut buf);
            println!(
                "bake {}^3 ({} points): {:.2} ms",
                size,
                total,
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
