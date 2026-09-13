use anyhow::Context;

pub type Rgb = (u8, u8, u8);
pub type Gradient = Vec<Rgb>;

pub fn hex_to_rgb(hex: &str) -> anyhow::Result<Rgb> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        anyhow::bail!("Invalid hex color length");
    }

    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;

    Ok((r, g, b))
}

pub fn rgb_to_hls(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        return (0.0, l, 0.0);
    }

    let s = if l <= 0.5 {
        (max - min) / (max + min)
    } else {
        (max - min) / (2.0 - max - min)
    };

    let rc = (max - r) / (max - min);
    let gc = (max - g) / (max - min);
    let bc = (max - b) / (max - min);

    let mut h = match () {
        _ if r == max => bc - gc,
        _ if g == max => 2.0 + rc - bc,
        _ => 4.0 + gc - rc,
    };

    h = (h / 6.0) % 1.0;
    (h, l, s)
}

pub fn hls_to_rgb(h: f32, l: f32, s: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }

    let m2 = if l <= 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let m1 = 2.0 * l - m2;

    let r = hue_to_rgb(m1, m2, h + 1.0 / 3.0);
    let g = hue_to_rgb(m1, m2, h);
    let b = hue_to_rgb(m1, m2, h - 1.0 / 3.0);

    (r, g, b)
}

fn hue_to_rgb(m1: f32, m2: f32, mut h: f32) -> f32 {
    h = h.rem_euclid(1.0);
    if h * 6.0 < 1.0 {
        m1 + (m2 - m1) * h * 6.0
    } else if h < 0.5 {
        m2
    } else if h * 3.0 < 2.0 {
        m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0
    } else {
        m1
    }
}

pub fn generate_colors(
    hex_color: &str,
    min_lightness: f32,
    max_lightness: f32,
) -> anyhow::Result<Gradient> {
    let (r, g, b) = hex_to_rgb(hex_color)?;
    let (h, _, s) = rgb_to_hls(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);

    const STEPS: usize = 16;

    let colors = (0..STEPS)
        .map(|i| {
            let l =
                min_lightness + (i as f32 / (STEPS - 1) as f32) * (max_lightness - min_lightness);
            let (r, g, b) = hls_to_rgb(h, l, s);
            [
                (r * 255.0).round().clamp(0.0, 255.0) as u8,
                (g * 255.0).round().clamp(0.0, 255.0) as u8,
                (b * 255.0).round().clamp(0.0, 255.0) as u8,
            ]
        })
        .map(|arr| (arr[0], arr[1], arr[2]))
        .collect::<Vec<_>>();

    Ok(colors)
}

pub fn invert_color(r: u8, g: u8, b: u8) -> Rgb {
    (255 - r, 255 - g, 255 - b)
}

pub fn rgb_to_hsl_css(r: u8, g: u8, b: u8) -> (i32, i32, i32) {
    let (h, l, s) = rgb_to_hls(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);

    let hue = (h * 360.0).round() as i32;
    let saturation = (s * 100.0).round() as i32;
    let lightness = (l * 100.0).round() as i32;

    (hue, saturation, lightness)
}

pub fn write_if_changed(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<bool> {
    if let Ok(existing) = std::fs::read(path) {
        if existing == bytes {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, bytes)?;
    Ok(true)
}

fn rgb_to_lab(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    fn gamma_correct(c: f64) -> f64 {
        if c > 0.04045 {
            ((c + 0.055) / 1.055).powf(2.4)
        } else {
            c / 12.92
        }
    }
    fn lab_f(t: f64) -> f64 {
        if t > 0.008856 {
            t.powf(1.0 / 3.0)
        } else {
            7.787 * t + 16.0 / 116.0
        }
    }

    let r = gamma_correct(r as f64 / 255.0);
    let g = gamma_correct(g as f64 / 255.0);
    let b = gamma_correct(b as f64 / 255.0);

    let x = (r * 0.4124564 + g * 0.3575761 + b * 0.1804375) / 0.95047;
    let y = r * 0.2126729 + g * 0.7151522 + b * 0.0721750;
    let z = (r * 0.0193339 + g * 0.1191920 + b * 0.9503041) / 1.08883;

    let fx = lab_f(x);
    let fy = lab_f(y);
    let fz = lab_f(z);

    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

pub struct Candidate {
    pub name: String,
    pub rgb: Rgb,
}

pub fn parse_candidate_list(list: &str) -> anyhow::Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for raw in list.split(',') {
        let entry = raw.trim();
        if entry.is_empty() {
            continue;
        }
        let (name, hex) = entry
            .split_once('=')
            .map(|(n, h)| (n.trim(), h.trim()))
            .unwrap_or(("", ""));
        if name.is_empty() || hex.is_empty() {
            anyhow::bail!("Invalid match candidate {:?}: expected name=#rrggbb", entry);
        }
        let rgb = hex_to_rgb(hex.trim_start_matches('#'))
            .with_context(|| format!("Invalid hex color in match candidate {:?}", entry))?;
        out.push(Candidate {
            name: name.to_string(),
            rgb,
        });
    }
    if out.is_empty() {
        anyhow::bail!("Empty --match candidate list");
    }
    Ok(out)
}

pub fn nearest_candidate(hex: &str, candidates: &[Candidate]) -> anyhow::Result<usize> {
    let (r, g, b) = hex_to_rgb(hex)?;
    let input = rgb_to_lab(r, g, b);

    let mut best = 0usize;
    let mut best_d = f64::INFINITY;
    for (i, c) in candidates.iter().enumerate() {
        let lab = rgb_to_lab(c.rgb.0, c.rgb.1, c.rgb.2);
        let dl = input.0 - lab.0;
        let da = input.1 - lab.1;
        let db = input.2 - lab.2;
        let d = (dl * dl + da * da + db * db).sqrt();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    Ok(best)
}
