// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

//! One heal spot, end to end.

use image::RgbImage;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use super::inpaint::{best_offset, complete, dilate};
use super::poisson::{membrane_fill, seamless_clone};
use super::raster::{FloatRegion, Rect};

/// A single heal spot. Resolution-independent: the center is normalized to
/// `0..1` and the radius to the longer image side, so the same layer applies
/// identically to the preview and the full-resolution export.
#[derive(Clone, Copy, PartialEq, Debug)]
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
    // The region must cover the hole plus the whole search window plus the ring.
    let margin = radius_px.ceil() as i32 + max_radius + ring as i32 + 2;

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

/// Applies one spot to an 8-bit RGB image.
pub(crate) fn apply_spot(base: &mut RgbImage, spot: &Spot) {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let Some(geom) = spot_geometry(w, h, spot) else {
        return;
    };
    let rect = geom.rect;
    let radius_px = geom.radius_px;
    let (ring, max_radius) = (geom.ring, geom.max_radius);
    let (lcx, lcy) = (geom.lcx, geom.lcy);

    let region = FloatRegion::from_rgb8(base, rect);
    let (rw, rh) = (rect.w, rect.h);
    let ch = region.ch;

    // Circular hole mask.
    let mut hole = vec![false; rw * rh];
    let r2 = radius_px * radius_px;
    for y in 0..rh {
        for x in 0..rw {
            let dx = (x as f32 + 0.5) - lcx;
            let dy = (y as f32 + 0.5) - lcy;
            if dx * dx + dy * dy <= r2 {
                hole[y * rw + x] = true;
            }
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

    // Composite with opacity and a feathered coverage mask.
    let hard = spot.hardness.clamp(0.0, 1.0);
    let opacity = spot.opacity.clamp(0.0, 1.0);
    let inner = hard * radius_px;
    let feather = (radius_px - inner).max(1.0e-3);

    let mut out = region.data.clone();
    for y in 0..rh {
        for x in 0..rw {
            let dx = (x as f32 + 0.5) - lcx;
            let dy = (y as f32 + 0.5) - lcy;
            let d = (dx * dx + dy * dy).sqrt();
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
            let i = (y * rw + x) * ch;
            for c in 0..ch {
                out[i + c] = region.data[i + c] * (1.0 - a) + filled[i + c] * a;
            }
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
fn spot_cache_key(w: usize, h: usize, spot: &Spot, geom: &SpotGeom, base: &RgbImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    w.hash(&mut hasher);
    h.hash(&mut hasher);
    spot.center[0].to_bits().hash(&mut hasher);
    spot.center[1].to_bits().hash(&mut hasher);
    spot.radius.to_bits().hash(&mut hasher);
    spot.hardness.to_bits().hash(&mut hasher);
    spot.opacity.to_bits().hash(&mut hasher);
    (spot.kind as u8).hash(&mut hasher);
    geom.rect.x.hash(&mut hasher);
    geom.rect.y.hash(&mut hasher);
    geom.rect.w.hash(&mut hasher);
    geom.rect.h.hash(&mut hasher);
    hasher.write(&read_region(base, &geom.rect));
    hasher.finish()
}

/// `apply_spot`, but memoized through `cache`. On a hit the already-healed
/// region is blitted back, skipping the expensive fill entirely.
pub(crate) fn apply_spot_cached(cache: &mut SpotCache, base: &mut RgbImage, spot: &Spot) {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let Some(geom) = spot_geometry(w, h, spot) else {
        return;
    };
    let key = spot_cache_key(w, h, spot, &geom, base);
    if let Some(cached) = cache.map.get(&key) {
        write_region(base, &cached.rect, &cached.data);
        return;
    }
    apply_spot(base, spot);
    let region = CachedRegion {
        rect: geom.rect,
        data: read_region(base, &geom.rect),
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
