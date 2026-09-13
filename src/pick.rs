use crate::utils::write_if_changed;
use anyhow::{Context, Result};
use image::{imageops::FilterType, load_from_memory_with_format};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const SAMPLE_MAX_DIM: u32 = 64;

pub fn default_base_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("Could not find home directory")?
        .join(".cache/bnchroma"))
}

pub fn compute_cache_key(image_path: &Path) -> Result<String> {
    let metadata = image_path.metadata()?;
    let size = metadata.len();
    let mtime = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();
    let mut h = DefaultHasher::new();
    "bnchroma-pick-v2".hash(&mut h);
    image_path.as_os_str().hash(&mut h);
    size.hash(&mut h);
    mtime.hash(&mut h);
    Ok(format!("{:016x}", h.finish()))
}

fn bin_cache_path(base: &Path, key: &str) -> PathBuf {
    base.join("palettes").join(format!("{}.bin", key))
}

fn read_bin_cache(path: &Path) -> Result<Vec<[u8; 3]>> {
    let bytes = fs::read(path)?;
    if bytes.len() != 24 || bytes.len() % 3 != 0 {
        anyhow::bail!("Bad palette cache length");
    }
    Ok(bytes.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect())
}

fn palette_bytes(colors: &[[u8; 3]]) -> [u8; 24] {
    let mut out = [0u8; 24];
    for (i, c) in colors.iter().take(8).enumerate() {
        out[i * 3] = c[0];
        out[i * 3 + 1] = c[1];
        out[i * 3 + 2] = c[2];
    }
    out
}

fn render_hex(colors: &[[u8; 3]]) -> String {
    colors
        .iter()
        .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn publish_colors(colors: &[[u8; 3]], base_cache_dir: &Path) -> Result<()> {
    write_if_changed(
        &base_cache_dir.join("colors"),
        render_hex(colors).as_bytes(),
    )?;
    Ok(())
}

pub fn ensure_pick_colors(
    image_path: &Path,
    force: bool,
    base_cache_dir: &Path,
) -> Result<Vec<[u8; 3]>> {
    let cache_key = compute_cache_key(image_path)?;
    let cache_file = bin_cache_path(base_cache_dir, &cache_key);

    if !force {
        if let Ok(colors) = read_bin_cache(&cache_file) {
            publish_colors(&colors, base_cache_dir)?;
            return Ok(colors);
        }
    }

    let colors = process_image(image_path)?;
    if let Some(parent) = cache_file.parent() {
        fs::create_dir_all(parent).context("Failed to create cache directory")?;
    }
    fs::write(&cache_file, palette_bytes(&colors))?;
    publish_colors(&colors, base_cache_dir)?;
    Ok(colors)
}

fn process_image(image_path: &Path) -> Result<Vec<[u8; 3]>> {
    let bytes = fs::read(image_path)?;
    let format = image::guess_format(&bytes)?;
    let img = load_from_memory_with_format(&bytes, format).context("Failed to decode image")?;
    let img_rgb = img.to_rgb8();

    let (w, h) = (img_rgb.width(), img_rgb.height());
    let longest = w.max(h).max(1);
    let small = if longest > SAMPLE_MAX_DIM {
        let scale = longest as f32 / SAMPLE_MAX_DIM as f32;
        let sw = ((w as f32 / scale).round() as u32).max(1);
        let sh = ((h as f32 / scale).round() as u32).max(1);
        image::imageops::resize(&img_rgb, sw, sh, FilterType::Triangle)
    } else {
        img_rgb
    };

    let pixels: Vec<[u8; 3]> = small.pixels().map(|p| p.0).collect();

    Ok(median_cut(&pixels))
}

fn median_cut(pixels: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![pixels.to_vec()];

    while boxes.len() < 8 {
        let mut best_idx = None;
        let mut best_range = 0u16;
        let mut best_chan = 0usize;
        for (i, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            for c in 0..3 {
                let mut lo = 255u8;
                let mut hi = 0u8;
                for p in b.iter() {
                    lo = lo.min(p[c]);
                    hi = hi.max(p[c]);
                }
                let range = hi as u16 - lo as u16;
                if range > best_range {
                    best_range = range;
                    best_idx = Some(i);
                    best_chan = c;
                }
            }
        }
        let idx = match best_idx {
            Some(i) => i,
            None => break,
        };
        let mut b = std::mem::take(&mut boxes[idx]);
        b.sort_by_key(|p| p[best_chan]);
        let mid = b.len() / 2;
        let upper = b.split_off(mid);
        boxes[idx] = b;
        boxes.push(upper);
    }

    let mut avgs: Vec<(usize, [u8; 3])> = boxes
        .iter()
        .map(|b| {
            let n = b.len().max(1) as u32;
            let (mut r, mut g, mut bl) = (0u32, 0u32, 0u32);
            for p in b.iter() {
                r += p[0] as u32;
                g += p[1] as u32;
                bl += p[2] as u32;
            }
            (b.len(), [(r / n) as u8, (g / n) as u8, (bl / n) as u8])
        })
        .collect();
    avgs.sort_by_key(|a| std::cmp::Reverse(a.0));

    let mut colors: Vec<[u8; 3]> = avgs.into_iter().map(|(_, c)| c).collect();
    while colors.len() < 8 {
        colors.push(*colors.last().unwrap_or(&[0, 0, 0]));
    }
    colors.truncate(8);
    colors
}
