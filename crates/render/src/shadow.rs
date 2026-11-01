// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карта теней кадра в видеокарте, перенос `render/shadowmap.py` модуля:
//! текстура глубины со стороны солнца. Тайлы читают её в группе воздуха,
//! модели - в своей группе 2. Матрицы света - `globe_core::shadowmap`.

use globe_core::shadowmap::MAP_SIZE;

use crate::DEPTH_FORMAT;

/// Наклон смещения глубины в проходе карты - против полос самозатенения
/// склонов под косым солнцем, величины модуля.
pub const SLOPE_BIAS: f32 = 2.0;
pub const UNITS_BIAS: i32 = 4;

/// Записи текстуры карты и сравнивающего сампления с номера first.
pub fn entries(first: u32) -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        wgpu::BindGroupLayoutEntry {
            binding: first,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: first + 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
    ]
}

/// Глубина прохода карты со смещением.
pub fn depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState { constant: UNITS_BIAS, slope_scale: SLOPE_BIAS, clamp: 0.0 },
    }
}

/// Текстура карты, её вид и сампление, группа 2 моделей.
pub struct ShadowMap {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub model_layout: wgpu::BindGroupLayout,
    pub model_bind: wgpu::BindGroup,
}

impl ShadowMap {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d { width: MAP_SIZE, height: MAP_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow map"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("model shadow"), entries: &entries(0) });
        let model_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("model shadow"),
            layout: &model_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        Self { view, sampler, model_layout, model_bind }
    }
}
