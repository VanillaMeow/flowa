//! Theme palettes, color mapping, and egui styling

use std::{collections::HashMap, path::PathBuf};

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Visuals};

use crate::settings::{Accent, Theme};

pub fn accent_color(accent: Accent, fallback: Color32) -> Color32 {
    rgb(match accent {
        Accent::Theme => return fallback,
        Accent::Violet => [177, 146, 245],
        Accent::Iris => [144, 151, 255],
        Accent::Blue => [108, 164, 245],
        Accent::Sky => [105, 195, 233],
        Accent::Teal => [74, 191, 181],
        Accent::Mint => [139, 213, 182],
        Accent::Green => [151, 200, 113],
        Accent::Amber => [233, 183, 87],
        Accent::Peach => [239, 172, 133],
        Accent::Rose => [224, 148, 185],
        Accent::Coral => [239, 137, 137],
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub canvas: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub dark: bool,
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    egui::lerp(egui::Rgba::from(a)..=egui::Rgba::from(b), amount).into()
}

pub fn colors(theme: Theme, accent: Accent, system_dark: bool) -> Colors {
    if theme == Theme::System {
        if let Some(mut system_colors) = kde_colors() {
            system_colors.accent = accent_color(accent, system_colors.accent);
            return system_colors;
        }

        return colors(
            if system_dark {
                Theme::Charcoal
            } else {
                Theme::Paper
            },
            accent,
            system_dark,
        );
    }

    let (canvas, panel, text, highlight) = match theme {
        Theme::Charcoal | Theme::System => {
            ([24, 25, 28], [31, 32, 36], [232, 232, 235], [163, 190, 164])
        }
        Theme::Midnight => {
            ([15, 20, 31], [22, 28, 42], [224, 231, 244], [136, 166, 243])
        }
        Theme::Nord => ([36, 42, 53], [46, 52, 64], [236, 239, 244], [136, 192, 208]),
        Theme::Catppuccin => {
            ([24, 24, 37], [30, 30, 46], [205, 214, 244], [203, 166, 247])
        }
        Theme::Dracula => {
            ([30, 31, 42], [40, 42, 54], [248, 248, 242], [189, 147, 249])
        }
        Theme::Forest => ([20, 29, 27], [27, 38, 34], [224, 235, 222], [163, 190, 140]),
        Theme::Oled => ([0, 0, 0], [12, 12, 15], [236, 236, 240], [185, 174, 235]),
        Theme::Paper => (
            [242, 243, 245],
            [252, 252, 253],
            [36, 40, 48],
            [88, 103, 173],
        ),
        Theme::Sand => (
            [235, 231, 223],
            [248, 245, 238],
            [54, 49, 45],
            [148, 99, 75],
        ),
        Theme::Latte => (
            [230, 233, 239],
            [239, 241, 245],
            [76, 79, 105],
            [136, 57, 239],
        ),
        Theme::HighContrast => {
            ([0, 0, 0], [10, 10, 10], [255, 255, 255], [255, 221, 82])
        }
    };

    let panel = rgb(panel);
    let text = rgb(text);

    Colors {
        canvas: rgb(canvas),
        panel,
        text,
        accent: accent_color(accent, rgb(highlight)),
        raised: mix(panel, text, 0.065),
        muted: mix(panel, text, 0.63),
        border: mix(
            panel,
            text,
            if theme == Theme::HighContrast {
                0.65
            } else {
                0.13
            },
        ),
        dark: panel.r() < 128,
    }
}

pub fn apply(ctx: &egui::Context, colors: Colors) {
    let mut style = egui::Style {
        visuals: if colors.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        },
        ..Default::default()
    };

    style.text_styles = [
        (TextStyle::Small, FontId::proportional(13.0)),
        (TextStyle::Body, FontId::proportional(16.0)),
        (TextStyle::Button, FontId::proportional(15.0)),
        (TextStyle::Heading, FontId::proportional(23.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.interact_size.y = 34.0;

    style.visuals.panel_fill = colors.panel;
    style.visuals.window_fill = colors.panel;
    style.visuals.extreme_bg_color = colors.canvas;
    style.visuals.faint_bg_color = colors.raised;
    style.visuals.override_text_color = Some(colors.text);
    style.visuals.window_stroke = Stroke::new(1.0, colors.border);
    style.visuals.window_corner_radius = CornerRadius::same(12);

    style.visuals.selection.bg_fill = mix(colors.panel, colors.accent, 0.28);
    style.visuals.selection.stroke = Stroke::new(1.0, colors.accent);
    style.visuals.hyperlink_color = colors.accent;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.border);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, colors.text);

    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(7);
        widget.fg_stroke = Stroke::new(1.5, colors.text);
        widget.bg_stroke = Stroke::NONE;
        widget.bg_fill = colors.raised;
        widget.weak_bg_fill = colors.raised;
    }

    style.visuals.widgets.hovered.bg_fill = mix(colors.panel, colors.text, 0.12);
    style.visuals.widgets.hovered.weak_bg_fill = mix(colors.panel, colors.text, 0.12);
    style.visuals.widgets.active.bg_fill = mix(colors.panel, colors.accent, 0.24);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, colors.accent);

    ctx.set_global_style(style);
}

pub fn kde_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            directories::BaseDirs::new().map(|d| d.config_dir().to_path_buf())
        })?;

    Some(base.join("kdeglobals"))
}

fn kde_colors() -> Option<Colors> {
    parse_kde_colors(&std::fs::read_to_string(kde_config_path()?).ok()?)
}

fn parse_kde_colors(input: &str) -> Option<Colors> {
    // KDE stores palette roles by section rather than as a single theme color
    let mut group = "";
    let mut values = HashMap::new();

    for line in input.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            group = &line[1..line.len() - 1];
        } else if let Some((key, value)) = line.split_once('=') {
            values.insert(format!("{group}/{}", key.trim()), value.trim());
        }
    }

    let get = |group: &str, key: &str| -> Option<Color32> {
        let value = values.get(&format!("{group}/{key}"))?;
        let mut parts = value.split(',').map(str::trim).map(str::parse::<u8>);
        Some(Color32::from_rgb(
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        ))
    };

    let panel = get("Colors:Window", "BackgroundNormal")?;
    let text = get("Colors:Window", "ForegroundNormal")?;

    Some(Colors {
        panel,
        text,
        canvas: get("Colors:View", "BackgroundNormal").unwrap_or(panel),
        raised: get("Colors:Button", "BackgroundNormal")
            .unwrap_or_else(|| mix(panel, text, 0.08)),
        muted: mix(panel, text, 0.65),
        border: mix(panel, text, 0.16),
        accent: get("Colors:Selection", "BackgroundNormal")
            .unwrap_or(Color32::from_rgb(137, 161, 235)),
        dark: panel.r() as u16 + (panel.g() as u16) + (panel.b() as u16) < 384,
    })
}

#[cfg(test)]
mod tests {
    use egui::Color32;

    use super::parse_kde_colors;

    #[test]
    fn kde_palette_parser_reads_colors_and_rejects_incomplete_input() {
        let c = parse_kde_colors("[Colors:Window]\nBackgroundNormal=24,27,40\nForegroundNormal=211,218,227\n[Colors:Selection]\nBackgroundNormal=197,14,210").expect("valid palette");
        assert_eq!(c.accent, Color32::from_rgb(197, 14, 210));
        assert!(c.dark);

        assert!(parse_kde_colors("[General]\nColorScheme=Sweet").is_none());
    }
}
