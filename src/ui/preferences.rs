//! Preferences window, tabs, and shortcut reference

use egui::{Color32, RichText, vec2};
use strum::VariantArray;

use crate::settings::{Accent, Canvas, RendererPreference, Settings, Theme};
use crate::video::VideoSession;

use super::{
    actions, icons,
    theme::{self, Colors},
};

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
        video: &VideoSession,
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
                    .show(ui, |ui| {
                        preference_tab(ui, self.tab, settings, colors, gpu, video)
                    });
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
    video: &VideoSession,
) {
    match tab {
        0 => appearance(ui, settings, colors),
        1 => viewing(ui, settings),
        2 => performance(ui, settings, colors, gpu),
        3 => shortcuts(ui),
        _ => about(ui, colors, video),
    }
}

fn viewing(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.checkbox(&mut settings.filmstrip, "Show thumbnail strip");
    ui.checkbox(
        &mut settings.compact_video_controls,
        "Compact video controls",
    );
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
        .selected_text(renderer.as_ref())
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
    for &choice in RendererPreference::SUPPORTED {
        ui.selectable_value(renderer, choice, choice.as_ref());
    }
}

fn shortcuts(ui: &mut egui::Ui) {
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing(vec2(36., 10.))
        .show(ui, shortcut_rows);
}

fn shortcut_rows(ui: &mut egui::Ui) {
    for action in actions::GLOBAL
        .iter()
        .chain(actions::NAVIGATION)
        .chain(actions::VIEWER)
    {
        ui.monospace(action.shortcut_text(ui.ctx()));
        ui.label(action.label.trim_end_matches('…'));
        ui.end_row();
    }

    for (gesture, description) in [
        ("Wheel / pinch", "Zoom toward the cursor"),
        ("Drag", "Pan the image"),
        ("Double-click", "Fit to window"),
    ] {
        ui.monospace(gesture);
        ui.label(description);
        ui.end_row();
    }
}

fn about(ui: &mut egui::Ui, colors: Colors, video: &VideoSession) {
    ui.vertical_centered(|ui| about_contents(ui, colors, video));
}

fn about_contents(ui: &mut egui::Ui, colors: Colors, video: &VideoSession) {
    icons::logo(ui, 52., colors.accent);
    ui.heading("flowa");

    ui.add_space(12.);
    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
    ui.label("A native image and video viewer");

    ui.add_space(16.);
    ui.label(RichText::new("Images").strong());
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

    if cfg!(all(feature = "video", target_os = "linux")) {
        about_video(ui, colors, video);
    }

    ui.add_space(12.);
    icon_licenses(ui);
}

fn about_video(ui: &mut egui::Ui, colors: Colors, video: &VideoSession) {
    ui.add_space(18.);
    ui.label(RichText::new("Video").strong());
    ui.label("MP4 · MKV · WebM · MOV · AVI · MPEG · M2TS / MTS · TS · OGV");
    ui.label("FFmpeg hardware decoding · Vulkan rendering · SDL3 audio");
    ui.label("Video frames stay on the GPU; software decoding is disabled");
    ui.add_space(8.);

    let status = if video.available() {
        "Hardware video backend is ready · Vulkan"
    } else {
        "Video playback is unavailable"
    };
    ui.label(RichText::new(status).color(colors.muted));

    if !video.available() {
        ui.add(egui::Label::new(video.unavailable_reason()).wrap());
    }

    ui.add_space(8.);
    ui.horizontal_wrapped(|ui| {
        ui.hyperlink_to("libplacebo", "https://libplacebo.org");
        ui.hyperlink_to("FFmpeg", "https://ffmpeg.org");
        ui.hyperlink_to("SDL3", "https://libsdl.org");
    });
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
            for (index, theme) in Theme::VARIANTS.iter().copied().enumerate() {
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
    let title = RichText::new(theme.as_ref()).color(preview.text);

    egui::Button::new(title)
        .fill(preview.panel)
        .stroke(egui::Stroke::new(border_width, border_color))
        .min_size(vec2(width, 54.))
}

fn accent_picker(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    for &accent in Accent::VARIANTS {
        let theme_accent =
            theme::colors(settings.theme, Accent::Theme, colors.dark).accent;
        let color = theme::accent_color(accent, theme_accent);
        let selected = settings.accent == accent;

        if icons::swatch(ui, color, selected, accent.as_ref()).clicked() {
            settings.accent = accent;
        }
    }
}

fn canvas_picker(ui: &mut egui::Ui, settings: &mut Settings) {
    for &canvas in Canvas::VARIANTS {
        ui.selectable_value(&mut settings.canvas, canvas, canvas.as_ref());
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
