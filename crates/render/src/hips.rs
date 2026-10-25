// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тайлы снимков неба HiPS в видеокарте, перенос `HipsSky`
//! (`render/hips.py`) модуля. Тайл - сетка направлений J2000 с долями
//! картинки и его картинка. Какие тайлы лежат в видеокарте, с какой
//! непрозрачностью и в каком порядке рисуются, решает слой выше: здесь
//! только хранение и отрисовка.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use globe_core::camera::Camera;
use globe_core::{DMat4, DVec4};
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Тайл: номер слоя, порядок HEALPix, номер ячейки. Номер ячейки
/// u64::MAX у слоя - картинка Allsky.jpg порядка 3 под тайлом.
pub type HipsKey = (u8, u32, u64);

/// Вершина тайла: направление и доля картинки.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct HipsVertex {
    pub dir: [f32; 3],
    pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HipsUniform {
    mvp: [f32; 16],
    params: [f32; 4],
}

struct GpuTile {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    bind: wgpu::BindGroup,
}

/// Снимки неба и их конвейер.
pub struct HipsSky {
    pipeline: wgpu::RenderPipeline,
    tile_layout: wgpu::BindGroupLayout,
    draw_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    stride: u64,
    tiles: HashMap<HipsKey, GpuTile>,
    /// Что рисовать в кадре: тайл и непрозрачность, по порядку.
    order: Vec<(HipsKey, f32)>,
}

impl HipsSky {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hips"), source: wgpu::ShaderSource::Wgsl(include_str!("hips.wgsl").into()) });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hips draw"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<HipsUniform>() as u64) },
                count: None,
            }],
        });
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hips tile"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("hips"), bind_group_layouts: &[Some(&draw_layout), Some(&tile_layout)], immediate_size: 0 });
        // Слой ложится поверх Млечного пути с непрозрачностью проявления,
        // альфа кадра остаётся.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hips"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("hips_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HipsVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
                })],
            },
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
                entry_point: Some("hips_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hips"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = (std::mem::size_of::<HipsUniform>() as u64).div_ceil(align) * align;
        Self { pipeline, tile_layout, draw_layout, sampler, stride, tiles: HashMap::new(), order: Vec::new() }
    }

    /// Положить тайл: вершины, треугольники и картинку RGBA. Картинка
    /// не своего размера отбрасывается.
    #[allow(clippy::too_many_arguments)]
    pub fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: HipsKey, vertices: &[HipsVertex], indices: &[u32], width: u32, height: u32, rgba: &[u8]) {
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 || vertices.is_empty() {
            return;
        }
        let Ok(count) = u32::try_from(indices.len()) else {
            return;
        };
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hips tile"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hips tile"),
            layout: &self.tile_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hips vertices"), contents: bytemuck::cast_slice(vertices), usage: wgpu::BufferUsages::VERTEX });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hips indices"), contents: bytemuck::cast_slice(indices), usage: wgpu::BufferUsages::INDEX });
        self.tiles.insert(key, GpuTile { vertices, indices, count, bind });
    }

    pub fn has(&self, key: HipsKey) -> bool {
        self.tiles.contains_key(&key)
    }

    /// Убрать тайлы, которых нет в keep.
    pub fn retain(&mut self, keep: &dyn Fn(HipsKey) -> bool) {
        self.tiles.retain(|k, _| keep(*k));
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Что рисовать в следующих кадрах: тайлы с непрозрачностью по порядку.
    pub fn set_order(&mut self, order: Vec<(HipsKey, f32)>) {
        self.order = order;
    }

    /// Нарисовать тайлы в открытом проходе кадра неба.
    pub fn draw(&self, device: &wgpu::Device, pass: &mut wgpu::RenderPass<'_>, camera: &Camera) -> usize {
        let order: Vec<(&GpuTile, f32)> = self.order.iter().filter_map(|(k, a)| self.tiles.get(k).map(|t| (t, *a))).filter(|(_, a)| *a > 0.0).collect();
        if order.is_empty() {
            return 0;
        }
        let view = camera.rotation.transpose();
        let model = DMat4::from_cols(view.x_axis.extend(0.0), view.y_axis.extend(0.0), view.z_axis.extend(0.0), DVec4::W);
        let mvp = (depth_fix() * camera.projection() * model).as_mat4().to_cols_array();
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * order.len()];
        for (chunk, (_, alpha)) in bytes.chunks_exact_mut(stride).zip(&order) {
            let raw = bytemuck::bytes_of(&HipsUniform { mvp, params: [*alpha, 0.0, 0.0, 0.0] }).to_vec();
            chunk[..raw.len()].copy_from_slice(&raw);
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("hips draws"), contents: &bytes, usage: wgpu::BufferUsages::UNIFORM });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hips draws"),
            layout: &self.draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &uniform, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<HipsUniform>() as u64) }),
            }],
        });
        pass.set_pipeline(&self.pipeline);
        for (slot, (tile, _)) in order.iter().enumerate() {
            let Ok(offset) = u32::try_from(slot as u64 * self.stride) else {
                continue;
            };
            pass.set_bind_group(0, &bind, &[offset]);
            pass.set_bind_group(1, &tile.bind, &[]);
            pass.set_vertex_buffer(0, tile.vertices.slice(..));
            pass.set_index_buffer(tile.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..tile.count, 0, 0..1);
        }
        order.len()
    }
}
