// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облака точек в видеокарте.
//!
//! Облако считает слой выше: центр в f64, смещения точек от него в f32
//! и цвета. Точка - квадрат размера [`PointsMesh::size`] логических
//! пикселей, с проверкой и записью глубины: ближние точки закрывают
//! дальние. Облака рисуются после тайлов, до своих объектов.
//!
//! Те же облака рисуют слои точек каталога - землетрясения, вулканы,
//! пожары: круг с тёмным ободком, как шейдер `render/quakes.py` модуля,
//! размер у каждой точки свой, глубина чуть ближе к глазу, чтобы точка
//! на рельефе не уходила в него.

use bytemuck::{Pod, Zeroable};
use globe_core::DVec3;
use globe_core::camera::Camera;
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Облако для отрисовки: центр, смещения точек от центра, цвета RGB
/// в значениях экрана, размер точки в логических пикселях.
#[derive(Debug, Clone, Default)]
pub struct PointsMesh {
    pub center: DVec3,
    pub offsets: Vec<[f32; 3]>,
    pub colors: Vec<[u8; 3]>,
    pub size: f32,
    /// Размеры точек в логических пикселях. Пусто - у всех size.
    pub sizes: Vec<f32>,
    /// Круг с тёмным ободком и непрозрачность вместо квадрата.
    pub dot: Option<Dot>,
}

/// Вид точек слоя: непрозрачность от 0 до 1 и сдвиг глубины к глазу
/// в долях глубины кадра.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dot {
    pub alpha: f32,
    pub lift: f32,
    /// Точки в космосе, спутники: глубина не дальше дальней плоскости,
    /// иначе спутник за ней обрезается. Закрытые Землёй точки убирает
    /// вызывающий.
    pub space: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PointsUniform {
    mv: [f32; 16],
    projection: [f32; 16],
    params: [f32; 4],
    style: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    offset: [f32; 3],
    color: [u8; 4],
    size: f32,
}

struct GpuCloud {
    center: DVec3,
    instances: wgpu::Buffer,
    count: u32,
    dot: Option<Dot>,
}

/// Облака и их конвейер.
pub struct Points {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    stride: u64,
    uniform: wgpu::Buffer,
    capacity: u64,
    bind: wgpu::BindGroup,
    items: Vec<GpuCloud>,
}

impl Points {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("points"),
            source: wgpu::ShaderSource::Wgsl(include_str!("points.wgsl").into()),
        });
        let size = std::mem::size_of::<PointsUniform>() as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("points"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("points"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("points"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("points_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Float32],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("points_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = size.div_ceil(align) * align;
        let capacity = 4;
        let (uniform, bind) = Self::uniform_buffer(device, &layout, stride, capacity);
        Self { pipeline, layout, stride, uniform, capacity, bind, items: Vec::new() }
    }

    fn uniform_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, stride: u64, capacity: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("points draws"),
            size: stride * capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("points draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<PointsUniform>() as u64),
                }),
            }],
        });
        (buffer, bind)
    }

    /// Новый набор облаков. Лишние цвета или смещения отбрасываются.
    pub fn set(&mut self, device: &wgpu::Device, clouds: &[PointsMesh]) {
        self.items = clouds
            .iter()
            .filter_map(|c| {
                let alpha = c.dot.map_or(255, |d| (d.alpha.clamp(0.0, 1.0) * 255.0).round() as u8);
                let instances: Vec<Instance> = c
                    .offsets
                    .iter()
                    .zip(&c.colors)
                    .enumerate()
                    .map(|(n, (o, [r, g, b]))| Instance { offset: *o, color: [*r, *g, *b, alpha], size: c.sizes.get(n).copied().unwrap_or(c.size) })
                    .collect();
                let count = u32::try_from(instances.len()).ok().filter(|n| *n > 0)?;
                Some(GpuCloud {
                    center: c.center,
                    instances: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("points"),
                        contents: bytemuck::cast_slice(&instances),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    count,
                    dot: c.dot,
                })
            })
            .collect();
    }

    /// Нарисовать облака в открытом проходе кадра. ratio - пикселей
    /// кадра на логический пиксель. Возвращает количество отрисовок.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, camera: &Camera, ratio: f32) -> usize {
        if self.items.is_empty() {
            return 0;
        }
        let count = self.items.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            (self.uniform, self.bind) = Self::uniform_buffer(device, &self.layout, self.stride, self.capacity);
        }
        let projection = (depth_fix() * camera.projection()).as_mat4().to_cols_array();
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * self.items.len()];
        for (chunk, item) in bytes.chunks_exact_mut(stride).zip(&self.items) {
            let u = PointsUniform {
                mv: camera.tile_model_view(item.center, 1.0).as_mat4().to_cols_array(),
                projection,
                params: [ratio, item.dot.map_or(0.0, |d| d.lift), camera.width as f32, camera.height as f32],
                style: [f32::from(item.dot.is_some()), f32::from(item.dot.is_some_and(|d| d.space)), 0.0, 0.0],
            };
            let raw = bytemuck::bytes_of(&u);
            chunk[..raw.len()].copy_from_slice(raw);
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        pass.set_pipeline(&self.pipeline);
        let mut drawn = 0;
        for (slot, item) in self.items.iter().enumerate() {
            let Ok(offset) = u32::try_from(slot as u64 * self.stride) else {
                continue;
            };
            pass.set_bind_group(0, &self.bind, &[offset]);
            pass.set_vertex_buffer(0, item.instances.slice(..));
            pass.draw(0..6, 0..item.count);
            drawn += 1;
        }
        drawn
    }
}
