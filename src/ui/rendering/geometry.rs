//! Textured geometry shared by canvas rendering code

use egui::{Color32, Pos2, Shape, pos2};

pub fn uv_quad(
    painter: &egui::Painter,
    texture: egui::TextureId,
    positions: [Pos2; 4],
) {
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in positions.into_iter().zip([
        pos2(0., 0.),
        pos2(1., 0.),
        pos2(1., 1.),
        pos2(0., 1.),
    ]) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}
