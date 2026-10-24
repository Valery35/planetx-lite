// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Подземные сетки в видеокарте, перенос `render/subsurface.py` модуля:
//! стволы скважин, кровли пластов, разрезы, стенки и дно выреза.
//!
//! Сетки собирает слой выше: центр в ECEF в f64, вершины смещениями
//! от него в f32 с нормалью и цветом. Сетка именованная, новая сетка
//! с тем же именем заменяет прежнюю. Сетки рисуются до тайлов, с
//! проверкой и записью глубины: прозрачная поверхность ложится поверх
//! них смешиванием, непрозрачная закрывает их проверкой глубины.

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use globe_core::DVec3;
use globe_core::camera::Camera;
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Доля рассеянного света, `AMBIENT` отмывки модуля.
pub const AMBIENT: f32 = 0.35;
/// Высота света над горизонтом, как у отмывки рельефа.
pub const LIGHT_ELEVATION_DEG: f64 = 45.0;

/// Вершина подземной сетки: смещение от центра, нормаль, цвет RGBA.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SubVertex {
    pub offset: [f32; 3],
    pub normal: [f32; 3],
    pub color: [u8; 4],
}

/// Подземная сетка: центр в ECEF, вершины, треугольники. cull_front -
/// грани, обращённые к глазу, не рисуются: сквозь ближнюю стенку блока
/// виден блок изнутри, как `cull` модуля.
#[derive(Debug, Clone, Default)]
pub struct SubMesh {
    pub center: DVec3,
    pub vertices: Vec<SubVertex>,
    pub indices: Vec<u32>,
    pub cull_front: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SubUniform {
    mvp: [f32; 16],
    light: [f32; 4],
}

struct GpuSub {
    center: DVec3,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    cull_front: bool,
}

/// Подземные сетки и их конвейеры.
pub struct Subsurface {
    both: wgpu::RenderPipeline,
    front_culled: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    stride: u64,
    uniform: wgpu::Buffer,
    capacity: u64,
    bind: wgpu::BindGroup,
    meshes: BTreeMap<String, GpuSub>,
}

/// Направление света в ECEF у точки, `light_at` модуля: с северо-запада
/// под LIGHT_ELEVATION_DEG.
pub fn light_at(lat: f64, lon: f64) -> DVec3 {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    let up = DVec3::new(cl * co, cl * so, sl);
    let east = DVec3::new(-so, co, 0.0);
    let north = DVec3::new(-sl * co, -sl * so, cl);
    let elevation = LIGHT_ELEVATION_DEG.to_radians();
    up * elevation.sin() + (north - east) * (elevation.cos() / std::f64::consts::SQRT_2)
}

impl Subsurface {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("subsurface"),
            source: wgpu::ShaderSource::Wgsl(include_str!("subsurface.wgsl").into()),
        });
        let size = std::mem::size_of::<SubUniform>() as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("subsurface"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(size) },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("subsurface"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("subsurface"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("sub_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SubVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4],
                    })],
                },
                primitive: wgpu::PrimitiveState { front_face: wgpu::FrontFace::Ccw, cull_mode: cull, ..Default::default() },
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
                    entry_point: Some("sub_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent::OVER,
                            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let both = make(None);
        let front_culled = make(Some(wgpu::Face::Front));
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = size.div_ceil(align) * align;
        let capacity = 8;
        let (uniform, bind) = uniform_buffer(device, &layout, stride, capacity);
        Self { both, front_culled, layout, stride, uniform, capacity, bind, meshes: BTreeMap::new() }
    }

    /// Новая сетка name, None или пустая - убрать.
    pub fn set(&mut self, device: &wgpu::Device, name: &str, mesh: Option<&SubMesh>) {
        self.meshes.remove(name);
        let Some(mesh) = mesh else {
            return;
        };
        let n = mesh.vertices.len();
        let indices: Vec<u32> = mesh.indices.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|i| (*i as usize) < n)).flatten().copied().collect();
        let Some(count) = u32::try_from(indices.len()).ok().filter(|c| *c > 0) else {
            return;
        };
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("subsurface vertices"), contents: bytemuck::cast_slice(&mesh.vertices), usage: wgpu::BufferUsages::VERTEX });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("subsurface indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
        self.meshes.insert(name.to_string(), GpuSub { center: mesh.center, vertices, indices, count, cull_front: mesh.cull_front });
    }

    /// Убрать все сетки.
    pub fn clear(&mut self) {
        self.meshes.clear();
    }

    /// Нарисовать сетки в открытом проходе кадра, до тайлов. Возвращает
    /// количество отрисовок. scale - вертикальный масштаб рельефа не
    /// нужен: высоты уже на экране.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, camera: &Camera) -> usize {
        if self.meshes.is_empty() {
            return 0;
        }
        let count = self.meshes.len() as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            (self.uniform, self.bind) = uniform_buffer(device, &self.layout, self.stride, self.capacity);
        }
        let (lat, lon, _) = camera.ellipsoid.ecef_to_geodetic(camera.eye);
        let l = light_at(lat, lon);
        let light = [l.x as f32, l.y as f32, l.z as f32, AMBIENT];
        let projection = depth_fix() * camera.projection();
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * self.meshes.len()];
        for (chunk, mesh) in bytes.chunks_exact_mut(stride).zip(self.meshes.values()) {
            let u = SubUniform { mvp: camera.tile_mvp(mesh.center, Some(projection), 1.0).to_cols_array(), light };
            let raw = bytemuck::bytes_of(&u);
            chunk[..raw.len()].copy_from_slice(raw);
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        let mut drawn = 0;
        for (slot, mesh) in self.meshes.values().enumerate() {
            let Ok(offset) = u32::try_from(slot as u64 * self.stride) else {
                continue;
            };
            pass.set_pipeline(if mesh.cull_front { &self.front_culled } else { &self.both });
            pass.set_bind_group(0, &self.bind, &[offset]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, 0..1);
            drawn += 1;
        }
        drawn
    }
}

fn uniform_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, stride: u64, capacity: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("subsurface draws"),
        size: stride * capacity,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("subsurface draws"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<SubUniform>() as u64) }),
        }],
    });
    (buffer, bind)
}
