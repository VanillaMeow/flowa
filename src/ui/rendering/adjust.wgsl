// Draws an image texture with libplacebo's brightness, contrast, saturation, and gamma
//
// Textures hold egui's premultiplied, gamma-encoded colors, and the output follows
// egui's own shader so adjusted quads blend exactly like the rest of the canvas

struct Quad {
    // Screen corners in clip space: left top and right top, then right bottom and
    // left bottom
    corners: array<vec4<f32>, 2>,
    // Brightness, contrast, saturation, and the reciprocal of gamma
    adjust: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> quad: Quad;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // A triangle strip visits left top, right top, left bottom, then right bottom
    var positions = array<vec2<f32>, 4>(
        quad.corners[0].xy,
        quad.corners[0].zw,
        quad.corners[1].zw,
        quad.corners[1].xy,
    );
    var uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    var out: VertexOutput;
    out.position = vec4<f32>(positions[index], 0.0, 1.0);
    out.uv = uvs[index];
    return out;
}

fn adjust(color: vec4<f32>) -> vec4<f32> {
    if color.a <= 0.0 {
        return vec4<f32>(0.0);
    }

    let brightness = quad.adjust.x;
    let contrast = quad.adjust.y;
    let saturation = quad.adjust.z;
    let inverse_gamma = quad.adjust.w;

    let gained = color.rgb / color.a * contrast;
    let luma = dot(gained, LUMA);
    let saturated = vec3<f32>(luma) + (gained - vec3<f32>(luma)) * saturation;
    let lifted = max(saturated + vec3<f32>(brightness), vec3<f32>(0.0));
    let curved = min(pow(lifted, vec3<f32>(inverse_gamma)), vec3<f32>(1.0));

    return vec4<f32>(curved * color.a, color.a);
}

// Interleaved gradient noise, as egui dithers its output
fn dither(rgb: vec3<f32>, position: vec4<f32>) -> vec3<f32> {
    let f = 0.06711056 * position.x + 0.00583715 * position.y;
    let noise = (fract(52.9829189 * fract(f)) - 0.5) * 0.95;
    return rgb + noise / 255.0;
}

fn linear_from_gamma(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_gamma_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = adjust(textureSample(image, image_sampler, in.uv));
    return vec4<f32>(dither(color.rgb, in.position), color.a);
}

@fragment
fn fs_linear_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = adjust(textureSample(image, image_sampler, in.uv));
    return vec4<f32>(linear_from_gamma(dither(color.rgb, in.position)), color.a);
}
