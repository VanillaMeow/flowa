//! A wgpu pipeline that draws image textures with color adjustments inside egui's pass
//!
//! Each adjusted quad is an egui paint callback. Callbacks stage their corners and
//! adjustments in `prepare`, the first `finish_prepare` uploads them into one uniform
//! buffer, and `paint` binds its slot by dynamic offset

use std::collections::HashMap;

use eframe::{
    egui_wgpu::{
        self, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor,
    },
    wgpu,
};
use egui::Pos2;

use crate::viewer::adjustments::Adjustments;

/// Corners and adjustments of one quad, as `Quad` in `adjust.wgsl`
const QUAD_BYTES: u64 = 48;
/// Quads the uniform buffer starts with room for, enough for a full tile cache
const INITIAL_QUADS: u64 = 64;

/// The pipeline and per-frame quad storage, kept in egui's callback resources
struct AdjustPipeline {
    pipeline: wgpu::RenderPipeline,
    quad_layout: wgpu::BindGroupLayout,
    quads: wgpu::Buffer,
    quad_bind_group: wgpu::BindGroup,
    /// Bytes between quads, a multiple of the device's uniform offset alignment
    stride: u64,
    capacity: u64,
    /// The egui pass whose quads are staged
    pass: u64,
    slots: HashMap<usize, u32>,
    staging: Vec<u8>,
}

impl AdjustPipeline {
    fn new(graphics: &RenderState, texture_layout: &wgpu::BindGroupLayout) -> Self {
        let device = &graphics.device;
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = QUAD_BYTES.div_ceil(alignment) * alignment;
        let quad_layout = quad_layout(device);
        let pipeline = create_pipeline(
            device,
            graphics.target_format,
            &quad_layout,
            texture_layout,
        );
        let (quads, quad_bind_group) =
            quad_buffer(device, &quad_layout, stride, INITIAL_QUADS);

        Self {
            pipeline,
            quad_layout,
            quads,
            quad_bind_group,
            stride,
            capacity: INITIAL_QUADS,
            pass: u64::MAX,
            slots: HashMap::new(),
            staging: Vec::new(),
        }
    }

    /// Queues a quad's uniform, starting over when a new egui pass begins
    fn stage(&mut self, key: usize, pass: u64, uniform: &[u8]) {
        if self.pass != pass {
            self.pass = pass;
            self.slots.clear();
            self.staging.clear();
        }

        let slot = self.slots.len() as u32;
        self.slots.insert(key, slot);

        let start = self.staging.len();
        self.staging.extend_from_slice(uniform);
        self.staging.resize(start + self.stride as usize, 0);
    }

    /// Uploads the staged quads, growing the buffer when the pass needs more room
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.staging.is_empty() {
            return;
        }

        let needed = self.staging.len() as u64 / self.stride;

        if needed > self.capacity {
            let capacity = needed.next_power_of_two();
            (self.quads, self.quad_bind_group) =
                quad_buffer(device, &self.quad_layout, self.stride, capacity);
            self.capacity = capacity;
        }

        queue.write_buffer(&self.quads, 0, &self.staging);
        self.staging.clear();
    }

    fn offset(&self, key: usize) -> Option<u32> {
        let slot = self.slots.get(&key)?;

        Some(slot * self.stride as u32)
    }
}

/// Installs the pipeline for the window's renderer and returns the texture layout that
/// adjusted textures must create their bind groups with
pub fn install(graphics: &RenderState) -> wgpu::BindGroupLayout {
    let texture_layout = texture_layout(&graphics.device);
    let pipeline = AdjustPipeline::new(graphics, &texture_layout);

    graphics
        .renderer
        .write()
        .callback_resources
        .insert(pipeline);

    texture_layout
}

/// One textured quad drawn with color adjustments
pub struct AdjustedQuad {
    /// The egui pass that painted this quad
    pub pass: u64,
    /// Screen corners in points: left top, right top, right bottom, left bottom
    pub corners: [Pos2; 4],
    pub adjustments: Adjustments,
    /// The texture and its sampler, laid out by `install`'s texture layout
    pub texture: wgpu::BindGroup,
}

impl AdjustedQuad {
    pub fn into_shape(self, clip: egui::Rect) -> egui::Shape {
        egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(clip, self))
    }

    /// Identifies this quad within a pass; egui keeps each callback in place until
    /// painting finishes
    fn key(&self) -> usize {
        std::ptr::from_ref(self) as usize
    }

    fn uniform(&self, screen: &ScreenDescriptor) -> Vec<u8> {
        let [width, height] = screen.size_in_pixels.map(|side| side.max(1) as f32);
        let scale = screen.pixels_per_point;
        let clip = |point: Pos2| {
            [
                point.x * scale / width * 2. - 1.,
                1. - point.y * scale / height * 2.,
            ]
        };
        let adjustments = self.adjustments.clamped();
        let values = [
            clip(self.corners[0]),
            clip(self.corners[1]),
            clip(self.corners[2]),
            clip(self.corners[3]),
            [adjustments.brightness, adjustments.contrast],
            [adjustments.saturation, 1. / adjustments.gamma],
        ];

        values
            .as_flattened()
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect()
    }
}

impl CallbackTrait for AdjustedQuad {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(pipeline) = resources.get_mut::<AdjustPipeline>() {
            pipeline.stage(self.key(), self.pass, &self.uniform(screen));
        }

        Vec::new()
    }

    fn finish_prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(pipeline) = resources.get_mut::<AdjustPipeline>() {
            pipeline.upload(device, queue);
        }

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(pipeline) = resources.get::<AdjustPipeline>() else {
            return;
        };
        let Some(offset) = pipeline.offset(self.key()) else {
            return;
        };
        let [width, height] = info.screen_size_px.map(|side| side as f32);

        // Corners are in whole-screen clip space; egui's scissor still clips the quad
        pass.set_viewport(0., 0., width, height, 0., 1.);
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &pipeline.quad_bind_group, &[offset]);
        pass.set_bind_group(1, &self.texture, &[]);
        pass.draw(0..4, 0..1);
    }
}

fn quad_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("flowa-adjusted-quads"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(QUAD_BYTES),
            },
            count: None,
        }],
    })
}

fn texture_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("flowa-adjusted-texture"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn quad_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flowa-adjusted-quads"),
        size: stride * capacity,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("flowa-adjusted-quads"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(QUAD_BYTES),
            }),
        }],
    });

    (buffer, bind_group)
}

/// Matches egui's pipeline: premultiplied blending, one sample, and no depth buffer
fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    quad_layout: &wgpu::BindGroupLayout,
    texture_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::include_wgsl!("adjust.wgsl"));
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("flowa-adjusted"),
        bind_group_layouts: &[Some(quad_layout), Some(texture_layout)],
        immediate_size: 0,
    });
    let fragment = if format.is_srgb() {
        "fs_linear_framebuffer"
    } else {
        "fs_gamma_framebuffer"
    };
    let blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("flowa-adjusted"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(fragment),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(all(test, video_playback))]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use anyhow::{Context, Result, ensure};
    use eframe::egui_wgpu::{RendererOptions, WgpuConfiguration, WgpuSetup};
    use egui::{Color32, ColorImage, Rect, epaint, pos2};

    use super::*;
    use crate::ui::rendering::image_texture::TextureStore;

    const SIDE: u32 = 8;
    const SOURCE: [u8; 3] = [64, 128, 192];

    #[test]
    #[ignore = "Requires a GPU"]
    fn adjusted_quads_match_the_reference_and_use_their_own_slots() -> Result<()> {
        let graphics = graphics()?;
        let mut store = TextureStore::new(egui::Context::default());
        store.attach(&graphics);

        let [red, green, blue] = SOURCE;
        let image =
            ColorImage::new([2, 2], vec![Color32::from_rgb(red, green, blue); 4]);
        let texture = store.load("test", image, false);
        let bind_group = texture.adjustable().context("No GPU texture")?.clone();

        let left = Adjustments {
            brightness: 0.1,
            contrast: 0.8,
            ..Adjustments::NEUTRAL
        };
        let right = Adjustments {
            saturation: 0.,
            gamma: 2.,
            ..Adjustments::NEUTRAL
        };
        let half = SIDE as f32 / 2.;
        let quads = [(left, 0.), (right, half)].map(|(adjustments, x)| AdjustedQuad {
            pass: 1,
            corners: [
                pos2(x, 0.),
                pos2(x + half, 0.),
                pos2(x + half, SIDE as f32),
                pos2(x, SIDE as f32),
            ],
            adjustments,
            texture: bind_group.clone(),
        });

        let pixels = render(&graphics, quads)?;
        let bgra = matches!(
            graphics.target_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );

        for (x, adjustments) in [(1, left), (SIDE - 2, right)] {
            let expected = adjustments
                .apply(SOURCE.map(|channel| f32::from(channel) / 255.))
                .map(|channel| (channel * 255.).round() as i32);
            let mut actual = pixels[(SIDE * (SIDE / 2) + x) as usize].map(i32::from);
            if bgra {
                actual.swap(0, 2);
            }

            let close =
                (0..3).all(|channel| (actual[channel] - expected[channel]).abs() <= 3);
            ensure!(close, "Pixel {x} was {actual:?}, expected {expected:?}");
        }

        Ok(())
    }

    fn graphics() -> Result<RenderState> {
        let setup = WgpuSetup::without_display_handle();
        let instance = pollster::block_on(setup.new_instance());
        let config = WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        };

        Ok(pollster::block_on(RenderState::create(
            &config,
            &instance,
            None,
            RendererOptions::default(),
        ))?)
    }

    fn render(
        graphics: &RenderState,
        quads: [AdjustedQuad; 2],
    ) -> Result<Vec<[u8; 4]>> {
        let device = &graphics.device;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("adjusted-test-target"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: graphics.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let full =
            Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(SIDE as f32));
        let primitives: Vec<_> = quads
            .into_iter()
            .map(|quad| epaint::ClippedPrimitive {
                clip_rect: full,
                primitive: epaint::Primitive::Callback(
                    egui_wgpu::Callback::new_paint_callback(full, quad),
                ),
            })
            .collect();
        let screen = ScreenDescriptor {
            size_in_pixels: [SIDE; 2],
            pixels_per_point: 1.,
        };

        let mut encoder = device.create_command_encoder(&Default::default());
        let mut renderer = graphics.renderer.write();
        let mut buffers = renderer.update_buffers(
            device,
            &graphics.queue,
            &mut encoder,
            &primitives,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("adjusted-test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            renderer.render(&mut pass.forget_lifetime(), &primitives, &screen);
        }
        drop(renderer);

        let row_bytes = 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("adjusted-test-readback"),
            size: u64::from(row_bytes * SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(SIDE),
                },
            },
            target.size(),
        );
        buffers.push(encoder.finish());
        graphics.queue.submit(buffers);

        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv_timeout(Duration::from_secs(5))??;

        let bytes = readback.slice(..).get_mapped_range()?;
        let pixels = bytes
            .chunks(row_bytes as usize)
            .flat_map(|row| row[..(SIDE * 4) as usize].as_chunks::<4>().0.to_vec())
            .collect();

        Ok(pixels)
    }
}
