// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линии фигур созвездий вида неба, перенос `render/constellations.py`
//! модуля. Отрезки - пары направлений J2000 в буфере, рисуются списком
//! линий после неба, глубину не пишут, свет прибавляется.

use bytemuck::{Pod, Zeroable};
use globe_core::camera::Camera;
use globe_core::{DMat4, DVec4};
use wgpu::util::DeviceExt;

use crate::{COLOR_FORMAT, DEPTH_FORMAT, depth_fix};

/// Цвет линий созвездий с непрозрачностью, выбор помощника модуля.
pub const LINE_COLOR: [f32; 4] = [0.35, 0.55, 0.85, 0.55];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LinesUniform {
    mvp: [f32; 16],
    color: [f32; 4],
}

/// Линии созвездий и их конвейер.
pub struct SkyLines {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    vertices: Option<(wgpu::Buffer, u32)>,
}

impl SkyLines {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky lines"),
            source: wgpu::ShaderSource::Wgsl(include_str!("skylines.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sky lines"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky lines"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Свет линии прибавляется к небу, альфа кадра остаётся.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("lines_vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
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
                entry_point: Some("lines_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR_FORMAT, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky lines"),
            size: std::mem::size_of::<LinesUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky lines"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        Self { pipeline, uniform, bind, vertices: None }
    }

    /// Новые отрезки парами направлений, пусто - линий нет.
    pub fn set(&mut self, device: &wgpu::Device, segments: &[[f32; 3]]) {
        let n = segments.len() - segments.len() % 2;
        self.vertices = u32::try_from(n).ok().filter(|n| *n > 0).map(|count| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("sky lines"), contents: bytemuck::cast_slice(&segments[..n]), usage: wgpu::BufferUsages::VERTEX });
            (buffer, count)
        });
    }

    /// Нарисовать линии в открытом проходе кадра неба: камера в центре
    /// сферы, её поворот - взгляд на небо.
    pub fn draw(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, camera: &Camera) -> usize {
        let Some((buffer, count)) = &self.vertices else {
            return 0;
        };
        let view = camera.rotation.transpose();
        let model = DMat4::from_cols(view.x_axis.extend(0.0), view.y_axis.extend(0.0), view.z_axis.extend(0.0), DVec4::W);
        let u = LinesUniform { mvp: (depth_fix() * camera.projection() * model).as_mat4().to_cols_array(), color: LINE_COLOR };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..*count, 0..1);
        1
    }
}
