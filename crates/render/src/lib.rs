// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Отрисовка глобуса на wgpu, слой L3.
//!
//! Глобус рисуется в свою текстуру с буфером глубины. Окно показывает
//! эту текстуру, тест - читает счётчик дыр. Мировые координаты
//! в видеокарту не попадают: вершины тайла - смещения от его центра
//! в f32, матрица каждого тайла считается в f64 от разности «центр
//! тайла - глаз» (`Camera::tile_mvp`).
//!
//! Порядок кадра, как в модуле: тайлы выбора с заплатками, подстилка -
//! тайлы уровня 2 глубже поверхности на `UNDERLAY_DEPTH`, шапки
//! у полюсов. Подстилка и шапки рисуются последними и почти целиком
//! отбрасываются проверкой глубины. Счётчик дыр - полноэкранный
//! треугольник на глубине 1.0 с запросом перекрытия.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use globe_core::camera::Camera;
use globe_core::lod::{Selection, TileKey, hole_rect};
use globe_core::tiling::{HOLE_MARGIN, MeshId, TileMesh, UNDERLAY_DEPTH};
use terrain::HeightStore;
use terrain::mesh::{TerrainMesh, polar_cap, terrain_mesh};
use globe_core::{DMat4, DVec3};
use wgpu::util::DeviceExt;

pub use wgpu;

/// Формат кадра глобуса.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Фон - космос.
pub const SPACE: wgpu::Color = wgpu::Color::BLACK;
/// Ровный цвет тайла, снимок которого ещё не пришёл, - вода OSM.
pub const PLACEHOLDER: [u8; 3] = [0xAA, 0xD3, 0xDF];
/// Шапка над северным краем сетки - вода OSM.
pub const NORTH_CAP: [u8; 3] = [0xAA, 0xD3, 0xDF];
/// Шапка над южным краем сетки - ледник OSM.
pub const SOUTH_CAP: [u8; 3] = [0xDD, 0xEC, 0xEC];
/// Уровень тайлов подстилки.
pub const UNDERLAY_LEVEL: u32 = 2;
/// Вершин края шапки у полюса.
pub const CAP_SEGMENTS: u32 = 256;

/// Цвет sRGB в линейные доли для шейдера.
fn linear(rgb: [u8; 3]) -> [f32; 3] {
    rgb.map(|c| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
}

/// Переход от глубины OpenGL (-1..1) к глубине wgpu (0..1).
fn depth_fix() -> DMat4 {
    DMat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0])
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    offset: [f32; 3],
    uv: [f32; 2],
    shade: f32,
}

/// Сеток, пересобираемых за кадр, когда пришли более точные высоты.
/// Сетка без высот строится всегда, иначе в кадре была бы дыра.
pub const REBUILDS_PER_FRAME: u32 = 24;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawUniform {
    mvp: [f32; 16],
    hole: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RaysUniform {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    back: [f32; 4],
    frame: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    center: DVec3,
    last_used: u64,
    /// Уровень тайла высот, по которому собрана сетка, -1 - без высот.
    level: i32,
    /// Вертикальный масштаб рельефа при сборке.
    scale: f64,
}

struct GpuTexture {
    bind_group: wgpu::BindGroup,
    last_used: u64,
}

struct Target {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

/// Настройки кадра.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameOptions {
    /// Считать дыры. Чтение счётчика ждёт видеокарту и задерживает
    /// кадр, поэтому оно только для проверок.
    pub count_holes: bool,
    /// Красить дыры пурпурным.
    pub show_holes: bool,
}

/// Итог кадра.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Вызовов отрисовки.
    pub draws: usize,
    /// Пикселей Земли, не закрытых ни одним тайлом, если считались.
    pub holes: Option<u64>,
    /// Пикселей, где видна подстилка, если считались. В исправном кадре
    /// это щели уже пикселя на стыках тайлов разных уровней. Пропавший
    /// тайл подстилка закрывает целиком, дырой он не считается, а здесь
    /// виден сразу.
    pub underlay: Option<u64>,
}

/// Что рисует отрисовка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Tile,
    Underlay,
    Cap,
}

/// Одна отрисовка сетки.
struct Item {
    kind: Kind,
    mesh: MeshId,
    texture: Option<TileKey>,
    uniform: DrawUniform,
}

/// Запросы перекрытия кадра: дыры и видимая подстилка.
const QUERIES: u32 = 2;

/// Дыры считаются запросами перекрытия. На странице - нет: бэкенд
/// WebGPU в wgpu 30 не передаёт браузеру `occlusion_query_set` прохода,
/// и запрос перекрытия делает весь кадр недействительным, 22 октября
/// 2026 года. Там дыры красятся пурпурным и считаются по пикселям
/// кадра, видимая подстилка не считается.
const USE_QUERIES: bool = !cfg!(target_arch = "wasm32");

/// Прочитанные счётчики кадра: дыры и подстилка, если считалась.
type Counts = Arc<Mutex<Option<(u64, Option<u64>)>>>;

/// Пурпурный пиксель дыры в кадре RGBA.
const HOLE_PIXEL: [u8; 4] = [255, 0, 255, 255];

/// Буфер чтения кадра для счёта дыр по пикселям.
struct Pixels {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    /// Байтов в строке буфера, кратно 256.
    row: u32,
}

/// Глобус на видеокарте: конвейеры, сетки и снимки тайлов, кадр.
pub struct Globe {
    device: wgpu::Device,
    queue: wgpu::Queue,
    tile_pipeline: wgpu::RenderPipeline,
    hole_count_pipeline: wgpu::RenderPipeline,
    hole_show_pipeline: wgpu::RenderPipeline,
    draw_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    stride: u64,
    uniform: wgpu::Buffer,
    uniform_capacity: u64,
    uniform_bind: wgpu::BindGroup,
    rays: wgpu::Buffer,
    rays_bind: wgpu::BindGroup,
    blank: wgpu::BindGroup,
    meshes: HashMap<MeshId, GpuMesh>,
    textures: HashMap<TileKey, GpuTexture>,
    target: Option<Target>,
    query_set: wgpu::QuerySet,
    query_resolve: wgpu::Buffer,
    query_read: wgpu::Buffer,
    /// Буфер чтения счётчиков ещё отображается в память.
    query_busy: Arc<AtomicBool>,
    /// Прочитанные счётчики: дыры и подстилка, если считалась.
    query_result: Counts,
    pixels: Option<Pixels>,
    frame: u64,
}

fn shader(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

impl Globe {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let globe_shader = shader(device, "globe", include_str!("globe.wgsl"));
        let holes_shader = shader(device, "holes", include_str!("holes.wgsl"));
        let draw_size = std::mem::size_of::<DrawUniform>() as u64;
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(draw_size),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile texture"),
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
        let rays_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rays"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile"),
            bind_group_layouts: &[Some(&draw_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let hole_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("holes"),
            bind_group_layouts: &[Some(&rays_layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32],
        };
        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile"),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &globe_shader,
                entry_point: Some("tile_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &globe_shader,
                entry_point: Some("tile_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let hole_pipeline = |writes: wgpu::ColorWrites, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&hole_layout),
                vertex: wgpu::VertexState {
                    module: &holes_shader,
                    entry_point: Some("hole_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(depth_state(false, wgpu::CompareFunction::LessEqual)),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &holes_shader,
                    entry_point: Some("hole_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: None, write_mask: writes })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let hole_count_pipeline = hole_pipeline(wgpu::ColorWrites::empty(), "holes count");
        let hole_show_pipeline = hole_pipeline(wgpu::ColorWrites::ALL, "holes show");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment.max(1));
        let stride = draw_size.div_ceil(align) * align;
        let uniform_capacity = 256;
        let (uniform, uniform_bind) = Self::uniform_buffer(device, &draw_layout, stride, uniform_capacity);
        let rays = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rays"),
            size: std::mem::size_of::<RaysUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let rays_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rays"),
            layout: &rays_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: rays.as_entire_binding() }],
        });
        let blank = Self::texture_bind(device, queue, &texture_layout, &sampler, 1, 1, &[255, 255, 255, 255]);
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("holes"),
            ty: wgpu::QueryType::Occlusion,
            count: QUERIES,
        });
        let query_resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("holes resolve"),
            size: 8 * u64::from(QUERIES),
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let query_read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("holes read"),
            size: 8 * u64::from(QUERIES),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            tile_pipeline,
            hole_count_pipeline,
            hole_show_pipeline,
            draw_layout,
            texture_layout,
            sampler,
            stride,
            uniform,
            uniform_capacity,
            uniform_bind,
            rays,
            rays_bind,
            blank,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            target: None,
            query_set,
            query_resolve,
            query_read,
            query_busy: Arc::new(AtomicBool::new(false)),
            query_result: Arc::new(Mutex::new(None)),
            pixels: None,
            frame: 0,
        }
    }

    fn uniform_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        stride: u64,
        capacity: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draws"),
            size: stride * capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draws"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<DrawUniform>() as u64),
                }),
            }],
        });
        (buffer, bind)
    }

    fn texture_bind(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> wgpu::BindGroup {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// Снимок тайла пришёл в видеокарту.
    pub fn has_texture(&self, key: TileKey) -> bool {
        self.textures.contains_key(&key)
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Загрузить снимок тайла, RGBA по 8 бит. Картинка не того размера
    /// отбрасывается.
    pub fn upload(&mut self, key: TileKey, width: u32, height: u32, rgba: &[u8]) {
        let expected = u64::from(width) * u64::from(height) * 4;
        if width == 0 || height == 0 || rgba.len() as u64 != expected {
            return;
        }
        let bind_group =
            Self::texture_bind(&self.device, &self.queue, &self.texture_layout, &self.sampler, width, height, rgba);
        self.textures.insert(key, GpuTexture { bind_group, last_used: self.frame });
    }

    /// Вытеснить снимки и сетки, давно не нужные кадру, пока их больше
    /// budget. Тайлы из keep не вытесняются.
    pub fn evict(&mut self, keep: &HashSet<TileKey>, budget: usize) {
        if self.textures.len() > budget {
            let mut old: Vec<(u64, TileKey)> =
                self.textures.iter().filter(|(k, _)| !keep.contains(k)).map(|(k, t)| (t.last_used, *k)).collect();
            old.sort_unstable();
            let extra = self.textures.len() - budget;
            for (_, key) in old.into_iter().take(extra) {
                self.textures.remove(&key);
            }
        }
        if self.meshes.len() > budget {
            let mut old: Vec<(u64, MeshId)> = self
                .meshes
                .iter()
                .filter(|(id, _)| match id {
                    MeshId::Tile { z, x, y } => !keep.contains(&(*z, *x, *y)),
                    MeshId::PolarCap { .. } => false,
                })
                .map(|(id, m)| (m.last_used, *id))
                .collect();
            old.sort_unstable_by_key(|(t, _)| *t);
            let extra = self.meshes.len() - budget;
            for (_, id) in old.into_iter().take(extra) {
                self.meshes.remove(&id);
            }
        }
    }

    /// Вид кадра глобуса для показа в окне.
    pub fn color_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.color_view)
    }

    /// Текстура кадра глобуса.
    pub fn color_texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|t| &t.color)
    }

    /// Наибольшая сторона кадра, которую даёт видеокарта.
    pub fn max_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d.max(1)
    }

    /// Размер кадра под камеру. true - кадр пересоздан. Сторона больше
    /// [`Globe::max_side`] прижимается к ней.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let max = self.max_side();
        let (width, height) = (width.clamp(1, max), height.clamp(1, max));
        if self.target.as_ref().is_some_and(|t| t.width == width && t.height == height) {
            return false;
        }
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let make = |format, usage, label| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = make(
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            "globe color",
        );
        let depth = make(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT, "globe depth");
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some(Target { width, height, color, color_view, depth_view });
        true
    }

    /// Сетка на видеокарте и её центр. Сетка пересобирается, когда для
    /// неё пришёл более точный тайл высот или сменился масштаб рельефа,
    /// но не больше rebuilds раз за кадр.
    fn mesh_for(
        &mut self,
        id: MeshId,
        camera: &Camera,
        heights: Option<&HeightStore>,
        rebuilds: &mut u32,
    ) -> Option<DVec3> {
        let frame = self.frame;
        let (tile, level, scale) = match (id, heights) {
            (MeshId::Tile { z, x, y }, Some(store)) => {
                let (tile, level) = store.for_mesh((z, x, y));
                (tile, level, store.scale())
            }
            _ => (None, -1, 0.0),
        };
        if let Some(m) = self.meshes.get_mut(&id) {
            m.last_used = frame;
            let stale = level > m.level || scale != m.scale;
            if !stale || *rebuilds == 0 {
                return Some(m.center);
            }
            *rebuilds -= 1;
        }
        let e = &camera.ellipsoid;
        let mesh: TerrainMesh = match id {
            MeshId::Tile { z, x, y } => terrain_mesh(e, z, x, y, tile.as_deref(), scale),
            MeshId::PolarCap { north } => polar_cap(e, north, CAP_SEGMENTS),
        };
        let shade = &mesh.shade;
        let mesh: TileMesh = mesh.mesh.clone();
        let vertices: Vec<Vertex> = mesh
            .positions
            .iter()
            .zip(&mesh.uv)
            .zip(shade.iter().chain(std::iter::repeat(&1.0)))
            .map(|((p, uv), s)| Vertex { offset: p.to_array(), uv: uv.to_array(), shade: *s })
            .collect();
        let vertices = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let index_count = u32::try_from(mesh.indices.len()).ok()?;
        self.meshes.insert(
            id,
            GpuMesh { vertices, indices, index_count, center: mesh.center, last_used: frame, level, scale },
        );
        Some(mesh.center)
    }

    fn items(
        &mut self,
        camera: &Camera,
        selection: &Selection,
        heights: Option<&HeightStore>,
        projection: DMat4,
    ) -> Vec<Item> {
        let mut out = Vec::new();
        let placeholder = linear(PLACEHOLDER);
        let mut rebuilds = REBUILDS_PER_FRAME;
        let mut push = |me: &mut Self,
                        kind: Kind,
                        mesh: MeshId,
                        texture: Option<TileKey>,
                        scale: f64,
                        hole: [f32; 4],
                        color: [f32; 4]| {
            if let Some(center) = me.mesh_for(mesh, camera, heights, &mut rebuilds) {
                let mvp = camera.tile_mvp(center, Some(projection), scale).to_cols_array();
                out.push(Item { kind, mesh, texture, uniform: DrawUniform { mvp, hole, color } });
            }
        };
        let flat = |rgb: [f32; 3]| [rgb[0], rgb[1], rgb[2], 1.0];
        for (key, hole) in selection.draw.iter().zip(&selection.holes) {
            let (z, x, y) = *key;
            let has = self.textures.contains_key(key);
            let hole = hole.map_or([0.0; 4], |h| {
                let (u, v, s) = hole_rect(*key, h);
                [u as f32, v as f32, s as f32, 1.0]
            });
            let color = if has { [0.0; 4] } else { flat(placeholder) };
            push(self, Kind::Tile, MeshId::Tile { z, x, y }, has.then_some(*key), 1.0, hole, color);
        }
        // Подстилка ниже самой низкой высоты рельефа, у Земли дно моря
        // прижато к нулю.
        let depth = heights.map_or(0.0, HeightStore::depth);
        let scale = 1.0 - (UNDERLAY_DEPTH + depth) / camera.ellipsoid.a;
        let mut under: Vec<TileKey> = selection.keep.iter().copied().filter(|k| k.0 == UNDERLAY_LEVEL).collect();
        under.sort_unstable();
        for key in under {
            let (z, x, y) = key;
            let has = self.textures.contains_key(&key);
            let color = if has { [0.0; 4] } else { flat(placeholder) };
            push(self, Kind::Underlay, MeshId::Tile { z, x, y }, has.then_some(key), scale, [0.0; 4], color);
        }
        let caps = [(true, NORTH_CAP), (false, SOUTH_CAP)];
        for (north, rgb) in caps {
            push(self, Kind::Cap, MeshId::PolarCap { north }, None, 1.0, [0.0; 4], flat(linear(rgb)));
        }
        out
    }

    fn rays_uniform(camera: &Camera, width: u32, height: u32) -> RaysUniform {
        let e = &camera.ellipsoid;
        let scale = DVec3::new(1.0 / (e.a - HOLE_MARGIN), 1.0 / (e.a - HOLE_MARGIN), 1.0 / (e.b - HOLE_MARGIN));
        let eye = camera.eye * scale;
        let t = (camera.fov_y.to_radians() / 2.0).tan();
        let r = camera.rotation;
        let v4 = |v: DVec3, w: f64| [v.x as f32, v.y as f32, v.z as f32, w as f32];
        RaysUniform {
            eye: v4(eye, 0.0),
            right: v4(r.x_axis * scale * (t * camera.aspect()), 0.0),
            up: v4(r.y_axis * scale * t, 0.0),
            back: v4(r.z_axis * scale, 0.0),
            frame: [width as f32, height as f32, (eye.length_squared() - 1.0) as f32, 0.0],
        }
    }

    /// Нарисовать кадр. Размер кадра - окно камеры. heights - высоты
    /// рельефа, None - гладкий эллипсоид.
    pub fn render(
        &mut self,
        camera: &Camera,
        selection: &Selection,
        heights: Option<&HeightStore>,
        options: FrameOptions,
    ) -> FrameStats {
        self.frame += 1;
        self.resize(camera.width, camera.height);
        let projection = depth_fix() * camera.projection();
        let items = self.items(camera, selection, heights, projection);
        let count = items.len() as u64;
        if count > self.uniform_capacity {
            let capacity = count.next_power_of_two();
            (self.uniform, self.uniform_bind) =
                Self::uniform_buffer(&self.device, &self.draw_layout, self.stride, capacity);
            self.uniform_capacity = capacity;
        }
        let stride = self.stride as usize;
        let mut bytes = vec![0u8; stride * items.len()];
        for (chunk, item) in bytes.chunks_exact_mut(stride).zip(&items) {
            let raw = bytemuck::bytes_of(&item.uniform);
            chunk[..raw.len()].copy_from_slice(raw);
        }
        self.queue.write_buffer(&self.uniform, 0, &bytes);
        for key in items.iter().filter_map(|i| i.texture) {
            if let Some(t) = self.textures.get_mut(&key) {
                t.last_used = self.frame;
            }
        }
        if options.count_holes && !USE_QUERIES {
            self.ensure_pixels();
        }
        let Some(target) = &self.target else {
            return FrameStats::default();
        };
        let counting = options.count_holes || options.show_holes;
        let queries = counting && USE_QUERIES;
        if counting {
            let rays = Self::rays_uniform(camera, target.width, target.height);
            self.queue.write_buffer(&self.rays, 0, bytemuck::bytes_of(&rays));
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("globe") });
        let mut draws = 0;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("globe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(SPACE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: queries.then_some(&self.query_set),
                multiview_mask: None,
            });
            pass.set_pipeline(&self.tile_pipeline);
            let mut underlay_open = false;
            for (i, item) in items.iter().enumerate() {
                // Подстилка идёт подряд, её видимые пиксели считает
                // второй запрос.
                if queries && !underlay_open && item.kind == Kind::Underlay {
                    pass.begin_occlusion_query(1);
                    underlay_open = true;
                }
                if underlay_open && item.kind != Kind::Underlay {
                    pass.end_occlusion_query();
                    underlay_open = false;
                }
                let Some(mesh) = self.meshes.get(&item.mesh) else {
                    continue;
                };
                let texture = item.texture.and_then(|k| self.textures.get(&k)).map_or(&self.blank, |t| &t.bind_group);
                let Ok(offset) = u32::try_from(i as u64 * self.stride) else {
                    continue;
                };
                pass.set_bind_group(0, &self.uniform_bind, &[offset]);
                pass.set_bind_group(1, texture, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                draws += 1;
            }
            if underlay_open {
                pass.end_occlusion_query();
            }
            if counting {
                let paint = options.show_holes || !USE_QUERIES;
                let pipeline = if paint { &self.hole_show_pipeline } else { &self.hole_count_pipeline };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.rays_bind, &[]);
                if queries {
                    pass.begin_occlusion_query(0);
                }
                pass.draw(0..3, 0..1);
                if queries {
                    pass.end_occlusion_query();
                }
            }
        }
        // Пока буфер чтения отображается в память, в него не копируют,
        // счётчики этого кадра пропускаются.
        let read = options.count_holes && !self.query_busy.load(Ordering::SeqCst);
        let mut pixel_read = None;
        if read && USE_QUERIES {
            encoder.resolve_query_set(&self.query_set, 0..QUERIES, &self.query_resolve, 0);
            encoder.copy_buffer_to_buffer(&self.query_resolve, 0, &self.query_read, 0, 8 * u64::from(QUERIES));
        } else if read && let Some(p) = &self.pixels {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target.color,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &p.buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(p.row), rows_per_image: Some(p.height) },
                },
                wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
            );
            pixel_read = Some((p.buffer.clone(), p.width, p.height, p.row));
        }
        self.queue.submit([encoder.finish()]);
        if read && USE_QUERIES {
            self.start_read();
        }
        if let Some((buffer, width, height, row)) = pixel_read {
            self.start_pixel_read(buffer, width, height, row);
        }
        let (holes, underlay) = match options.count_holes.then(|| self.take_counts()).flatten() {
            Some((holes, underlay)) => (Some(holes), underlay),
            None => (None, None),
        };
        FrameStats { draws, holes, underlay }
    }

    /// Буфер чтения кадра под текущий размер.
    fn ensure_pixels(&mut self) {
        let Some(target) = &self.target else {
            return;
        };
        if self.pixels.as_ref().is_some_and(|p| p.width == target.width && p.height == target.height) {
            return;
        }
        if self.query_busy.load(Ordering::SeqCst) {
            return;
        }
        let row = (4 * target.width).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("holes pixels"),
            size: u64::from(row) * u64::from(target.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.pixels = Some(Pixels { buffer, width: target.width, height: target.height, row });
    }

    /// Начать чтение кадра и счёт пурпурных пикселей дыр.
    fn start_pixel_read(&self, buffer: wgpu::Buffer, width: u32, height: u32, row: u32) {
        self.query_busy.store(true, Ordering::SeqCst);
        let busy = self.query_busy.clone();
        let result = self.query_result.clone();
        let source = buffer.clone();
        source.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            if r.is_ok() {
                let holes = buffer.slice(..).get_mapped_range().ok().map(|data| {
                    (0..height as usize)
                        .filter_map(|y| data.get(y * row as usize..y * row as usize + 4 * width as usize))
                        .map(|line| line.as_chunks::<4>().0.iter().filter(|px| **px == HOLE_PIXEL).count() as u64)
                        .sum::<u64>()
                });
                buffer.unmap();
                if let Ok(mut slot) = result.lock() {
                    *slot = holes.map(|h| (h, None));
                }
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Начать чтение счётчиков дыр и подстилки.
    fn start_read(&self) {
        self.query_busy.store(true, Ordering::SeqCst);
        let buffer = self.query_read.clone();
        let busy = self.query_busy.clone();
        let result = self.query_result.clone();
        self.query_read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            if r.is_ok() {
                let values = buffer.slice(..).get_mapped_range().ok().and_then(|data| {
                    let word =
                        |i: usize| -> Option<u64> { Some(u64::from_le_bytes(data.get(8 * i..8 * i + 8)?.try_into().ok()?)) };
                    Some((word(0)?, word(1)?))
                });
                buffer.unmap();
                if let Ok(mut slot) = result.lock() {
                    *slot = values.map(|(h, u)| (h, Some(u)));
                }
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Прочитанные счётчики. В программе чтение ждёт видеокарту и даёт
    /// счётчики этого кадра. На странице ждать нельзя, счётчики приходят
    /// через кадр или позже, None - ещё не пришли.
    fn take_counts(&self) -> Option<(u64, Option<u64>)> {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.query_result.lock().ok()?.take()
    }
}
