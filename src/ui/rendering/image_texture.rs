//! Image textures that egui draws and the color adjustment shader can also sample
//!
//! With the window's wgpu renderer attached, textures are created here and registered
//! with egui, so the adjustment shader has them in the same frame they are uploaded.
//! Without it, as in tests, they are ordinary egui textures drawn without adjustments

use std::sync::Arc;

use eframe::{
    egui_wgpu::{self, RenderState},
    wgpu,
};
use egui::{ColorImage, TextureHandle, TextureOptions, mutex::RwLock};

use super::adjusted;

/// Creates image textures, on the window's device when one is attached
pub struct TextureStore {
    ctx: egui::Context,
    gpu: Option<GpuTextures>,
}

struct GpuTextures {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<RwLock<egui_wgpu::Renderer>>,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
}

pub enum ImageTexture {
    Egui(TextureHandle),
    Gpu(GpuTexture),
}

pub struct GpuTexture {
    texture: wgpu::Texture,
    id: egui::TextureId,
    bind_group: wgpu::BindGroup,
    renderer: Arc<RwLock<egui_wgpu::Renderer>>,
}

impl TextureStore {
    pub fn new(ctx: egui::Context) -> Self {
        Self { ctx, gpu: None }
    }

    /// Creates later textures on the window's device so adjustments can draw them
    pub fn attach(&mut self, graphics: &RenderState) {
        let layout = adjusted::install(graphics);
        let device = graphics.device.clone();

        self.gpu = Some(GpuTextures {
            linear: device.create_sampler(&sampler(false)),
            nearest: device.create_sampler(&sampler(true)),
            queue: graphics.queue.clone(),
            renderer: Arc::clone(&graphics.renderer),
            layout,
            device,
        });
    }

    pub fn load(&self, name: &str, image: ColorImage, nearest: bool) -> ImageTexture {
        match &self.gpu {
            Some(gpu) if !image.size.contains(&0) => {
                ImageTexture::Gpu(gpu.create(name, &image, nearest))
            }
            _ => ImageTexture::Egui(self.ctx.load_texture(
                name,
                image,
                texture_options(nearest),
            )),
        }
    }

    /// Replaces a texture's pixels, reusing it when the size is unchanged
    pub fn replace(
        &self,
        texture: &mut Option<ImageTexture>,
        name: &str,
        image: ColorImage,
        nearest: bool,
    ) {
        match (texture.as_mut(), &self.gpu) {
            (Some(ImageTexture::Egui(handle)), None) => {
                handle.set(image, texture_options(nearest));
            }
            (Some(ImageTexture::Gpu(existing)), Some(gpu))
                if existing.size() == image.size =>
            {
                gpu.write(&existing.texture, &image);
            }
            _ => *texture = Some(self.load(name, image, nearest)),
        }
    }
}

impl GpuTextures {
    fn create(&self, name: &str, image: &ColorImage, nearest: bool) -> GpuTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: extent(image),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.write(&texture, image);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = self
            .renderer
            .write()
            .register_native_texture_with_sampler_options(
                &self.device,
                &view,
                sampler(nearest),
            );
        let sampler = if nearest { &self.nearest } else { &self.linear };
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(name),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        GpuTexture {
            texture,
            id,
            bind_group,
            renderer: Arc::clone(&self.renderer),
        }
    }

    fn write(&self, texture: &wgpu::Texture, image: &ColorImage) {
        let size = extent(image);

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.width),
                rows_per_image: Some(size.height),
            },
            size,
        );
    }
}

impl ImageTexture {
    /// The texture as egui's mesh painter knows it
    pub fn id(&self) -> egui::TextureId {
        match self {
            Self::Egui(handle) => handle.id(),
            Self::Gpu(texture) => texture.id,
        }
    }

    /// The bind group the adjustment shader samples, when this texture has one
    pub fn adjustable(&self) -> Option<&wgpu::BindGroup> {
        match self {
            Self::Egui(_) => None,
            Self::Gpu(texture) => Some(&texture.bind_group),
        }
    }
}

impl GpuTexture {
    fn size(&self) -> [usize; 2] {
        [self.texture.width(), self.texture.height()].map(|side| side as usize)
    }
}

impl Drop for GpuTexture {
    fn drop(&mut self) {
        // Textures drop during UI updates, never while egui holds the renderer to paint
        self.renderer.write().free_texture(&self.id);
    }
}

fn extent(image: &ColorImage) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: image.size[0] as u32,
        height: image.size[1] as u32,
        depth_or_array_layers: 1,
    }
}

/// Matches the sampling of egui's own linear and nearest textures
fn sampler(nearest: bool) -> wgpu::SamplerDescriptor<'static> {
    let filter = if nearest {
        wgpu::FilterMode::Nearest
    } else {
        wgpu::FilterMode::Linear
    };

    wgpu::SamplerDescriptor {
        label: Some("flowa-image"),
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    }
}

fn texture_options(nearest: bool) -> TextureOptions {
    if nearest {
        TextureOptions::NEAREST
    } else {
        TextureOptions::LINEAR
    }
}
