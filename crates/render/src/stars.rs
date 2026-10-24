// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Звёзды в видеокарте, перенос класса `Stars` (`render/stars.py`)
//! модуля. Направления звёзд в экваториальной системе лежат в буфере
//! один раз, поворот неба на звёздное время и видимость от высоты камеры
//! ставит окно глобуса каждый кадр. Звёзды рисуются после неба, глубину
//! не пишут, свет прибавляется к небу.

use bytemuck::{Pod, Zeroable};
use globe_core::camera::Camera;
use globe_core::{DMat3, DMat4};
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Звезда для видеокарты: направление в экваториальной системе, размер
/// в логических пикселях, цвет RGB и яркость.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarPoint {
    pub dir: [f32; 3],
    pub size: f32,
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    dir: [f32; 3],
    size: f32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StarsUniform {
    mvp: [f32; 16],
    params: [f32; 4],
}

/// Звёзды и их конвейер.
pub struct Stars {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    instances: Option<(wgpu::Buffer, u32)>,
    /// Поворот неба из экваториальной системы в ECEF и видимость.
    frame: Option<(DMat3, f32)>,
}

impl Stars {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stars"),
            source: wgpu::ShaderSource::Wgsl(include_str!("stars.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stars"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stars"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Свет звезды прибавляется к небу, альфа кадра остаётся.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stars"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("stars_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x4],
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
                entry_point: Some("stars_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stars"),
            size: std::mem::size_of::<StarsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stars"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        Self { pipeline, uniform, bind, instances: None, frame: None }
    }

    /// Новый набор звёзд, пустой - звёзд нет.
    pub fn set(&mut self, device: &wgpu::Device, stars: &[StarPoint]) {
        let instances: Vec<Instance> = stars.iter().map(|s| Instance { dir: s.dir, size: s.size, color: s.color }).collect();
        self.instances = u32::try_from(instances.len()).ok().filter(|n| *n > 0).map(|count| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("stars"), contents: bytemuck::cast_slice(&instances), usage: wgpu::BufferUsages::VERTEX });
            (buffer, count)
        });
    }

    /// Поворот неба и видимость кадра. None - звёзды не рисуются.
    pub fn set_frame(&mut self, frame: Option<(DMat3, f32)>) {
        self.frame = frame;
    }

    /// Нарисовать звёзды в открытом проходе кадра после неба. ratio -
    /// пикселей кадра на логический пиксель.
    pub fn draw(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, camera: &Camera, ratio: f32) -> usize {
        let (Some((buffer, count)), Some((sky, share))) = (&self.instances, self.frame) else {
            return 0;
        };
        if share <= 0.0 {
            return 0;
        }
        // Поворот камеры без сдвига: звёзды на бесконечности.
        let view = camera.rotation.transpose() * sky;
        let model = DMat4::from_cols(view.x_axis.extend(0.0), view.y_axis.extend(0.0), view.z_axis.extend(0.0), globe_core::DVec4::W);
        let u = StarsUniform {
            mvp: (depth_fix() * camera.projection() * model).as_mat4().to_cols_array(),
            params: [ratio, share, camera.width as f32, camera.height as f32],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..6, 0..*count);
        1
    }
}
