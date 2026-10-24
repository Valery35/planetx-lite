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

mod features;
pub use features::{FeatureMesh, PULL, WALL_ALPHA};
use features::Features;
mod pictures;
mod models;
pub use models::{ModelMesh, ModelPart, ModelPlace};
use models::Models;
mod points;
pub use points::{Dot, PointsMesh};
mod stars;
pub use stars::StarPoint;
use points::Points;
pub use pictures::{PictureMesh, mip_levels};
use pictures::Pictures;

/// Формат кадра глобуса - без sRGB: перевод в sRGB делает шейдер, egui
/// показывает значения как есть.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Формат снимков тайлов - sRGB: выборка и фильтрация идут в линейном
/// цвете.
pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Формат наложения линий: значения экрана без перевода sRGB, альфа
/// умножена на цвет. Смешение с подложкой - как у картинки наложения
/// модуля поверх снимка.
pub const OVERLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Фон - космос.
pub const SPACE: wgpu::Color = wgpu::Color::BLACK;
/// Ровный цвет тайла, снимок которого ещё не пришёл, - вода OSM.
pub const PLACEHOLDER: [u8; 3] = [0xAA, 0xD3, 0xDF];
/// Шапка над северным краем сетки до прихода тайла 0/0/0 - вода OSM.
pub const NORTH_CAP: [u8; 3] = [0xAA, 0xD3, 0xDF];
/// Шапка над южным краем сетки до прихода тайла 0/0/0 - ледник OSM.
pub const SOUTH_CAP: [u8; 3] = [0xDD, 0xEC, 0xEC];
/// Уровень тайлов подстилки.
pub const UNDERLAY_LEVEL: u32 = 2;
/// Вершин края шапки у полюса.
pub const CAP_SEGMENTS: u32 = 256;

/// Цвет в доли значений экрана для шейдера. Шейдер считает отмывку
/// и дымку в значениях экрана, как модуль.
fn display(rgb: [u8; 3]) -> [f32; 3] {
    rgb.map(|c| f32::from(c) / 255.0)
}

/// Толщина оболочки атмосферы, километры, как в модуле.
pub const SHELL_KM: f64 = 300.0;

/// Переход от глубины OpenGL (-1..1) к глубине wgpu (0..1).
pub(crate) fn depth_fix() -> DMat4 {
    DMat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0])
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    offset: [f32; 3],
    uv: [f32; 2],
    shade: f32,
    /// Нормаль рельефа в ECEF для света солнца.
    normal: [f32; 3],
    /// Вверх по нормали эллипсоида и высота вершины на экране,
    /// `TerrainMesh::sea`. Проход воды опускает по ним сетку на уровень
    /// моря.
    sea: [f32; 4],
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
    window: [f32; 4],
    /// Окно наложения линий: u0, v0, сторона, признак (1 - есть).
    over: [f32; 4],
    /// Окно огней городов: u0, v0, сторона, признак (1 - есть).
    lights: [f32; 4],
    /// Проход воды: цвет rgb в значениях экрана и непрозрачность.
    water: [f32; 4],
    /// Проход воды: признак (1 - вода), глубина полной воды на экране.
    sea: [f32; 4],
}

/// Окно всего снимка.
const WHOLE: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
/// Самый глубокий предок, чей снимок рисуется вместо отсутствующего
/// тайла, как в модуле.
pub const MAX_FILL_DEPTH: u32 = 6;

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

/// Текстура наложения или огней: группа привязки собирается парой
/// «наложение + огни» тайла.
struct GpuView {
    view: wgpu::TextureView,
}

/// Пара текстур группы 3 тайла: ключ тайла наложения и тайла огней.
type OverPair = (Option<TileKey>, Option<TileKey>);

/// Наибольший уровень тайлов огней городов, `LIGHTS_LEVEL` модуля.
pub const LIGHTS_LEVEL: u32 = 8;

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
    /// Атмосфера: небо, гало и дымка.
    pub air: bool,
    /// Пикселей кадра на логический пиксель, толщина линий объектов
    /// умножается на него. Ноль и меньше - 1.
    pub ratio: f32,
    /// Свет солнца: направление на солнце в ECEF, None - отмывка.
    pub sun: Option<[f64; 3]>,
    /// Вода над дном моря: цвет rgb в значениях экрана и непрозрачность.
    /// Вода рисуется там, где рельеф ниже нуля, сетки тайлов при этом
    /// опускаются на уровень моря. None - воды нет, у Земли высоты ниже
    /// нуля тогда обнулены.
    pub water: Option<[f32; 4]>,
}

/// Глубина полной воды в метрах, мельче вода прозрачнее, `SHALLOW`
/// модуля. Умножается на масштаб рельефа.
pub const SHALLOW: f32 = 200.0;

/// Воздух кадра, см. air.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AirUniform {
    eye: [f32; 4],
    axes: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    back: [f32; 4],
    flags: [f32; 4],
    /// Направление на солнце в ECEF, w - 1, когда свет солнца включён.
    sun: [f32; 4],
}

/// Снятый кадр глобуса, RGBA по 8 бит, строки сверху вниз.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Снятый кадр, когда он прочитан с видеокарты.
type Grabbed = Arc<Mutex<Option<Frame>>>;

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
    /// Вода над тайлом ниже уровня моря.
    Water,
}

/// Одна отрисовка сетки.
struct Item {
    kind: Kind,
    mesh: MeshId,
    texture: Option<TileKey>,
    /// Наложение линий векторной основы: ключ тайла с картинкой.
    overlay: Option<TileKey>,
    /// Огни городов: ключ тайла с картинкой.
    lights: Option<TileKey>,
    /// Снимок шапки: 0 - северной, 1 - южной.
    cap: Option<usize>,
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
    water_pipeline: wgpu::RenderPipeline,
    hole_count_pipeline: wgpu::RenderPipeline,
    hole_show_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    features: Features,
    pictures: Pictures,
    points: Points,
    stars: stars::Stars,
    models: Models,
    air: wgpu::Buffer,
    air_bind: wgpu::BindGroup,
    /// Снять следующий кадр.
    grab_next: bool,
    grabbed: Grabbed,
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
    /// Пустое прозрачное наложение.
    clear: wgpu::TextureView,
    /// Наложения линий векторной основы по тайлам, overlay.py модуля.
    overlays: HashMap<TileKey, GpuView>,
    /// Огни городов на ночной стороне по тайлам, core/sun.py модуля.
    lights: HashMap<TileKey, GpuView>,
    /// Группа 3 тайла: наложение и огни, собранные парой.
    over_layout: wgpu::BindGroupLayout,
    over_binds: HashMap<OverPair, wgpu::BindGroup>,
    /// Группа 3 без наложения и огней.
    clear_bind: wgpu::BindGroup,
    meshes: HashMap<MeshId, GpuMesh>,
    textures: HashMap<TileKey, GpuTexture>,
    /// Тайлы, которых нет у источника.
    missing: HashSet<TileKey>,
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
    /// Цвета шапок, северной и южной: край тайла 0/0/0 основы, до его
    /// прихода - [`NORTH_CAP`] и [`SOUTH_CAP`].
    caps: [[u8; 3]; 2],
    /// Снимки шапок, [`cap_strip`].
    cap_binds: [Option<wgpu::BindGroup>; 2],
    /// Края снимков тайлов у северного и южного края сетки, [`edge_row`].
    edges: [HashMap<TileKey, Vec<[u8; 3]>>; 2],
    /// Из каких тайлов собран снимок шапки: тайл и источник снимка.
    cap_from: [Vec<(TileKey, TileKey)>; 2],
}

/// Ширина снимка шапки, столбцов на 360° долготы.
pub const CAP_STRIP: usize = 1024;

/// Край тайла для шапки: среднее двух крайних строк по столбцам,
/// верхних у северного края и нижних у южного. None - картинка не того
/// размера.
pub fn edge_row(width: u32, height: u32, rgba: &[u8], north: bool) -> Option<Vec<[u8; 3]>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h < 2 || rgba.len() != w * h * 4 {
        return None;
    }
    let first = if north { 0 } else { h - 2 };
    let row = w * 4;
    Some(
        (0..w)
            .map(|x| {
                let a = &rgba[first * row + x * 4..];
                let b = &rgba[(first + 1) * row + x * 4..];
                [0, 1, 2].map(|c| (u16::from(a[c]) + u16::from(b[c])).div_ceil(2) as u8)
            })
            .collect(),
    )
}

/// Доля края, нарисованная тайлом: тайл подложки, окно его снимка
/// (u0, v0, сторона, как у [`Globe`]) и край снимка.
pub struct EdgePiece<'a> {
    pub key: TileKey,
    pub window: [f32; 4],
    pub row: &'a [[u8; 3]],
}

/// Снимок шапки [`CAP_STRIP`] × 2, RGBA. Верхняя строка - край тайлов,
/// нарисованных у края сетки: у Esri тайлы разных уровней у 85° разного
/// цвета, и шапка из одного тайла 0/0/0 выходила светлее соседей,
/// 23 октября 2026 года. Где у края тайлов на экране нет, берётся
/// fallback - край тайла 0/0/0. Строка сглажена по долготе, окно 1/128
/// ширины по кругу, иначе край растягивается к полюсу лучами. Окно шире
/// смешивало край с соседними тайлами другого цвета, у края оставался
/// шов. Нижняя -
/// медиана верхней, к ней шапка переходит до широты
/// [`globe_core::tiling::CAP_FADE_LAT`].
pub fn cap_strip(fallback: &[[u8; 3]], pieces: &[EdgePiece<'_>]) -> Vec<u8> {
    let w = CAP_STRIP;
    let pick = |row: &[[u8; 3]], u: f64| row[((u * row.len() as f64) as usize).min(row.len().saturating_sub(1))];
    let mut edge: Vec<[u8; 3]> = (0..w)
        .map(|c| if fallback.is_empty() { [0; 3] } else { pick(fallback, (c as f64 + 0.5) / w as f64) })
        .collect();
    let mut order: Vec<&EdgePiece<'_>> = pieces.iter().filter(|p| !p.row.is_empty()).collect();
    order.sort_by_key(|p| p.key.0);
    for p in order {
        let (z, x, _) = p.key;
        let n = f64::from(1u32 << z.min(30));
        let c0 = (f64::from(x) / n * w as f64).floor() as usize;
        let c1 = ((f64::from(x + 1) / n * w as f64).ceil() as usize).clamp(c0 + 1, w);
        for (c, slot) in edge.iter_mut().enumerate().take(c1).skip(c0) {
            let u = ((c as f64 + 0.5) / w as f64 * n - f64::from(x)).clamp(0.0, 1.0);
            *slot = pick(p.row, f64::from(p.window[0]) + u * f64::from(p.window[2]));
        }
    }
    let radius = w / 256;
    let mut out = Vec::with_capacity(w * 8);
    for c in 0..w {
        let mut sum = [0u32; 3];
        for d in 0..=2 * radius {
            let px = edge[(c + w + d - radius) % w];
            for k in 0..3 {
                sum[k] += u32::from(px[k]);
            }
        }
        let n = 2 * radius as u32 + 1;
        out.extend(sum.map(|s| ((s + n / 2) / n) as u8));
        out.push(255);
    }
    let median: [u8; 3] = [0, 1, 2].map(|k| {
        let mut v: Vec<u8> = edge.iter().map(|p| p[k]).collect();
        v.sort_unstable();
        v[v.len() / 2]
    });
    for _ in 0..w {
        out.extend([median[0], median[1], median[2], 255]);
    }
    out
}

/// Цвета полярных шапок по краю тайла 0/0/0, как в модуле
/// (`render/view.py`, `_paint_caps`): медиана каждого канала двух
/// крайних строк, верхних для севера и нижних для юга. Медиана не
/// замечает редких островов и подписей. None - картинка не того размера.
pub fn cap_colors(width: u32, height: u32, rgba: &[u8]) -> Option<[[u8; 3]; 2]> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h < 2 || rgba.len() != w * h * 4 {
        return None;
    }
    let median = |rows: &[u8]| {
        let mut out = [0u8; 3];
        for (c, slot) in out.iter_mut().enumerate() {
            let mut v: Vec<u8> = rows.iter().skip(c).step_by(4).copied().collect();
            v.sort_unstable();
            *slot = v[v.len() / 2];
        }
        out
    };
    let row = w * 4;
    Some([median(&rgba[..2 * row]), median(&rgba[(h - 2) * row..])])
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
        let globe_shader = shader(device, "globe", concat!(include_str!("air.wgsl"), include_str!("globe.wgsl")));
        let sky_shader = shader(device, "sky", concat!(include_str!("air.wgsl"), include_str!("sky.wgsl")));
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
        // Группа 3 тайла: наложение линий и огни городов. Групп привязки
        // в WebGPU не больше четырёх, огни идут в группу наложения.
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let over_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay and lights"),
            entries: &[texture_entry(0), sampler_entry(1), texture_entry(2), sampler_entry(3)],
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
        let air_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("air"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        // Небо рисуется с тем же расположением, что и тайлы: воздух
        // в группе 2 у обоих.
        let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile"),
            bind_group_layouts: &[Some(&draw_layout), Some(&texture_layout), Some(&air_layout), Some(&over_layout)],
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
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32, 3 => Float32x3, 4 => Float32x4],
        };
        // Вода - та же программа тайла: глубину не пишет, дно видно
        // сквозь неё, суша выше нуля закрывает её проверкой глубины, как
        // `_draw_water` модуля. Альфа кадра остаётся.
        let water_blend = wgpu::BlendState {
            color: wgpu::BlendComponent::OVER,
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let make_tile_pipeline = |label: &str, water: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&tile_layout),
                vertex: wgpu::VertexState {
                    module: &globe_shader,
                    entry_point: Some("tile_vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(if water { depth_state(false, wgpu::CompareFunction::LessEqual) } else { depth_state(true, wgpu::CompareFunction::Less) }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &globe_shader,
                    entry_point: Some("tile_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: water.then_some(water_blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let tile_pipeline = make_tile_pipeline("tile", false);
        let water_pipeline = make_tile_pipeline("water", true);
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
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &sky_shader,
                entry_point: Some("sky_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::LessEqual)),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
                entry_point: Some("sky_fs"),
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
        let air = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("air"),
            size: std::mem::size_of::<AirUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let air_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("air"),
            layout: &air_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: air.as_entire_binding() }],
        });
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
        let clear = Self::upload_view(device, queue, 1, 1, &[0, 0, 0, 0], OVERLAY_FORMAT);
        let clear_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay and lights"),
            layout: &over_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&clear) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&clear) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
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
            water_pipeline,
            hole_count_pipeline,
            hole_show_pipeline,
            sky_pipeline,
            features: Features::new(device),
            pictures: Pictures::new(device),
            points: Points::new(device),
            stars: stars::Stars::new(device),
            models: Models::new(device, queue),
            air,
            air_bind,
            grab_next: false,
            grabbed: Arc::new(Mutex::new(None)),
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
            clear,
            overlays: HashMap::new(),
            lights: HashMap::new(),
            over_layout,
            over_binds: HashMap::new(),
            clear_bind,
            meshes: HashMap::new(),
            textures: HashMap::new(),
            missing: HashSet::new(),
            target: None,
            query_set,
            query_resolve,
            query_read,
            query_busy: Arc::new(AtomicBool::new(false)),
            query_result: Arc::new(Mutex::new(None)),
            pixels: None,
            frame: 0,
            caps: [NORTH_CAP, SOUTH_CAP],
            cap_binds: [None, None],
            edges: [HashMap::new(), HashMap::new()],
            cap_from: [Vec::new(), Vec::new()],
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
        Self::texture_bind_as(device, queue, layout, sampler, width, height, rgba, TEXTURE_FORMAT)
    }

    /// Текстура RGBA в формате format и её группа привязки.
    #[allow(clippy::too_many_arguments)]
    fn texture_bind_as(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
        rgba: &[u8],
        format: wgpu::TextureFormat,
    ) -> wgpu::BindGroup {
        let view = Self::upload_view(device, queue, width, height, rgba, format);
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// Текстура RGBA в формате format и её вид.
    fn upload_view(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8], format: wgpu::TextureFormat) -> wgpu::TextureView {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
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
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Группа 3 тайла по паре текстур: собирается один раз.
    fn ensure_over(&mut self, pair: OverPair) {
        if pair == (None, None) || self.over_binds.contains_key(&pair) {
            return;
        }
        let over = pair.0.and_then(|k| self.overlays.get(&k)).map_or(&self.clear, |t| &t.view);
        let lights = pair.1.and_then(|k| self.lights.get(&k)).map_or(&self.clear, |t| &t.view);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("overlay and lights"),
            layout: &self.over_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(over) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(lights) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        // Пар не бывает много больше тайлов кадра, на случай сбоя - предел.
        if self.over_binds.len() > 8192 {
            self.over_binds.clear();
        }
        self.over_binds.insert(pair, bind);
    }

    /// Огни городов для тайла: свои или доля огней предка уровня не
    /// глубже [`LIGHTS_LEVEL`], как окна слоёв GIBS модуля.
    fn lights_of(&self, key: TileKey) -> Option<(TileKey, [f32; 4])> {
        let (z, x, y) = key;
        (z.saturating_sub(LIGHTS_LEVEL)..=z).find_map(|depth| {
            let parent = (z - depth, x >> depth, y >> depth);
            self.lights.contains_key(&parent).then(|| {
                let cells = 1u32 << depth.min(31);
                let side = 1.0 / cells as f32;
                (parent, [(x % cells) as f32 * side, (y % cells) as f32 * side, side, 1.0])
            })
        })
    }

    /// Положить огни городов тайла key: RGBA в значениях экрана, чёрное -
    /// огней нет. Картинка не того размера отбрасывается.
    pub fn set_lights(&mut self, key: TileKey, width: u32, height: u32, rgba: &[u8]) {
        let expected = u64::from(width) * u64::from(height) * 4;
        if width == 0 || height == 0 || rgba.len() as u64 != expected {
            return;
        }
        let view = Self::upload_view(&self.device, &self.queue, width, height, rgba, OVERLAY_FORMAT);
        self.lights.insert(key, GpuView { view });
        self.over_binds.retain(|(_, l), _| *l != Some(key));
    }

    /// Есть ли огни тайла key.
    pub fn has_lights(&self, key: TileKey) -> bool {
        self.lights.contains_key(&key)
    }

    /// Забыть огни городов: солнце выключено.
    /// Сетки тайлов устарели: высоты сменились без смены уровня, например
    /// дно моря включено или пришло от предка. under - тайлы высот, сетки
    /// внутри них пересобираются, None - все сетки. Сетка пересобирается
    /// в бюджете кадра, до того рисуется прежняя.
    pub fn stale_meshes(&mut self, under: Option<&[TileKey]>) {
        for (id, mesh) in &mut self.meshes {
            let hit = match (id, under) {
                (_, None) => true,
                (MeshId::Tile { z, x, y }, Some(keys)) => keys.iter().any(|k| k.0 <= *z && (x >> (z - k.0).min(31), y >> (z - k.0).min(31)) == (k.1, k.2)),
                (MeshId::PolarCap { .. }, Some(_)) => false,
            };
            if hit {
                mesh.level = -2;
            }
        }
    }

    pub fn clear_lights(&mut self) {
        self.lights.clear();
        self.over_binds.retain(|(_, l), _| l.is_none());
    }

    /// Снимок для тайла: свой или доля снимка предка, если тайла нет
    /// у источника. Ключ тайла со снимком и окно в нём (u0, v0, сторона).
    fn picture(&self, key: TileKey) -> Option<(TileKey, [f32; 4])> {
        if self.textures.contains_key(&key) {
            return Some((key, WHOLE));
        }
        if !self.missing.contains(&key) {
            return None;
        }
        let (z, x, y) = key;
        (1..=MAX_FILL_DEPTH.min(z)).find_map(|depth| {
            let parent = (z - depth, x >> depth, y >> depth);
            self.textures.contains_key(&parent).then(|| {
                let cells = 1u32 << depth;
                let side = 1.0 / cells as f32;
                (parent, [(x % cells) as f32 * side, (y % cells) as f32 * side, side, 0.0])
            })
        })
    }

    /// Наложение линий для тайла: своё или доля наложения предка, пока
    /// своё не готово.
    fn overlay_of(&self, key: TileKey) -> Option<(TileKey, [f32; 4])> {
        let (z, x, y) = key;
        (0..=MAX_FILL_DEPTH.min(z)).find_map(|depth| {
            let parent = (z - depth, x >> depth, y >> depth);
            self.overlays.contains_key(&parent).then(|| {
                let cells = 1u32 << depth;
                let side = 1.0 / cells as f32;
                (parent, [(x % cells) as f32 * side, (y % cells) as f32 * side, side, 1.0])
            })
        })
    }

    /// Положить наложение линий тайла: RGBA с альфой, умноженной на цвет,
    /// в значениях экрана. Картинка не того размера отбрасывается.
    pub fn set_overlay(&mut self, key: TileKey, width: u32, height: u32, rgba: &[u8]) {
        let expected = u64::from(width) * u64::from(height) * 4;
        if width == 0 || height == 0 || rgba.len() as u64 != expected {
            return;
        }
        let view = Self::upload_view(&self.device, &self.queue, width, height, rgba, OVERLAY_FORMAT);
        self.overlays.insert(key, GpuView { view });
        self.over_binds.retain(|(o, _), _| *o != Some(key));
    }

    /// Есть ли своё наложение у тайла.
    pub fn has_overlay(&self, key: TileKey) -> bool {
        self.overlays.contains_key(&key)
    }

    /// Забыть все наложения: сменились группы векторной основы.
    pub fn clear_overlays(&mut self) {
        self.overlays.clear();
        self.over_binds.retain(|(o, _), _| o.is_none());
    }

    /// Тайл готов к рисованию: свой снимок или доля снимка предка.
    pub fn has_texture(&self, key: TileKey) -> bool {
        self.picture(key).is_some()
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Тайла key нет у источника: вместо него рисуется доля снимка
    /// ближайшего предка, не глубже [`MAX_FILL_DEPTH`] уровней.
    pub fn mark_missing(&mut self, key: TileKey) {
        self.missing.insert(key);
    }

    /// Забыть все снимки - сменилась основа.
    pub fn clear_textures(&mut self) {
        self.textures.clear();
        self.missing.clear();
        self.caps = [NORTH_CAP, SOUTH_CAP];
        self.cap_binds = [None, None];
        self.edges = [HashMap::new(), HashMap::new()];
        self.cap_from = [Vec::new(), Vec::new()];
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
        if key == (0, 0, 0)
            && let Some(caps) = cap_colors(width, height, rgba)
        {
            self.caps = caps;
        }
        let last = (1u32 << key.0.min(30)) - 1;
        for (i, edge) in [(0usize, 0u32), (1, last)] {
            if key.2 == edge
                && let Some(row) = edge_row(width, height, rgba, i == 0)
            {
                self.edges[i].insert(key, row);
                self.cap_from[i].clear();
            }
        }
    }

    /// Вытеснить снимки и сетки, давно не нужные кадру, пока их больше
    /// budget. Тайлы из keep не вытесняются. Снимки уровней 0-2 тоже:
    /// окно просит их всегда, на них держится подстилка. Вытесненные,
    /// они грузились снова и снова, когда снимков кадра было больше
    /// budget - у Эльбруса на кадре страницы 3840 × 1726, 22 октября
    /// 2026 года.
    pub fn evict(&mut self, keep: &HashSet<TileKey>, budget: usize) {
        if self.textures.len() > budget {
            let mut old: Vec<(u64, TileKey)> = self
                .textures
                .iter()
                .filter(|(k, _)| !keep.contains(k) && k.0 > UNDERLAY_LEVEL)
                .map(|(k, t)| (t.last_used, *k))
                .collect();
            old.sort_unstable();
            let extra = self.textures.len() - budget;
            for (_, key) in old.into_iter().take(extra) {
                self.textures.remove(&key);
            }
            let textures = &self.textures;
            self.overlays.retain(|k, _| keep.contains(k) || textures.contains_key(k) || k.0 <= UNDERLAY_LEVEL);
            let (overlays, lights) = (&self.overlays, &self.lights);
            self.over_binds.retain(|(o, l), _| o.is_none_or(|k| overlays.contains_key(&k)) && l.is_none_or(|k| lights.contains_key(&k)));
            for edges in &mut self.edges {
                edges.retain(|k, _| textures.contains_key(k));
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
            // Шапка стоит на высотах края: пересборка, когда пришли
            // точнее высоты всех тайлов подстилки у края.
            (MeshId::PolarCap { north }, Some(store)) => {
                let n = 1u32 << UNDERLAY_LEVEL;
                let y = if north { 0 } else { n - 1 };
                let level = (0..n).map(|x| store.for_mesh((UNDERLAY_LEVEL, x, y)).1).min().unwrap_or(-1);
                (None, level, store.scale())
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
            MeshId::PolarCap { north } => polar_cap(e, north, CAP_SEGMENTS, heights),
        };
        let shade = &mesh.shade;
        let normals = &mesh.normals;
        let sea = &mesh.sea;
        let mesh: TileMesh = mesh.mesh.clone();
        let vertices: Vec<Vertex> = mesh
            .positions
            .iter()
            .zip(&mesh.uv)
            .zip(shade.iter().chain(std::iter::repeat(&1.0)))
            .enumerate()
            .map(|(i, ((p, uv), s))| Vertex {
                offset: p.to_array(),
                uv: uv.to_array(),
                shade: *s,
                normal: normals.get(i).map_or([0.0, 0.0, 0.0], |n| n.to_array()),
                sea: sea.get(i).map_or([0.0; 4], |s| s.to_array()),
            })
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

    /// Пересобрать снимки шапок, если у края сетки нарисованы другие
    /// тайлы или пришли новые снимки края.
    fn update_caps(&mut self, selection: &Selection) {
        for i in 0..2 {
            let Some(fallback) = self.edges[i].get(&(0, 0, 0)) else {
                continue;
            };
            let from: Vec<(TileKey, TileKey, [f32; 4])> = selection
                .draw
                .iter()
                .filter(|k| k.2 == if i == 0 { 0 } else { (1u32 << k.0.min(30)) - 1 })
                .filter_map(|k| self.picture(*k).map(|(src, window)| (*k, src, window)))
                .filter(|(_, src, _)| self.edges[i].contains_key(src))
                .collect();
            let sig: Vec<(TileKey, TileKey)> = from.iter().map(|(k, s, _)| (*k, *s)).collect();
            if sig == self.cap_from[i] && self.cap_binds[i].is_some() {
                continue;
            }
            let pieces: Vec<EdgePiece<'_>> = from
                .iter()
                .filter_map(|(k, s, window)| self.edges[i].get(s).map(|row| EdgePiece { key: *k, window: *window, row }))
                .collect();
            let strip = cap_strip(fallback, &pieces);
            let width = CAP_STRIP as u32;
            let bind = Self::texture_bind(&self.device, &self.queue, &self.texture_layout, &self.sampler, width, 2, &strip);
            self.cap_binds[i] = Some(bind);
            self.cap_from[i] = sig;
        }
    }

    fn items(
        &mut self,
        camera: &Camera,
        selection: &Selection,
        heights: Option<&HeightStore>,
        projection: DMat4,
        water: Option<[f32; 4]>,
    ) -> Vec<Item> {
        self.update_caps(selection);
        let mut out = Vec::new();
        let placeholder = display(PLACEHOLDER);
        let mut rebuilds = REBUILDS_PER_FRAME;
        let mut push = |me: &mut Self,
                        kind: Kind,
                        mesh: MeshId,
                        picture: Option<(TileKey, [f32; 4])>,
                        scale: f64,
                        hole: [f32; 4],
                        color: [f32; 4]| {
            if let Some(center) = me.mesh_for(mesh, camera, heights, &mut rebuilds) {
                let mvp = camera.tile_mvp(center, Some(projection), scale).to_cols_array();
                let (texture, window) = picture.map_or((None, WHOLE), |(k, w)| (Some(k), w));
                let ((overlay, over), (lights, glow)) = match (kind, mesh) {
                    (Kind::Tile, MeshId::Tile { z, x, y }) => (
                        me.overlay_of((z, x, y)).map_or((None, [0.0; 4]), |(k, w)| (Some(k), w)),
                        me.lights_of((z, x, y)).map_or((None, [0.0; 4]), |(k, w)| (Some(k), w)),
                    ),
                    _ => ((None, [0.0; 4]), (None, [0.0; 4])),
                };
                me.ensure_over((overlay, lights));
                out.push(Item { kind, mesh, texture, overlay, lights, cap: None, uniform: DrawUniform { mvp, hole, color, window, over, lights: glow, water: [0.0; 4], sea: [0.0; 4] } });
            }
        };
        let flat = |rgb: [f32; 3]| [rgb[0], rgb[1], rgb[2], 1.0];
        for (key, hole) in selection.draw.iter().zip(&selection.holes) {
            let (z, x, y) = *key;
            let picture = self.picture(*key);
            let hole = hole.map_or([0.0; 4], |h| {
                let (u, v, s) = hole_rect(*key, h);
                [u as f32, v as f32, s as f32, 1.0]
            });
            let color = if picture.is_some() { [0.0; 4] } else { flat(placeholder) };
            push(self, Kind::Tile, MeshId::Tile { z, x, y }, picture, 1.0, hole, color);
        }
        // Подстилка ниже самой низкой высоты рельефа, у Земли дно моря
        // прижато к нулю.
        let depth = heights.map_or(0.0, HeightStore::depth);
        let scale = 1.0 - (UNDERLAY_DEPTH + depth) / camera.ellipsoid.a;
        let mut under: Vec<TileKey> = selection.keep.iter().copied().filter(|k| k.0 == UNDERLAY_LEVEL).collect();
        under.sort_unstable();
        for key in under {
            let (z, x, y) = key;
            let picture = self.picture(key);
            let color = if picture.is_some() { [0.0; 4] } else { flat(placeholder) };
            push(self, Kind::Underlay, MeshId::Tile { z, x, y }, picture, scale, [0.0; 4], color);
        }
        let caps = [(true, self.caps[0]), (false, self.caps[1])];
        for (north, rgb) in caps {
            push(self, Kind::Cap, MeshId::PolarCap { north }, None, 1.0, [0.0; 4], flat(display(rgb)));
        }
        // С приходом тайла 0/0/0 шапка берёт свой снимок вместо ровного
        // цвета.
        for item in out.iter_mut() {
            if let MeshId::PolarCap { north } = item.mesh {
                let i = usize::from(!north);
                if self.cap_binds[i].is_some() {
                    item.cap = Some(i);
                    item.uniform.color[3] = 0.0;
                }
            }
        }
        // Вода - копии тайлов после всех тайлов, только с рельефом, как
        // у модуля.
        if let Some(color) = water
            && let Some(store) = heights.filter(|s| s.scale() > 0.0)
        {
            let sea = [1.0, (f64::from(SHALLOW) * store.scale()).max(1.0) as f32, 0.0, 0.0];
            let copies: Vec<Item> = out
                .iter()
                .filter(|i| i.kind == Kind::Tile)
                .map(|i| Item {
                    kind: Kind::Water,
                    mesh: i.mesh,
                    texture: i.texture,
                    overlay: i.overlay,
                    lights: i.lights,
                    cap: None,
                    uniform: DrawUniform { water: color, sea, ..i.uniform },
                })
                .collect();
            out.extend(copies);
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

    /// Воздух кадра: глаз в долях полуосей, оси камеры в ECEF. Разности
    /// квадратов считаются в f64.
    fn air_uniform(camera: &Camera, width: u32, height: u32, on: bool, sun: Option<[f64; 3]>) -> AirUniform {
        let e = &camera.ellipsoid;
        let axes = DVec3::new(1.0 / e.a, 1.0 / e.a, 1.0 / e.b);
        let eye = camera.eye * axes;
        let shell = 1.0 + SHELL_KM * 1000.0 / e.a;
        let t = (camera.fov_y.to_radians() / 2.0).tan();
        let r = camera.rotation;
        let v4 = |v: DVec3, w: f64| [v.x as f32, v.y as f32, v.z as f32, w as f32];
        let squared = eye.length_squared();
        AirUniform {
            eye: v4(eye, squared - shell * shell),
            axes: v4(axes, e.a / 1000.0),
            right: v4(r.x_axis * (t * camera.aspect()), f64::from(width)),
            up: v4(r.y_axis * t, f64::from(height)),
            back: v4(r.z_axis, squared - 1.0),
            flags: [if on { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            sun: sun.map_or([0.0, 0.0, 1.0, 0.0], |s| [s[0] as f32, s[1] as f32, s[2] as f32, 1.0]),
        }
    }

    /// Свои объекты кадра: линии, заливки, стены. Заменяют прежние.
    pub fn set_features(&mut self, meshes: &[FeatureMesh]) {
        self.features.set(&self.device, meshes);
    }

    /// 3D-модели. Заменяют прежние.
    pub fn set_models(&mut self, models: &[ModelMesh]) {
        self.models.set(&self.device, &self.queue, models);
    }

    /// Новое место модели номер index из [`Globe::set_models`]: модель
    /// едет по пути.
    pub fn place_model(&mut self, index: usize, place: ModelPlace) {
        self.models.place(index, place);
    }

    /// Солнце для моделей: направление в ECEF, None - свет модели свой.
    pub fn set_model_sun(&mut self, sun: Option<[f32; 3]>) {
        self.models.sun = sun;
    }

    /// Матрицы дорожек анимации модели номер index на кадр.
    pub fn animate_model(&mut self, index: usize, deltas: std::collections::HashMap<usize, DMat4>) {
        self.models.animate(index, deltas);
    }

    /// Звёзды каталога в экваториальной системе. Заменяют прежние.
    pub fn set_stars(&mut self, stars: &[StarPoint]) {
        self.stars.set(&self.device, stars);
    }

    /// Поворот неба из экваториальной системы в ECEF и видимость звёзд
    /// кадра, None - звёзды не рисуются.
    pub fn set_star_frame(&mut self, frame: Option<(globe_core::DMat3, f32)>) {
        self.stars.set_frame(frame);
    }

    /// Облака точек. Заменяют прежние.
    pub fn set_points(&mut self, clouds: &[PointsMesh]) {
        self.points.set(&self.device, clouds);
    }

    /// Сетки картинок-наложений кадра. Заменяют прежние.
    pub fn set_pictures(&mut self, meshes: &[PictureMesh]) {
        self.pictures.set(&self.device, meshes);
    }

    /// Картинка наложения по ключу, RGBA в значениях экрана. false -
    /// размер не годится видеокарте.
    pub fn set_picture_image(&mut self, key: &str, width: u32, height: u32, rgba: &[u8]) -> bool {
        self.pictures.set_image(&self.device, &self.queue, key, width, height, rgba)
    }

    /// Оставить в видеокарте только картинки с ключами keep.
    pub fn keep_picture_images(&mut self, keep: &[String]) {
        self.pictures.keep_images(keep);
    }

    pub fn has_picture_image(&self, key: &str) -> bool {
        self.pictures.has_image(key)
    }

    /// Снять следующий кадр. Снимок забирает [`Globe::take_grab`].
    pub fn grab(&mut self) {
        self.grab_next = true;
    }

    /// Снятый кадр. В программе чтение ждёт видеокарту, на странице
    /// кадр приходит позже, None - ещё не пришёл.
    pub fn take_grab(&self) -> Option<Frame> {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.grabbed.lock().ok()?.take()
    }

    /// Скопировать кадр в буфер и начать его чтение.
    fn copy_grab(&self, encoder: &mut wgpu::CommandEncoder, target: &Target) -> (wgpu::Buffer, u32, u32, u32) {
        let (width, height) = (target.width, target.height);
        let row = (4 * width).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grab"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        (buffer, width, height, row)
    }

    fn start_grab_read(&self, buffer: wgpu::Buffer, width: u32, height: u32, row: u32) {
        let result = self.grabbed.clone();
        let source = buffer.clone();
        source.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            if r.is_err() {
                return;
            }
            let rgba = buffer.slice(..).get_mapped_range().ok().map(|data| {
                (0..height as usize)
                    .filter_map(|y| data.get(y * row as usize..y * row as usize + 4 * width as usize))
                    .flatten()
                    .copied()
                    .collect::<Vec<u8>>()
            });
            buffer.unmap();
            if let (Some(rgba), Ok(mut slot)) = (rgba, result.lock()) {
                *slot = Some(Frame { width, height, rgba });
            }
        });
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
        let items = self.items(camera, selection, heights, projection, options.water);
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
        let grab_now = std::mem::take(&mut self.grab_next);
        let Some(target) = &self.target else {
            return FrameStats::default();
        };
        let counting = options.count_holes || options.show_holes;
        let queries = counting && USE_QUERIES;
        if counting {
            let rays = Self::rays_uniform(camera, target.width, target.height);
            self.queue.write_buffer(&self.rays, 0, bytemuck::bytes_of(&rays));
        }
        // Пурпурные дыры видны только без воздуха, как в модуле.
        let air_on = options.air && !options.show_holes;
        let air = Self::air_uniform(camera, target.width, target.height, air_on, options.sun);
        self.queue.write_buffer(&self.air, 0, bytemuck::bytes_of(&air));
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
            pass.set_bind_group(2, &self.air_bind, &[]);
            let mut underlay_open = false;
            let mut water_open = false;
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
                if item.kind == Kind::Water && !water_open {
                    pass.set_pipeline(&self.water_pipeline);
                    water_open = true;
                }
                let Some(mesh) = self.meshes.get(&item.mesh) else {
                    continue;
                };
                let texture = match item.cap.and_then(|i| self.cap_binds.get(i)).and_then(Option::as_ref) {
                    Some(bind) => bind,
                    None => item.texture.and_then(|k| self.textures.get(&k)).map_or(&self.blank, |t| &t.bind_group),
                };
                let Ok(offset) = u32::try_from(i as u64 * self.stride) else {
                    continue;
                };
                pass.set_bind_group(0, &self.uniform_bind, &[offset]);
                pass.set_bind_group(1, texture, &[]);
                let over = self.over_binds.get(&(item.overlay, item.lights)).unwrap_or(&self.clear_bind);
                pass.set_bind_group(3, over, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                draws += 1;
            }
            if underlay_open {
                pass.end_occlusion_query();
            }
            // Небо и гало - там, где нет тайлов, до счётчика дыр: небо
            // глубину не пишет, дыры оно не закрывает.
            if air_on {
                pass.set_pipeline(&self.sky_pipeline);
                pass.set_bind_group(0, &self.uniform_bind, &[0]);
                pass.set_bind_group(1, &self.blank, &[]);
                pass.set_bind_group(2, &self.air_bind, &[]);
                pass.set_bind_group(3, &self.clear_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            // Свои объекты - после неба: небо глубину не пишет и закрыло
            // бы линию над горизонтом.
            let ratio = if options.ratio > 0.0 { options.ratio } else { 1.0 };
            // Звёзды - после неба, свет прибавляется к нему.
            draws += self.stars.draw(&self.queue, &mut pass, camera, ratio);
            // Картинки-наложения - под своими объектами.
            draws += self.models.draw(&self.device, &self.queue, &mut pass, camera);
            draws += self.points.draw(&self.device, &self.queue, &mut pass, camera, ratio);
            draws += self.pictures.draw(&self.device, &self.queue, &mut pass, camera);
            draws += self.features.draw(&self.device, &self.queue, &mut pass, camera, ratio);
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
        let grab = grab_now.then(|| self.copy_grab(&mut encoder, target));
        self.queue.submit([encoder.finish()]);
        if let Some((buffer, width, height, row)) = grab {
            self.start_grab_read(buffer, width, height, row);
        }
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
