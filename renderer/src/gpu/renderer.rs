// pathfinder/renderer/src/gpu/renderer-old.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::gpu_data;
use crate::post::DefringingKernel;
use crate::scene;
use crate::tiles;
use hal;
use pathfinder_geometry as pfgeom;
use pathfinder_gpu as pfgpu;
use pathfinder_simd as pfsimd;
use std::cmp;
use std::collections::VecDeque;
use std::mem;
use std::ops::{Add, Div};
use std::time::Duration;
use std::u32;

static QUAD_VERTEX_POSITIONS: [u8; 8] = [0, 0, 1, 0, 1, 1, 0, 1];

// FIXME(pcwalton): Shrink this again!
const MASK_FRAMEBUFFER_WIDTH: i32 = tiles::TILE_WIDTH as i32 * 256;
const MASK_FRAMEBUFFER_HEIGHT: i32 = tiles::TILE_HEIGHT as i32 * 256;

// TODO(pcwalton): Replace with `mem::size_of` calls?
const FILL_INSTANCE_SIZE: usize = 8;
const SOLID_TILE_INSTANCE_SIZE: usize = 6;
const MASK_TILE_INSTANCE_SIZE: usize = 8;

const FILL_COLORS_TEXTURE_WIDTH: i32 = 256;
const FILL_COLORS_TEXTURE_HEIGHT: i32 = 256;

const MAX_FILLS_PER_BATCH: usize = 0x4000;

pub struct Renderer {
    // Device
    pub device: pfgpu::PfDevice,

    dest_framebuffer: pfgpu::Framebuffer,
    fill_pipeline: pfgpu::pipeline::FillPipeline,
    solid_multicolor_pipeline: pfgpu::pipeline::SolidMulticolorPipeline,
    alpha_multicolor_pipeline: pfgpu::pipeline::AlphaMulticolorPipeline,
    solid_monochrome_pipeline: pfgpu::pipeline::SolidMonochromePipeline,
    alpha_monochrome_pipeline: pfgpu::pipeline::AlphaMonochromePipeline,

    area_lut_texture: pfgpu::Texture,
    quad_vertex_positions_buffer: pfgpu::Texture,
    fill_vertex_array: pfgpu::Buffer,
    mask_framebuffer: pfgpu::Framebuffer,
    fill_colors_texture: pfgpu::Texture,

    // Postprocessing shader
    postprocess_source_framebuffer: pfgpu::Framebuffer,
    postprocess_pipeline: pfgpu::pipeline::PostprocessPipeline,
    gamma_lut_texture: pfgpu::Texture,

    // Stencil shader
    stencil_pipeline: pfgpu::pipeline::StencilPipeline,

    // Reprojection shader
    reprojection_pipeline: pfgpu::pipeline::ReprojectionPipeline,

    // Rendering state
    mask_framebuffer_cleared: bool,
    buffered_fills: Vec<gpu_data::FillBatchPrimitive>,

    // Extra info
    use_depth: bool,
}

impl Renderer {
    pub fn new(
        window: &winit::Window,
        instance_name: &str,
        resources: &dyn ResourceLoader,
        dest_framebuffer: Framebuffer,
    ) -> Renderer {
        let device = Device::new(window, instance_name);

        let fill_pipeline = FillPipeline::new(&device, resources);
        let solid_multicolor_pipeline = SolidMulticolorPipeline::new(&device, resources);
        let alpha_multicolor_pipeline = AlphaMulticolorPipeline::new(&device, resources);
        let solid_monochrome_pipeline = SolidTileMonochromePipeline::new(&device, resources);
        let alpha_monochrome_pipeline = AlphaTileMonochromePipeline::new(&device, resources);
        let postprocess_pipeline = PostprocessPipeline::new(&device, resources);
        let stencil_pipeline = StencilProgram::new(&device, resources);
        let reprojection_pipeline = ReprojectionProgram::new(&device, resources);

        let area_lut_texture = device.create_texture_from_png(resources, "area-lut");
        let gamma_lut_texture = device.create_texture_from_png(resources, "gamma-lut");

        let quad_vertex_positions_buffer = device.create_vertex_buffer(QUAD_VERTEX_POSITIONS.len());
        quad_vertex_positions_buffer.upload_data(&QUAD_VERTEX_POSITIONS);

        let fill_vertex_buffer = FillVertexBuffer::new(&device, MAX_FILLS_PER_BATCH);
        let alpha_multicolor_tile_vertex_buffer = AlphaTileVertexBuffer::new(&device, MAX_ALPHA_MULTICOLOR_TILES_PER_BATCH);
        let solid_multicolor_tile_vertex_buffer = SolidTileVertexBuffer::new(&device, &solid_multicolor_pipeline);
        let alpha_monochrome_tile_vertex_buffer = AlphaTileVertexBuffer::new(&device, &alpha_monochrome_pipeline);
        let solid_monochrome_tile_vertex_buffer = SolidTileVertexBuffer::new(&device, &solid_monochrome_pipeline);
        let postprocess_vertex_buffer = PostprocessVertexBuffer::new(&device, &postprocess_pipeline);
        let stencil_vertex_buffer = StencilVertexBuffer::new(&device, &stencil_pipeline);
        let reprojection_vertex_array = ReprojectionVertexArray::new(
            &device,
            &reprojection_pipeline,
            &quad_vertex_positions_buffer,
        );

        let mask_framebuffer_size =
            pfgeom::basic::point::Point2DI32::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT);
        let mask_framebuffer_texture =
            device.create_texture(TextureFormat::R16F, mask_framebuffer_size);
        let mask_framebuffer = device.create_framebuffer(mask_framebuffer_texture);

        let fill_colors_size =
            pfgeom::basic::point::Point2DI32::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT);
        let fill_colors_texture = device.create_texture(TextureFormat::RGBA8, fill_colors_size);

        let debug_ui = DebugUI::new(&device, resources, dest_framebuffer.window_size(&device));

        let renderer = Renderer {
            device,
            dest_framebuffer,
            fill_pipeline,
            solid_monochrome_pipeline,
            alpha_monochrome_pipeline,
            solid_multicolor_pipeline,
            alpha_multicolor_pipeline,
            solid_monochrome_tile_vertex_array,
            alpha_monochrome_tile_vertex_array,
            solid_multicolor_tile_vertex_array,
            alpha_multicolor_tile_vertex_array,
            area_lut_texture,
            quad_vertex_positions_buffer,
            fill_vertex_array,
            mask_framebuffer,
            fill_colors_texture,

            postprocess_source_framebuffer: None,
            postprocess_pipeline,
            postprocess_vertex_array,
            gamma_lut_texture,

            stencil_pipeline,
            stencil_vertex_array,

            reprojection_pipeline,
            reprojection_vertex_array,

            stats: RenderStats::default(),
            current_timer_query: None,
            pending_timer_queries: VecDeque::new(),
            free_timer_queries: vec![],
            debug_ui,

            mask_framebuffer_cleared: false,
            buffered_fills: vec![],

            render_mode: RenderMode::default(),
            use_depth: false,
        };

        // As a convenience, bind the destination framebuffer.
        renderer.bind_dest_framebuffer();

        renderer
    }

    pub fn begin_scene(&mut self) {
        self.init_postprocessing_framebuffer();

        let timer_query = self
            .free_timer_queries
            .pop()
            .unwrap_or_else(|| self.device.create_timer_query());
        self.device.begin_timer_query(&timer_query);
        self.current_timer_query = Some(timer_query);

        self.mask_framebuffer_cleared = false;
        self.stats = RenderStats::default();
    }

    pub fn render_command(&mut self, command: &gpu_data::RenderCommand) {
        match *command {
            gpu_data::RenderCommand::Start {
                bounding_quad,
                object_count,
            } => {
                if self.use_depth {
                    self.draw_stencil(&bounding_quad);
                }
                self.stats.object_count = object_count;
            }
            gpu_data::RenderCommand::AddShaders(ref shaders) => self.upload_shaders(shaders),
            gpu_data::RenderCommand::AddFills(ref fills) => self.add_fills(fills),
            gpu_data::RenderCommand::FlushFills => self.draw_buffered_fills(),
            gpu_data::RenderCommand::SolidTile(ref solid_tiles) => {
                let count = solid_tiles.len();
                self.stats.solid_tile_count += count;
                self.upload_solid_tiles(solid_tiles);
                self.draw_solid_tiles(count as u32);
            }
            gpu_data::RenderCommand::AlphaTile(ref alpha_tiles) => {
                let count = alpha_tiles.len();
                self.stats.alpha_tile_count += count;
                self.upload_alpha_tiles(alpha_tiles);
                self.draw_alpha_tiles(count as u32);
            }
            gpu_data::RenderCommand::Finish { .. } => {}
        }
    }

    pub fn end_scene(&mut self) {
        if self.postprocessing_needed() {
            self.postprocess();
        }

        let timer_query = self.current_timer_query.take().unwrap();
        self.device.end_timer_query(&timer_query);
        self.pending_timer_queries.push_back(timer_query);
    }

    pub fn draw_debug_ui(&self) {
        self.bind_dest_framebuffer();
        self.debug_ui.draw(&self.device);
    }

    pub fn shift_timer_query(&mut self) -> Option<Duration> {
        let query = self.pending_timer_queries.front()?;
        if !self.device.timer_query_is_available(&query) {
            return None;
        }
        let query = self.pending_timer_queries.pop_front().unwrap();
        let result = self.device.get_timer_query(&query);
        self.free_timer_queries.push(query);
        Some(result)
    }

    #[inline]
    pub fn dest_framebuffer(&self) -> &DestFramebuffer<D> {
        &self.dest_framebuffer
    }

    #[inline]
    pub fn replace_dest_framebuffer(
        &mut self,
        new_dest_framebuffer: DestFramebuffer<D>,
    ) -> DestFramebuffer<D> {
        mem::replace(&mut self.dest_framebuffer, new_dest_framebuffer)
    }

    #[inline]
    pub fn set_main_framebuffer_size(&mut self, new_framebuffer_size: pfgeom::basic::point::Point2DI32) {
        self.debug_ui.ui.set_framebuffer_size(new_framebuffer_size);
    }

    #[inline]
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.render_mode = mode;
    }

    #[inline]
    pub fn disable_depth(&mut self) {
        self.use_depth = false;
    }

    #[inline]
    pub fn enable_depth(&mut self) {
        self.use_depth = true;
    }

    #[inline]
    pub fn quad_vertex_positions_buffer(&self) -> &D::Buffer {
        &self.quad_vertex_positions_buffer
    }

    fn upload_shaders(&mut self, shaders: &[scene::ObjectShader]) {
        let size = pfgeom::basic::point::Point2DI32::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT);
        let mut fill_colors = vec![0; size.x() as usize * size.y() as usize * 4];
        for (shader_index, shader) in shaders.iter().enumerate() {
            fill_colors[shader_index * 4 + 0] = shader.fill_color.r;
            fill_colors[shader_index * 4 + 1] = shader.fill_color.g;
            fill_colors[shader_index * 4 + 2] = shader.fill_color.b;
            fill_colors[shader_index * 4 + 3] = shader.fill_color.a;
        }
        self.device
            .upload_to_texture(&self.fill_colors_texture, size, &fill_colors);
    }

    fn upload_solid_tiles(&mut self, solid_tiles: &[gpu_data::SolidTileBatchPrimitive]) {
        self.device.allocate_buffer(
            &self.solid_tile_vertex_array().vertex_buffer,
            BufferData::Memory(&solid_tiles),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );
    }

    fn upload_alpha_tiles(&mut self, alpha_tiles: &[gpu_data::AlphaTileBatchPrimitive]) {
        self.device.allocate_buffer(
            &self.alpha_tile_vertex_array().vertex_buffer,
            BufferData::Memory(&alpha_tiles),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );
    }

    fn clear_mask_framebuffer(&mut self) {
        self.device.bind_framebuffer(&self.mask_framebuffer);

        // TODO(pcwalton): Only clear the appropriate portion?
        self.device.clear(&ClearParams {
            color: Some(pfgeom::basic::color::ColorF::transparent_black()),
            ..ClearParams::default()
        });
    }

    fn add_fills(&mut self, mut fills: &[gpu_data::FillBatchPrimitive]) {
        if fills.is_empty() {
            return;
        }

        self.stats.fill_count += fills.len();

        while !fills.is_empty() {
            let count = cmp::min(fills.len(), MAX_FILLS_PER_BATCH - self.buffered_fills.len());
            self.buffered_fills.extend_from_slice(&fills[0..count]);
            fills = &fills[count..];
            if self.buffered_fills.len() == MAX_FILLS_PER_BATCH {
                self.draw_buffered_fills();
            }
        }
    }

    fn draw_buffered_fills(&mut self) {
        if self.buffered_fills.is_empty() {
            return;
        }

        self.device.allocate_buffer(
            &self.fill_vertex_array.vertex_buffer,
            BufferData::Memory(&self.buffered_fills),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );

        if !self.mask_framebuffer_cleared {
            self.clear_mask_framebuffer();
            self.mask_framebuffer_cleared = true;
        }

        self.device.bind_framebuffer(&self.mask_framebuffer);

        self.device
            .bind_vertex_array(&self.fill_vertex_array.vertex_array);
        self.device.use_pipeline(&self.fill_pipeline.program);
        self.device.set_uniform(
            &self.fill_pipeline.framebuffer_size_uniform,
            UniformData::Vec2(
                pfsimd::defau::I32x4::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT, 0, 0).to_f32x4(),
            ),
        );
        self.device.set_uniform(
            &self.fill_pipeline.tile_size_uniform,
            UniformData::Vec2(pfsimd::defau::I32x4::new(tiles::TILE_WIDTH as i32, tiles::TILE_HEIGHT as i32, 0, 0).to_f32x4()),
        );
        self.device.bind_texture(&self.area_lut_texture, 0);
        self.device.set_uniform(
            &self.fill_pipeline.area_lut_uniform,
            UniformData::TextureUnit(0),
        );
        let render_state = RenderState {
            blend: BlendState::RGBOneAlphaOne,
            ..RenderState::default()
        };
        debug_assert!(self.buffered_fills.len() <= u32::MAX as usize);
        self.device.draw_arrays_instanced(
            Primitive::TriangleFan,
            4,
            self.buffered_fills.len() as u32,
            &render_state,
        );

        self.buffered_fills.clear()
    }

    fn draw_alpha_tiles(&mut self, count: u32) {
        self.bind_draw_framebuffer();

        let alpha_tile_vertex_array = self.alpha_tile_vertex_array();
        let alpha_pipeline = self.alpha_pipeline();

        self.device
            .bind_vertex_array(&alpha_tile_vertex_array.vertex_array);
        self.device.use_pipeline(&alpha_pipeline.program);
        self.device.set_uniform(
            &alpha_pipeline.framebuffer_size_uniform,
            UniformData::Vec2(self.draw_viewport().size().to_f32().0),
        );
        self.device.set_uniform(
            &alpha_pipeline.tile_size_uniform,
            UniformData::Vec2(pfsimd::defau::I32x4::new(tiles::TILE_WIDTH as i32, tiles::TILE_HEIGHT as i32, 0, 0).to_f32x4()),
        );
        self.device
            .bind_texture(self.device.framebuffer_texture(&self.mask_framebuffer), 0);
        self.device.set_uniform(
            &alpha_pipeline.stencil_texture_uniform,
            UniformData::TextureUnit(0),
        );
        self.device.set_uniform(
            &alpha_pipeline.stencil_texture_size_uniform,
            UniformData::Vec2(
                pfsimd::defau::I32x4::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT, 0, 0).to_f32x4(),
            ),
        );

        match self.render_mode {
            RenderMode::Multicolor => {
                self.device.bind_texture(&self.fill_colors_texture, 1);
                self.device.set_uniform(
                    &self.alpha_multicolor_pipeline.fill_colors_texture_uniform,
                    UniformData::TextureUnit(1),
                );
                self.device.set_uniform(
                    &self
                        .alpha_multicolor_pipeline
                        .fill_colors_texture_size_uniform,
                    UniformData::Vec2(
                        pfsimd::defau::I32x4::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT, 0, 0)
                            .to_f32x4(),
                    ),
                );
            }
            RenderMode::Monochrome { .. } if self.postprocessing_needed() => {
                self.device.set_uniform(
                    &self.alpha_monochrome_pipeline.fill_color_uniform,
                    UniformData::Vec4(pfsimd::defau::F32x4::splat(1.0)),
                );
            }
            RenderMode::Monochrome { fg_color, .. } => {
                self.device.set_uniform(
                    &self.alpha_monochrome_pipeline.fill_color_uniform,
                    UniformData::Vec4(fg_color.0),
                );
            }
        }

        // FIXME(pcwalton): Fill this in properly!
        self.device.set_uniform(
            &alpha_pipeline.view_box_origin_uniform,
            UniformData::Vec2(pfsimd::defau::F32x4::default()),
        );
        let render_state = RenderState {
            blend: BlendState::RGBSrcAlphaAlphaOneMinusSrcAlpha,
            stencil: self.stencil_state(),
            ..RenderState::default()
        };
        self.device
            .draw_arrays_instanced(Primitive::TriangleFan, 4, count, &render_state);
    }

    fn draw_solid_tiles(&mut self, count: u32) {
        self.bind_draw_framebuffer();

        let solid_tile_vertex_array = self.solid_tile_vertex_array();
        let solid_pipeline = self.solid_pipeline();

        self.device
            .bind_vertex_array(&solid_tile_vertex_array.vertex_array);
        self.device.use_pipeline(&solid_pipeline.program);
        self.device.set_uniform(
            &solid_pipeline.framebuffer_size_uniform,
            UniformData::Vec2(self.draw_viewport().size().0.to_f32x4()),
        );
        self.device.set_uniform(
            &solid_pipeline.tile_size_uniform,
            UniformData::Vec2(pfsimd::defau::I32x4::new(tiles::TILE_WIDTH as i32, tiles::TILE_HEIGHT as i32, 0, 0).to_f32x4()),
        );

        match self.render_mode {
            RenderMode::Multicolor => {
                self.device.bind_texture(&self.fill_colors_texture, 0);
                self.device.set_uniform(
                    &self.solid_multicolor_pipeline.fill_colors_texture_uniform,
                    UniformData::TextureUnit(0),
                );
                self.device.set_uniform(
                    &self
                        .solid_multicolor_pipeline
                        .fill_colors_texture_size_uniform,
                    UniformData::Vec2(
                        pfsimd::defau::I32x4::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT, 0, 0)
                            .to_f32x4(),
                    ),
                );
            }
            RenderMode::Monochrome { .. } if self.postprocessing_needed() => {
                self.device.set_uniform(
                    &self.solid_monochrome_pipeline.fill_color_uniform,
                    UniformData::Vec4(pfsimd::defau::F32x4::splat(1.0)),
                );
            }
            RenderMode::Monochrome { fg_color, .. } => {
                self.device.set_uniform(
                    &self.solid_monochrome_pipeline.fill_color_uniform,
                    UniformData::Vec4(fg_color.0),
                );
            }
        }

        // FIXME(pcwalton): Fill this in properly!
        self.device.set_uniform(
            &solid_pipeline.view_box_origin_uniform,
            UniformData::Vec2(pfsimd::defau::F32x4::default()),
        );
        let render_state = RenderState {
            stencil: self.stencil_state(),
            ..RenderState::default()
        };
        self.device
            .draw_arrays_instanced(Primitive::TriangleFan, 4, count, &render_state);
    }

    fn postprocess(&mut self) {
        let (fg_color, bg_color, defringing_kernel, gamma_correction_enabled);
        match self.render_mode {
            RenderMode::Multicolor => return,
            RenderMode::Monochrome {
                fg_color: fg,
                bg_color: bg,
                defringing_kernel: kernel,
                gamma_correction,
            } => {
                fg_color = fg;
                bg_color = bg;
                defringing_kernel = kernel;
                gamma_correction_enabled = gamma_correction;
            }
        }

        self.bind_dest_framebuffer();

        self.device
            .bind_vertex_array(&self.postprocess_vertex_array.vertex_array);
        self.device.use_pipeline(&self.postprocess_pipeline.program);
        self.device.set_uniform(
            &self.postprocess_pipeline.framebuffer_size_uniform,
            UniformData::Vec2(self.main_viewport().size().to_f32().0),
        );
        match defringing_kernel {
            Some(ref kernel) => {
                self.device.set_uniform(
                    &self.postprocess_pipeline.kernel_uniform,
                    UniformData::Vec4(pfsimd::defau::F32x4::from_slice(&kernel.0)),
                );
            }
            None => {
                self.device.set_uniform(
                    &self.postprocess_pipeline.kernel_uniform,
                    UniformData::Vec4(pfsimd::defau::F32x4::default()),
                );
            }
        }

        let postprocess_source_framebuffer = self.postprocess_source_framebuffer.as_ref().unwrap();
        let source_texture = self
            .device
            .framebuffer_texture(postprocess_source_framebuffer);
        let source_texture_size = self.device.texture_size(source_texture);
        self.device.bind_texture(&source_texture, 0);
        self.device.set_uniform(
            &self.postprocess_pipeline.source_uniform,
            UniformData::TextureUnit(0),
        );
        self.device.set_uniform(
            &self.postprocess_pipeline.source_size_uniform,
            UniformData::Vec2(source_texture_size.0.to_f32x4()),
        );
        self.device.bind_texture(&self.gamma_lut_texture, 1);
        self.device.set_uniform(
            &self.postprocess_pipeline.gamma_lut_uniform,
            UniformData::TextureUnit(1),
        );
        self.device.set_uniform(
            &self.postprocess_pipeline.fg_color_uniform,
            UniformData::Vec4(fg_color.0),
        );
        self.device.set_uniform(
            &self.postprocess_pipeline.bg_color_uniform,
            UniformData::Vec4(bg_color.0),
        );
        self.device.set_uniform(
            &self.postprocess_pipeline.gamma_correction_enabled_uniform,
            UniformData::Int(gamma_correction_enabled as i32),
        );
        self.device
            .draw_arrays(Primitive::TriangleFan, 4, &RenderState::default());
    }

    fn solid_pipeline(&self) -> &SolidTileProgram<D> {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.solid_monochrome_pipeline.solid_pipeline,
            RenderMode::Multicolor => &self.solid_multicolor_pipeline.solid_pipeline,
        }
    }

    fn alpha_pipeline(&self) -> &AlphaTileProgram<D> {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.alpha_monochrome_pipeline.alpha_pipeline,
            RenderMode::Multicolor => &self.alpha_multicolor_pipeline.alpha_pipeline,
        }
    }

    fn solid_tile_vertex_array(&self) -> &SolidTileVertexArray<D> {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.solid_monochrome_tile_vertex_array,
            RenderMode::Multicolor => &self.solid_multicolor_tile_vertex_array,
        }
    }

    fn alpha_tile_vertex_array(&self) -> &AlphaTileVertexArray<D> {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.alpha_monochrome_tile_vertex_array,
            RenderMode::Multicolor => &self.alpha_multicolor_tile_vertex_array,
        }
    }

    fn draw_stencil(&self, quad_positions: &[pfgeom::basic::point::Point3DF32]) {
        self.device.allocate_buffer(
            &self.stencil_vertex_array.vertex_buffer,
            BufferData::Memory(quad_positions),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );
        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_array(&self.stencil_vertex_array.vertex_array);
        self.device.use_pipeline(&self.stencil_pipeline.program);
        self.device.draw_arrays(
            Primitive::TriangleFan,
            4,
            &RenderState {
                // FIXME(pcwalton): Should we really write to the depth buffer?
                depth: Some(DepthState {
                    func: DepthFunc::Less,
                    write: true,
                }),
                stencil: Some(StencilState {
                    func: StencilFunc::Always,
                    reference: 1,
                    mask: 1,
                    write: true,
                }),
                color_mask: false,
                ..RenderState::default()
            },
        )
    }

    pub fn reproject_texture(
        &self,
        texture: &pfgpu::Texture,
        old_transform: &pfgeom::basic::transform3d::Transform3DF32,
        new_transform: &pfgeom::basic::transform3d::Transform3DF32,
    ) {
        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_array(&self.reprojection_vertex_array.vertex_array);
        self.device
            .use_pipeline(&self.reprojection_pipeline.program);
        self.device.set_uniform(
            &self.reprojection_pipeline.old_transform_uniform,
            UniformData::from_transform_3d(old_transform),
        );
        self.device.set_uniform(
            &self.reprojection_pipeline.new_transform_uniform,
            UniformData::from_transform_3d(new_transform),
        );
        self.device.bind_texture(texture, 0);
        self.device.set_uniform(
            &self.reprojection_pipeline.texture_uniform,
            UniformData::TextureUnit(0),
        );
        self.device.draw_arrays(
            Primitive::TriangleFan,
            4,
            &RenderState {
                blend: BlendState::RGBSrcAlphaAlphaOneMinusSrcAlpha,
                depth: Some(DepthState {
                    func: DepthFunc::Less,
                    write: false,
                }),
                ..RenderState::default()
            },
        );
    }

    pub fn bind_draw_framebuffer(&self) {
        if self.postprocessing_needed() {
            self.device
                .bind_framebuffer(self.postprocess_source_framebuffer.as_ref().unwrap());
        } else {
            self.bind_dest_framebuffer();
        }
    }

    pub fn bind_dest_framebuffer(&self) {
        match self.dest_framebuffer {
            DestFramebuffer::Default { viewport, .. } => {
                self.device.bind_default_framebuffer(viewport)
            }
            DestFramebuffer::Other(ref framebuffer) => self.device.bind_framebuffer(framebuffer),
        }
    }

    fn init_postprocessing_framebuffer(&mut self) {
        if !self.postprocessing_needed() {
            self.postprocess_source_framebuffer = None;
            return;
        }

        let source_framebuffer_size = self.draw_viewport().size();
        match self.postprocess_source_framebuffer {
            Some(ref framebuffer)
            if self
                .device
                .texture_size(self.device.framebuffer_texture(framebuffer))
                == source_framebuffer_size => {}
            _ => {
                let texture = self
                    .device
                    .create_texture(TextureFormat::R8, source_framebuffer_size);
                self.postprocess_source_framebuffer = Some(self.device.create_framebuffer(texture))
            }
        };

        self.device
            .bind_framebuffer(self.postprocess_source_framebuffer.as_ref().unwrap());
        self.device.clear(&ClearParams {
            color: Some(pfgeom::basic::color::ColorF::transparent_black()),
            ..ClearParams::default()
        });
    }

    fn postprocessing_needed(&self) -> bool {
        match self.render_mode {
            RenderMode::Monochrome {
                ref defringing_kernel,
                gamma_correction,
                ..
            } => defringing_kernel.is_some() || gamma_correction,
            _ => false,
        }
    }

    fn draw_viewport(&self) -> pfgeom::basic::rect::RectI32 {
        let main_viewport = self.main_viewport();
        match self.render_mode {
            RenderMode::Monochrome {
                defringing_kernel: Some(..),
                ..
            } => {
                let scale = pfgeom::basic::point::Point2DI32::new(3, 1);
                pfgeom::basic::rect::RectI32::new(pfgeom::basic::point::Point2DI32::default(), main_viewport.size().scale_xy(scale))
            }
            _ => main_viewport,
        }
    }

    fn main_viewport(&self) -> pfgeom::basic::rect::RectI32 {
        match self.dest_framebuffer {
            DestFramebuffer::Default { viewport, .. } => viewport,
            DestFramebuffer::Other(ref framebuffer) => {
                let size = self
                    .device
                    .texture_size(self.device.framebuffer_texture(framebuffer));
                pfgeom::basic::rect::RectI32::new(pfgeom::basic::point::Point2DI32::default(), size)
            }
        }
    }
}

struct PostprocessVertexArray<D>
where
    D: Device,
{
    vertex_array: D::VertexArray,
}

impl<D> PostprocessVertexArray<D>
where
    D: Device,
{
    fn new(
        device: &D,
        postprocess_pipeline: &PostprocessProgram<D>,
        quad_vertex_positions_buffer: &D::Buffer,
    ) -> PostprocessVertexArray<D> {
        let vertex_array = device.create_vertex_array();
        let position_attr = device.get_vertex_attr(&postprocess_pipeline.program, "Position");

        device.bind_vertex_array(&vertex_array);
        device.use_pipeline(&postprocess_pipeline.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&position_attr, 2, VertexAttrType::U8, false, 0, 0, 0);

        PostprocessVertexArray { vertex_array }
    }
}

struct StencilProgram<D>
where
    D: Device,
{
    program: D::Program,
}

impl<D> StencilProgram<D>
where
    D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> StencilProgram<D> {
        let program = device.create_pipeline(resources, "stencil");
        StencilProgram { program }
    }
}

struct StencilVertexArray<D>
where
    D: Device,
{
    vertex_array: D::VertexArray,
    vertex_buffer: D::Buffer,
}

impl<D> StencilVertexArray<D>
where
    D: Device,
{
    fn new(device: &D, stencil_pipeline: &StencilProgram<D>) -> StencilVertexArray<D> {
        let (vertex_array, vertex_buffer) = (device.create_vertex_array(), device.create_buffer());

        let position_attr = device.get_vertex_attr(&stencil_pipeline.program, "Position");

        device.bind_vertex_array(&vertex_array);
        device.use_pipeline(&stencil_pipeline.program);
        device.bind_buffer(&vertex_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(
            &position_attr,
            3,
            VertexAttrType::F32,
            false,
            4 * 4,
            0,
            0,
        );

        StencilVertexArray {
            vertex_array,
            vertex_buffer,
        }
    }
}

struct ReprojectionProgram<D>
where
    D: Device,
{
    program: D::Program,
    old_transform_uniform: D::Uniform,
    new_transform_uniform: D::Uniform,
    texture_uniform: D::Uniform,
}

impl<D> ReprojectionProgram<D>
where
    D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> ReprojectionProgram<D> {
        let program = device.create_pipeline(resources, "reproject");
        let old_transform_uniform = device.get_uniform(&program, "OldTransform");
        let new_transform_uniform = device.get_uniform(&program, "NewTransform");
        let texture_uniform = device.get_uniform(&program, "Texture");

        ReprojectionProgram {
            program,
            old_transform_uniform,
            new_transform_uniform,
            texture_uniform,
        }
    }
}

struct ReprojectionVertexArray<D>
where
    D: Device,
{
    vertex_array: D::VertexArray,
}

impl<D> ReprojectionVertexArray<D>
where
    D: Device,
{
    fn new(
        device: &D,
        reprojection_pipeline: &ReprojectionProgram<D>,
        quad_vertex_positions_buffer: &D::Buffer,
    ) -> ReprojectionVertexArray<D> {
        let vertex_array = device.create_vertex_array();

        let position_attr = device.get_vertex_attr(&reprojection_pipeline.program, "Position");

        device.bind_vertex_array(&vertex_array);
        device.use_pipeline(&reprojection_pipeline.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&position_attr, 2, VertexAttrType::U8, false, 0, 0, 0);

        ReprojectionVertexArray { vertex_array }
    }
}

#[derive(Clone)]
pub enum DestFramebuffer<D>
where
    D: Device,
{
    Default {
        viewport: pfgeom::basic::rect::RectI32,
        window_size: pfgeom::basic::point::Point2DI32,
    },
    Other(D::Framebuffer),
}

impl<D> DestFramebuffer<D>
where
    D: Device,
{
    #[inline]
    pub fn full_window(window_size: pfgeom::basic::point::Point2DI32) -> DestFramebuffer<D> {
        let viewport = pfgeom::basic::rect::RectI32::new(pfgeom::basic::point::Point2DI32::default(), window_size);
        DestFramebuffer::Default {
            viewport,
            window_size,
        }
    }

    fn window_size(&self, device: &D) -> pfgeom::basic::point::Point2DI32 {
        match *self {
            DestFramebuffer::Default { window_size, .. } => window_size,
            DestFramebuffer::Other(ref framebuffer) => {
                device.texture_size(device.framebuffer_texture(framebuffer))
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum RenderMode {
    Multicolor,
    Monochrome {
        fg_color: pfgeom::basic::color::ColorF,
        bg_color: pfgeom::basic::color::ColorF,
        defringing_kernel: Option<post::DefringingKernel>,
        gamma_correction: bool,
    },
}

impl Default for RenderMode {
    #[inline]
    fn default() -> RenderMode {
        RenderMode::Multicolor
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub object_count: usize,
    pub fill_count: usize,
    pub alpha_tile_count: usize,
    pub solid_tile_count: usize,
}

impl Add<RenderStats> for RenderStats {
    type Output = RenderStats;
    fn add(self, other: RenderStats) -> RenderStats {
        RenderStats {
            object_count: self.object_count + other.object_count,
            solid_tile_count: self.solid_tile_count + other.solid_tile_count,
            alpha_tile_count: self.alpha_tile_count + other.alpha_tile_count,
            fill_count: self.fill_count + other.fill_count,
        }
    }
}

impl Div<usize> for RenderStats {
    type Output = RenderStats;
    fn div(self, divisor: usize) -> RenderStats {
        RenderStats {
            object_count: self.object_count / divisor,
            solid_tile_count: self.solid_tile_count / divisor,
            alpha_tile_count: self.alpha_tile_count / divisor,
            fill_count: self.fill_count / divisor,
        }
    }
}
