// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use super::TinyLumaApp;
use crate::crop::Crop;
use crate::retouch::RetouchLayer;
use crate::settings::{FilterSettings, SaveFormat, SaveSettings};

/// Background batch export event.
pub(crate) enum ExportEvent {
    /// One more file processed (`done` out of `total`), `name` — the file name.
    Progress {
        done: usize,
        total: usize,
        name: String,
    },
    /// Export finished (or cancelled).
    Done {
        saved: usize,
        errors: Vec<String>,
        cancelled: bool,
    },
}

/// Batch export job. Fully owns its data, so it can be safely
/// moved into a background thread (it holds no references to UI state).
pub(crate) struct ExportJob {
    pub(crate) items: Vec<(PathBuf, FilterSettings, Option<PathBuf>, RetouchLayer, Crop)>,
    pub(crate) output_dir: PathBuf,
    pub(crate) format: SaveFormat,
    pub(crate) quality: u8,
    pub(crate) postfix: String,
    pub(crate) embed_png_metadata: bool,
}

/// Runs a batch export on a background thread. Processes frames
/// sequentially (to avoid multiplying the peak memory of a full-res render), after
/// each one sends progress and checks the cancellation flag.
pub(crate) fn run_export(job: ExportJob, tx: Sender<ExportEvent>, cancel: Arc<AtomicBool>) {
    let ExportJob {
        items,
        output_dir,
        format,
        quality,
        postfix,
        embed_png_metadata,
    } = job;

    let total = items.len();
    let ext = match format {
        SaveFormat::Png => "png",
        SaveFormat::Jpg => "jpg",
        SaveFormat::WebP => "webp",
    };

    let mut saved = 0usize;
    let mut errors = Vec::new();

    for (i, (path, settings, lut_path, layer, crop)) in items.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(ExportEvent::Done {
                saved,
                errors,
                cancelled: true,
            });
            return;
        }

        let name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();

        match image::open(&path) {
            Ok(img) => {
                let rgb = img.to_rgb8();
                let (w, h) = rgb.dimensions();
                // Full render of the original: color + spatial effects.
                let raw = TinyLumaApp::render_full_image(&rgb, &settings, &lut_path, &layer);
                // Orientation, straighten, then crop (the last steps).
                let (raw, w, h) = crop.orientation.apply_rgb(&raw, w, h);
                let (raw, w, h) = crate::crop::pixel::rotate_rgb(&raw, w, h, crop.angle);
                let (raw, w, h) = crate::crop::pixel::crop_rgb(&raw, w, h, crop.rect);

                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let out_name = format!("{}{}.{}", stem, postfix, ext);
                let out_path = output_dir.join(&out_name);

                // Metadata is carried over from each source separately.
                let metadata = if format == SaveFormat::Png && embed_png_metadata {
                    read_png_metadata_chunks(&path)
                } else {
                    Vec::new()
                };

                if write_image_file(&out_path, format, quality, &raw, w, h, &metadata) {
                    saved += 1;
                } else {
                    errors.push(out_name);
                }
            }
            Err(_) => errors.push(path.to_string_lossy().to_string()),
        }

        if tx
            .send(ExportEvent::Progress {
                done: i + 1,
                total,
                name,
            })
            .is_err()
        {
            // The receiver was dropped (window closed) — stop working.
            return;
        }
    }

    let _ = tx.send(ExportEvent::Done {
        saved,
        errors,
        cancelled: false,
    });
}

/// Single image writing routine to disk (used by all export paths).
/// `png_metadata` — raw PNG ancillary chunks (length+type+data+crc) that
/// need to be carried over from the original; applied to PNG only.
pub(crate) fn write_image_file(
    path: &PathBuf,
    format: SaveFormat,
    quality: u8,
    raw: &[u8],
    w: u32,
    h: u32,
    png_metadata: &[Vec<u8>],
) -> bool {
    if format == SaveFormat::Png && !png_metadata.is_empty() {
        return write_png_with_metadata(path, raw, w, h, png_metadata);
    }

    match format {
        SaveFormat::Png => {
            let Ok(file) = File::create(path) else {
                return false;
            };
            PngEncoder::new(BufWriter::new(file))
                .write_image(raw, w, h, ExtendedColorType::Rgb8)
                .is_ok()
        }
        SaveFormat::Jpg => {
            let Ok(file) = File::create(path) else {
                return false;
            };
            JpegEncoder::new_with_quality(BufWriter::new(file), quality)
                .encode(raw, w, h, ExtendedColorType::Rgb8)
                .is_ok()
        }
        // Lossy WebP (VP8) through libwebp: the `image` crate encoder is
        // lossless-only, so the quality slider is applied via the `webp` binding.
        SaveFormat::WebP => {
            let encoder = webp::Encoder::from_rgb(raw, w, h);
            let memory = encoder.encode(quality as f32);
            std::fs::write(path, &*memory).is_ok()
        }
    }
}

/// PNG chunks that depend on the source's color type/palette and can make
/// the RGB8 output invalid. They carry no useful metadata — skip them.
const PNG_STRUCTURAL_CHUNKS: [[u8; 4]; 4] = [*b"tRNS", *b"hIST", *b"sBIT", *b"bKGD"];

/// Standard ancillary chunks we carry over deliberately (including ones with
/// the "unsafe-to-copy" bit): color profiles, text, EXIF, physical dimensions, time.
const PNG_KNOWN_COPYABLE_CHUNKS: [[u8; 4]; 11] = [
    *b"cHRM", *b"gAMA", *b"iCCP", *b"sRGB", *b"tEXt", *b"zTXt", *b"iTXt", *b"eXIf", *b"pHYs",
    *b"sPLT", *b"tIME",
];

/// Whether a chunk can be carried over safely:
/// - ancillary only (first letter lowercase);
/// - not structural (does not depend on the color type/palette);
/// - either from the whitelist, or with the safe-to-copy bit set (4th letter lowercase).
///
/// The last one matters: unknown chunks with an uppercase 4th letter are tied to
/// the source pixels (APNG `fdAT`, `dSIG`, etc.) and would become invalid in the
/// edited file — they must not be copied.
fn png_chunk_copyable(ctype: &[u8; 4]) -> bool {
    if ctype[0] & 0x20 == 0 {
        return false; // critical (IHDR/PLTE/IDAT/IEND etc.)
    }
    if PNG_STRUCTURAL_CHUNKS.iter().any(|s| s == ctype) {
        return false;
    }
    if PNG_KNOWN_COPYABLE_CHUNKS.iter().any(|s| s == ctype) {
        return true;
    }
    ctype[3] & 0x20 != 0
}

/// Splits a PNG into chunks. Returns the offset right after IHDR and a list
/// of `(type, start, end)` — without copying data (IDAT is not duplicated).
fn parse_png_chunks(bytes: &[u8]) -> Option<(usize, Vec<([u8; 4], usize, usize)>)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || bytes[..8] != SIG {
        return None;
    }

    let mut pos = 8usize;
    let mut after_ihdr = None;
    let mut chunks = Vec::new();

    loop {
        if pos + 12 > bytes.len() {
            break;
        }
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let Some(end) = pos.checked_add(12).and_then(|e| e.checked_add(len)) else {
            break;
        };
        if end > bytes.len() {
            break;
        }

        let ctype = [
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ];
        if &ctype == b"IHDR" {
            after_ihdr = Some(end);
        }
        let is_end = &ctype == b"IEND";
        chunks.push((ctype, pos, end));
        if is_end {
            break;
        }
        pos = end;
    }

    Some((after_ihdr?, chunks))
}

/// Reads all ancillary PNG chunks (text/EXIF/ICC/pHYs/private, etc.),
/// except the structurally dependent ones. An empty vector means the file is not PNG or has no metadata.
pub(crate) fn read_png_metadata_chunks(path: &Path) -> Vec<Vec<u8>> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Some((_, chunks)) = parse_png_chunks(&bytes) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (ctype, start, end) in chunks {
        if png_chunk_copyable(&ctype) {
            out.push(bytes[start..end].to_vec());
        }
    }
    out
}

/// Inserts chunks right after IHDR, without duplicating types already present
/// in the output PNG. CRC is not recomputed — chunks are carried over byte-for-byte.
fn inject_png_chunks(png: Vec<u8>, extra: &[Vec<u8>]) -> Vec<u8> {
    if extra.is_empty() {
        return png;
    }
    let Some((after_ihdr, chunks)) = parse_png_chunks(&png) else {
        return png;
    };

    let mut insert = Vec::new();
    for raw in extra {
        if raw.len() < 8 {
            continue;
        }
        let ctype = [raw[4], raw[5], raw[6], raw[7]];
        if chunks.iter().any(|(t, _, _)| *t == ctype) {
            continue;
        }
        insert.extend_from_slice(raw);
    }
    if insert.is_empty() {
        return png;
    }

    let mut out = Vec::with_capacity(png.len() + insert.len());
    out.extend_from_slice(&png[..after_ihdr]);
    out.extend_from_slice(&insert);
    out.extend_from_slice(&png[after_ihdr..]);
    out
}

/// Encodes a PNG into memory, adds metadata and writes the file.
fn write_png_with_metadata(
    path: &Path,
    raw: &[u8],
    w: u32,
    h: u32,
    png_metadata: &[Vec<u8>],
) -> bool {
    let mut buf = Vec::new();
    if PngEncoder::new(&mut buf)
        .write_image(raw, w, h, ExtendedColorType::Rgb8)
        .is_err()
    {
        return false;
    }
    let buf = inject_png_chunks(buf, png_metadata);
    std::fs::write(path, buf).is_ok()
}

impl TinyLumaApp {
    /// Loads the persisted config. A missing or corrupt file yields defaults, so
    /// the caller never has to special-case the first run or a hand-edited JSON.
    pub(crate) fn load_save_settings() -> SaveSettings {
        let path = PathBuf::from("config/save_settings.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<SaveSettings>(&content) {
                println!(
                    "📂 Settings loaded: {:?}, quality {}, split {:.1}%",
                    settings.format,
                    settings.quality,
                    settings.split_position * 100.0
                );
                return settings;
            }
        }
        SaveSettings::default()
    }

    // Save settings to JSON
    pub(crate) fn save_save_settings(&self) {
        let settings = SaveSettings {
            format: self.save_format,
            quality: self.save_quality,
            last_input_dir: Some(self.last_input_dir.to_string_lossy().to_string())
                .filter(|s| !s.is_empty()),
            last_output_dir: Some(self.last_output_dir.to_string_lossy().to_string())
                .filter(|s| !s.is_empty()),
            split_position: self.split_position,
            open_groups: self.open_groups,
            open_curve_group: self.open_curve_group,
            open_mixer_group: self.open_mixer_group,
            postfix: self.batch_postfix.clone(),
            embed_png_metadata: self.embed_png_metadata,
            lut_favorites_only: self.lut_favorites_only,
            open_lut_categories: self.open_lut_categories.clone(),
            brush_size: self.retouch.brush.size,
            brush_hardness: self.retouch.brush.hardness,
        };
        if let Ok(json) = serde_json::to_string_pretty(&settings) {
            let _ = std::fs::write(&self.save_settings_path, json);
        }
    }

    /// Single save: render the FULL image from the original with all
    /// effects (color + spatial) — exactly like the batch export.
    pub(crate) fn save_current_preview(
        &self,
        save_path: &PathBuf,
        format: SaveFormat,
        quality: u8,
    ) -> bool {
        let Some(path) = self.image_path.as_ref() else {
            return false;
        };
        let Ok(img) = image::open(path) else {
            return false;
        };
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();

        let raw = Self::render_full_image(&rgb, &self.settings, &self.lut_path, &self.retouch.layer);
        // Orientation, then straighten, then crop (the last, non-destructive steps).
        let (raw, w, h) = self.crop.crop.orientation.apply_rgb(&raw, w, h);
        let (raw, w, h) = crate::crop::pixel::rotate_rgb(&raw, w, h, self.crop.crop.angle);
        let (raw, out_w, out_h) = crate::crop::pixel::crop_rgb(&raw, w, h, self.crop.crop.rect);

        // Carry over metadata from the original if it is a PNG and the option is enabled.
        let metadata = if format == SaveFormat::Png && self.embed_png_metadata {
            read_png_metadata_chunks(path)
        } else {
            Vec::new()
        };

        write_image_file(save_path, format, quality, &raw, out_w, out_h, &metadata)
    }

    /// Collects the list of frames to export: only the modified ones
    /// (settings ≠ default or a LUT is selected). Used by the batch export.
    pub(crate) fn collect_modified_items(
        &self,
    ) -> Vec<(PathBuf, FilterSettings, Option<PathBuf>, RetouchLayer, Crop)> {
        let default_settings = FilterSettings::default();
        self.session
            .image_list
            .iter()
            .filter_map(|path| {
                let settings = self
                    .session
                    .settings_map
                    .get(path)
                    .copied()
                    .unwrap_or(default_settings);
                let lut_path = self.session.lut_map.get(path).and_then(|o| o.clone());
                let layer = self.session.load_retouch(path);
                let crop = self.session.load_crop(path);
                if settings == default_settings
                    && lut_path.is_none()
                    && layer.is_empty()
                    && crop.is_identity()
                {
                    None
                } else {
                    Some((path.clone(), settings, lut_path, layer, crop))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    /// Builds a chunk: length + type + data + crc. The CRC here is fake —
    /// the parser/injector do not verify it (chunks are carried over byte-for-byte).
    fn chunk(ctype: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend_from_slice(&(data.len() as u32).to_be_bytes());
        c.extend_from_slice(ctype);
        c.extend_from_slice(data);
        c.extend_from_slice(&[0, 0, 0, 0]);
        c
    }

    fn base_png() -> Vec<u8> {
        let mut png = SIG.to_vec();
        png.extend(chunk(b"IHDR", &[0u8; 13]));
        png.extend(chunk(b"IDAT", &[1, 2, 3, 4]));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn injects_metadata_right_after_ihdr() {
        let meta = chunk(b"tEXt", b"workflow\0{}");
        let out = inject_png_chunks(base_png(), &[meta.clone()]);

        let (after_ihdr, chunks) = parse_png_chunks(&out).unwrap();
        assert_eq!(chunks[1].0, *b"tEXt");
        assert_eq!(&out[after_ihdr..after_ihdr + meta.len()], &meta[..]);
    }

    #[test]
    fn skips_duplicate_chunk_types() {
        let png = {
            let mut p = SIG.to_vec();
            p.extend(chunk(b"IHDR", &[0u8; 13]));
            p.extend(chunk(b"tIME", &[0u8; 7]));
            p.extend(chunk(b"IDAT", &[1, 2, 3]));
            p.extend(chunk(b"IEND", &[]));
            p
        };
        let before = png.len();
        let out = inject_png_chunks(png, &[chunk(b"tIME", &[9u8; 7])]);
        assert_eq!(out.len(), before, "duplicate tIME must not be added");
    }

    #[test]
    fn read_metadata_skips_critical_and_structural() {
        // tEXt (ancillary) + tRNS (structural) + IDAT (critical)
        let mut png = SIG.to_vec();
        png.extend(chunk(b"IHDR", &[0u8; 13]));
        png.extend(chunk(b"tEXt", b"prompt\0hello"));
        png.extend(chunk(b"tRNS", &[0u8; 6]));
        png.extend(chunk(b"IDAT", &[1, 2, 3]));
        png.extend(chunk(b"IEND", &[]));

        let mut chunks = Vec::new();
        let (_, parsed) = parse_png_chunks(&png).unwrap();
        for (ctype, start, end) in parsed {
            if png_chunk_copyable(&ctype) {
                chunks.push(png[start..end].to_vec());
            }
        }
        assert_eq!(chunks.len(), 1);
        assert_eq!(&chunks[0][4..8], b"tEXt");
    }

    #[test]
    fn skips_unsafe_unknown_chunks() {
        // APNG chunks `acTL`/`fdAT` — ancillary, but with an uppercase 4th letter
        // (unsafe-to-copy). Must not be carried over — they would break the static PNG.
        assert!(!png_chunk_copyable(b"acTL"));
        assert!(!png_chunk_copyable(b"fdAT"));
        assert!(!png_chunk_copyable(b"fcTL"));
        assert!(!png_chunk_copyable(b"dSIG"));
    }

    #[test]
    fn copies_safe_unknown_chunks() {
        // Private, but safe-to-copy (4th letter lowercase) — carry it over.
        assert!(png_chunk_copyable(b"prVt"));
        // Known standard ones — carried over even with an uppercase 4th letter.
        assert!(png_chunk_copyable(b"iCCP"));
        assert!(png_chunk_copyable(b"tEXt"));
        assert!(png_chunk_copyable(b"pHYs"));
        // Critical/structural — no.
        assert!(!png_chunk_copyable(b"IHDR"));
        assert!(!png_chunk_copyable(b"bKGD"));
    }

    #[test]
    fn webp_lossy_round_trip_honors_quality() {
        let dir = std::env::temp_dir();
        let out = dir.join("tinyluma_webp_quality.webp");

        let (w, h) = (16u32, 16u32);
        let raw: Vec<u8> = (0..(w * h * 3)).map(|i| (i * 7) as u8).collect();
        assert!(write_image_file(&out, SaveFormat::WebP, 90, &raw, w, h, &[]));

        // The file must be a lossy VP8 bitstream, not VP8L (lossless).
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        assert_eq!(&bytes[12..16], b"VP8 ", "expected lossy VP8 chunk");

        // And it must decode back to the same dimensions.
        let img = image::open(&out).expect("output WebP must open");
        assert_eq!(img.to_rgb8().dimensions(), (w, h));

        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn round_trip_real_png_keeps_metadata_and_decodes() {
        let dir = std::env::temp_dir();
        let src = dir.join("tinyluma_meta_src.png");
        let out = dir.join("tinyluma_meta_out.png");

        // 1. Source: a real PNG with a text chunk (valid CRC).
        {
            let file = std::fs::File::create(&src).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 2, 2);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.add_text_chunk("workflow".to_string(), "{\"nodes\":[]}".to_string())
                .unwrap();
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&[0u8; 2 * 2 * 3]).unwrap();
        }

        // 2. Carry the metadata over into the new image.
        let meta = read_png_metadata_chunks(&src);
        assert!(!meta.is_empty(), "metadata must be read");

        let raw = vec![128u8; 2 * 2 * 3];
        assert!(write_png_with_metadata(&out, &raw, 2, 2, &meta));

        // 3. The result must decode validly and contain tEXt.
        let img = image::open(&out).expect("output PNG must open");
        assert_eq!(img.to_rgb8().as_raw().len(), raw.len());

        let bytes = std::fs::read(&out).unwrap();
        let (_, chunks) = parse_png_chunks(&bytes).unwrap();
        assert!(chunks.iter().any(|(t, _, _)| t == b"tEXt"));

        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&out);
    }
}
