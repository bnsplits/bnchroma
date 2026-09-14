use crate::utils::{self, write_if_changed};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::{fs, process::Command};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub vars: HashMap<String, toml::Value>,
    #[serde(default)]
    pub templates: Vec<TemplateConfig>,
}

#[derive(Debug, Deserialize)]
pub struct TemplateConfig {
    pub name: String,
    /// Input template. Must come together with `output`.
    #[serde(default, deserialize_with = "optional_string")]
    pub template: Option<String>,
    /// Output path(s). Must come together with `template`.
    #[serde(default, deserialize_with = "string_or_vec")]
    pub output: Option<Vec<String>>,
    /// Post-command(s). May stand alone without template/outputs.
    #[serde(default, deserialize_with = "string_or_vec")]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub footer: Option<String>,
}

fn optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Option::<String>::deserialize(deserializer)
}

fn value_to_string(name: &str, v: &toml::Value) -> Result<String> {
    match v {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Float(f) => Ok(f.to_string()),
        toml::Value::Boolean(b) => Ok(b.to_string()),
        _ => anyhow::bail!(
            "Invalid value for variable {:?} (only strings, numbers and booleans allowed)",
            name
        ),
    }
}

pub struct RenderCtx {
    pub vars: HashMap<String, String>,
}

impl RenderCtx {
    pub fn new(vars: &HashMap<String, toml::Value>) -> Result<Self> {
        let mut resolved = HashMap::with_capacity(vars.len());
        let mut names: Vec<&String> = vars.keys().collect();
        names.sort();
        for name in names {
            let value = value_to_string(name, &vars[name])
                .with_context(|| format!("Failed to resolve variable {:?}", name))?;
            resolved.insert(name.clone(), value);
        }
        Ok(Self { vars: resolved })
    }
}

fn validate_entry(cfg: &TemplateConfig) -> Result<()> {
    let has_template = cfg.template.is_some();
    let has_outputs = cfg.output.as_ref().map(|o| !o.is_empty()).unwrap_or(false);
    let has_commands = cfg.command.as_ref().map(|c| !c.is_empty()).unwrap_or(false);

    if has_template != has_outputs {
        anyhow::bail!(
            "Entry {:?}: template and output must be specified together (both or neither)",
            cfg.name
        );
    }
    if (cfg.header.is_some() || cfg.footer.is_some()) && !has_template {
        anyhow::bail!("Entry {:?}: header/footer require a template", cfg.name);
    }
    if !has_template && !has_commands {
        anyhow::bail!(
            "Entry {:?}: nothing to do (no template/outputs and no commands)",
            cfg.name
        );
    }
    Ok(())
}

fn string_or_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let opt = Option::<toml::Value>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(toml::Value::String(s)) => Ok(Some(vec![s])),
        Some(toml::Value::Array(arr)) => arr
            .into_iter()
            .map(|v| match v {
                toml::Value::String(s) => Ok(s),
                _ => Err(serde::de::Error::custom("expected string in array")),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(serde::de::Error::custom(
            "expected string or array of strings",
        )),
    }
}

pub struct PreparedColor {
    pub raw: utils::Rgb,
    hex: String,
    hex_stripped: String,
    rgb: String,
    rgb_stripped: String,
    hsl: String,
    hsl_stripped: String,
}

impl PreparedColor {
    fn new(raw: utils::Rgb) -> Self {
        let (r, g, b) = raw;
        let (h, s, l) = utils::rgb_to_hsl_css(r, g, b);
        Self {
            raw,
            hex: format!("#{:02X}{:02X}{:02X}", r, g, b),
            hex_stripped: format!("{:02X}{:02X}{:02X}", r, g, b),
            rgb: format!("rgb({}, {}, {})", r, g, b),
            rgb_stripped: format!("{}, {}, {}", r, g, b),
            hsl: format!("hsl({}, {}%, {}%)", h, s, l),
            hsl_stripped: format!("{}, {}%, {}%", h, s, l),
        }
    }
}

pub struct Palette {
    grad: Vec<PreparedColor>,
    source: PreparedColor,
}

pub fn prepare_palette(colors: &[utils::Rgb], source: utils::Rgb) -> Palette {
    Palette {
        grad: colors.iter().map(|&c| PreparedColor::new(c)).collect(),
        source: PreparedColor::new(source),
    }
}

pub fn ensure_fade_colors(
    hex_color_clean: &str,
    min_lightness: f32,
    max_lightness: f32,
) -> Result<(utils::Gradient, utils::Rgb)> {
    let source_color = utils::hex_to_rgb(hex_color_clean)
        .with_context(|| format!("Invalid hex color: {}", hex_color_clean))?;
    let colors = utils::generate_colors(hex_color_clean, min_lightness, max_lightness)
        .context("Failed to generate colors")?;
    Ok((colors, source_color))
}

fn find_sub(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    let first = needle[0];
    let mut i = from;
    while i + needle.len() <= haystack.len() {
        if haystack[i] == first && &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn render_into(
    pal: &Palette,
    ctx: &RenderCtx,
    content: &str,
    out: &mut String,
) -> (usize, usize) {
    let mut replacements = 0usize;
    let mut errors = 0usize;
    let b = content.as_bytes();
    let mut i = 0usize;

    while i < b.len() {
        let rel = match find_sub(b, i, b"{{") {
            Some(p) => p,
            None => {
                out.push_str(&content[i..]);
                break;
            }
        };
        out.push_str(&content[i..rel]);
        let rest = &b[rel..];
        if rest.starts_with(b"{{col.") {
            i = render_col(pal, content, b, rel, out, &mut replacements, &mut errors);
        } else if rest.starts_with(b"{{var.") {
            let n_start = rel + 6;
            match find_sub(b, n_start, b"}}") {
                Some(end) => {
                    let inner = content[n_start..end].trim();
                    let mut done = false;
                    if let Some(stem) = inner.strip_suffix(".rgb") {
                        let stem = stem.trim();
                        if !stem.is_empty() {
                            if let Some(v) = ctx.vars.get(stem) {
                                match utils::hex_to_rgb(v.trim_start_matches('#')) {
                                    Ok((r, g, b)) => {
                                        out.push_str(&format!("{}, {}, {}", r, g, b));
                                        replacements += 1;
                                    }
                                    Err(_) => {
                                        errors += 1;
                                        out.push_str("{{ INVALID_VAR }}");
                                    }
                                }
                                done = true;
                            }
                        }
                    }
                    if !done {
                        match ctx.vars.get(inner) {
                            Some(v) if !inner.is_empty() => {
                                out.push_str(v);
                                replacements += 1;
                            }
                            _ => {
                                errors += 1;
                                out.push_str("{{ INVALID_VAR }}");
                            }
                        }
                    }
                    i = end + 2;
                }
                None => {
                    out.push_str(&content[rel..]);
                    break;
                }
            }
        } else {
            out.push_str("{{");
            i = rel + 2;
        }
    }

    (replacements, errors)
}

fn render_col(
    pal: &Palette,
    content: &str,
    b: &[u8],
    rel: usize,
    out: &mut String,
    replacements: &mut usize,
    errors: &mut usize,
) -> usize {
    let mut j = rel + 6;

    let name_start = j;
    while j < b.len() && b[j] != b'.' && b[j] != b'}' {
        j += 1;
    }
    if j >= b.len() || b[j] != b'.' {
        out.push_str(&content[rel..rel + 6]);
        let _ = name_start;
        return rel + 6;
    }
    let name = &content[name_start..j];
    j += 1;

    let fmt_start = j;
    while j < b.len() && b[j] != b'|' && b[j] != b'}' {
        j += 1;
    }
    let fmt_raw = &content[fmt_start..j];

    // Modifiers + closing "}}".
    let mut mods: Option<&str> = None;
    let mut fmt = fmt_raw;
    if j < b.len() && b[j] == b'|' {
        fmt = fmt_raw.trim_end();
        let m_start = j + 1;
        match find_sub(b, m_start, b"}}") {
            Some(end) => {
                if m_start == end {
                    out.push_str(&content[rel..end + 2]);
                    return end + 2;
                }
                mods = Some(&content[m_start..end]);
                j = end + 2;
            }
            None => {
                out.push_str(&content[rel..]);
                return b.len();
            }
        }
    } else if j + 1 < b.len() && b[j] == b'}' && b[j + 1] == b'}' {
        j += 2;
    } else {
        out.push_str(&content[rel..j]);
        return j;
    }

    let base: Option<&PreparedColor> = if name == "source" {
        Some(&pal.source)
    } else if is_all_digits(name) {
        match name.parse::<usize>() {
            Ok(n) if (1..=16).contains(&n) => Some(&pal.grad[n - 1]),
            _ => {
                *errors += 1;
                out.push_str("{{ INVALID_INDEX }}");
                return j;
            }
        }
    } else {
        out.push_str(&content[rel..j]);
        return j;
    };
    let base = base.expect("resolved above");

    let (mut r, mut g, mut bl) = base.raw;
    let mut alpha = 1.0f32;
    let mut strip = false;
    let mut inverted = false;
    if let Some(m) = mods {
        for part in m.split('|') {
            let t = part.trim();
            if let Some(a) = t.strip_prefix("alpha:") {
                alpha = a.trim().parse().unwrap_or(1.0);
            } else if t == "invert" {
                inverted = true;
            } else if t == "strip" {
                strip = true;
            }
        }
    }
    if inverted {
        let (ir, ig, ib) = utils::invert_color(r, g, bl);
        r = ir;
        g = ig;
        bl = ib;
    }

    *replacements += 1;
    match fmt {
        "hex" => {
            if inverted {
                if strip {
                    out.push_str(&format!("{:02X}{:02X}{:02X}", r, g, bl));
                } else {
                    out.push_str(&format!("#{:02X}{:02X}{:02X}", r, g, bl));
                }
            } else if strip {
                out.push_str(&base.hex_stripped);
            } else {
                out.push_str(&base.hex);
            }
        }
        "rgb" => {
            if inverted {
                if strip {
                    out.push_str(&format!("{}, {}, {}", r, g, bl));
                } else {
                    out.push_str(&format!("rgb({}, {}, {})", r, g, bl));
                }
            } else if strip {
                out.push_str(&base.rgb_stripped);
            } else {
                out.push_str(&base.rgb);
            }
        }
        "rgba" => {
            if strip {
                out.push_str(&format!("{},{},{},{}", r, g, bl, alpha));
            } else {
                out.push_str(&format!("rgba({},{},{},{})", r, g, bl, alpha));
            }
        }
        "hsl" => {
            if inverted {
                let (h, s, l) = utils::rgb_to_hsl_css(r, g, bl);
                if strip {
                    out.push_str(&format!("{}, {}%, {}%", h, s, l));
                } else {
                    out.push_str(&format!("hsl({}, {}%, {}%)", h, s, l));
                }
            } else if strip {
                out.push_str(&base.hsl_stripped);
            } else {
                out.push_str(&base.hsl);
            }
        }
        _ => {
            *errors += 1;
            out.push_str("{{ INVALID_FORMAT }}");
        }
    }
    j
}

pub fn get_default_config_path() -> Result<PathBuf> {
    let home = dirs::config_dir().context("Could not find config directory")?;
    Ok(home.join("bnchroma/config.toml"))
}

pub fn default_template_path() -> Result<PathBuf> {
    let home = dirs::config_dir().context("Could not find config directory")?;
    Ok(home.join("bnchroma/templates/example.template"))
}

const DEFAULT_CONFIG_CONTENT: &str = "[vars]\nradius = \"12\"\naccent = \"#3a3a3a\"\n\n[[templates]]\nname = \"example\"\ntemplate = \"~/.config/bnchroma/templates/example.template\"\noutput = \"/tmp/bnchroma-example.output\"\n";

const DEFAULT_EXAMPLE_TEMPLATE: &str = include_str!("../assets/example.template");

fn write_if_missing(path: &PathBuf, content: &str) -> Result<bool> {
    if path.exists() {
        println!("Already exists, skipping: {}", path.display());
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    fs::write(path, content)
        .with_context(|| format!("Failed to write file: {}", path.display()))?;
    println!("Created: {}", path.display());
    Ok(true)
}

pub fn init_default_config() -> Result<()> {
    let config_path = get_default_config_path()?;
    let template_path = default_template_path()?;
    write_if_missing(&config_path, DEFAULT_CONFIG_CONTENT)?;
    write_if_missing(&template_path, DEFAULT_EXAMPLE_TEMPLATE)?;
    Ok(())
}

pub fn load_config(path: &str) -> Result<Config> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path))?;
    let cfg: Config = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path))?;

    for entry in &cfg.templates {
        validate_entry(entry).with_context(|| format!("Invalid entry in {}", path))?;
    }
    RenderCtx::new(&cfg.vars).with_context(|| format!("Invalid [vars] in {}", path))?;
    Ok(cfg)
}

pub fn render_templates(
    configs: &[TemplateConfig],
    colors: &[utils::Rgb],
    source_color: utils::Rgb,
    ctx: &RenderCtx,
    quiet: bool,
) -> Result<()> {
    let pal = prepare_palette(colors, source_color);
    let accent = format!(
        "#{:02X}{:02X}{:02X}",
        source_color.0, source_color.1, source_color.2
    );
    let mut cmd_env: Vec<(String, String)> = Vec::with_capacity(36);
    cmd_env.push(("BNCHROMA_ACCENT".to_string(), accent.clone()));
    cmd_env.push((
        "BNCHROMA_ACCENT_RGB".to_string(),
        format!("{}, {}, {}", source_color.0, source_color.1, source_color.2),
    ));
    for (i, c) in pal.grad.iter().enumerate() {
        let tag = format!("BNCHROMA_C{:02}", i + 1);
        cmd_env.push((format!("{tag}_HEX"), c.hex.clone()));
        cmd_env.push((format!("{tag}_RGB"), c.rgb_stripped.clone()));
    }

    if !quiet {
        let first_color = colors[0];
        let last_color = colors[colors.len() - 1];
        if first_color == (0, 0, 0) {
            print_warning("First color is pure black (should be avoided)");
        }
        if last_color == (255, 255, 255) {
            print_warning("Last color is pure white (should be avoided)");
        }

        print_info(&format!(
            "First color: #{:02X}{:02X}{:02X}",
            first_color.0, first_color.1, first_color.2
        ));
        print_info(&format!(
            "Last color: #{:02X}{:02X}{:02X}",
            last_color.0, last_color.1, last_color.2
        ));
        print_header(&format!("Processing {} templates", configs.len()));
    }

    let mut processed = String::new();
    let mut header_buf = String::new();
    let mut footer_buf = String::new();
    let mut final_content = String::new();

    for cfg in configs {
        if !quiet {
            print_header(&format!("Processing '{}'", cfg.name));
            if let Some(t) = &cfg.template {
                print_info(&format!("Template: {}", t));
            }
        }

        processed.clear();
        let mut replacements = 0usize;
        let mut errors = 0usize;
        if let Some(t) = &cfg.template {
            let template = shellexpand::tilde(t);
            let content = fs::read_to_string(template.as_ref())
                .with_context(|| format!("Failed to read template: {}", template))?;
            let (r, e) = render_into(&pal, ctx, &content, &mut processed);
            replacements += r;
            errors += e;
        }

        header_buf.clear();
        if let Some(header) = &cfg.header {
            let (h_replacements, h_errors) = render_into(&pal, ctx, header, &mut header_buf);
            replacements += h_replacements;
            errors += h_errors;
        }

        footer_buf.clear();
        if let Some(footer) = &cfg.footer {
            let (f_replacements, f_errors) = render_into(&pal, ctx, footer, &mut footer_buf);
            replacements += f_replacements;
            errors += f_errors;
        }

        let trimmed_main = processed.trim_end_matches('\n');
        let trimmed_header = header_buf.trim_end_matches('\n');
        let trimmed_footer = footer_buf.trim_start_matches('\n');

        final_content.clear();
        if !trimmed_header.is_empty() {
            final_content.push_str(trimmed_header);
            final_content.push_str("\n\n");
        }
        final_content.push_str(trimmed_main);
        if !trimmed_footer.is_empty() {
            if !trimmed_main.is_empty() || !trimmed_header.is_empty() {
                final_content.push_str("\n\n");
            }
            final_content.push_str(trimmed_footer);
        }

        if !quiet {
            print_info(&format!(
                "Replacements: {}, Errors: {}",
                replacements, errors
            ));
        }

        let output_paths: &[String] = cfg.output.as_deref().unwrap_or(&[]);
        let has_outputs = !output_paths.is_empty();

        let mut any_changed = false;
        for output in output_paths {
            let expanded = shellexpand::tilde(output).into_owned();
            if !quiet {
                print_info(&format!("Output: {}", expanded));
            }
            let path = std::path::Path::new(&expanded);
            if write_if_changed(path, final_content.as_bytes())
                .with_context(|| format!("Failed to write output: {}", expanded))?
            {
                any_changed = true;
                if !quiet {
                    print_success(&format!(
                        "Generated: {} ({} bytes)",
                        expanded,
                        final_content.len()
                    ));
                }
            }
        }
        if has_outputs && !any_changed {
            if !quiet {
                print_info("Skipped (unchanged outputs)");
            }
            continue;
        }

        if let Some(cmds) = &cfg.command {
            for cmd in cmds {
                if !quiet {
                    print_info(&format!("Executing: {}", cmd));
                }
                let status = Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .envs(cmd_env.iter().map(|(k, v)| (k, v)))
                    .status()
                    .with_context(|| format!("Failed to execute command: {}", cmd))?;

                if !quiet {
                    if status.success() {
                        print_success("Command executed successfully");
                    } else {
                        print_warning("Command failed !")
                    }
                }
            }
        }
    }

    if !quiet {
        print_header("Process completed");
    }
    Ok(())
}

pub fn print_header(message: &str) {
    println!("\n[{}]", message);
}

pub fn print_info(message: &str) {
    println!("[INFO] {}", message);
}

pub fn print_warning(message: &str) {
    println!("[WARNING] {}", message);
}

pub fn print_success(message: &str) {
    println!("[SUCCESS] {}", message);
}
