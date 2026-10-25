// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Картинка неба с Млечным путём позади Земли, перенос класса `Sky`
//! (`render/sky.py`) модуля. Картинка 8192×4096 в равнопромежуточной
//! проекции, прямое восхождение 0h у левого края, север вверху. Она
//! грузится в видеокарту один раз, без мипмапов, шире наибольшей стороны
//! текстуры - вдвое мельче, пока не влезет. Рисуется после неба с гало
//! и до точечных звёзд.

use bytemuck::{Pod, Zeroable};
use globe_core::DMat3;
use globe_core::camera::Camera;

use crate::{COLOR_FORMAT, DEPTH_FORMAT};

/// Яркость картинки и порог чёрного, `SKY_GAIN` и `SKY_FLOOR` модуля.
pub const GAIN: f32 = 0.5;
pub const FLOOR: f32 = 0.2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MapUniform {
    right: [f32; 4],
    up: [f32; 4],
    back: [f32; 4],
    params: [f32; 4],
}

/// Картинка неба и её конвейер.
pub struct SkyMap {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind: Option<wgpu::BindGroup>,
    /// Поворот неба из экваториальной системы в ECEF и видимость.
    frame: Option<(DMat3, f32)>,
}

/// Картинка вдвое мельче: каждая вторая точка по строкам и столбцам.
fn halve(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w, h) = (width / 2, height / 2);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h {
        let row = (2 * r * width) as usize * 4;
        for c in 0..w {
            let i = row + (2 * c) as usize * 4;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    (w, h, out)
}

impl SkyMap {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky map"),
            source: wgpu::ShaderSource::Wgsl(include_str!("skymap.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sky map"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        // Свет картинки прибавляется к небу, альфа кадра остаётся.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("skymap_vs"), compilation_options: Default::default(), buffers: &[] },
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
                entry_point: Some("skymap_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky map"),
            size: std::mem::size_of::<MapUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // По долготе картинка повторяется, по широте - край.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky map"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, layout, uniform, sampler, bind: None, frame: None }
    }

    /// Новая картинка RGBA, пустая - картинки нет.
    pub fn set_image(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8]) {
        self.bind = None;
        if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
            return;
        }
        let max = device.limits().max_texture_dimension_2d.max(1);
        let (mut w, mut h, mut data) = (width, height, std::borrow::Cow::Borrowed(rgba));
        while w > max || h > max {
            let (nw, nh, next) = halve(w, h, &data);
            (w, h, data) = (nw, nh, std::borrow::Cow::Owned(next));
        }
        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky map"),
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
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky map"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        }));
    }

    pub fn has_image(&self) -> bool {
        self.bind.is_some()
    }

    /// Поворот неба и видимость кадра. None - картинка не рисуется.
    pub fn set_frame(&mut self, frame: Option<(DMat3, f32)>) {
        self.frame = frame;
    }

    /// Нарисовать картинку в открытом проходе кадра после неба с гало.
    pub fn draw(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, camera: &Camera) -> usize {
        let (Some(bind), Some((sky, share))) = (&self.bind, self.frame) else {
            return 0;
        };
        if share <= 0.0 {
            return 0;
        }
        // Оси камеры в экваториальной системе: sky переводит из неё в ECEF.
        let axes = sky.transpose() * camera.rotation;
        let t = (camera.fov_y.to_radians() / 2.0).tan();
        let v4 = |v: globe_core::DVec3, w: f64| [v.x as f32, v.y as f32, v.z as f32, w as f32];
        let u = MapUniform {
            right: v4(axes.x_axis * (t * camera.aspect()), f64::from(camera.width)),
            up: v4(axes.y_axis * t, f64::from(camera.height)),
            back: v4(axes.z_axis, 0.0),
            params: [GAIN * share, FLOOR, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
        1
    }
}

#[cfg(test)]
mod tests {
    use super::halve;

    #[test]
    fn halve_keeps_every_second_point() {
        let rgba: Vec<u8> = (0..4 * 4 * 2).map(|i| i as u8).collect();
        let (w, h, out) = halve(4, 2, &rgba);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, [0, 1, 2, 3, 8, 9, 10, 11]);
    }
}
