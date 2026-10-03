// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 ThetaCursed

/// Three-dimensional LUT filter (loaded from a `.cube` file).
#[derive(Clone, PartialEq)]
pub(crate) struct Lut3D {
    pub(crate) size: usize,
    pub(crate) data: Vec<[f32; 3]>,
}

impl Lut3D {
    pub(crate) fn load_from_file(path: &std::path::Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut size = 0;
        let mut data = Vec::with_capacity(32 * 32 * 32); // pre-allocation

        for line in content.lines() {
            let line = line.trim();

            // 1. Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // 2. Read the grid size
            if line.starts_with("LUT_3D_SIZE") {
                size = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                continue;
            }

            // 3. Skip TITLE, DOMAIN_MIN, DOMAIN_MAX and other keywords
            let first_char = line.chars().next().unwrap_or(' ');
            if first_char.is_alphabetic() {
                continue;
            }

            // 4. Parse only lines that start with numbers
            let nums: Vec<f32> = line
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();

            if nums.len() == 3 {
                data.push([nums[0], nums[1], nums[2]]);
            }
        }

        if size > 0 && data.len() >= size * size * size {
            // Trim the excess if the file had extra lines (there is sometimes
            // trailing garbage).
            data.truncate(size * size * size);
            println!("✅ LUT loaded: {}, points: {}", size, data.len());
            Some(Self { size, data })
        } else {
            println!("❌ LUT error: size={}, points={}", size, data.len());
            None
        }
    }

    pub(crate) fn apply(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        let s = (self.size - 1) as f32;
        let x = (r.clamp(0.0, 1.0) * s).max(0.0).min(s);
        let y = (g.clamp(0.0, 1.0) * s).max(0.0).min(s);
        let z = (b.clamp(0.0, 1.0) * s).max(0.0).min(s);

        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let z0 = z.floor() as usize;
        let x1 = (x0 + 1).min(self.size - 1);
        let y1 = (y0 + 1).min(self.size - 1);
        let z1 = (z0 + 1).min(self.size - 1);

        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let fz = z - z0 as f32;

        let get = |ix: usize, iy: usize, iz: usize| -> [f32; 3] {
            self.data[ix + iy * self.size + iz * self.size * self.size]
        };

        let c000 = get(x0, y0, z0);
        let c100 = get(x1, y0, z0);
        let c010 = get(x0, y1, z0);
        let c110 = get(x1, y1, z0);
        let c001 = get(x0, y0, z1);
        let c101 = get(x1, y0, z1);
        let c011 = get(x0, y1, z1);
        let c111 = get(x1, y1, z1);

        let mix = |c1: [f32; 3], c2: [f32; 3], a: f32| -> [f32; 3] {
            [
                c1[0] + (c2[0] - c1[0]) * a,
                c1[1] + (c2[1] - c1[1]) * a,
                c1[2] + (c2[2] - c1[2]) * a,
            ]
        };

        let c00 = mix(c000, c100, fx);
        let c10 = mix(c010, c110, fx);
        let c01 = mix(c001, c101, fx);
        let c11 = mix(c011, c111, fx);
        mix(mix(c00, c10, fy), mix(c01, c11, fy), fz)
    }
}
