use crate::{
    icons,
    settings::{Canvas, Settings},
    theme::{self, Accent, Colors, Theme},
};
use egui::{Color32, RichText, vec2};

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

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    s: &mut Settings,
    tab: &mut usize,
    c: Colors,
    gpu: &str,
) -> bool {
    let before = serde_json::to_string(s).unwrap_or_default();
    egui::Window::new("Preferences").id(egui::Id::unique("preferences")).open(open).collapsible(false).resizable(true).default_size(vec2(660.,620.)).default_pos((ctx.viewport_rect().center()-vec2(330.,310.)).max(egui::pos2(12.,12.))).min_size(vec2(460.,360.)).show(ctx,|ui| {
        ui.horizontal(|ui| {for (i,label) in ["Appearance","Viewing","Performance","Shortcuts","About"].iter().enumerate() {ui.selectable_value(tab,i,*label);}});
        ui.add_space(12.);ui.separator();ui.add_space(8.);
        egui::ScrollArea::vertical().max_height(540.).show(ui,|ui| {
            match *tab {
                0=>appearance(ui,s,c),
                1=>{
                    ui.heading("Make yourself at home");ui.add_space(12.);
                    ui.checkbox(&mut s.filmstrip,"Show thumbnail strip");
                    ui.checkbox(&mut s.info,"Show image details");
                    ui.checkbox(&mut s.fit_small,"Enlarge small images when fitting");
                    ui.checkbox(&mut s.remember_view,"Keep zoom and pan between images");
                    ui.checkbox(&mut s.wrap_navigation,"Wrap from the last image to the first");
                    ui.checkbox(&mut s.reopen_last,"Reopen the last image on launch");
                    ui.checkbox(&mut s.nearest,"Use nearest-neighbor sampling for pixel art");
                    ui.add_space(16.);ui.label("Slideshow interval");ui.add(egui::Slider::new(&mut s.slideshow_seconds,1.0..=60.0).suffix(" seconds"));
                    ui.add_space(16.);if ui.button("Clear recent images").clicked() {s.recent.clear();}
                    ui.small("Rotation and flips affect the view. Export creates a new file.");
                }
                2=>{
                    ui.heading("Room for bigger images");ui.add_space(12.);
                    ui.label("Decoded image memory limit");
                    ui.add(egui::Slider::new(&mut s.memory_mb,128..=4096).logarithmic(true).suffix(" MB"));
                    ui.label(RichText::new("Applies when opening the next image. A 100-megapixel RGBA image uses about 400 MB before decoder overhead.").color(c.muted));
                    ui.add_space(16.);ui.label("Full-resolution images use 1024-pixel tiles. Only visible tiles upload to the GPU, with a bounded cache and a preview underneath.");
                    ui.add_space(12.);ui.label("GPU tile cache: up to 192 MB. Thumbnail cache: up to 128 images. Animation frames share the decoded image limit.");
                    ui.add_space(20.);ui.label(RichText::new("Renderer").color(c.muted));ui.label(gpu);
                    ui.add_space(16.);ui.small("The current display pipeline converts to 8-bit sRGB. HDR output, camera RAW, and layered PSD are not supported in this version.");
                }
                3=>{
                    ui.heading("Keep your hands on the keys");ui.add_space(12.);
                    egui::Grid::new("shortcuts").num_columns(2).spacing(vec2(36.,10.)).show(ui,|ui| {for &(key,description) in SHORTCUTS {ui.monospace(key);ui.label(description);ui.end_row();}});
                }
                _=>{
                    icons::logo(ui,52.,c.accent);ui.heading("flowa");ui.label("An open space for your images.");ui.add_space(12.);ui.label(format!("Version {}",env!("CARGO_PKG_VERSION")));
                    ui.label("Built with Rust, egui, and wgpu.");ui.add_space(16.);
                    ui.label("JPEG · PNG / APNG · GIF · WebP · SVG · TIFF · BMP · ICO · JPEG XL · OpenEXR · HDR · TGA · DDS · QOI · PNM · farbfeld");
                    if cfg!(feature="avif") {ui.label("AVIF decoding enabled through dav1d.");}
                    if cfg!(feature="heif") {ui.label("HEIC / HEIF decoding enabled through libheif.");}
                    ui.add_space(12.);ui.small("GIF, APNG, and WebP animation are supported. SVG is rendered at its intrinsic size, capped at 16384 pixels. JPEG XL currently displays the first frame.");
                    ui.add_space(12.);ui.label("Embedded RGB ICC profiles are converted to sRGB. Monitor-specific profiling and full HDR output are not implemented.");
                }
            }
        });
    });
    before != serde_json::to_string(s).unwrap_or_default()
}
fn appearance(ui: &mut egui::Ui, s: &mut Settings, c: Colors) {
    ui.heading("A space that feels like yours");
    ui.add_space(8.);
    ui.label(RichText::new("System / Qt follows KDE’s color palette on Linux. Other desktops use the system light or dark preference.").color(c.muted));
    ui.add_space(14.);
    let columns = ((ui.available_width() + 9.) / 170.).floor().clamp(1., 3.) as usize;
    let card_width = (ui.available_width() - (columns - 1) as f32 * 9.) / columns as f32;
    egui::Grid::new("themes")
        .num_columns(columns)
        .spacing(vec2(9., 9.))
        .show(ui, |ui| {
            for (i, t) in Theme::ALL.into_iter().enumerate() {
                let colors = theme::colors(t, Accent::Theme, c.dark);
                let button = egui::Button::new(RichText::new(t.name()).color(colors.text))
                    .fill(colors.panel)
                    .stroke(egui::Stroke::new(
                        if s.theme == t { 2. } else { 1. },
                        if s.theme == t { c.accent } else { c.border },
                    ))
                    .min_size(vec2(card_width, 54.));
                if ui.add(button).clicked() {
                    s.theme = t;
                }
                if i % columns == columns - 1 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(18.);
    ui.label("Accent palette");
    ui.horizontal_wrapped(|ui| {
        for a in Accent::ALL {
            if icons::swatch(
                ui,
                a.color(theme::colors(s.theme, Accent::Theme, c.dark).accent),
                s.accent == a,
                a.name(),
            )
            .clicked()
            {
                s.accent = a;
            }
        }
    });
    ui.add_space(18.);
    ui.label("Canvas background");
    ui.horizontal_wrapped(|ui| {
        for (v, label) in [
            (Canvas::Theme, "Theme"),
            (Canvas::Dark, "Neutral dark"),
            (Canvas::Black, "Black"),
            (Canvas::White, "White"),
            (Canvas::Custom, "Custom"),
        ] {
            ui.selectable_value(&mut s.canvas, v, label);
        }
    });
    if s.canvas == Canvas::Custom {
        let mut color = Color32::from_rgb(s.canvas_color[0], s.canvas_color[1], s.canvas_color[2]);
        if ui.color_edit_button_srgba(&mut color).changed() {
            s.canvas_color = [color.r(), color.g(), color.b()];
        }
    }
    ui.add_space(14.);
    ui.checkbox(
        &mut s.checkerboard,
        "Show checkerboard behind transparent pixels",
    );
    if s.checkerboard {
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut s.checker_size, 4.0..=40.0).text("Square size"));
            ui.add(egui::Slider::new(&mut s.checker_contrast, 0.02..=0.3).text("Contrast"));
        });
    }
    ui.add_space(16.);
    ui.label("Interface size");
    ui.add(
        egui::Slider::new(&mut s.ui_scale, 0.8..=1.75)
            .step_by(0.05)
            .suffix("×"),
    );
}
