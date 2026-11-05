// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-модели в видеокарте: части с картинкой или цветом материала.
//!
//! Модель считает слой выше: точка модели в f64, вершины частей
//! смещениями от неё (поворот и масштаб уже применены), нормали в тех
//! же осях. Поворот `orient` переводит вершины в оси ECEF в кадре: у
//! едущей модели меняются только точка и поворот. Картинки частей - по ключу модели и пути,
//! грузятся при [`Models::set`]. Модели рисуются после тайлов, с проверкой
//! и записью глубины, грани видны с обеих сторон. Модели отбрасывают
//! тени и принимают их: `draw_depth` рисует глубину в карту теней.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use globe_core::camera::Camera;
use globe_core::shadowmap::{LightFrame, REACH};
use globe_core::{DMat3, DMat4, DVec3};
use wgpu::util::DeviceExt;

use crate::pictures::mip_levels;
use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Доля рассеянного света.
pub const AMBIENT: f32 = 0.55;

/// Часть модели: треугольники вершинами подряд.
#[derive(Debug, Clone, Default)]
pub struct ModelPart {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// Ключ картинки в [`ModelMesh::images`] или None.
    pub image: Option<String>,
    /// Цвет материала RGBA 0-1, у картинки - множитель.
    pub rgba: [f32; 4],
    /// Дорожка анимации или None: часть стоит.
    pub track: Option<usize>,
}

/// Модель: точка в ECEF, части, картинки по ключу (ширина, высота,
/// RGBA), направление на солнце в ECEF.
#[derive(Debug, Clone, Default)]
pub struct ModelMesh {
    pub center: DVec3,
    pub parts: Vec<ModelPart>,
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    pub sun: [f32; 3],
    /// Поворот вершин частей перед сдвигом в точку модели. У стоящей
    /// модели - единичный, у модели на пути меняется каждый кадр
    /// ([`Models::place`]), вершины в видеокарте остаются прежними.
    pub orient: DMat3,
}

/// Место модели в кадре: точка, поворот, солнце, видна ли.
#[derive(Debug, Clone, Copy)]
pub struct ModelPlace {
    pub center: DVec3,
    pub orient: DMat3,
    pub sun: [f32; 3],
    pub shown: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniform {
    mv: [f32; 16],
    projection: [f32; 16],
    rgba: [f32; 4],
    light: [f32; 4],
    shadow_mvp: [f32; 16],
    shadow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    offset: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

struct GpuPart {
    vertices: wgpu::Buffer,
    count: u32,
    /// Номер картинки в [`GpuModel::textures`].
    texture: Option<usize>,
    rgba: [f32; 4],
    track: Option<usize>,
}

/// Модель в видеокарте: ключ, место, матрицы дорожек, картинки, части.
struct GpuModel {
    key: u64,
    place: ModelPlace,
    /// Матрицы дорожек анимации в осях вершин частей.
    deltas: HashMap<usize, DMat4>,
    textures: Vec<wgpu::BindGroup>,
    parts: Vec<GpuPart>,
}

/// Модели и их конвейер.
pub struct Models {
    pipeline: wgpu::RenderPipeline,
    /// Проход карты теней.
    depth_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::BindGroup,
    stride: u64,
    uniform: wgpu::Buffer,
    capacity: u64,
    bind: wgpu::BindGroup,
    /// Настоящее солнце для всех моделей, None - солнце каждой модели.
    pub sun: Option<[f32; 3]>,
    models: Vec<GpuModel>,
}

impl Models {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, shadow_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("models"),
            source: wgpu::ShaderSource::Wgsl(include_str!("models.wgsl").into()),
        });
        let size = std::mem::size_of::<ModelUniform>() as u64;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model"),
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
            label: Some("model texture"),
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
            label: Some("models"),
            bind_group_layouts: &[Some(&layout), Some(&texture_layout), Some(shadow_layout)],
            immediate_size: 0,
        });
        let depth_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("models depth"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
        };
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("models depth"),
            layout: Some(&depth_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("model_depth_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout.clone())],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(crate::shadow::depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("models"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("model_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
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
                entry_point: Some("model_fs"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("model"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let white = texture_bind(device, queue, &texture_layout, &sampler, 1, 1, &[255, 255, 255, 255]);
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = size.div_ceil(align) * align;
        let capacity = 16;
        let (uniform, bind) = uniform_buffer(device, &layout, stride, capacity);
        Self {
            pipeline,
            depth_pipeline,
            layout,
            texture_layout,
            sampler,
            white,
            stride,
            uniform,
            capacity,
            bind,
            sun: None,
            models: Vec::new(),
        }
    }

    /// Новый набор моделей, ключ модели - её номер в списке.
    pub fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, models: &[ModelMesh]) {
        self.models.clear();
        for (m, model) in models.iter().enumerate() {
            self.add(device, queue, m as u64, model);
        }
    }

    /// Модель под ключом key в конец списка, прежняя под тем же ключом
    /// убирается. Картинка размера не по пределу видеокарты или не
    /// совпавшая с байтами - часть без картинки.
    pub fn add(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: u64, model: &ModelMesh) {
        self.remove(key);
        let limit = device.limits().max_texture_dimension_2d;
        let mut textures = Vec::new();
        let mut keys: HashMap<&str, usize> = HashMap::new();
        for (name, w, h, rgba) in &model.images {
            if *w == 0 || *h == 0 || *w > limit || *h > limit || rgba.len() != (*w as usize) * (*h as usize) * 4 {
                continue;
            }
            keys.insert(name.as_str(), textures.len());
            textures.push(texture_bind(device, queue, &self.texture_layout, &self.sampler, *w, *h, rgba));
        }
        let mut parts = Vec::new();
        for part in &model.parts {
            let n = part.positions.len().min(part.normals.len()) / 3 * 3;
            let vertices: Vec<Vertex> = (0..n)
                .map(|i| Vertex {
                    offset: part.positions[i],
                    normal: part.normals[i],
                    uv: part.uv.get(i).copied().unwrap_or([0.0, 0.0]),
                })
                .collect();
            let Some(count) = u32::try_from(vertices.len()).ok().filter(|c| *c > 0) else {
                continue;
            };
            parts.push(GpuPart {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("model vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                count,
                texture: part.image.as_deref().and_then(|k| keys.get(k).copied()),
                rgba: part.rgba,
                track: part.track,
            });
        }
        let place = ModelPlace { center: model.center, orient: model.orient, sun: model.sun, shown: true };
        self.models.push(GpuModel { key, place, deltas: HashMap::new(), textures, parts });
    }

    /// Убрать модель под ключом key.
    pub fn remove(&mut self, key: u64) {
        self.models.retain(|m| m.key != key);
    }

    /// Ключи моделей.
    pub fn keys(&self) -> impl Iterator<Item = u64> + '_ {
        self.models.iter().map(|m| m.key)
    }

    /// Видна ли модель под ключом key.
    pub fn show(&mut self, key: u64, shown: bool) {
        if let Some(m) = self.models.iter_mut().find(|m| m.key == key) {
            m.place.shown = shown;
        }
    }

    /// Новое место модели номер index из [`Models::set`]: модель едет.
    pub fn place(&mut self, index: usize, place: ModelPlace) {
        if let Some(m) = self.models.get_mut(index) {
            m.place = place;
        }
    }

    /// Матрицы дорожек анимации модели номер index на кадр: дорожка -
    /// матрица в осях вершин частей.
    pub fn animate(&mut self, index: usize, deltas: HashMap<usize, DMat4>) {
        if let Some(m) = self.models.get_mut(index) {
            m.deltas = deltas;
        }
    }

    /// Части по порядку слотов буфера: модель и часть.
    fn slots(&self) -> impl Iterator<Item = (&GpuModel, &GpuPart)> + '_ {
        self.models.iter().flat_map(|m| m.parts.iter().map(move |p| (m, p)))
    }

    /// Значения кадра в буфер, до прохода карты теней и прохода кадра.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera: &Camera, light: Option<&LightFrame>) {
        let total = self.models.iter().map(|m| m.parts.len()).sum::<usize>();
        if total == 0 {
            return;
        }
        let count = total as u64;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            (self.uniform, self.bind) = uniform_buffer(device, &self.layout, self.stride, self.capacity);
        }
        let projection = (depth_fix() * camera.projection()).as_mat4().to_cols_array();
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * total];
        for (chunk, (model, part)) in bytes.chunks_exact_mut(stride).zip(self.slots()) {
            let place = &model.place;
            // Часть с дорожкой анимации - её матрица на кадр.
            let delta = part.track.and_then(|t| model.deltas.get(&t).copied()).unwrap_or(DMat4::IDENTITY);
            let turn = DMat4::from_mat3(place.orient) * delta;
            // Нормали вершин - в осях до поворота, солнце поворачивается
            // к ним обратным поворотом.
            let s = self.sun.unwrap_or(place.sun);
            let sun = DVec3::new(f64::from(s[0]), f64::from(s[1]), f64::from(s[2]));
            let sun = (DMat3::from_mat4(turn).transpose() * sun).normalize_or_zero();
            let u = ModelUniform {
                mv: (camera.tile_model_view(place.center, 1.0) * turn).as_mat4().to_cols_array(),
                projection,
                rgba: part.rgba,
                light: [sun.x as f32, sun.y as f32, sun.z as f32, AMBIENT],
                shadow_mvp: light.map_or([0.0; 16], |f| (f.matrix(place.center) * turn).as_mat4().to_cols_array()),
                shadow: light.map_or([0.0; 4], LightFrame::shader_params),
            };
            let raw = bytemuck::bytes_of(&u);
            chunk[..raw.len()].copy_from_slice(raw);
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
    }

    /// Глубина моделей у карты теней в открытом проходе карты. Модель
    /// дальше [`REACH`] от края карты тени в неё не бросает.
    pub fn draw_depth(&self, pass: &mut wgpu::RenderPass<'_>, frame: &LightFrame) {
        if self.models.is_empty() {
            return;
        }
        pass.set_pipeline(&self.depth_pipeline);
        let reach = 1.0 + REACH / frame.radius;
        for (slot, (model, part)) in self.slots().enumerate() {
            let p = &model.place;
            let near = p.shown && frame.project(p.center).truncate().length() <= reach;
            let Some(offset) = u32::try_from(slot as u64 * self.stride).ok().filter(|_| near) else {
                continue;
            };
            pass.set_bind_group(0, &self.bind, &[offset]);
            pass.set_vertex_buffer(0, part.vertices.slice(..));
            pass.draw(0..part.count, 0..1);
        }
    }

    /// Нарисовать модели в открытом проходе кадра. Возвращает
    /// количество отрисовок.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, shadow: &wgpu::BindGroup) -> usize {
        if self.models.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(2, shadow, &[]);
        let mut drawn = 0;
        for (slot, (model, part)) in self.slots().enumerate() {
            if !model.place.shown {
                continue;
            }
            let Ok(offset) = u32::try_from(slot as u64 * self.stride) else {
                continue;
            };
            let texture = part.texture.and_then(|t| model.textures.get(t)).unwrap_or(&self.white);
            pass.set_bind_group(0, &self.bind, &[offset]);
            pass.set_bind_group(1, texture, &[]);
            pass.set_vertex_buffer(0, part.vertices.slice(..));
            pass.draw(0..part.count, 0..1);
            drawn += 1;
        }
        drawn
    }
}

fn uniform_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, stride: u64, capacity: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("model draws"),
        size: stride * capacity,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("model draws"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<ModelUniform>() as u64),
            }),
        }],
    });
    (buffer, bind)
}

/// Картинка с мип-уровнями в группу привязки.
fn texture_bind(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::BindGroup {
    let levels = mip_levels(width, height, rgba);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("model"),
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
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
            wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("model"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}
