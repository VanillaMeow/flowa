//! Preferences window, tabs, and shortcut reference

use egui::{Color32, RichText, vec2};
use strum::{AsRefStr, VariantArray};

use crate::{
    settings::{
        Accent, CHECKER_CONTRAST, CHECKER_SIZE, Canvas, MEMORY_MB, RendererPreference,
        SLIDESHOW_SECONDS, Settings, Theme, UI_SCALE, VideoDecoder,
    },
    video::VideoSession,
};

use super::{
    actions,
    icons::{self, Icon},
    style::{space, text},
    theme::{self, Colors},
    widgets::{self, hint, section},
};

const AUTHOR: &str = "VanillaMeow";
const HEART_SIZE: f32 = 14.;
const HEART_COLOR: Color32 = Color32::from_rgb(232, 89, 107);

#[derive(Clone, Copy, Default, PartialEq, Eq, AsRefStr, VariantArray)]
enum Tab {
    #[default]
    Appearance,
    Viewing,
    Performance,
    Shortcuts,
    About,
}

#[derive(Default)]
pub struct PreferencesWindow {
    pub open: bool,
    tab: Tab,
}

impl PreferencesWindow {
    pub fn new(open: bool) -> Self {
        Self {
            open,
            tab: Tab::default(),
        }
    }

    /// Edits settings in place; the caller compares them to detect changes
    ///
    /// Call this every frame: egui keeps drawing the window while it fades out
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        colors: Colors,
        gpu: &str,
        video: &VideoSession,
    ) {
        let default_position =
            (ctx.viewport_rect().center() - vec2(330., 310.)).max(egui::pos2(12., 12.));

        let style = ctx.global_style();
        let title_frame = egui::Frame::window(&style);
        let mut content_frame = title_frame;

        // Keep the resize grip outside the content viewport
        let resize_space =
            style.visuals.resize_corner_size + style.spacing.item_spacing.y;
        content_frame.inner_margin.bottom += resize_space.ceil() as i8;

        let tab = &mut self.tab;

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

                preference_tabs(ui, tab, colors);
                ui.add_space(space::LG);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(540.)
                    .show(ui, |ui| {
                        preference_tab(ui, *tab, settings, colors, gpu, video)
                    });
            });
    }
}

fn preference_tabs(ui: &mut egui::Ui, selected: &mut Tab, colors: Colors) {
    let tabs = Tab::VARIANTS.iter().map(|tab| (*tab, tab.as_ref()));
    widgets::segmented(ui, selected, tabs, colors);
}

fn preference_tab(
    ui: &mut egui::Ui,
    tab: Tab,
    settings: &mut Settings,
    colors: Colors,
    gpu: &str,
    video: &VideoSession,
) {
    match tab {
        Tab::Appearance => appearance(ui, settings, colors),
        Tab::Viewing => viewing(ui, settings, colors),
        Tab::Performance => performance(ui, settings, colors, gpu, video),
        Tab::Shortcuts => shortcuts(ui, colors),
        Tab::About => about(ui, colors),
    }
}

fn viewing(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    section(ui, "Images", colors, |ui| {
        ui.checkbox(&mut settings.fit_small, "Enlarge small images when fitting");
        ui.checkbox(
            &mut settings.remember_view,
            "Keep zoom and pan between images",
        );
        ui.checkbox(
            &mut settings.nearest,
            "Use nearest-neighbor sampling for pixel art",
        );
        ui.checkbox(
            &mut settings.wrap_navigation,
            "Wrap from the last image to the first",
        );
    });

    section(ui, "Panels", colors, |ui| {
        ui.checkbox(&mut settings.filmstrip, "Show thumbnail strip");
        ui.checkbox(
            &mut settings.compact_video_controls,
            "Compact video controls",
        );
    });

    section(ui, "Slideshow", colors, |ui| {
        ui.add(
            egui::Slider::new(&mut settings.slideshow_seconds, SLIDESHOW_SECONDS)
                .logarithmic(true)
                .suffix(" seconds"),
        );
    });

    section(ui, "History", colors, |ui| {
        ui.checkbox(&mut settings.reopen_last, "Reopen the last image on launch");
        ui.add_space(space::XS);

        let clear = egui::Button::new("Clear recent files and folders");

        if ui.add_enabled(!settings.recent.is_empty(), clear).clicked() {
            settings.recent.clear();
        }
    });
}

fn performance(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    colors: Colors,
    gpu: &str,
    video: &VideoSession,
) {
    section(ui, "Decoded image memory limit", colors, |ui| {
        ui.add(
            egui::Slider::new(&mut settings.memory_mb, MEMORY_MB)
                .logarithmic(true)
                .suffix(" MiB"),
        );
        hint(ui, "Applies to the next image opened", colors);
    });

    section(ui, "Video decoder", colors, |ui| {
        video_decoder(ui, &mut settings.video_decoder, video, colors);
    });

    section(ui, "Renderer", colors, |ui| {
        renderer_preference(ui, &mut settings.renderer, colors);
        ui.add_space(space::SM);
        widgets::caption(ui, "Active renderer", colors);
        ui.label(gpu);
    });
}

fn video_decoder(
    ui: &mut egui::Ui,
    selected: &mut VideoDecoder,
    video: &VideoSession,
    colors: Colors,
) {
    let decoders = video.available_decoders();
    let label = match decoders {
        Some(available) if !available.contains(selected) => {
            format!("{} (unavailable)", selected.as_ref())
        }
        _ => selected.as_ref().to_owned(),
    };

    ui.add_enabled_ui(decoders.is_some(), |ui| {
        egui::ComboBox::from_id_salt("video_decoder")
            .selected_text(label)
            .show_ui(ui, |ui| decoder_choices(ui, selected, decoders));
    });

    let explanation = if !video.available() {
        video.unavailable_reason()
    } else if decoders.is_none() {
        "Detecting hardware decoders…"
    } else {
        "Videos a decoder can't play fall back to another decoder"
    };
    hint(ui, explanation, colors);
}

fn decoder_choices(
    ui: &mut egui::Ui,
    selected: &mut VideoDecoder,
    decoders: Option<&[VideoDecoder]>,
) {
    for &choice in decoders.unwrap_or_default() {
        ui.selectable_value(selected, choice, choice.as_ref());
    }
}

fn renderer_preference(
    ui: &mut egui::Ui,
    renderer: &mut RendererPreference,
    colors: Colors,
) {
    egui::ComboBox::from_id_salt("renderer_preference")
        .selected_text(renderer.as_ref())
        .show_ui(ui, |ui| renderer_choices(ui, renderer));

    hint(ui, "Changes apply after restarting flowa", colors);

    if cfg!(any(target_os = "linux", target_os = "windows"))
        && *renderer == RendererPreference::Auto
    {
        let explanation = if cfg!(target_os = "windows") {
            "Auto tries Vulkan first, then DirectX 12 if needed"
        } else {
            "Auto tries Vulkan first, then OpenGL if needed"
        };

        hint(ui, explanation, colors);
    }

    if std::env::var_os("WGPU_BACKEND").is_some() {
        hint(ui, "WGPU_BACKEND is overriding this preference", colors);
    }
}

fn renderer_choices(ui: &mut egui::Ui, renderer: &mut RendererPreference) {
    for &choice in RendererPreference::SUPPORTED {
        ui.selectable_value(renderer, choice, choice.as_ref());
    }
}

fn shortcuts(ui: &mut egui::Ui, colors: Colors) {
    section(ui, "Keyboard", colors, |ui| {
        let shortcuts = actions::GLOBAL
            .iter()
            .chain(actions::NAVIGATION)
            .chain(actions::VIEWER)
            .map(|action| {
                (
                    action.shortcut_text(ui.ctx()),
                    action.label.trim_end_matches('…'),
                )
            })
            .collect::<Vec<_>>();

        shortcut_grid(ui, "keyboard_shortcuts", shortcuts);
    });

    section(ui, "Mouse", colors, |ui| {
        let gestures = [
            ("Wheel / pinch", "Zoom toward the cursor"),
            ("Drag", "Pan the image"),
            ("Double-click", "Fit to window"),
        ]
        .map(|(gesture, description)| (gesture.to_owned(), description));

        shortcut_grid(ui, "mouse_shortcuts", gestures);
    });
}

fn shortcut_grid(
    ui: &mut egui::Ui,
    id: &str,
    rows: impl IntoIterator<Item = (String, &'static str)>,
) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(vec2(36., space::SM))
        .show(ui, |ui| {
            for (keys, description) in rows {
                ui.monospace(keys);
                ui.label(description);
                ui.end_row();
            }
        });
}

fn about(ui: &mut egui::Ui, colors: Colors) {
    ui.vertical_centered(|ui| about_contents(ui, colors));
}

fn about_contents(ui: &mut egui::Ui, colors: Colors) {
    icons::logo(ui, 56., colors.accent);
    ui.label(RichText::new("flowa").size(text::HEADING));
    ui.label(
        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
            .color(colors.muted),
    );

    ui.add_space(space::SM);
    ui.label("A native image and video viewer");
    credit(ui, colors);

    ui.add_space(space::XL);
    widgets::caption(ui, "Images", colors);
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

    if cfg!(video_playback) {
        about_video(ui, colors);
    }

    ui.add_space(space::LG);
    icon_licenses(ui);
}

/// "Made with ♥ by …", with the heart drawn as an icon since egui has no emoji font
fn credit(ui: &mut egui::Ui, colors: Colors) {
    let gap = space::XS;
    let label = |text: &str| RichText::new(text).size(text::LABEL).color(colors.muted);
    let byline = format!("by {AUTHOR}");
    let width = text_width(ui, label("Made with"))
        + HEART_SIZE
        + text_width(ui, label(&byline))
        + gap * 2.;

    centered_row(ui, width, |ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.label(label("Made with"));

        let (rect, _) =
            ui.allocate_exact_size(vec2(HEART_SIZE, HEART_SIZE), egui::Sense::hover());
        icons::paint(
            ui.painter(),
            Icon::Heart,
            rect.center(),
            HEART_SIZE,
            HEART_COLOR,
        );

        ui.label(label(&byline));
    });
}

fn about_video(ui: &mut egui::Ui, colors: Colors) {
    ui.add_space(space::LG);
    widgets::caption(ui, "Video", colors);
    ui.label("MP4 · MKV · WebM · MOV · AVI · MPEG · M2TS / MTS · TS · OGV");
    ui.label("FFmpeg decoding · Vulkan rendering · SDL3 audio");
    ui.add_space(space::SM);
    centered_links(
        ui,
        &[
            ("libplacebo", "https://libplacebo.org"),
            ("FFmpeg", "https://ffmpeg.org"),
            ("SDL3", "https://libsdl.org"),
        ],
    );
}

fn centered_links(ui: &mut egui::Ui, links: &[(&str, &str)]) {
    let gap = ui.spacing().item_spacing.x;
    let labels: f32 = links.iter().map(|(label, _)| text_width(ui, *label)).sum();
    let width = labels + gap * links.len().saturating_sub(1) as f32;

    centered_row(ui, width, |ui| {
        for (label, url) in links {
            ui.hyperlink_to(*label, *url);
        }
    });
}

/// Horizontal layouts ignore centering, so the row is indented by its measured width
fn centered_row(
    ui: &mut egui::Ui,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - width) / 2.).max(0.));
        add_contents(ui);
    });
}

fn text_width(ui: &egui::Ui, text: impl Into<egui::WidgetText>) -> f32 {
    text.into()
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
        .size()
        .x
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
    section(ui, "Theme", colors, |ui| theme_picker(ui, settings, colors));

    section(ui, "Accent", colors, |ui| {
        ui.horizontal_wrapped(|ui| accent_picker(ui, settings, colors));
    });

    section(ui, "Canvas background", colors, |ui| {
        canvas_picker(ui, settings, colors);

        if settings.canvas == Canvas::Custom {
            custom_canvas_color(ui, settings);
        }

        ui.add_space(space::XS);
        checkerboard_controls(ui, settings);
    });

    section(ui, "Interface size", colors, |ui| {
        interface_scale(ui, &mut settings.ui_scale);
    });
}

fn interface_scale(ui: &mut egui::Ui, scale: &mut f32) {
    let response = ui.add(
        egui::Slider::new(scale, UI_SCALE)
            .step_by(0.05)
            .suffix("×")
            .update_while_editing(false),
    );

    // Rescaling mid-drag would move the slider under the pointer, so wait for release
    let finished = response.drag_stopped() || response.changed() && !response.dragged();

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
    let theme_accent = theme::colors(settings.theme, Accent::Theme, colors.dark).accent;

    for &accent in Accent::VARIANTS {
        let color = theme::accent_color(accent, theme_accent);
        let selected = settings.accent == accent;

        if icons::swatch(ui, color, selected, accent.as_ref()).clicked() {
            settings.accent = accent;
        }
    }
}

fn canvas_picker(ui: &mut egui::Ui, settings: &mut Settings, colors: Colors) {
    let canvases = Canvas::VARIANTS
        .iter()
        .map(|canvas| (*canvas, canvas.as_ref()));
    widgets::segmented(ui, &mut settings.canvas, canvases, colors);
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
            egui::Slider::new(&mut settings.checker_size, CHECKER_SIZE)
                .text("Square size"),
        );
        ui.add(
            egui::Slider::new(&mut settings.checker_contrast, CHECKER_CONTRAST)
                .text("Contrast"),
        );
    });
}
