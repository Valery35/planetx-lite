// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сетка тайла на рельефе: вершины на высотах, отмывка, нормали для
//! света солнца, данные прохода воды.
//!
//! Перенос рельефной части `planetx/core/tiling.py` модуля. Без высот
//! сетка совпадает с сеткой гладкого эллипсоида `tiling::tile_mesh`.

use globe_core::ellipsoid::Ellipsoid;
use globe_core::tiling::{
    MAX_LEVEL as TILE_MAX_LEVEL, MeshId, TileMesh, grid_latlon, index_buffer, polar_cap_mesh, ring, segments,
    skirt_depth,
};
use globe_core::{DVec3, Vec2, Vec3, Vec4};

use crate::{HeightTile, grid_shares, sample};

/// Свет отмывки: с северо-запада, 45° над горизонтом. Привязан
/// к местности, а не к камере, поэтому затенение считается один раз
/// при сборке тайла.
pub const AMBIENT: f64 = 0.35;
pub const LIGHT_ELEVATION_DEG: f64 = 45.0;
pub const SHADE_LIMITS: (f64, f64) = (0.4, 1.35);

/// Сетка тайла и данные вершин для рельефа.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainMesh {
    pub mesh: TileMesh,
    /// Множитель яркости вершин от отмывки рельефа.
    pub shade: Vec<f32>,
    /// Единичные нормали рельефа в ECEF, для света солнца. У юбки -
    /// нормаль её края.
    pub normals: Vec<Vec3>,
    /// Вверх по нормали эллипсоида и высота вершины. По ним проход воды
    /// опускает сетку на уровень моря, у юбки высота ниже края на глубину
    /// юбки.
    pub sea: Vec<Vec4>,
}

/// Восток, север и верх в точке.
fn enu(lat: f64, lon: f64) -> [DVec3; 3] {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    [DVec3::new(-so, co, 0.0), DVec3::new(-sl * co, -sl * so, cl), DVec3::new(cl * co, cl * so, sl)]
}

/// Единичные нормали рельефа во внутренних узлах сетки с полосой в один
/// узел, wide - сторона сетки с полосой. Нормаль считается центральными
/// разностями, поэтому соседние тайлы с одной картой высот получают
/// на общем краю одну нормаль.
fn terrain_normals(positions: &[DVec3], wide: usize) -> Vec<DVec3> {
    let inner = wide - 2;
    let at = |row: usize, col: usize| positions[row * wide + col];
    (1..=inner)
        .flat_map(|j| (1..=inner).map(move |i| (j, i)))
        .map(|(j, i)| {
            let east = at(j, i + 1) - at(j, i - 1);
            let north = at(j - 1, i) - at(j + 1, i);
            east.cross(north).normalize_or_zero()
        })
        .collect()
}

/// Множитель яркости узла по нормали рельефа. Ровная местность даёт 1.
fn shade(lat: f64, lon: f64, normal: DVec3) -> f64 {
    let [east, north, up] = enu(lat, lon);
    let elevation = LIGHT_ELEVATION_DEG.to_radians();
    let flat = elevation.sin();
    let across = elevation.cos() / std::f64::consts::SQRT_2;
    let light = up * flat + (north - east) * across;
    let lambert = normal.dot(light).clamp(0.0, 1.0);
    let value = (AMBIENT + (1.0 - AMBIENT) * lambert) / (AMBIENT + (1.0 - AMBIENT) * flat);
    value.clamp(SHADE_LIMITS.0, SHADE_LIMITS.1)
}

/// Сетка вершин тайла с юбкой на рельефе.
///
/// tile - тайл высот или None для гладкого эллипсоида. Вершины
/// поднимаются по нормали эллипсоида на высоту, умноженную
/// на exaggeration. Юбка опускается ещё на размах высот тайла: у соседа
/// высоты могут прийти с другого уровня.
pub fn terrain_mesh(e: &Ellipsoid, z: u32, x: u32, y: u32, tile: Option<&HeightTile>, exaggeration: f64) -> TerrainMesh {
    let z = z.min(TILE_MAX_LEVEL);
    let seg = segments(z);
    let side = (seg + 1) as usize;
    let latlon = grid_latlon(z, x, y, 0);
    let up: Vec<DVec3> = latlon.iter().map(|(lat, lon)| e.surface_normal(*lat, *lon)).collect();
    let (heights, shading, lighting, spread) = match tile {
        None => (vec![0.0; side * side], vec![1.0; side * side], up.clone(), 0.0),
        Some(tile) => {
            let wide = side + 2;
            let shares = grid_shares(z, x, y, seg, 1);
            let wide_h: Vec<f64> = shares.iter().map(|(u, v)| sample(tile, *u, *v) * exaggeration).collect();
            let wide_ll = grid_latlon(z, x, y, 1);
            let wide_ecef: Vec<DVec3> =
                wide_ll.iter().zip(&wide_h).map(|((lat, lon), h)| e.geodetic_to_ecef(*lat, *lon, *h)).collect();
            let normals = terrain_normals(&wide_ecef, wide);
            let heights: Vec<f64> =
                (1..=side).flat_map(|j| (1..=side).map(move |i| (j, i))).map(|(j, i)| wide_h[j * wide + i]).collect();
            let shading: Vec<f64> =
                latlon.iter().zip(&normals).map(|((lat, lon), n)| shade(*lat, *lon, *n)).collect();
            let top = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let bottom = heights.iter().copied().fold(f64::INFINITY, f64::min);
            let spread = if top >= bottom { top - bottom } else { 0.0 };
            (heights, shading, normals, spread)
        }
    };
    let grid: Vec<DVec3> =
        latlon.iter().zip(&heights).map(|((lat, lon), h)| e.geodetic_to_ecef(*lat, *lon, *h)).collect();
    let rim: Vec<usize> = ring(seg).into_iter().map(|i| i as usize).collect();
    let drop = skirt_depth(e, z) + spread;
    let mut world = grid.clone();
    world.extend(rim.iter().map(|i| grid[*i] - up[*i] * drop));

    let center = grid[(seg as usize / 2) * side + seg as usize / 2];
    let radius = world.iter().map(|p| (*p - center).length()).fold(0.0, f64::max);
    let positions = world.iter().map(|p| (*p - center).as_vec3()).collect();

    let step = f64::from(seg);
    let mut uv: Vec<Vec2> = (0..side)
        .flat_map(|j| (0..side).map(move |i| Vec2::new((i as f64 / step) as f32, (j as f64 / step) as f32)))
        .collect();
    let rim_uv: Vec<Vec2> = rim.iter().map(|i| uv[*i]).collect();
    uv.extend(rim_uv);
    let with_rim = |v: &[f64]| -> Vec<f32> { v.iter().chain(rim.iter().map(|i| &v[*i])).map(|s| *s as f32).collect() };
    let normals: Vec<Vec3> =
        lighting.iter().chain(rim.iter().map(|i| &lighting[*i])).map(|n| n.as_vec3()).collect();
    let mut sea: Vec<Vec4> = up.iter().zip(&heights).map(|(u, h)| u.as_vec3().extend(*h as f32)).collect();
    sea.extend(rim.iter().map(|i| up[*i].as_vec3().extend((heights[*i] - drop) as f32)));

    TerrainMesh {
        mesh: TileMesh {
            id: MeshId::Tile { z, x, y },
            segments: seg,
            center,
            radius,
            positions,
            uv,
            indices: index_buffer(seg),
        },
        shade: with_rim(&shading),
        normals,
        sea,
    }
}

/// Шапка у полюса с данными вершин: отмывки нет, нормаль - от центра
/// тела, высота нулевая.
pub fn polar_cap(e: &Ellipsoid, north: bool, count: u32) -> TerrainMesh {
    let mesh = polar_cap_mesh(e, north, count);
    let ups: Vec<Vec3> =
        mesh.positions.iter().map(|p| (mesh.center + p.as_dvec3()).normalize_or_zero().as_vec3()).collect();
    TerrainMesh {
        shade: vec![1.0; ups.len()],
        sea: ups.iter().map(|u| u.extend(0.0)).collect(),
        normals: ups,
        mesh,
    }
}
