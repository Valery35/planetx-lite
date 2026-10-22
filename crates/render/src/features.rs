// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свои объекты глобуса в видеокарте: линии, заливка многоугольников,
//! стены до земли. Перенос `render/features.py` модуля.
//!
//! Вершины объекта считает слой выше: центр в f64 и смещения от него
//! в f32. Здесь буферы и отрисовка. Объекты рисуются после тайлов и неба,
//! с проверкой глубины и без записи в неё: стены, заливки, линии.
//! Точки рисует система надписей.

use bytemuck::{Pod, Zeroable};
use globe_core::DVec3;
use globe_core::camera::Camera;
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Подтяжка к глазу в доле расстояния. На 2 км это 6 м, на 100 км 300 м.
pub const PULL: f32 = 0.003;
/// Непрозрачность стены линии без заливки, 0-255.
pub const WALL_ALPHA: u8 = 100;

/// Объект для отрисовки: центр, смещения вершин от центра, пары вершин
/// отрезков, тройки треугольников заливки и стены, цвета RGBA 0-255
/// в значениях экрана, толщина линии в логических пикселях.
#[derive(Debug, Clone, Default)]
pub struct FeatureMesh {
    pub center: DVec3,
    pub offsets: Vec<[f32; 3]>,
    pub lines: Vec<u32>,
    pub fill: Vec<u32>,
    pub walls: Vec<u32>,
    pub line_color: [u8; 4],
    pub fill_color: Option<[u8; 4]>,
    pub width: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FeatureUniform {
    mv: [f32; 16],
    projection: [f32; 16],
    color: [f32; 4],
    params: [f32; 4],
}

/// Объект в видеокарте.
struct GpuFeature {
    center: DVec3,
    vertices: Option<wgpu::Buffer>,
    segments: Option<(wgpu::Buffer, u32)>,
    fill: Option<(wgpu::Buffer, u32)>,
    walls: Option<(wgpu::Buffer, u32)>,
    line_color: [u8; 4],
    fill_color: Option<[u8; 4]>,
    width: f32,
}

/// Что рисуется: стена, заливка, линия.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Walls,
    Fill,
    Lines,
}

/// Объекты и их конвейеры.
pub struct Features {
    line_pipeline: wgpu::RenderPipeline,
    fill_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    stride: u64,
    uniform: wgpu::Buffer,
    capacity: u64,
    bind: wgpu::BindGroup,
    items: Vec<GpuFeature>,
}

fn rgba(c: [u8; 4]) -> [f32; 4] {
    c.map(|v| f32::from(v) / 255.0)
}

impl Features {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("features"),
            source: wgpu::ShaderSource::Wgsl(include_str!("features.wgsl").into()),
        });
        let size = std::mem::size_of::<FeatureUniform>() as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("feature"),
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
            label: Some("features"),
            bind_group_layouts: &[Some(&layout)],
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
        let depth = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let fragment = wgpu::FragmentState {
            module: &shader,
            entry_point: Some("feature_fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        };
        let stride3 = std::mem::size_of::<[f32; 3]>() as u64;
        let fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("feature fill"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("fill_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: stride3,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            // Заливка видна с обеих сторон: порядок вершин чужого
            // многоугольника неизвестен.
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth.clone()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(fragment.clone()),
            multiview_mask: None,
            cache: None,
        });
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("feature lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("line_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 2 * stride3,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(fragment),
            multiview_mask: None,
            cache: None,
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = size.div_ceil(align) * align;
        let capacity = 64;
        let (uniform, bind) = Self::uniform_buffer(device, &layout, stride, capacity);
        Self { line_pipeline, fill_pipeline, layout, stride, uniform, capacity, bind, items: Vec::new() }
    }

    fn uniform_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        stride: u64,
        capacity: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("feature draws"),
            size: stride * capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("feature draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<FeatureUniform>() as u64),
                }),
            }],
        });
        (buffer, bind)
    }

    /// Новый набор объектов. Индексы за пределами вершин отбрасываются.
    pub fn set(&mut self, device: &wgpu::Device, meshes: &[FeatureMesh]) {
        let index = |usage: wgpu::BufferUsages, data: &[u32], label: &str| -> Option<(wgpu::Buffer, u32)> {
            let count = u32::try_from(data.len()).ok().filter(|c| *c > 0)?;
            let buffer =
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(data), usage });
            Some((buffer, count))
        };
        self.items = meshes
            .iter()
            .map(|m| {
                let n = m.offsets.len();
                let valid = |list: &[u32], group: usize| -> Vec<u32> {
                    list.chunks_exact(group)
                        .filter(|c| c.iter().all(|i| (*i as usize) < n))
                        .flatten()
                        .copied()
                        .collect()
                };
                let segments: Vec<[f32; 3]> =
                    valid(&m.lines, 2).iter().map(|i| m.offsets[*i as usize]).collect();
                let segment_count = u32::try_from(segments.len() / 2).unwrap_or(0);
                let segments = (segment_count > 0).then(|| {
                    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("feature segments"),
                        contents: bytemuck::cast_slice(&segments),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    (buffer, segment_count)
                });
                let vertices = (n > 0).then(|| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("feature vertices"),
                        contents: bytemuck::cast_slice(&m.offsets),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                });
                GpuFeature {
                    center: m.center,
                    vertices,
                    segments,
                    fill: index(wgpu::BufferUsages::INDEX, &valid(&m.fill, 3), "feature fill"),
                    walls: index(wgpu::BufferUsages::INDEX, &valid(&m.walls, 3), "feature walls"),
                    line_color: m.line_color,
                    fill_color: m.fill_color,
                    width: m.width,
                }
            })
            .collect();
    }

    /// Записать матрицы и цвета кадра. ratio - пикселей кадра
    /// на логический пиксель. Возвращает список отрисовок.
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera: &Camera, ratio: f32) -> Vec<(usize, Part)> {
        let projection = (depth_fix() * camera.projection()).as_mat4().to_cols_array();
        let viewport = [camera.width as f32, camera.height as f32];
        let mut draws = Vec::new();
        let mut uniforms = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            let mv = camera.tile_model_view(item.center, 1.0).as_mat4().to_cols_array();
            let parts = [
                (Part::Walls, item.walls.is_some(), item.fill_color.unwrap_or({
                    let [r, g, b, _] = item.line_color;
                    [r, g, b, WALL_ALPHA]
                })),
                (Part::Fill, item.fill.is_some() && item.fill_color.is_some(), item.fill_color.unwrap_or([0; 4])),
                (Part::Lines, item.segments.is_some(), item.line_color),
            ];
            for (part, present, color) in parts {
                if present {
                    draws.push((i, part));
                    uniforms.push(FeatureUniform {
                        mv,
                        projection,
                        color: rgba(color),
                        params: [PULL, item.width * ratio, viewport[0], viewport[1]],
                    });
                }
            }
        }
        let count = uniforms.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            (self.uniform, self.bind) = Self::uniform_buffer(device, &self.layout, self.stride, self.capacity);
        }
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * uniforms.len()];
        for (chunk, u) in bytes.chunks_exact_mut(stride).zip(&uniforms) {
            let raw = bytemuck::bytes_of(u);
            chunk[..raw.len()].copy_from_slice(raw);
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        draws
    }

    /// Нарисовать объекты в открытом проходе кадра.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        camera: &Camera,
        ratio: f32,
    ) -> usize {
        if self.items.is_empty() {
            return 0;
        }
        let draws = self.prepare(device, queue, camera, ratio);
        let mut count = 0;
        for (slot, (i, part)) in draws.iter().enumerate() {
            let Some(item) = self.items.get(*i) else {
                continue;
            };
            let Ok(offset) = u32::try_from(slot as u64 * self.stride) else {
                continue;
            };
            match part {
                Part::Lines => {
                    let Some((segments, n)) = &item.segments else {
                        continue;
                    };
                    pass.set_pipeline(&self.line_pipeline);
                    pass.set_bind_group(0, &self.bind, &[offset]);
                    pass.set_vertex_buffer(0, segments.slice(..));
                    pass.draw(0..6, 0..*n);
                }
                Part::Fill | Part::Walls => {
                    let index = if *part == Part::Fill { &item.fill } else { &item.walls };
                    let (Some(vertices), Some((indices, n))) = (&item.vertices, index) else {
                        continue;
                    };
                    pass.set_pipeline(&self.fill_pipeline);
                    pass.set_bind_group(0, &self.bind, &[offset]);
                    pass.set_vertex_buffer(0, vertices.slice(..));
                    pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                }
            }
            count += 1;
        }
        count
    }
}
