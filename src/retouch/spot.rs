// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! One heal spot, end to end.

use image::RgbImage;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use super::brush::stamp_segment_dist;
use super::inpaint::{best_offset, complete, dilate};
use super::poisson::{membrane_fill, seamless_clone};
use super::raster::{FloatRegion, Rect};

/// A single heal spot. Resolution-independent: the center is normalized to
/// `0..1` and the radius to the longer image side, so the same layer applies
/// identically to the preview and the full-resolution export.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Spot {
    /// Center in image space, `0..1` (x right, y down).
    pub(crate) center: [f32; 2],
    /// Radius as a fraction of the image's longer side.
    pub(crate) radius: f32,
    /// `0..1`, `1` = hard edge.
    pub(crate) hardness: f32,
    /// `0..1`, how strongly the fill is mixed over the original.
    pub(crate) opacity: f32,
    /// Which fill algorithm to use.
    pub(crate) kind: SpotKind,
    /// For a painted stroke: the dab centers of the whole path (normalized,
    /// in order). The entire gesture is healed by **one** union fill instead of
    /// one fill per dab, which removes the inter-dab seams and works on the
    /// micro-texture coherently. `None` for a single circular dab.
    pub(crate) path: Option<Vec<[f32; 2]>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[allow(dead_code)] // v3 variant is part of the planned API.
pub(crate) enum SpotKind {
    /// v1: find the best matching source offset around the spot and clone it.
    #[default]
    ProximityMatch,
    /// v2: PatchMatch content-aware completion. Continues structure and texture
    /// better than a single cloned offset, at a higher CPU cost.
    ContentAware,
    /// v3: texture synthesis (not implemented yet).
    CreateTexture,
}

/// Ordered list of spots (the retouch layer). Undo/redo operates on this.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct RetouchLayer {
    pub(crate) spots: Vec<Spot>,
}

impl RetouchLayer {
    pub(crate) fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }

    pub(crate) fn push(&mut self, spot: Spot) {
        self.spots.push(spot);
    }

    #[allow(dead_code)]
    pub(crate) fn pop(&mut self) -> Option<Spot> {
        self.spots.pop()
    }

    /// Heals `img` in place, spot by spot, in order. Later spots see the result
    /// of the earlier ones.
    pub(crate) fn apply_to(&self, img: &mut RgbImage) {
        for spot in &self.spots {
            apply_spot(img, spot);
        }
    }
}

/// Pixel geometry of a spot inside an image of a given size: the bounded
/// region, the local center, and the search parameters. Shared by `apply_spot`
/// and the cache so the region is derived in exactly one place.
struct SpotGeom {
    rect: Rect,
    lcx: f32,
    lcy: f32,
    radius_px: f32,
    ring: usize,
    max_radius: i32,
}

fn spot_geometry(w: usize, h: usize, spot: &Spot) -> Option<SpotGeom> {
    if w == 0 || h == 0 {
        return None;
    }
    let max_dim = w.max(h) as f32;
    let radius_px = (spot.radius * max_dim).max(1.0);
    let cx = spot.center[0] * w as f32;
    let cy = spot.center[1] * h as f32;

    // Search geometry (see `docs/RETOUCH.md` §5.3).
    let ring = ((radius_px / 8.0) as i32).clamp(3, 16) as usize;
    let max_radius = (radius_px as i32 + 8).max(4);
    // The region must cover the hole plus the whole search window. Proximity
    // Match also compares a ring of known pixels around the hole, so its source
    // rect needs the ring margin; content-aware searches source *patches* and
    // has no ring, so reserving that margin would only make it score extra area.
    let margin = radius_px.ceil() as i32
        + max_radius
        + 2
        + if matches!(spot.kind, SpotKind::ProximityMatch) {
            ring as i32
        } else {
            0
        };

    let x0 = ((cx - margin as f32).floor() as i32).clamp(0, w as i32) as usize;
    let y0 = ((cy - margin as f32).floor() as i32).clamp(0, h as i32) as usize;
    let x1 = ((cx + margin as f32).ceil() as i32).clamp(0, w as i32) as usize;
    let y1 = ((cy + margin as f32).ceil() as i32).clamp(0, h as i32) as usize;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(SpotGeom {
        rect: Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        },
        lcx: cx - x0 as f32,
        lcy: cy - y0 as f32,
        radius_px,
        ring,
        max_radius,
    })
}

/// Pixel geometry of a painted stroke: the bounded region and search params.
struct StrokeGeom {
    rect: Rect,
    radius_px: f32,
    ring: usize,
    max_radius: i32,
}

/// Bounding region + search parameters for a stroke's capsule.
fn stroke_geometry(w: usize, h: usize, spot: &Spot, path: &[[f32; 2]]) -> Option<StrokeGeom> {
    if w == 0 || h == 0 || path.is_empty() {
        return None;
    }
    let max_dim = w.max(h) as f32;
    let radius_px = (spot.radius * max_dim).max(1.0);
    let ring = ((radius_px / 8.0) as i32).clamp(3, 16) as usize;
    let max_radius = (radius_px as i32 + 8).max(4);
    // Same kind-aware margin as a dab (see `spot_geometry`).
    let margin = radius_px.ceil() as i32
        + max_radius
        + 2
        + if matches!(spot.kind, SpotKind::ProximityMatch) {
            ring as i32
        } else {
            0
        };

    let mut minx = f32::MAX;
    let mut miny = f32::MAX;
    let mut maxx = f32::MIN;
    let mut maxy = f32::MIN;
    for p in path {
        let x = p[0] * w as f32;
        let y = p[1] * h as f32;
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let x0 = ((minx - margin as f32).floor() as i32).clamp(0, w as i32) as usize;
    let y0 = ((miny - margin as f32).floor() as i32).clamp(0, h as i32) as usize;
    let x1 = ((maxx + margin as f32).ceil() as i32).clamp(0, w as i32) as usize;
    let y1 = ((maxy + margin as f32).ceil() as i32).clamp(0, h as i32) as usize;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(StrokeGeom {
        rect: Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        },
        radius_px,
        ring,
        max_radius,
    })
}

/// Region touched by an op: the dab circle or the stroke capsule, inflated by
/// the search margin. Shared by the cache and the geometry helpers.
fn op_rect(w: usize, h: usize, spot: &Spot) -> Option<Rect> {
    match &spot.path {
        Some(path) => stroke_geometry(w, h, spot, path).map(|g| g.rect),
        None => spot_geometry(w, h, spot).map(|g| g.rect),
    }
}

/// A copy of `spot` as a single circular dab at `center` (used by the stroke
/// size fallback).
fn dab_at(spot: &Spot, center: [f32; 2]) -> Spot {
    Spot {
        center,
        radius: spot.radius,
        hardness: spot.hardness,
        opacity: spot.opacity,
        kind: spot.kind,
        path: None,
    }
}

/// Fraction of the brush radius that is healed at full strength for a given
/// `hardness` (`0..1`).
///
/// A healing brush must fully repair almost the whole brush: if the composite
/// faded back to the original part-way out, the outer edge of the object being
/// removed would survive as a light/dark ring (a circular object with a bezel
/// leaves a visible arc at 50%). The solid core is therefore 80–100% of the
/// radius and `hardness` only widens the soft blend in that last band. The
/// Poisson blend already makes the seam invisible, so a large core costs
/// nothing in quality. Shared by `apply_spot` and the overlay's
/// `stroke_coverage`, so the indication always matches the healed region.
pub(crate) fn core_fraction(hardness: f32) -> f32 {
    0.8 + 0.2 * hardness.clamp(0.0, 1.0)
}

/// Single-fill budget. A stroke whose bounding region exceeds this is healed dab
/// by dab instead: one content-aware fill over tens of megapixels would freeze
/// the UI and allocate a huge pyramid. This only triggers on strokes that span a
/// large part of a high-resolution image, never on ordinary blemish removal.
const STROKE_FILL_MAX_PX: usize = 1_500_000;

/// Applies one heal operation to an 8-bit RGB image: either a single circular
/// dab or a whole painted stroke (`spot.path`).
pub(crate) fn apply_spot(base: &mut RgbImage, spot: &Spot) {
    match &spot.path {
        Some(path) => apply_stroke(base, spot, path),
        None => apply_dab(base, spot),
    }
}

/// A single circular dab. The distance field is the squared distance to the
/// centre.
fn apply_dab(base: &mut RgbImage, spot: &Spot) {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let Some(geom) = spot_geometry(w, h, spot) else {
        return;
    };
    let (rw, rh) = (geom.rect.w, geom.rect.h);
    let mut dist2 = vec![0.0f32; rw * rh];
    for y in 0..rh {
        let dy = (y as f32 + 0.5) - geom.lcy;
        for x in 0..rw {
            let dx = (x as f32 + 0.5) - geom.lcx;
            dist2[y * rw + x] = dx * dx + dy * dy;
        }
    }
    apply_shape(
        base,
        spot,
        geom.rect,
        &dist2,
        geom.radius_px,
        geom.ring,
        geom.max_radius,
    );
}

/// A whole painted stroke. The distance field is the squared distance to the
/// polyline through the resampled dab centres — the capsule swept by the brush.
/// The union is healed by **one** completion, so there are no per-dab seams and
/// the micro-texture is synthesized once, coherently.
fn apply_stroke(base: &mut RgbImage, spot: &Spot, path: &[[f32; 2]]) {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let Some(geom) = stroke_geometry(w, h, spot, path) else {
        return;
    };
    if geom.rect.w.saturating_mul(geom.rect.h) > STROKE_FILL_MAX_PX {
        // Too large for one coherent fill: heal dab by dab instead.
        for center in path {
            apply_dab(base, &dab_at(spot, *center));
        }
        return;
    }
    let (rw, rh) = (geom.rect.w, geom.rect.h);
    let pad = geom.radius_px + 1.0;
    let mut dist2 = vec![f32::MAX; rw * rh];
    let local = |p: &[f32; 2]| {
        (
            p[0] * w as f32 - geom.rect.x as f32,
            p[1] * h as f32 - geom.rect.y as f32,
        )
    };
    if path.len() == 1 {
        let a = local(&path[0]);
        stamp_segment_dist(&mut dist2, rw, rh, a, a, pad);
    } else {
        for seg in path.windows(2) {
            stamp_segment_dist(&mut dist2, rw, rh, local(&seg[0]), local(&seg[1]), pad);
        }
    }
    apply_shape(
        base,
        spot,
        geom.rect,
        &dist2,
        geom.radius_px,
        geom.ring,
        geom.max_radius,
    );
}

/// Shared fill + composite for a shape given as a squared-distance field over
/// the region (`f32::MAX` = outside the stamping neighbourhood).
fn apply_shape(
    base: &mut RgbImage,
    spot: &Spot,
    rect: Rect,
    dist2: &[f32],
    radius_px: f32,
    ring: usize,
    max_radius: i32,
) {
    let region = FloatRegion::from_rgb8(base, rect);
    let (rw, rh) = (rect.w, rect.h);
    let ch = region.ch;
    let r2 = radius_px * radius_px;

    let mut hole = vec![false; rw * rh];
    for (i, &d2) in dist2.iter().enumerate() {
        if d2 <= r2 {
            hole[i] = true;
        }
    }
    if hole.iter().all(|&b| !b) {
        return;
    }
    let heal_mask = dilate(&hole, rw, rh, 1);

    // Fill.
    let filled: Vec<f32> = match spot.kind {
        SpotKind::ProximityMatch => match best_offset(rw, rh, ch, &region.data, &hole, ring, max_radius)
        {
            Some((dx, dy)) => {
                // Sample the source into the heal mask, keep the original
                // elsewhere so the guidance field is continuous at the seam.
                let mut src = region.data.clone();
                for y in 0..rh {
                    for x in 0..rw {
                        let i = y * rw + x;
                        if !heal_mask[i] {
                            continue;
                        }
                        let sx = (x as i32 + dx).clamp(0, rw as i32 - 1) as usize;
                        let sy = (y as i32 + dy).clamp(0, rh as i32 - 1) as usize;
                        let si = (sy * rw + sx) * ch;
                        let di = i * ch;
                        for c in 0..ch {
                            src[di + c] = region.data[si + c];
                        }
                    }
                }
                seamless_clone(rw, rh, ch, &src, &region.data, &heal_mask)
            }
            None => membrane_fill(rw, rh, ch, &region.data, &hole),
        },
        // Content-aware: reconstruct the hole from the surrounding patches, then
        // blend the result through the Poisson solver so the seam disappears.
        SpotKind::ContentAware => {
            let completed = complete(rw, rh, ch, &region.data, &hole);
            seamless_clone(rw, rh, ch, &completed, &region.data, &heal_mask)
        }
        // v3 is not implemented yet — fall back to a smooth membrane fill.
        SpotKind::CreateTexture => membrane_fill(rw, rh, ch, &region.data, &hole),
    };

    // Composite with opacity and a feathered coverage mask. A fully hard brush
    // still gets a one-pixel ramp at the outline: without it the binary mask
    // edge lands exactly on the pixel grid and the healed patch reads as a
    // jagged circle (stair steps) when magnified.
    let hard = spot.hardness.clamp(0.0, 1.0);
    let opacity = spot.opacity.clamp(0.0, 1.0);
    const AA: f32 = 1.0;
    let inner = (core_fraction(hard) * radius_px).min(radius_px - AA).max(0.0);
    let feather = (radius_px - inner).max(AA);

    let mut out = region.data.clone();
    for (i, &d2) in dist2.iter().enumerate() {
        if d2 == f32::MAX {
            continue;
        }
        let d = d2.sqrt();
        if d >= radius_px {
            continue;
        }
        let coverage = if d <= inner {
            1.0
        } else {
            let t = ((radius_px - d) / feather).clamp(0.0, 1.0);
            // Smoothstep for a soft, banding-free falloff.
            t * t * (3.0 - 2.0 * t)
        };
        let a = coverage * opacity;
        let bi = i * ch;
        for c in 0..ch {
            out[bi + c] = region.data[bi + c] * (1.0 - a) + filled[bi + c] * a;
        }
    }

    let healed = FloatRegion {
        rect,
        ch,
        data: out,
    };
    healed.write_rgb8(base);
}

// ---------------------------------------------------------------------------
// Spot result cache
// ---------------------------------------------------------------------------
//
// Content-aware completion costs tens of milliseconds per dab. Whenever the
// layer is rebuilt from scratch — undo/redo, a frame switch or an export — every
// spot is re-applied to the same input pixels, so the results are identical.
// Memoizing the healed region per (spot + input pixels) turns those rebuilds
// into cheap blits instead of recomputing every expensive fill. The key is
// content-addressed, so an entry can never be stale; eviction is FIFO by bytes.

/// A bounded, content-addressed cache of healed spot regions.
#[derive(Default)]
pub(crate) struct SpotCache {
    map: HashMap<u64, CachedRegion>,
    order: VecDeque<u64>,
    bytes: usize,
}

struct CachedRegion {
    rect: Rect,
    data: Vec<u8>,
}

/// ~128 MB: room for a few thousand dabs, still bounded on long sessions.
const SPOT_CACHE_MAX_BYTES: usize = 128 << 20;

/// A single entry is skipped above this region size: a huge stroke would evict
/// most of the cache for one result that is unlikely to be replayed soon.
const CACHE_MAX_PX: usize = 4_000_000;

impl SpotCache {
    /// Drops every entry (e.g. when the image itself changed and old regions no
    /// longer describe anything useful).
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }

    fn insert(&mut self, key: u64, region: CachedRegion) {
        if self.map.contains_key(&key) {
            return;
        }
        self.bytes += region.data.len();
        self.map.insert(key, region);
        self.order.push_back(key);
        while self.bytes > SPOT_CACHE_MAX_BYTES {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(r) = self.map.remove(&old) {
                self.bytes -= r.data.len();
            }
        }
    }
}

/// Reads `rect` out of an RGB image as tightly packed bytes.
fn read_region(base: &RgbImage, rect: &Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity(rect.w * rect.h * 3);
    for y in rect.y..rect.y + rect.h {
        for x in rect.x..rect.x + rect.w {
            out.extend_from_slice(&base.get_pixel(x as u32, y as u32).0);
        }
    }
    out
}

/// Writes tightly packed RGB bytes back into `rect`.
fn write_region(base: &mut RgbImage, rect: &Rect, data: &[u8]) {
    for row in 0..rect.h {
        for x in 0..rect.w {
            let i = (row * rect.w + x) * 3;
            *base.get_pixel_mut((rect.x + x) as u32, (rect.y + row) as u32) =
                image::Rgb([data[i], data[i + 1], data[i + 2]]);
        }
    }
}

/// Cache key: the spot parameters + image size + the exact input pixels of the
/// region. Hashing ~66 KB is microseconds, negligible next to the fill it avoids.
fn spot_cache_key(w: usize, h: usize, spot: &Spot, rect: Rect, base: &RgbImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    w.hash(&mut hasher);
    h.hash(&mut hasher);
    spot.center[0].to_bits().hash(&mut hasher);
    spot.center[1].to_bits().hash(&mut hasher);
    spot.radius.to_bits().hash(&mut hasher);
    spot.hardness.to_bits().hash(&mut hasher);
    spot.opacity.to_bits().hash(&mut hasher);
    (spot.kind as u8).hash(&mut hasher);
    if let Some(path) = &spot.path {
        path.len().hash(&mut hasher);
        for p in path {
            p[0].to_bits().hash(&mut hasher);
            p[1].to_bits().hash(&mut hasher);
        }
    }
    rect.x.hash(&mut hasher);
    rect.y.hash(&mut hasher);
    rect.w.hash(&mut hasher);
    rect.h.hash(&mut hasher);
    hasher.write(&read_region(base, &rect));
    hasher.finish()
}

/// `apply_spot`, but memoized through `cache`. On a hit the already-healed
/// region is blitted back, skipping the expensive fill entirely.
pub(crate) fn apply_spot_cached(cache: &mut SpotCache, base: &mut RgbImage, spot: &Spot) {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let Some(rect) = op_rect(w, h, spot) else {
        return;
    };
    if rect.w.saturating_mul(rect.h) > CACHE_MAX_PX {
        apply_spot(base, spot);
        return;
    }
    let key = spot_cache_key(w, h, spot, rect, base);
    if let Some(cached) = cache.map.get(&key) {
        write_region(base, &cached.rect, &cached.data);
        return;
    }
    apply_spot(base, spot);
    let region = CachedRegion {
        rect,
        data: read_region(base, &rect),
    };
    cache.insert(key, region);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(center: [f32; 2], radius: f32) -> Spot {
        Spot {
            center,
            radius,
            hardness: 0.8,
            opacity: 1.0,
            kind: SpotKind::ProximityMatch,
            path: None,
        }
    }

    /// One fill per stroke vs one fill per dab, on a realistic 600 px stroke.
    /// `cargo test --release --bin TinyLuma bench_stroke_vs_dabs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_stroke_vs_dabs() {
        use crate::retouch::brush::BrushSettings;
        use std::time::Instant;
        let (w, h) = (1200u32, 800u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 3) % 200) as u8;
                img.put_pixel(x, y, image::Rgb([v, 255 - v, v / 2]));
            }
        }
        let brush = BrushSettings {
            size: 60.0,
            hardness: 0.5,
            spacing: 0.25,
            ..Default::default()
        };
        let path = [[0.25f32, 0.5], [0.75, 0.5]];

        let stroke = brush.to_stroke(&path, w as usize, h as usize);
        let mut single = img.clone();
        let t = Instant::now();
        apply_spot(&mut single, &stroke);
        let single_ms = t.elapsed().as_secs_f64() * 1000.0;

        let dabs = brush.spots_along(&path, w as usize, h as usize);
        let n = dabs.len();
        let mut per_dab = img.clone();
        let t = Instant::now();
        for d in &dabs {
            apply_spot(&mut per_dab, d);
        }
        let dabs_ms = t.elapsed().as_secs_f64() * 1000.0;

        println!("stroke single fill: {single_ms:.1} ms | {n} dabs: {dabs_ms:.1} ms");
    }

    /// A painted stroke is healed by one union fill: the whole band is repaired
    /// and a far pixel is untouched.
    #[test]
    fn apply_stroke_heals_the_whole_band() {
        use crate::retouch::brush::BrushSettings;
        let (w, h) = (120u32, 80u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = (100.0 + (x as f32 / w as f32) * 80.0) as u8;
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        // A dark band defect along the painted path.
        for y in 36..44 {
            for x in 20..100 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let far = *img.get_pixel(2, 2);
        let brush = BrushSettings {
            size: 30.0,
            hardness: 1.0,
            spacing: 0.25,
            ..Default::default()
        };
        let stroke = brush.to_stroke(&[[0.2f32, 0.5], [0.8, 0.5]], w as usize, h as usize);
        assert!(stroke.path.as_ref().is_some_and(|p| p.len() >= 2));

        apply_spot(&mut img, &stroke);

        assert_eq!(*img.get_pixel(2, 2), far, "far pixels must not change");
        for x in [40u32, 60, 80] {
            let got = img.get_pixel(x, 40).0[0] as i32;
            let want = (100.0 + (x as f32 / w as f32) * 80.0) as i32;
            assert!((got - want).abs() < 30, "x={x}: healed {got} vs background {want}");
        }
    }

    /// The outer edge of a removed object must not survive: a dark ring at ~85%
    /// of the brush radius has to be healed at the default hardness (the older
    /// `hardness * r` core left a visible arc there).
    #[test]
    fn apply_spot_removes_object_at_the_brush_edge() {
        let (w, h) = (160u32, 160u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(x, y, image::Rgb([190, 190, 190]));
            }
        }
        // A dark ring at radius 34 in a brush of radius 40 (0.85 r).
        let (cx, cy) = (80.0f32, 80.0f32);
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let r = (dx * dx + dy * dy).sqrt();
                if (33.0..35.0).contains(&r) {
                    img.put_pixel(x, y, image::Rgb([40, 40, 40]));
                }
            }
        }
        let s = Spot {
            center: [0.5, 0.5],
            radius: 40.0 / 160.0,
            hardness: 0.5,
            opacity: 1.0,
            kind: SpotKind::ContentAware,
            path: None,
        };
        apply_spot(&mut img, &s);
        for a in 0..16 {
            let ang = a as f32 / 16.0 * std::f32::consts::TAU;
            let x = (cx + ang.cos() * 34.0).round() as u32;
            let y = (cy + ang.sin() * 34.0).round() as u32;
            let v = img.get_pixel(x, y).0[0] as i32;
            assert!(v > 150, "ring remained at angle {a}: {v}");
        }
    }

    /// Healing changes the hole and leaves a far-away pixel byte-identical.
    #[test]
    fn apply_spot_changes_hole_only_locally() {
        let (w, h) = (80u32, 80u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // A smooth varying background.
                let v = ((x as f32 / w as f32) * 180.0
                    + (y as f32 / h as f32) * 40.0) as u8;
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        // A dark defect at the center.
        for y in 36..44 {
            for x in 36..44 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        let far = *img.get_pixel(2, 2);
        let before_center = *img.get_pixel(40, 40);

        apply_spot(&mut img, &spot([0.5, 0.5], 0.08));

        let after_center = *img.get_pixel(40, 40);
        assert_ne!(before_center, after_center, "the hole must be healed");
        assert_eq!(*img.get_pixel(2, 2), far, "far pixels must not change");

        // The healed center should be close to the smooth background.
        let expected = ((40.0f32 / w as f32) * 180.0 + (40.0f32 / h as f32) * 40.0) as i32;
        let got = after_center.0[0] as i32;
        assert!(
            (got - expected).abs() < 40,
            "healed value {got} far from background {expected}"
        );
    }

    /// Same input → identical output.
    #[test]
    fn apply_spot_is_deterministic() {
        let make = || {
            let (w, h) = (48u32, 48u32);
            let mut img = RgbImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let v = ((x * 7 + y * 3) % 200) as u8;
                    img.put_pixel(x, y, image::Rgb([v, 255 - v, v / 2]));
                }
            }
            for y in 20..28 {
                for x in 20..28 {
                    img.put_pixel(x, y, image::Rgb([10, 20, 30]));
                }
            }
            let mut a = img.clone();
            let mut b = img;
            apply_spot(&mut a, &spot([0.5, 0.5], 0.15));
            apply_spot(&mut b, &spot([0.5, 0.5], 0.15));
            a.into_raw() == b.into_raw()
        };
        assert!(make(), "heal must be deterministic");
    }

    /// The cache must be transparent: a cached replay produces byte-identical
    /// output to an uncached run, including for the expensive content-aware fill.
    #[test]
    fn spot_cache_replays_identical_result() {
        let (w, h) = (96u32, 96u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 3) % 200) as u8;
                img.put_pixel(x, y, image::Rgb([v, 255 - v, v / 2]));
            }
        }
        let mk = |c: f32| Spot {
            center: [c, 0.5],
            radius: 0.12,
            hardness: 0.5,
            opacity: 1.0,
            kind: SpotKind::ContentAware,
            path: None,
        };
        let spots = vec![mk(0.4), mk(0.6)];

        let mut uncached = img.clone();
        for s in &spots {
            apply_spot(&mut uncached, s);
        }

        let mut cache = SpotCache::default();
        let mut cached = img.clone();
        for s in &spots {
            apply_spot_cached(&mut cache, &mut cached, s);
        }
        assert_eq!(cached.as_raw(), uncached.as_raw(), "cached run differs");

        // Replaying the same spots (the undo/redo path) must hit the cache and
        // still produce the same image.
        let mut replay = img.clone();
        for s in &spots {
            apply_spot_cached(&mut cache, &mut replay, s);
        }
        assert_eq!(replay.as_raw(), uncached.as_raw(), "replay differs");
    }

    #[test]
    fn empty_layer_does_nothing() {
        let layer = RetouchLayer::default();
        let mut img = RgbImage::new(8, 8);
        let before = img.clone();
        layer.apply_to(&mut img);
        assert_eq!(img.into_raw(), before.into_raw());
    }

    /// Heal budget at a realistic preview size and brush.
    /// `cargo test --release -- --ignored bench_apply_spot --nocapture`
    #[test]
    #[ignore]
    fn bench_apply_spot() {
        use std::time::Instant;
        let (w, h) = (1200u32, 800u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 3) % 200) as u8;
                img.put_pixel(x, y, image::Rgb([v, 255 - v, v / 2]));
            }
        }
        for size in [30.0f32, 60.0, 120.0] {
            let s = Spot {
                center: [0.5, 0.5],
                radius: (size * 0.5) / 1200.0,
                hardness: 0.5,
                opacity: 1.0,
                kind: SpotKind::ProximityMatch,
                path: None,
            };
            let mut copy = img.clone();
            let t = Instant::now();
            apply_spot(&mut copy, &s);
            println!("apply_spot brush {size}px: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }

    /// Content-aware heal budget at a realistic preview size and brush.
    /// `cargo test --release -- --ignored bench_apply_spot_content_aware --nocapture`
    #[test]
    #[ignore]
    fn bench_apply_spot_content_aware() {
        use std::time::Instant;
        let (w, h) = (1200u32, 800u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 3) % 200) as u8;
                img.put_pixel(x, y, image::Rgb([v, 255 - v, v / 2]));
            }
        }
        for size in [30.0f32, 60.0, 120.0] {
            let s = Spot {
                center: [0.5, 0.5],
                radius: (size * 0.5) / 1200.0,
                hardness: 0.5,
                opacity: 1.0,
                kind: SpotKind::ContentAware,
                path: None,
            };
            let mut copy = img.clone();
            let t = Instant::now();
            apply_spot(&mut copy, &s);
            println!(
                "content-aware apply_spot brush {size}px: {:.2} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Undo/redo speedup: the first pass computes, the replay must hit the cache.
    /// `cargo test --release -- --ignored bench_spot_cache_replay --nocapture`
    #[test]
    #[ignore]
    fn bench_spot_cache_replay() {
        use std::time::Instant;
        let (w, h) = (1200u32, 800u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 3) % 200) as u8;
                img.put_pixel(x, y, image::Rgb([v, 255 - v, v / 2]));
            }
        }
        let spots: Vec<Spot> = (0..20)
            .map(|i| Spot {
                center: [0.3 + i as f32 * 0.02, 0.5],
                radius: 30.0 / 1200.0,
                hardness: 0.5,
                opacity: 1.0,
                kind: SpotKind::ContentAware,
                path: None,
            })
            .collect();

        let mut cache = SpotCache::default();
        let mut first = img.clone();
        let t = Instant::now();
        for s in &spots {
            apply_spot_cached(&mut cache, &mut first, s);
        }
        println!("first run (20 dabs): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);

        let mut replay = img.clone();
        let t = Instant::now();
        for s in &spots {
            apply_spot_cached(&mut cache, &mut replay, s);
        }
        println!("cached replay (20 dabs): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
}
