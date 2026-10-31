//! Preferences window, tabs, and shortcut reference

use egui::{Color32, RichText, vec2};

use crate::settings::{Accent, Canvas, Settings, Theme};

use super::{
    icons,
    theme::{self, Colors},
};

pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+O", "Open image"),
    ("Ctrl+Shift+O", "Open folder"),
    ("Left / Right", "Previous / next image"),
    ("Home / End", "First / last image"),
    ("F5", "Refresh folder and reload image"),
    ("Wheel / pinch", "Zoom toward the cursor"),
    ("Drag", "Pan the image"),
    ("Double-click / 0", "Fit without enlarging"),
    ("1", "Actual pixels"),
    ("+ / -", "Zoom in / out"),
    ("R / Shift+R", "Rotate clockwise / counterclockwise"),
    ("H / V", "Flip horizontally / vertically"),
    ("T", "Toggle thumbnail strip"),
    ("I", "Toggle image details"),
    ("Space", "Pause animation / toggle slideshow"),
    ("F / F11", "Toggle fullscreen"),
    ("Esc", "Close dialog / leave fullscreen"),
    ("Ctrl+C", "Copy image"),
    ("Ctrl+Shift+C", "Copy file path"),
    ("Ctrl+Shift+S", "Export a copy"),
    ("F2", "Rename image"),
    ("Delete", "Move to trash, with confirmation"),
    ("Ctrl+,", "Preferences"),
    ("Ctrl+Q", "Quit"),
];

#[derive(Default)]
pub struct PreferencesWindow {
    pub open: bool,
    tab: usize,
}

impl PreferencesWindow {
    pub fn new(open: bool) -> Self {
        Self { open, tab: 0 }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        colors: Colors,
        gpu: &str,
    ) -> bool {
        if !self.open {
            return false;
        }

        let before = serde_json::to_string(settings).unwrap_or_default();
        let default_position =
            (ctx.viewport_rect().center() - vec2(330., 310.)).max(egui::pos2(12., 12.));

        egui::Window::new("Preferences")
            .id(egui::Id::unique("preferences"))
            .open(&mut self.open)
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(660., 620.))
            .default_pos(default_position)
            .min_size(vec2(460., 360.))
            .show(ctx, |ui| {
                preference_tabs(ui, &mut self.tab);

                ui.add_space(12.);
                ui.separator();
                ui.add_space(8.);

                egui::ScrollArea::vertical()
                    .max_height(540.)
                    .show(ui, |ui| preference_tab(ui, self.tab, settings, colors, gpu));
            });

        before != serde_json::to_string(settings).unwrap_or_default()
    }
}

fn preference_tabs(ui: &mut egui::Ui, tab: &mut usize) {
    let tabs = ["Appearance", "Viewing", "Performance", "Shortcuts", "About"];

    ui.horizontal(|ui| {
        for (index, label) in tabs.iter().enumerate() {
            ui.selectable_value(tab, index, *label);
        }
    });
}

fn preference_tab(
    ui: &mut egui::Ui,
    tab: usize,
    settings: &mut Settings,
    colors: Colors,
    gpu: &str,
) {
    match tab {
        0 => appearance(ui, settings, colors),
        1 => viewing(ui, settings),
        2 => performance(ui, settings, colors, gpu),
        3 => shortcuts(ui),
        _ => about(ui, colors),
    }
}

fn viewing(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Make yourself at home");
    ui.add_space(12.);

    ui.checkbox(&mut settings.filmstrip, "Show thumbnail strip");
    ui.checkbox(&mut settings.info, "Show image details");
    ui.checkbox(&mut settings.fit_small, "Enlarge small images when fitting");
    ui.checkbox(
        &mut settings.remember_view,
        "Keep zoom and pan between images",
    );
    ui.checkbox(
        &mut settings.wrap_navigation,
        "Wrap from the last image to the first",
    );
    ui.checkbox(&mut settings.reopen_last, "Reopen the last image on launch");
    ui.checkbox(
        &mut settings.nearest,
        "Use nearest-neighbor sampling for pixel art",
    );

    ui.add_space(16.);
    ui.label("Slideshow interval");
    ui.add(
        egui::Slider::new(&mut settings.slideshow_seconds, 1.0..=60.0)
            .suffix(" seconds"),
    );

    ui.add_space(16.);

    if ui.button("Clear recent images").clicked() {
        settings.recent.clear();
    }

    ui.small("Rotation and flips affect the view. Export creates a new file.");
}

fn performance(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors, gpu: &str) {
    ui.heading("Room for bigger images");
    ui.add_space(12.);

    ui.label("Decoded image memory limit");
    ui.add(
        egui::Slider::new(&mut settings.memory_mb, 128..=4096)
            .logarithmic(true)
            .suffix(" MB"),
    );
    ui.label(
        RichText::new(concat!(
            "Applies when opening the next image. A 100-megapixel RGBA image uses ",
            "about 400 MB before decoder overhead.",
        ))
        .color(colors.muted),
    );

    ui.add_space(16.);
    ui.label(concat!(
        "Full-resolution images use 1024-pixel tiles. Only visible tiles upload ",
        "to the GPU, with a bounded cache and a preview underneath.",
    ));

    ui.add_space(12.);
    ui.label(concat!(
        "GPU tile cache: up to 192 MB. Thumbnail cache: up to 128 images. ",
        "Animation frames share the decoded image limit.",
    ));

    ui.add_space(20.);
    ui.label(RichText::new("Renderer").color(colors.muted));
    ui.label(gpu);

    ui.add_space(16.);
    ui.small(concat!(
        "The current display pipeline converts to 8-bit sRGB. HDR output, ",
        "camera RAW, and layered PSD are not supported in this version.",
    ));
}

fn shortcuts(ui: &mut egui::Ui) {
    ui.heading("Keep your hands on the keys");
    ui.add_space(12.);

    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing(vec2(36., 10.))
        .show(ui, |ui| {
            for &(key, description) in SHORTCUTS {
                ui.monospace(key);
                ui.label(description);
                ui.end_row();
            }
        });
}

fn about(ui: &mut egui::Ui, colors: Colors) {
    icons::logo(ui, 52., colors.accent);
    ui.heading("flowa");
    ui.label("An open space for your images.");

    ui.add_space(12.);
    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
    ui.label("Built with Rust, egui, and wgpu.");

    ui.add_space(16.);
    ui.label(concat!(
        "JPEG · PNG / APNG · GIF · WebP · SVG · TIFF · BMP · ICO · JPEG XL · ",
        "OpenEXR · HDR · TGA · DDS · QOI · PNM · farbfeld",
    ));

    if cfg!(feature = "avif") {
        ui.label("AVIF decoding enabled through dav1d.");
    }

    if cfg!(feature = "heif") {
        ui.label("HEIC / HEIF decoding enabled through libheif.");
    }

    ui.add_space(12.);
    ui.small(concat!(
        "GIF, APNG, and WebP animation are supported. SVG is rendered at its ",
        "intrinsic size, capped at 16384 pixels. JPEG XL currently displays ",
        "the first frame.",
    ));

    ui.add_space(12.);
    ui.label(concat!(
        "Embedded RGB ICC profiles are converted to sRGB. Monitor-specific ",
        "profiling and full HDR output are not implemented.",
    ));

    ui.add_space(12.);
    ui.collapsing("Icon licenses", |ui| {
        ui.label("Lucide icons");
        ui.add(egui::Label::new(icons::LICENSE).wrap().selectable(true));
    });
}

fn appearance(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    ui.heading("A space that feels like yours");
    ui.add_space(8.);
    ui.label(
        RichText::new(concat!(
            "System / Qt follows KDE’s color palette on Linux. ",
            "Other desktops use the system light or dark preference.",
        ))
        .color(colors.muted),
    );
    ui.add_space(14.);

    theme_picker(ui, settings, colors);

    ui.add_space(18.);
    ui.label("Accent palette");
    ui.horizontal_wrapped(|ui| accent_picker(ui, settings, colors));

    ui.add_space(18.);
    ui.label("Canvas background");
    ui.horizontal_wrapped(|ui| canvas_picker(ui, settings));

    if settings.canvas == Canvas::Custom {
        custom_canvas_color(ui, settings);
    }

    ui.add_space(14.);
    checkerboard_controls(ui, settings);

    ui.add_space(16.);
    ui.label("Interface size");
    ui.add(
        egui::Slider::new(&mut settings.ui_scale, 0.8..=1.75)
            .step_by(0.05)
            .suffix("×"),
    );
}

fn theme_picker(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    let columns = ((ui.available_width() + 9.) / 170.).floor().clamp(1., 3.) as usize;
    let card_width =
        (ui.available_width() - (columns - 1) as f32 * 9.) / columns as f32;

    egui::Grid::new("themes")
        .num_columns(columns)
        .spacing(vec2(9., 9.))
        .show(ui, |ui| {
            for (index, theme) in Theme::ALL.into_iter().enumerate() {
                let selected = settings.theme == theme;
                let button = theme_card(theme, selected, card_width, colors);

                if ui.add(button).clicked() {
                    settings.theme = theme;
                }

                if index % columns == columns - 1 {
                    ui.end_row();
                }
            }
        });
}

fn theme_card(
    theme: Theme,
    selected: bool,
    width: f32,
    colors: Colors,
) -> egui::Button<'static> {
    let preview = theme::colors(theme, Accent::Theme, colors.dark);
    let border_width = if selected { 2. } else { 1. };
    let border_color = if selected {
        colors.accent
    } else {
        colors.border
    };
    let title = RichText::new(theme.name()).color(preview.text);

    egui::Button::new(title)
        .fill(preview.panel)
        .stroke(egui::Stroke::new(border_width, border_color))
        .min_size(vec2(width, 54.))
}

fn accent_picker(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    for accent in Accent::ALL {
        let theme_accent =
            theme::colors(settings.theme, Accent::Theme, colors.dark).accent;
        let color = theme::accent_color(accent, theme_accent);
        let selected = settings.accent == accent;

        if icons::swatch(ui, color, selected, accent.name()).clicked() {
            settings.accent = accent;
        }
    }
}

fn canvas_picker(ui: &mut egui::Ui, settings: &mut Settings) {
    for (canvas, label) in [
        (Canvas::Theme, "Theme"),
        (Canvas::Dark, "Neutral dark"),
        (Canvas::Black, "Black"),
        (Canvas::White, "White"),
        (Canvas::Custom, "Custom"),
    ] {
        ui.selectable_value(&mut settings.canvas, canvas, label);
    }
}

fn custom_canvas_color(ui: &mut egui::Ui, settings: &mut Settings) {
    let [red, green, blue] = settings.canvas_color;
    let mut color = Color32::from_rgb(red, green, blue);

    if ui.color_edit_button_srgba(&mut color).changed() {
        settings.canvas_color = [color.r(), color.g(), color.b()];
    }
}

fn checkerboard_controls(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.checkbox(
        &mut settings.checkerboard,
        "Show checkerboard behind transparent pixels",
    );

    if !settings.checkerboard {
        return;
    }

    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut settings.checker_size, 4.0..=40.0)
                .text("Square size"),
        );
        ui.add(
            egui::Slider::new(&mut settings.checker_contrast, 0.02..=0.3)
                .text("Contrast"),
        );
    });
}
