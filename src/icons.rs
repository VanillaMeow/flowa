use egui::{Color32, Pos2, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Open,
    Left,
    Right,
    Fit,
    Actual,
    Minus,
    Plus,
    Rotate,
    Filmstrip,
    Info,
    Settings,
    Fullscreen,
    Close,
    Play,
    Pause,
    Image,
    Check,
}

pub fn button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let response = ui.add(
        egui::Button::new("")
            .min_size(vec2(34.0, 34.0))
            .selected(selected),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    paint(
        ui.painter(),
        icon,
        response.rect.center(),
        18.0,
        if ui.is_enabled() {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color().gamma_multiply(0.5)
        },
    );
    response.on_hover_text(label)
}
pub fn logo(ui: &mut Ui, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    flower(ui.painter(), rect.center(), size * 0.43, color);
    response
}
pub fn flower(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    for n in 0..6 {
        let a = n as f32 * std::f32::consts::TAU / 6.0;
        painter.circle_filled(
            center + vec2(a.cos(), a.sin()) * radius * 0.56,
            radius * 0.44,
            color,
        );
    }
    painter.circle_filled(center, radius * 0.28, color);
}
pub fn paint(p: &egui::Painter, icon: Icon, center: Pos2, size: f32, color: Color32) {
    let s = size / 20.0;
    let point = |x: f32, y: f32| center + vec2(x - 10.0, y - 10.0) * s;
    let stroke = Stroke::new(1.5 * s, color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([point(a.0, a.1), point(b.0, b.1)], stroke);
    };
    let poly = |v: &[(f32, f32)]| {
        p.add(Shape::line(
            v.iter().map(|&(x, y)| point(x, y)).collect(),
            stroke,
        ));
    };
    let rect = |a: (f32, f32), b: (f32, f32)| {
        p.rect_stroke(
            egui::Rect::from_min_max(point(a.0, a.1), point(b.0, b.1)),
            2.0,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Left => poly(&[(12., 4.), (6., 10.), (12., 16.)]),
        Icon::Right => poly(&[(8., 4.), (14., 10.), (8., 16.)]),
        Icon::Plus => {
            line((4., 10.), (16., 10.));
            line((10., 4.), (10., 16.));
        }
        Icon::Minus => line((4., 10.), (16., 10.)),
        Icon::Close => {
            line((5., 5.), (15., 15.));
            line((15., 5.), (5., 15.));
        }
        Icon::Open => {
            rect((3., 8.), (17., 17.));
            line((10., 12.), (10., 2.));
            poly(&[(6., 6.), (10., 2.), (14., 6.)]);
        }
        Icon::Fit => {
            for (x, y, dx, dy) in [
                (3., 3., 1., 1.),
                (17., 3., -1., 1.),
                (3., 17., 1., -1.),
                (17., 17., -1., -1.),
            ] {
                poly(&[(x + 5. * dx, y), (x, y), (x, y + 5. * dy)]);
            }
            rect((7., 7.), (13., 13.));
        }
        Icon::Actual => {
            line((4., 6.), (6., 4.));
            line((6., 4.), (6., 16.));
            line((12., 6.), (14., 4.));
            line((14., 4.), (14., 16.));
            p.circle_filled(point(10., 8.), s, color);
            p.circle_filled(point(10., 13.), s, color);
        }
        Icon::Rotate => {
            poly(&[
                (5., 7.),
                (6., 4.),
                (10., 2.5),
                (14., 4.),
                (17., 8.),
                (16., 13.),
                (12., 17.),
                (7., 17.),
            ]);
            poly(&[(2., 3.), (5., 7.), (9., 6.)]);
        }
        Icon::Filmstrip => {
            rect((2., 3.), (18., 17.));
            line((6., 3.), (6., 17.));
            line((14., 3.), (14., 17.));
            for y in [7., 12.] {
                line((2., y), (6., y));
                line((14., y), (18., y));
            }
        }
        Icon::Info => {
            p.circle_stroke(center, size * 0.43, stroke);
            line((10., 9.), (10., 14.));
            p.circle_filled(point(10., 6.), s, color);
        }
        Icon::Settings => {
            p.circle_stroke(center, size * 0.29, stroke);
            p.circle_stroke(center, size * 0.1, stroke);
            for n in 0..8 {
                let a = n as f32 * std::f32::consts::TAU / 8.;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([center + d * size * 0.31, center + d * size * 0.45], stroke);
            }
        }
        Icon::Fullscreen => {
            for (x, y, dx, dy) in [
                (2., 2., 1., 1.),
                (18., 2., -1., 1.),
                (2., 18., 1., -1.),
                (18., 18., -1., -1.),
            ] {
                poly(&[(x + 5. * dx, y), (x, y), (x, y + 5. * dy)]);
            }
        }
        Icon::Play => {
            p.add(Shape::convex_polygon(
                vec![point(6., 3.), point(17., 10.), point(6., 17.)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            line((6., 4.), (6., 16.));
            line((14., 4.), (14., 16.));
        }
        Icon::Image => {
            rect((2., 3.), (18., 17.));
            poly(&[(3., 15.), (7., 10.), (11., 14.), (14., 11.), (18., 15.)]);
            p.circle_stroke(point(13., 7.), 1.5 * s, stroke);
        }
        Icon::Check => poly(&[(3., 10.), (8., 15.), (17., 5.)]),
    }
}

pub fn swatch(ui: &mut Ui, color: Color32, selected: bool, label: &str) -> Response {
    let response = ui.add(
        egui::Button::new("")
            .fill(color)
            .min_size(vec2(31., 31.))
            .corner_radius(16.),
    );
    if selected {
        let luminance = u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b());
        paint(
            ui.painter(),
            Icon::Check,
            response.rect.center(),
            17.,
            if luminance > 400 {
                Color32::from_rgb(20, 22, 30)
            } else {
                Color32::WHITE
            },
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_text(label)
}

pub fn uv_quad(painter: &egui::Painter, texture: egui::TextureId, positions: [Pos2; 4]) {
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in
        positions
            .into_iter()
            .zip([pos2(0., 0.), pos2(1., 0.), pos2(1., 1.), pos2(0., 1.)])
    {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}
