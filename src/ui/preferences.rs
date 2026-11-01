//! Preferences window, tabs, and shortcut reference

use egui::{Color32, RichText, vec2};

use crate::settings::{Accent, Canvas, RendererPreference, Settings, Theme};

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

        let style = ctx.global_style();
        let title_frame = egui::Frame::window(&style);
        let mut content_frame = title_frame;

        // Keep the resize grip outside the content viewport
        let resize_space =
            style.visuals.resize_corner_size + style.spacing.item_spacing.y;
        content_frame.inner_margin.bottom += resize_space.ceil() as i8;

        egui::Window::new("Preferences")
            .id(egui::Id::unique("preferences"))
            .open(&mut self.open)
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(660., 620.))
            .default_pos(default_position)
            .min_size(vec2(460., 360.))
            .frame(content_frame)
            .title_frame(title_frame)
            .show(ctx, |ui| {
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();

                preference_tabs(ui, &mut self.tab);

                ui.add_space(12.);
                ui.separator();
                ui.add_space(8.);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
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
}

fn performance(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors, gpu: &str) {
    ui.label("Decoded image memory limit");
    ui.add(
        egui::Slider::new(&mut settings.memory_mb, 128..=4096)
            .logarithmic(true)
            .suffix(" MB"),
    )
    .on_hover_text("Applies to the next image opened");

    ui.add_space(20.);
    renderer_preference(ui, &mut settings.renderer, colors);

    ui.add_space(16.);
    ui.label(RichText::new("Active renderer").color(colors.muted));
    ui.label(gpu);
}

fn renderer_preference(
    ui: &mut egui::Ui,
    renderer: &mut RendererPreference,
    colors: Colors,
) {
    ui.label("Renderer");

    egui::ComboBox::from_id_salt("renderer_preference")
        .selected_text(renderer.name())
        .show_ui(ui, |ui| renderer_choices(ui, renderer));

    ui.label(
        RichText::new("Changes apply after restarting Flowa")
            .small()
            .color(colors.muted),
    );

    if cfg!(target_os = "linux") && *renderer == RendererPreference::Auto {
        ui.label(
            RichText::new("Auto tries Vulkan first, then OpenGL if needed")
                .small()
                .color(colors.muted),
        );
    }

    if std::env::var_os("WGPU_BACKEND").is_some() {
        ui.label(
            RichText::new("WGPU_BACKEND is overriding this preference")
                .small()
                .color(colors.muted),
        );
    }
}

fn renderer_choices(ui: &mut egui::Ui, renderer: &mut RendererPreference) {
    for &choice in RendererPreference::ALL {
        ui.selectable_value(renderer, choice, choice.name());
    }
}

fn shortcuts(ui: &mut egui::Ui) {
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
    ui.vertical_centered(|ui| about_contents(ui, colors));
}

fn about_contents(ui: &mut egui::Ui, colors: Colors) {
    icons::logo(ui, 52., colors.accent);
    ui.heading("flowa");

    ui.add_space(12.);
    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));

    ui.add_space(16.);
    ui.label("Supported formats");
    ui.label(concat!(
        "JPEG · PNG / APNG · GIF · WebP · SVG · TIFF · BMP · ICO · JPEG XL · ",
        "OpenEXR · HDR · TGA · DDS · QOI · PNM · farbfeld",
    ));

    if cfg!(feature = "avif") {
        ui.label("AVIF");
    }

    if cfg!(feature = "heif") {
        ui.label("HEIC / HEIF");
    }

    ui.add_space(12.);
    icon_licenses(ui);
}

fn icon_licenses(ui: &mut egui::Ui) {
    let id = ui.make_persistent_id("icon_licenses");
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        false,
    );
    let button = egui::Button::new("Icon licenses").selected(state.is_open());

    if ui.add(button).clicked() {
        state.toggle(ui);
    }

    state.show_body_unindented(ui, icon_license_text);
}

fn icon_license_text(ui: &mut egui::Ui) {
    ui.label("Lucide icons");
    ui.add(egui::Label::new(icons::LICENSE).wrap().selectable(true));
}

fn appearance(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    ui.label("Theme");
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
    interface_scale(ui, &mut settings.ui_scale);
}

fn interface_scale(ui: &mut egui::Ui, scale: &mut f32) {
    ui.label("Interface size");
    let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
    let response = ui.add(
        egui::Slider::new(scale, 0.8..=1.75)
            .step_by(0.05)
            .suffix("×")
            .update_while_editing(false),
    );

    let finished = response.drag_stopped() || (response.lost_focus() && enter);

    if finished {
        ui.ctx().set_zoom_factor(*scale);
    }
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

                let response = ui.add(button);

                if response.clicked() {
                    settings.theme = theme;
                }

                if theme == Theme::System {
                    response.on_hover_text(concat!(
                        "Uses KDE colors on Linux, or the system light/dark theme ",
                        "on other desktops",
                    ));
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
