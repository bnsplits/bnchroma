mod fade;
mod pick;
mod utils;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "bnchroma",
    version,
    about = "Pick colors from an image and render templates in one process"
)]
struct Args {
    /// Path to the image
    #[arg(long = "path")]
    path: Option<PathBuf>,

    /// Palette index used as gradient source (1 = first color)
    #[arg(long = "color-index", default_value_t = 1)]
    color_index: usize,

    /// Explicit source color, bypassing pick (must be #rrggbb)
    #[arg(long = "color")]
    color: Option<String>,

    /// Minimum lightness value (0.0 to 1.0)
    #[arg(long = "min-lightness", default_value_t = 0.025)]
    min_lightness: f32,

    /// Maximum lightness value (0.0 to 1.0)
    #[arg(long = "max-lightness", default_value_t = 0.92)]
    max_lightness: f32,

    /// Path to the configuration file (TOML format)
    #[arg(short = 'f', long = "config")]
    config: Option<String>,

    /// Force regeneration (ignore the pick cache)
    #[arg(long)]
    force: bool,

    /// Suppress informational output
    #[arg(long)]
    quiet: bool,

    /// Comma-separated name=hex pairs matched against --color
    #[arg(long = "match", value_name = "LIST")]
    match_list: Option<String>,

    /// Create default config and example template
    #[arg(long)]
    init: bool,
}

fn main() -> Result<()> {
    let a = Args::parse();
    if a.init {
        return fade::init_default_config();
    }
    if a.match_list.is_some() {
        return run_match(a);
    }
    run_sync(a)
}

fn run_match(a: Args) -> Result<()> {
    let list = a.match_list.as_deref().unwrap_or("");
    let candidates = utils::parse_candidate_list(list).context("Invalid --match list")?;
    let color = a.color.as_deref().context("--match requires --color")?;
    let clean = color.trim_start_matches('#');
    utils::hex_to_rgb(clean).with_context(|| format!("Invalid hex color: {}", color))?;
    let idx = utils::nearest_candidate(clean, &candidates)?;
    let c = &candidates[idx];
    println!("{}:#{:02X}{:02X}{:02X}", c.name, c.rgb.0, c.rgb.1, c.rgb.2);
    Ok(())
}

fn run_sync(a: Args) -> Result<()> {
    let path = a.path.clone().context("--path is required")?;
    let pick_base = pick::default_base_dir()?;

    let colors = pick::ensure_pick_colors(&path, a.force, &pick_base)?;

    let source_hex = match &a.color {
        Some(c) => {
            let clean = c.trim_start_matches('#');
            utils::hex_to_rgb(clean).with_context(|| format!("Invalid hex color: {}", clean))?;
            format!("#{}", clean)
        }
        None => {
            let idx = a
                .color_index
                .saturating_sub(1)
                .min(colors.len().saturating_sub(1));
            let c = colors[idx];
            format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
        }
    };
    let hex_clean = source_hex.trim_start_matches('#').to_string();

    let (gradient, source) =
        fade::ensure_fade_colors(&hex_clean, a.min_lightness, a.max_lightness)?;

    let is_default_config = a.config.is_none();
    let config_path = match a.config {
        Some(p) => p,
        None => fade::get_default_config_path()?
            .to_string_lossy()
            .into_owned(),
    };
    let cfg = fade::load_config(&config_path).with_context(|| {
        if is_default_config {
            format!(
                "Failed to load config ({}). Run `bnchroma --init` to create a default config",
                config_path
            )
        } else {
            format!("Failed to load config ({})", config_path)
        }
    })?;
    let ctx = fade::RenderCtx::new(&cfg.vars).context("Failed to resolve variables")?;

    fade::render_templates(&cfg.templates, &gradient, source, &ctx, a.quiet)
}
