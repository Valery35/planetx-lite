// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Картинки-наложения в видеокарте: GroundOverlay на поверхности
//! и PhotoOverlay перед камерой.
//!
//! Сетку картинки считает слой выше: центр в f64, смещения от него
//! в f32 и координаты в картинке. Картинки хранятся по ключу отдельно
//! от сеток: сетка садится на новые высоты, картинка грузится один раз.
//! Сетка без картинки не рисуется. Картинки рисуются после неба и до
//! своих объектов, с проверкой глубины и без записи в неё, в порядке
//! drawOrder.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use globe_core::DVec3;
use globe_core::camera::Camera;
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Сетка картинки: ключ картинки, центр, смещения вершин, координаты
/// в картинке (0, 0 - левый верхний угол), тройки треугольников, цвет
/// KML RGBA 0-255, подтяжка к глазу в доле расстояния, порядок.
#[derive(Debug, Clone, Default)]
pub struct PictureMesh {
    pub key: String,
    pub center: DVec3,
    pub offsets: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub tint: [u8; 4],
    pub pull: f32,
    pub order: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PictureUniform {
    mv: [f32; 16],
    projection: [f32; 16],
    tint: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    offset: [f32; 3],
    uv: [f32; 2],
}

struct GpuPicture {
    key: String,
    center: DVec3,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    tint: [u8; 4],
    pull: f32,
}

/// Картинки, их сетки и конвейер.
pub struct Pictures {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    stride: u64,
    uniform: wgpu::Buffer,
    capacity: u64,
    bind: wgpu::BindGroup,
    textures: HashMap<String, wgpu::BindGroup>,
    items: Vec<GpuPicture>,
}

/// Уровни уменьшения картинки для мип-карты: каждый вдвое меньше,
/// пиксель - среднее четырёх. Первый уровень - сама картинка.
pub fn mip_levels(width: u32, height: u32, rgba: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, rgba.to_vec())];
    while let Some((w, h, data)) = levels.last() {
        if *w <= 1 && *h <= 1 {
            break;
        }
        let (w, h) = (*w as usize, *h as usize);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (2 * x + dx).min(w - 1);
                        let sy = (2 * y + dy).min(h - 1);
                        sum += u32::from(data.get((sy * w + sx) * 4 + c).copied().unwrap_or(0));
                    }
                    if let Some(v) = next.get_mut((y * nw + x) * 4 + c) {
                        *v = ((sum + 2) / 4) as u8;
                    }
                }
            }
        }
        levels.push((nw as u32, nh as u32, next));
    }
    levels
}

impl Pictures {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pictures"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pictures.wgsl").into()),
        });
        let size = std::mem::size_of::<PictureUniform>() as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture texture"),
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
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pictures"),
            bind_group_layouts: &[Some(&layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        // Цвет смешивается по прозрачности, альфа кадра остаётся 1.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let stride = std::mem::size_of::<Vertex>() as u64;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pictures"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("picture_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                })],
            },
            // Фото видно с обеих сторон.
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("picture_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("picture"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = size.div_ceil(align) * align;
        let capacity = 16;
        let (uniform, bind) = Self::uniform_buffer(device, &layout, stride, capacity);
        Self { pipeline, layout, texture_layout, sampler, stride, uniform, capacity, bind, textures: HashMap::new(), items: Vec::new() }
    }

    fn uniform_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        stride: u64,
        capacity: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture draws"),
            size: stride * capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<PictureUniform>() as u64),
                }),
            }],
        });
        (buffer, bind)
    }

    /// Картинка по ключу, RGBA в значениях экрана. Размер больше
    /// предела видеокарты или не совпавший с байтами отбрасывается.
    pub fn set_image(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: &str, width: u32, height: u32, rgba: &[u8]) -> bool {
        let limit = device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit || rgba.len() != (width as usize) * (height as usize) * 4 {
            return false;
        }
        let levels = mip_levels(width, height, rgba);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("picture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in levels.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
                wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("picture"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.textures.insert(key.to_string(), bind);
        true
    }

    /// Оставить только картинки с ключами keep.
    pub fn keep_images(&mut self, keep: &[String]) {
        self.textures.retain(|k, _| keep.contains(k));
    }

    pub fn has_image(&self, key: &str) -> bool {
        self.textures.contains_key(key)
    }

    /// Новый набор сеток, рисуются в порядке order. Индексы
    /// за пределами вершин отбрасываются.
    pub fn set(&mut self, device: &wgpu::Device, meshes: &[PictureMesh]) {
        let mut sorted: Vec<&PictureMesh> = meshes.iter().collect();
        sorted.sort_by_key(|m| m.order);
        self.items = sorted
            .into_iter()
            .filter_map(|m| {
                let n = m.offsets.len().min(m.uv.len());
                let indices: Vec<u32> =
                    m.indices.as_chunks::<3>().0.iter().filter(|c| c.iter().all(|i| (*i as usize) < n)).flatten().copied().collect();
                let count = u32::try_from(indices.len()).ok().filter(|c| *c > 0)?;
                let vertices: Vec<Vertex> = m.offsets.iter().zip(&m.uv).map(|(o, t)| Vertex { offset: *o, uv: *t }).collect();
                Some(GpuPicture {
                    key: m.key.clone(),
                    center: m.center,
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("picture vertices"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("picture indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    count,
                    tint: m.tint,
                    pull: m.pull,
                })
            })
            .collect();
    }

    /// Нарисовать картинки в открытом проходе кадра. Возвращает
    /// количество отрисовок.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, camera: &Camera) -> usize {
        let ready: Vec<usize> = (0..self.items.len()).filter(|i| self.items.get(*i).is_some_and(|p| self.textures.contains_key(&p.key))).collect();
        if ready.is_empty() {
            return 0;
        }
        let projection = (depth_fix() * camera.projection()).as_mat4().to_cols_array();
        let count = ready.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            (self.uniform, self.bind) = Self::uniform_buffer(device, &self.layout, self.stride, self.capacity);
        }
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * ready.len()];
        for (chunk, i) in bytes.chunks_exact_mut(stride).zip(&ready) {
            let Some(item) = self.items.get(*i) else {
                continue;
            };
            let u = PictureUniform {
                mv: camera.tile_model_view(item.center, 1.0).as_mat4().to_cols_array(),
                projection,
                tint: item.tint.map(|v| f32::from(v) / 255.0),
                params: [item.pull, 0.0, 0.0, 0.0],
            };
            let raw = bytemuck::bytes_of(&u);
            chunk[..raw.len()].copy_from_slice(raw);
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        pass.set_pipeline(&self.pipeline);
        let mut drawn = 0;
        for (slot, i) in ready.iter().enumerate() {
            let (Some(item), Ok(offset)) = (self.items.get(*i), u32::try_from(slot as u64 * self.stride)) else {
                continue;
            };
            let Some(texture) = self.textures.get(&item.key) else {
                continue;
            };
            pass.set_bind_group(0, &self.bind, &[offset]);
            pass.set_bind_group(1, texture, &[]);
            pass.set_vertex_buffer(0, item.vertices.slice(..));
            pass.set_index_buffer(item.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..item.count, 0, 0..1);
            drawn += 1;
        }
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::mip_levels;

    #[test]
    fn mips_halve_to_one_pixel_and_average() {
        let rgba: Vec<u8> = [[0, 0, 0, 255], [200, 0, 0, 255], [0, 100, 0, 255], [200, 100, 40, 255]].concat();
        let levels = mip_levels(2, 2, &rgba);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], (1, 1, vec![100, 50, 10, 255]));
        // Неквадратная картинка доходит до 1x1.
        let tall = mip_levels(1, 5, &[7u8; 20]);
        assert_eq!(tall.iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(), [(1, 5), (1, 2), (1, 1)]);
    }
}
