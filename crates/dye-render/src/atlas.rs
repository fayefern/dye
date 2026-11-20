use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use dye_assets::{
    handle::Handle,
    kinds::Image,
    store::{Assets, State},
};

use crate::error::AtlasError;

#[derive(Clone, Copy, Debug)]
pub struct AtlasOpts {
    pub padding: u32,
    pub max_side: u32,
}

impl Default for AtlasOpts {
    fn default() -> Self {
        Self {
            padding: 2,
            max_side: 8192,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

struct Item {
    handle: Handle<Image>,
    image: Arc<Image>,
    key: String,
}

pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    tiles: HashMap<Handle<Image>, Tile>,
    nil: Tile,
    versions: Vec<(Handle<Image>, u32)>,
}

impl Atlas {
    pub fn init(
        opts: AtlasOpts,
        assets: &mut Assets,
        handles: &[Handle<Image>],
    ) -> Result<Self, AtlasError> {
        let nil = assets.handle::<Image>("dye:nil");

        let mut seen = HashSet::from([nil]);
        let mut versions = vec![(nil, assets.version(nil))];
        let mut items = vec![Item {
            handle: nil,
            image: assets.get_arc(nil),
            key: String::new(),
        }];
        for &h in handles {
            if !seen.insert(h) {
                continue;
            }
            versions.push((h, assets.version(h)));
            if assets.state(h) != State::Ready {
                continue;
            }
            items.push(Item {
                handle: h,
                image: assets.get_arc(h),
                key: assets.id(h).map(|i| i.to_string()).unwrap_or_default(),
            });
        }

        for item in &items {
            let (w, h) = (
                item.image.width() + 2 * opts.padding,
                item.image.height() + 2 * opts.padding,
            );
            if w > opts.max_side || h > opts.max_side {
                return Err(AtlasError::TileTooLarge {
                    id: item.key.clone(),
                    w,
                    h,
                    max: opts.max_side,
                });
            }
        }
        items.sort_by(|a, b| {
            b.image
                .height()
                .cmp(&a.image.height())
                .then_with(|| a.key.cmp(&b.key))
        });

        let sizes: Vec<(u32, u32)> = items
            .iter()
            .map(|i| {
                (
                    i.image.width() + 2 * opts.padding,
                    i.image.height() + 2 * opts.padding,
                )
            })
            .collect();
        let area: u64 = sizes.iter().map(|&(w, h)| w as u64 * h as u64).sum();
        let widest = sizes.iter().map(|s| s.0).max().unwrap_or(1);

        let mut width = ((area as f64).sqrt().ceil() as u32)
            .max(widest)
            .next_power_of_two()
            .min(opts.max_side);
        let (width, height, places) = loop {
            if let Some((places, used)) = shelf(&sizes, width)
                && used <= opts.max_side
            {
                break (width, used.next_power_of_two().min(opts.max_side), places);
            }
            if width >= opts.max_side {
                return Err(AtlasError::TooLarge { max: opts.max_side });
            }
            width = (width * 2).min(opts.max_side);
        };

        let stride = width as usize;
        let mut pixels = vec![0u8; stride * height as usize * 4];
        let mut tiles = HashMap::with_capacity(items.len());

        for (item, &(px, py)) in items.iter().zip(&places) {
            let (w, h) = (item.image.width(), item.image.height());
            let src = item.image.rgba();

            for dy in 0..h + 2 * opts.padding {
                let sy = dy.saturating_sub(opts.padding).min(h - 1) as usize;

                for dx in 0..w + 2 * opts.padding {
                    let sx = dx.saturating_sub(opts.padding).min(w - 1) as usize;
                    let s = (sy * w as usize + sx) * 4;
                    let d = ((py + dy) as usize * stride + (px + dx) as usize) * 4;
                    pixels[d..d + 4].copy_from_slice(&src[s..s + 4]);
                }
            }
            tiles.insert(
                item.handle,
                Tile {
                    x: px + opts.padding,
                    y: py + opts.padding,
                    w,
                    h,
                },
            );
        }
        let nil_tile = tiles[&nil];

        Ok(Self {
            width,
            height,
            pixels,
            tiles,
            nil: nil_tile,
            versions,
        })
    }

    pub fn tile(&self, h: Handle<Image>) -> Tile {
        self.tiles.get(&h).copied().unwrap_or(self.nil)
    }

    pub fn uv(&self, h: Handle<Image>) -> UvRect {
        let t = self.tile(h);
        let (aw, ah) = (self.width as f32, self.height as f32);

        UvRect {
            min: [t.x as f32 / aw, t.y as f32 / ah],
            max: [(t.x + t.w) as f32 / aw, (t.y + t.h) as f32 / ah],
        }
    }

    pub fn is_stale(&self, assets: &Assets) -> bool {
        self.versions.iter().any(|&(h, v)| assets.version(h) != v)
    }
}

fn shelf(sizes: &[(u32, u32)], width: u32) -> Option<(Vec<(u32, u32)>, u32)> {
    let mut result = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut row_h) = (0u32, 0u32, 0u32);

    for &(w, h) in sizes {
        if w > width {
            return None;
        }
        if x + w > width {
            y += row_h;
            x = 0;
            row_h = 0;
        }
        result.push((x, y));
        x += w;
        row_h = row_h.max(h);
    }

    Some((result, y + row_h))
}
