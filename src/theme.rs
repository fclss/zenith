use std::path::Path;

use ratatui::style::Color;
use serde::Deserialize;

/// Colors the UI draws with. Loaded from `themes/<name>.yaml` under the config
/// dir so users can add their own (see [`Theme::load`]); a built-in default is
/// used when the named file is missing or unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub dim: Color,
    pub selected_fg: Color,
    pub selected_bg: Color,
    pub up: Color,
    pub down: Color,
    pub ok: Color,
    pub warn: Color,
    pub error: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // Palette: lavender B2A6DF, white FFFFFF, maroon 7D3138, navy 2F3067,
        // green C8E3AB. Secondary text and the row highlight stay gray.
        Theme {
            accent: Color::Rgb(0xB2, 0xA6, 0xDF),
            text: Color::Rgb(0xFF, 0xFF, 0xFF),
            dim: Color::DarkGray,
            selected_fg: Color::Black,
            selected_bg: Color::Gray,
            up: Color::Rgb(0xC8, 0xE3, 0xAB),
            down: Color::Rgb(0xB2, 0xA6, 0xDF),
            ok: Color::Rgb(0xC8, 0xE3, 0xAB),
            warn: Color::Rgb(0xB2, 0xA6, 0xDF),
            error: Color::Rgb(0x7D, 0x31, 0x38),
        }
    }
}

/// The on-disk shape: every field optional, so a theme file may override only
/// the colors it cares about.
#[derive(Debug, Default, Deserialize)]
struct ThemeSpec {
    accent: Option<String>,
    text: Option<String>,
    dim: Option<String>,
    selected_fg: Option<String>,
    selected_bg: Option<String>,
    up: Option<String>,
    down: Option<String>,
    ok: Option<String>,
    warn: Option<String>,
    error: Option<String>,
}

impl Theme {
    /// Load `themes/<name>.yaml` beneath `config_dir`, falling back to the
    /// built-in default for any missing file or unparsable color.
    pub fn load(config_dir: &Path, name: &str) -> Theme {
        let path = config_dir.join("themes").join(format!("{name}.yaml"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Theme::default();
        };
        let spec: ThemeSpec = serde_yaml::from_str(&text).unwrap_or_default();
        let mut theme = Theme::default();
        set(&mut theme.accent, &spec.accent);
        set(&mut theme.text, &spec.text);
        set(&mut theme.dim, &spec.dim);
        set(&mut theme.selected_fg, &spec.selected_fg);
        set(&mut theme.selected_bg, &spec.selected_bg);
        set(&mut theme.up, &spec.up);
        set(&mut theme.down, &spec.down);
        set(&mut theme.ok, &spec.ok);
        set(&mut theme.warn, &spec.warn);
        set(&mut theme.error, &spec.error);
        theme
    }
}

fn set(slot: &mut Color, value: &Option<String>) {
    if let Some(color) = value.as_deref().and_then(parse_color) {
        *slot = color;
    }
}

/// Parse `#rrggbb`, `#rgb`, or a handful of common color names. Returns `None`
/// for anything unrecognised so the caller keeps the default.
fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    match s.to_ascii_lowercase().replace(['_', '-', ' '], "").as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "white" => Some(Color::White),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digits: Vec<u8> = hex
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<Vec<u8>>>()?;
    match digits.len() {
        3 => Some(Color::Rgb(digits[0] * 17, digits[1] * 17, digits[2] * 17)),
        6 => Some(Color::Rgb(
            digits[0] * 16 + digits[1],
            digits[2] * 16 + digits[3],
            digits[4] * 16 + digits[5],
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_from_yaml(text: &str) -> Theme {
        let spec: ThemeSpec = serde_yaml::from_str(text).unwrap_or_default();
        let mut theme = Theme::default();
        set(&mut theme.accent, &spec.accent);
        set(&mut theme.up, &spec.up);
        set(&mut theme.down, &spec.down);
        theme
    }

    #[test]
    fn parses_hex_and_named_colors() {
        assert_eq!(parse_color("#ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("light_blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("dark-gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("nonsense"), None);
    }

    #[test]
    fn file_theme_overrides_only_named_fields() {
        let theme = theme_from_yaml("accent: '#123456'\nup: light_red\n");
        assert_eq!(theme.accent, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.up, Color::LightRed);
        assert_eq!(
            theme.down,
            Theme::default().down,
            "unset fields keep defaults"
        );
    }

    #[test]
    fn missing_theme_file_falls_back_to_default() {
        let theme = Theme::load(Path::new("/nonexistent-zenith-dir"), "none");
        assert_eq!(theme, Theme::default());
    }
}
