# bnchroma

Theme your apps from your wallpaper with one command

- Grabs 8 colors from any wallpaper.
- Builds a smooth 16-color gradient from the one you pick.
- Fills in your app templates (kitty, gtk, quickshell, …).
- Only rewrites files that actually changed, and only then runs their post-commands.
- Hands the palette to your commands through environment variables.
- Can also find the closest named color in a list you give it (`--match`).

Requires [Rust](https://www.rust-lang.org/tools/install).

---

## Install

```sh
cargo install --path .
```

This puts the `bnchroma` binary in `~/.cargo/bin` — make sure it's in your `$PATH`.

---

## Usage

### Theme from a wallpaper

```sh
bnchroma --path ~/Pictures/Wallpapers/007.jpeg --quiet
```

Picks the palette, builds the gradient from its first color, and renders every template from your config. Run it on each wallpaper switch — repeats take milliseconds thanks to the cache in `~/.cache/bnchroma`.

```sh
bnchroma --path wall.jpg --color-index 2 --quiet   # gradient from palette color 2
bnchroma --path wall.jpg --color "#7b5244" --quiet # gradient from this color instead
bnchroma --path wall.jpg --force                   # ignore the cached palette
```

### Closest named color

```sh
bnchroma --color "#7b5244" --match "blue=#afd4ff,brown=#a5734b"
# brown:#A5734B
```

Give it a comma-separated `name=hex` list, it prints the closest entry as `NAME:#RRGGBB`.

### Placeholders

In your templates, `1`–`16` are the gradient steps from darkest to lightest, and `source` is the base color:

```
background = "{{col.1.hex}}"
foreground = "{{col.15.hex}}"
selection  = "{{col.5.hex | strip}}"
faded      = "{{col.9.rgba | alpha:0.5}}"
flipped    = "{{col.6.hex | invert}}"
```

Available formats are `hex`, `rgb`, `rgba` and `hsl`. A typo'd index or format renders as `{{ INVALID_INDEX }}` / `{{ INVALID_FORMAT }}` so you spot it in the output.

Your own values from `[vars]` work the same way:

```
radius = "{{var.radius}}"
accent = "{{var.accent}}"
overlay = "rgba({{var.accent.rgb}}, 0.9)"
```

Unknown names render as `{{ INVALID_VAR }}`.

---

## Configuration

`~/.config/bnchroma/config.toml` (or `-f`). Each entry needs a `name`; `template` and `output` go together (output accepts a list); `command` also accepts a list and may stand alone:

```toml
[vars]
radius = "12"
accent = "#3a3a3a"

[[templates]]
name = "kitty"
template = "~/.config/bnchroma/templates/kitty.conf"
output = "~/.config/kitty/colors.conf"
command = "pkill -SIGUSR1 kitty"

[[templates]]
name = "gtk"
template = "~/.config/bnchroma/templates/gtk.css"
output = [
  "~/.themes/my-theme/gtk-3.0/colors.css",
  "~/.themes/my-theme/gtk-4.0/colors.css",
]
command = [
  "dconf write /org/gnome/desktop/interface/gtk-theme \"''\"",
  "dconf write /org/gnome/desktop/interface/gtk-theme \"'my-theme'\"",
]
```

Every command runs with the palette exported: `BNCHROMA_ACCENT` is the base color as `#RRGGBB`, plus `BNCHROMA_C01_HEX`…`BNCHROMA_C16_HEX` and their `BNCHROMA_C01_RGB`… (`r, g, b`) counterparts.
