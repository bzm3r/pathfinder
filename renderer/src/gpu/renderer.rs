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
use pathfinder_resources as pfresources;
use std::cmp;
use std::collections::VecDeque;
use std::mem;
use std::ops::{Add, Div};
use std::time::Duration;
use std::u32;
use crate::gpu_data::{FillBatchPrimitive, AlphaTileBatchPrimitive, SolidTileBatchPrimitive, RenderCommand};

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

const MAX_FILLS_PER_BATCH: u64 = 0x4000;
const MAX_ALPHA_TILES_PER_BATCH: u64 = 0x4000;
const MAX_SOLID_TILES_PER_BATCH: u64 = 0x4000;
const MAX_POSTPROCESS_VERTICES: usize = 1; // what should this be?
const MAX_REPROJECTION_VERTICES: usize = 1; // what should this be?

pub struct Renderer {
    pub device: pfgpu::PfDevice,

    fill_pipeline: pfgpu::pipeline::FillPipeline,
    solid_multicolor_pipeline: pfgpu::pipeline::SolidMulticolorPipeline,
    alpha_multicolor_pipeline: pfgpu::pipeline::AlphaMulticolorPipeline,
    solid_monochrome_pipeline: pfgpu::pipeline::SolidMonochromePipeline,
    alpha_monochrome_pipeline: pfgpu::pipeline::AlphaMonochromePipeline,
    solid_monochrome_tile_vertex_buffer: SolidTileVertexBuffer,
    alpha_monochrome_tile_vertex_buffer: AlphaTileVertexBuffer,
    solid_multicolor_tile_vertex_buffer: SolidTileVertexBuffer,
    alpha_multicolor_tile_vertex_buffer: AlphaTileVertexBuffer,

    area_lut_texture: pfgpu::Texture,
    quad_vertex_positions_buffer: pfgpu::Buffer,
    fill_vertex_buffer: FillVertexBuffer,
    mask_framebuffer: pfgpu::Framebuffer,
    fill_colors_texture: pfgpu::Texture,

    // Postprocessing shader
    postprocess_source_framebuffer: pfgpu::Framebuffer,
    postprocess_pipeline: pfgpu::pipeline::PostprocessPipeline,
    postprocess_vertex_buffer: PostprocessVertexBuffer,
    gamma_lut_texture: pfgpu::Texture,

    // Stencil shader
    stencil_pipeline: pfgpu::pipeline::StencilPipeline,
    stencil_vertex_buffer: StencilVertexBuffer,

    // Reprojection shader
    //reprojection_pipeline: pfgpu::pipeline::ReprojectionPipeline,
    //reprojection_vertex_buffer: ReprojectionVertexBuffer,

    // Rendering state
    mask_framebuffer_cleared: bool,
    buffered_fills: Vec<gpu_data::FillBatchPrimitive>,
    buffered_alpha_tiles: Vec<gpu_data::AlphaTileBatchPrimitive>,
    buffered_solid_tiles: Vec<gpu_data::SolidTileBatchPrimitive>,

    // Extra info
    use_depth: bool,
}

impl Renderer {
    pub unsafe fn new(
        window: &winit::Window,
        instance_name: &str,
        resources: &dyn pfresources::ResourceLoader,
    ) -> Renderer {
        let device = Device::new(window, instance_name);

        let fill_pipeline = FillPipeline::new(&device, resources);
        let solid_multicolor_pipeline = SolidMulticolorPipeline::new(&device, resources);
        let alpha_multicolor_pipeline = AlphaMulticolorPipeline::new(&device, resources);
        let solid_monochrome_pipeline = SolidTileMonochromePipeline::new(&device, resources);
        let alpha_monochrome_pipeline = AlphaTileMonochromePipeline::new(&device, resources);
        let postprocess_pipeline = PostprocessPipeline::new(&device, resources);
        let stencil_pipeline = StencilProgram::new(&device, resources);
        //let reprojection_pipeline = ReprojectionProgram::new(&device, resources);

        let area_lut_texture = device.create_texture_from_png(resources, "area-lut");
        let gamma_lut_texture = device.create_texture_from_png(resources, "gamma-lut");

        let quad_vertex_positions_buffer = device.create_vertex_buffer(QUAD_VERTEX_POSITIONS.len());
        quad_vertex_positions_buffer.upload_data(&QUAD_VERTEX_POSITIONS);

        let fill_vertex_buffer = FillVertexBuffer::new(&device, MAX_FILLS_PER_BATCH);
        let alpha_multicolor_tile_vertex_buffer = AlphaTileVertexBuffer::new(&device, MAX_ALPHA_TILES_PER_BATCH);
        let solid_multicolor_tile_vertex_buffer = SolidTileVertexBuffer::new(&device, MAX_SOLID_TILES_PER_BATCH);
        let alpha_monochrome_tile_vertex_buffer = AlphaTileVertexBuffer::new(&device, MAX_ALPHA_TILES_PER_BATCH);
        let solid_monochrome_tile_vertex_buffer = SolidTileVertexBuffer::new(&device, MAX_SOLID_TILES_PER_BATCH);
        let postprocess_vertex_buffer = PostprocessVertexBuffer::new(&device, MAX_POSTPROCESS_VERTICES);
        let stencil_vertex_buffer = StencilVertexBuffer::new(&device, QUAD_VERTEX_POSITIONS.len() as u64);
        //let reprojection_vertex_buffer = ReprojectionVertexBuffer::new(&device, MAX_REPROJECTION_VERTICES);

        let mask_framebuffer_size =
            pfgeom::basic::point::Point2DI32::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT);
        let mask_framebuffer_texture = device.create_texture(hal::format::Format::R16Sfloat, mask_framebuffer_size);
        let mask_framebuffer = device.create_framebuffer(mask_framebuffer_texture, &fill_pipeline.render_pass);

        let fill_colors_size =
            pfgeom::basic::point::Point2DI32::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT);
        let fill_colors_texture = device.create_texture(hal::format::Format::Rgba8Srgb, fill_colors_size);

        Renderer {
            device,
            fill_pipeline,
            solid_monochrome_pipeline,
            alpha_monochrome_pipeline,
            solid_multicolor_pipeline,
            alpha_multicolor_pipeline,
            solid_monochrome_tile_vertex_buffer,
            alpha_monochrome_tile_vertex_buffer,
            solid_multicolor_tile_vertex_buffer,
            alpha_multicolor_tile_vertex_buffer,
            area_lut_texture,
            quad_vertex_positions_buffer,
            fill_vertex_buffer,
            mask_framebuffer,
            fill_colors_texture,

            postprocess_source_framebuffer: None,
            postprocess_pipeline,
            postprocess_vertex_buffer,
            gamma_lut_texture,

            stencil_pipeline,
            stencil_vertex_buffer,

            //reprojection_pipeline,
            //reprojection_vertex_buffer,

            mask_framebuffer_cleared: false,
            buffered_fills: vec![],
            buffered_alpha_tiles: vec![],
            buffered_solid_tiles: vec![],

            use_depth: false,
        }
    }

    pub fn begin_scene(&mut self) {
        self.init_postprocessing_framebuffer();

        self.mask_framebuffer_cleared = false;
    }

    pub fn render_command(&mut self, command: &gpu_data::RenderCommand) {
        match *command {
            RenderCommand::Start {
                bounding_quad,
                path_count,
            } => {
                if self.use_depth {
                    self.draw_stencil(&bounding_quad);
                }
                self.stats.path_count = path_count;
            }
            RenderCommand::AddShaders(ref shaders) => self.upload_shaders(shaders),
            RenderCommand::AddFills(ref fills) => self.add_fills(fills),
            RenderCommand::FlushFills => {
                self.begin_composite_timer_query();
                self.draw_buffered_fills();
            }
            RenderCommand::AddSolidTiles(ref solid_tiles) => self.add_solid_tiles(solid_tiles),
            RenderCommand::FlushSolidTiles => {
                self.begin_composite_timer_query();
                self.draw_buffered_solid_tiles();
            }
            RenderCommand::AddAlphaTiles(ref alpha_tiles) => self.add_alpha_tiles(alpha_tiles),
            RenderCommand::FlushAlphaTiles => {
                self.begin_composite_timer_query();
                self.draw_buffered_alpha_tiles();
            }
            gpu_data::RenderCommand::Finish { .. } => {}
        }
    }

    pub fn end_scene(&mut self) {
        if self.postprocessing_needed() {
            self.postprocess();
        }

        self.end_composite_timer_query();
        self.pending_timers
            .push_back(mem::replace(&mut self.current_timers, RenderTimers::new()));
    }

    pub fn draw_debug_ui(&self) {
        self.bind_dest_framebuffer();
        self.debug_ui_presenter.draw(&self.device);
    }

    pub fn shift_rendering_time(&mut self) -> Option<RenderTime> {
        let timers = self.pending_timers.front()?;

        // Accumulate stage-0 time.
        let mut total_stage_0_time = Duration::new(0, 0);
        for timer_query in &timers.stage_0 {
            if !self.device.timer_query_is_available(timer_query) {
                return None;
            }
            total_stage_0_time += self.device.get_timer_query(timer_query);
        }

        // Get stage-1 time.
        let stage_1_time = {
            let stage_1_timer_query = timers.stage_1.as_ref().unwrap();
            if !self.device.timer_query_is_available(&stage_1_timer_query) {
                return None;
            }
            self.device.get_timer_query(stage_1_timer_query)
        };

        // Recycle all timer queries.
        let timers = self.pending_timers.pop_front().unwrap();
        self.free_timer_queries.extend(timers.stage_0.into_iter());
        self.free_timer_queries.push(timers.stage_1.unwrap());

        Some(RenderTime {
            stage_0: total_stage_0_time,
            stage_1: stage_1_time,
        })
    }

    #[inline]
    pub fn dest_framebuffer(&self) -> &DestFramebuffer {
        &self.dest_framebuffer
    }

    #[inline]
    pub fn replace_dest_framebuffer(
        &mut self,
        new_dest_framebuffer: DestFramebuffer,
    ) -> DestFramebuffer {
        mem::replace(&mut self.dest_framebuffer, new_dest_framebuffer)
    }

    #[inline]
    pub fn set_main_framebuffer_size(&mut self, new_framebuffer_size: pfgeom::basic::point::Point2DI32) {
        self.debug_ui_presenter
            .ui_presenter
            .set_framebuffer_size(new_framebuffer_size);
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
    pub fn quad_vertex_positions_buffer(&self) -> &pfgpu::Buffer {
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

        let timer_query = self.allocate_timer_query();
        self.device.begin_timer_query(&timer_query);

        self.stats.fill_count += fills.len();

        while !fills.is_empty() {
            let count = cmp::min(fills.len(), (MAX_FILLS_PER_BATCH - self.buffered_fills.len()) as usize);
            self.buffered_fills.extend_from_slice(&fills[0..count]);
            fills = &fills[count..];
            if self.buffered_fills.len() == MAX_FILLS_PER_BATCH {
                self.draw_buffered_fills();
            }
        }

        self.device.end_timer_query(&timer_query);
        self.current_timers.stage_0.push(timer_query);
    }

    fn add_solid_tiles(&mut self, mut solid_tiles: &[SolidTileBatchPrimitive]) {
        if solid_tiles.is_empty() {
            return;
        }

        let timer_query = self.allocate_timer_query();
        self.device.begin_timer_query(&timer_query);

        self.stats.solid_tile_count += solid_tiles.len();

        while !solid_tiles.is_empty() {
            let count = cmp::min(
                solid_tiles.len(),
                (MAX_SOLID_TILES_PER_BATCH - self.buffered_solid_tiles.len()) as usize,
            );
            self.buffered_solid_tiles
                .extend_from_slice(&solid_tiles[0..count]);
            solid_tiles = &solid_tiles[count..];
            if self.buffered_solid_tiles.len() == MAX_SOLID_TILES_PER_BATCH {
                self.draw_buffered_solid_tiles();
            }
        }

        self.device.end_timer_query(&timer_query);
        self.current_timers.stage_0.push(timer_query);
    }

    fn add_alpha_tiles(&mut self, mut alpha_tiles: &[AlphaTileBatchPrimitive]) {
        if alpha_tiles.is_empty() {
            return;
        }

        let timer_query = self.allocate_timer_query();
        self.device.begin_timer_query(&timer_query);

        self.stats.alpha_tile_count += alpha_tiles.len();

        while !alpha_tiles.is_empty() {
            let count = cmp::min(
                alpha_tiles.len(),
                (MAX_ALPHA_TILES_PER_BATCH - self.buffered_alpha_tiles.len()) as usize,
            );
            self.buffered_alpha_tiles
                .extend_from_slice(&alpha_tiles[0..count]);
            alpha_tiles = &alpha_tiles[count..];
            if self.buffered_alpha_tiles.len() == MAX_ALPHA_TILES_PER_BATCH {
                self.draw_buffered_alpha_tiles();
            }
        }

        self.device.end_timer_query(&timer_query);
        self.current_timers.stage_0.push(timer_query);
    }

    fn draw_buffered_fills(&mut self) {
        if self.buffered_fills.is_empty() {
            return;
        }

        self.device.allocate_buffer(
            &self.fill_vertex_buffer.vertex_buffer,
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
            .bind_vertex_buffer(&self.fill_vertex_buffer.vertex_array);
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

        debug_assert!(self.buffered_fills.len() <= u32::MAX as usize);
        self.device.draw_arrays_instanced(
            Primitive::TriangleFan,
            4,
            self.buffered_fills.len() as u32,
            &render_state,
        );

        self.buffered_fills.clear()
    }

    fn draw_buffered_alpha_tiles(&mut self) {
        if self.buffered_alpha_tiles.is_empty() {
            return;
        }

        let alpha_tile_vertex_buffer = self.alpha_tile_vertex_buffer();

        self.device.allocate_buffer(
            &alpha_tile_vertex_buffer.vertex_buffer,
            BufferData::Memory(&self.buffered_alpha_tiles),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );

        let alpha_tile_pipeline = self.alpha_tile_pipeline();

        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_buffer(&alpha_tile_vertex_buffer.vertex_array);
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
        debug_assert!(self.buffered_alpha_tiles.len() <= u32::MAX as usize);
        self.device.draw_arrays_instanced(
            Primitive::TriangleFan,
            4,
            self.buffered_alpha_tiles.len() as u32,
            &render_state,
        );

        self.buffered_alpha_tiles.clear();
    }

    fn draw_buffered_solid_tiles(&mut self) {
        if self.buffered_solid_tiles.is_empty() {
            return;
        }

        let solid_tile_vertex_buffer = self.solid_tile_vertex_buffer();

        self.device.allocate_buffer(
            &solid_tile_vertex_buffer.vertex_buffer,
            BufferData::Memory(&self.buffered_solid_tiles),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );

        let solid_pipeline = self.solid_pipeline();

        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_buffer(&solid_tile_vertex_buffer.vertex_array);
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
        debug_assert!(self.buffered_solid_tiles.len() <= u32::MAX as usize);
        self.device.draw_arrays_instanced(
            Primitive::TriangleFan,
            4,
            self.buffered_solid_tiles.len() as u32,
            &render_state,
        );

        self.buffered_solid_tiles.clear();
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
            .bind_vertex_buffer(&self.postprocess_vertex_buffer.vertex_array);
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

    fn solid_pipeline(&self) -> &SolidTileProgram {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.solid_monochrome_pipeline.solid_pipeline,
            RenderMode::Multicolor => &self.solid_multicolor_pipeline.solid_pipeline,
        }
    }

    fn alpha_pipeline(&self) -> &AlphaTileProgram {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.alpha_monochrome_pipeline.alpha_pipeline,
            RenderMode::Multicolor => &self.alpha_multicolor_pipeline.alpha_pipeline,
        }
    }

    fn solid_tile_vertex_buffer(&self) -> &SolidTileVertexArray {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.solid_monochrome_tile_vertex_buffer,
            RenderMode::Multicolor => &self.solid_multicolor_tile_vertex_buffer,
        }
    }

    fn alpha_tile_vertex_buffer(&self) -> &AlphaTileVertexArray {
        match self.render_mode {
            RenderMode::Monochrome { .. } => &self.alpha_monochrome_tile_vertex_buffer,
            RenderMode::Multicolor => &self.alpha_multicolor_tile_vertex_buffer,
        }
    }

    fn draw_stencil(&self, quad_positions: &[pfgeom::basic::point::Point3DF32]) {
        self.device.allocate_buffer(
            &self.stencil_vertex_buffer.vertex_buffer,
            BufferData::Memory(quad_positions),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );
        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_buffer(&self.stencil_vertex_buffer.vertex_array);
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
            .bind_vertex_buffer(&self.reprojection_vertex_buffer.vertex_array);
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
        let size = pfgeom::basic::point::Point2DI32::new(self.device.extent.width as i32, self.device.extent.height as i32)
        let origin = pfgeom::basic::point::Point2DI32::new(0, 0);
        pfgeom::basic::rect::RectI32::new(origin, size);
    }

    fn allocate_timer_query(&mut self) -> D::TimerQuery {
        match self.free_timer_queries.pop() {
            Some(query) => query,
            None => self.device.create_timer_query(),
        }
    }

    fn begin_composite_timer_query(&mut self) {
        let timer_query = self.allocate_timer_query();
        self.device.begin_timer_query(&timer_query);
        self.current_timers.stage_1 = Some(timer_query);
    }

    fn end_composite_timer_query(&mut self) {
        let query = self
            .current_timers
            .stage_1
            .as_ref()
            .expect("No stage 1 timer query yet?!");
        self.device.end_timer_query(&query);
    }
}

struct FillVertexBuffer(pfgpu::Buffer);

impl FillVertexBuffer
{
    unsafe fn new(device: &pfgpu::PfDevice, size: u64) -> FillVertexBuffer {
        let buffer = device.create_vertex_buffer(size*std::mem::size_of::<gpu_data::FillBatchPrimitive>());
        FillVertexBuffer(buffer);
    }
}

struct AlphaTileVertexBuffer(pfgpu::Buffer);

impl AlphaTileVertexBuffer
{
    unsafe fn new(device: &pfgpu::PfDevice, size: u64) -> AlphaTileVertexBuffer {
        let buffer = device.create_vertex_buffer(size*std::mem::size_of::<gpu_data::AlphaTileBatchPrimitive>());
        AlphaTileVertexBuffer(buffer);
    }
}

struct SolidTileVertexBuffer(pfgpu::Buffer);

impl SolidTileVertexBuffer
{
    unsafe fn new(device: &pfgpu::PfDevice, size: u64) -> SolidTileVertexBuffer {
        let buffer = device.create_vertex_buffer(size*std::mem::size_of::<gpu_data::SolidTileBatchPrimitive>());
        SolidTileVertexBuffer(buffer);
    }
}

struct StencilVertexBuffer(pfgpu::Buffer);

impl StencilVertexBuffer
{
    unsafe fn new(device: &pfgpu::PfDevice, size: u64) -> StencilVertexBuffer {
        let buffer = device.create_vertex_buffer(size*std::mem::size_of::<u8>());
        StencilVertexBuffer(buffer);
    }
}

struct AlphaTileVertexArray
{
    vertex_array: D::VertexArray,
    vertex_buffer: pfgpu::Buffer,
}

impl AlphaTileVertexArray
{
    fn new(
        device: &pfgpu::PfDevice,
        alpha_tile_program: &AlphaTileProgram,
        quad_vertex_positions_buffer: &pfgpu::Buffer,
    ) -> AlphaTileVertexArray {
        let vertex_array = device.create_vertex_buffer();

        let vertex_buffer = device.create_buffer();
        let vertex_buffer_data: BufferData<AlphaTileBatchPrimitive> =
            BufferData::Uninitialized(MAX_ALPHA_TILES_PER_BATCH);
        device.allocate_buffer(
            &vertex_buffer,
            vertex_buffer_data,
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );

        let tess_coord_attr = device.get_vertex_attr(&alpha_tile_program.program, "TessCoord");
        let tile_origin_attr = device.get_vertex_attr(&alpha_tile_program.program, "TileOrigin");
        let backdrop_attr = device.get_vertex_attr(&alpha_tile_program.program, "Backdrop");
        let object_attr = device.get_vertex_attr(&alpha_tile_program.program, "Object");
        let tile_index_attr = device.get_vertex_attr(&alpha_tile_program.program, "TileIndex");

        // NB: The object must be of type `I16`, not `U16`, to work around a macOS Radeon
        // driver bug.
        device.bind_vertex_buffer(&vertex_array);
        device.use_program(&alpha_tile_program.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&tess_coord_attr, 2, VertexAttrType::U8, false, 0, 0, 0);
        device.bind_buffer(&vertex_buffer, BufferTarget::Vertex);
        device.configure_int_vertex_attr(
            &tile_origin_attr,
            3,
            VertexAttrType::U8,
            MASK_TILE_INSTANCE_SIZE,
            0,
            1,
        );
        device.configure_int_vertex_attr(
            &backdrop_attr,
            1,
            VertexAttrType::I8,
            MASK_TILE_INSTANCE_SIZE,
            3,
            1,
        );
        device.configure_int_vertex_attr(
            &object_attr,
            2,
            VertexAttrType::I16,
            MASK_TILE_INSTANCE_SIZE,
            4,
            1,
        );
        device.configure_int_vertex_attr(
            &tile_index_attr,
            2,
            VertexAttrType::I16,
            MASK_TILE_INSTANCE_SIZE,
            6,
            1,
        );

        AlphaTileVertexArray {
            vertex_array,
            vertex_buffer,
        }
    }
}

struct SolidTileVertexArray
{
    vertex_array: D::VertexArray,
    vertex_buffer: pfgpu::Buffer,
}

impl SolidTileVertexArray
{
    fn new(
        device: &pfgpu::PfDevice,
        solid_tile_program: &SolidTileProgram,
        quad_vertex_positions_buffer: &pfgpu::Buffer,
    ) -> SolidTileVertexArray {
        let vertex_array = device.create_vertex_buffer();

        let vertex_buffer = device.create_buffer();
        let vertex_buffer_data: BufferData<AlphaTileBatchPrimitive> =
            BufferData::Uninitialized(MAX_SOLID_TILES_PER_BATCH);
        device.allocate_buffer(
            &vertex_buffer,
            vertex_buffer_data,
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );

        let tess_coord_attr = device.get_vertex_attr(&solid_tile_program.program, "TessCoord");
        let tile_origin_attr = device.get_vertex_attr(&solid_tile_program.program, "TileOrigin");
        let object_attr = device.get_vertex_attr(&solid_tile_program.program, "Object");

        // NB: The object must be of type short, not unsigned short, to work around a macOS
        // Radeon driver bug.
        device.bind_vertex_buffer(&vertex_array);
        device.use_program(&solid_tile_program.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&tess_coord_attr, 2, VertexAttrType::U8, false, 0, 0, 0);
        device.bind_buffer(&vertex_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(
            &tile_origin_attr,
            2,
            VertexAttrType::I16,
            false,
            SOLID_TILE_INSTANCE_SIZE,
            0,
            1,
        );
        device.configure_int_vertex_attr(
            &object_attr,
            1,
            VertexAttrType::I16,
            SOLID_TILE_INSTANCE_SIZE,
            4,
            1,
        );

        SolidTileVertexArray {
            vertex_array,
            vertex_buffer,
        }
    }
}

struct FillProgram
{
    program: D::Program,
    framebuffer_size_uniform: D::Uniform,
    tile_size_uniform: D::Uniform,
    area_lut_uniform: D::Uniform,
}

impl FillProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> FillProgram {
        let program = device.create_program(resources, "fill");
        let framebuffer_size_uniform = device.get_uniform(&program, "FramebufferSize");
        let tile_size_uniform = device.get_uniform(&program, "TileSize");
        let area_lut_uniform = device.get_uniform(&program, "AreaLUT");
        FillProgram {
            program,
            framebuffer_size_uniform,
            tile_size_uniform,
            area_lut_uniform,
        }
    }
}

struct SolidTileProgram
{
    program: D::Program,
    framebuffer_size_uniform: D::Uniform,
    tile_size_uniform: D::Uniform,
    view_box_origin_uniform: D::Uniform,
}

impl SolidTileProgram
{
    fn new(device: &pfgpu::PfDevice, program_name: &str, resources: &dyn ResourceLoader) -> SolidTileProgram {
        let program = device.create_program_from_shader_names(
            resources,
            program_name,
            program_name,
            "tile_solid",
        );
        let framebuffer_size_uniform = device.get_uniform(&program, "FramebufferSize");
        let tile_size_uniform = device.get_uniform(&program, "TileSize");
        let view_box_origin_uniform = device.get_uniform(&program, "ViewBoxOrigin");
        SolidTileProgram {
            program,
            framebuffer_size_uniform,
            tile_size_uniform,
            view_box_origin_uniform,
        }
    }
}

struct SolidTileMulticolorProgram
{
    solid_tile_program: SolidTileProgram,
    fill_colors_texture_uniform: D::Uniform,
    fill_colors_texture_size_uniform: D::Uniform,
}

impl SolidTileMulticolorProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> SolidTileMulticolorProgram {
        let solid_tile_program = SolidTileProgram::new(device, "tile_solid_multicolor", resources);
        let fill_colors_texture_uniform =
            device.get_uniform(&solid_tile_program.program, "FillColorsTexture");
        let fill_colors_texture_size_uniform =
            device.get_uniform(&solid_tile_program.program, "FillColorsTextureSize");
        SolidTileMulticolorProgram {
            solid_tile_program,
            fill_colors_texture_uniform,
            fill_colors_texture_size_uniform,
        }
    }
}

struct SolidTileMonochromeProgram
{
    solid_tile_program: SolidTileProgram,
    fill_color_uniform: D::Uniform,
}

impl SolidTileMonochromeProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> SolidTileMonochromeProgram {
        let solid_tile_program = SolidTileProgram::new(device, "tile_solid_monochrome", resources);
        let fill_color_uniform = device.get_uniform(&solid_tile_program.program, "FillColor");
        SolidTileMonochromeProgram {
            solid_tile_program,
            fill_color_uniform,
        }
    }
}

struct AlphaTileProgram
{
    program: D::Program,
    framebuffer_size_uniform: D::Uniform,
    tile_size_uniform: D::Uniform,
    stencil_texture_uniform: D::Uniform,
    stencil_texture_size_uniform: D::Uniform,
    view_box_origin_uniform: D::Uniform,
}

impl AlphaTileProgram
{
    fn new(device: &pfgpu::PfDevice, program_name: &str, resources: &dyn ResourceLoader) -> AlphaTileProgram {
        let program = device.create_program_from_shader_names(
            resources,
            program_name,
            program_name,
            "tile_alpha",
        );
        let framebuffer_size_uniform = device.get_uniform(&program, "FramebufferSize");
        let tile_size_uniform = device.get_uniform(&program, "TileSize");
        let stencil_texture_uniform = device.get_uniform(&program, "StencilTexture");
        let stencil_texture_size_uniform = device.get_uniform(&program, "StencilTextureSize");
        let view_box_origin_uniform = device.get_uniform(&program, "ViewBoxOrigin");
        AlphaTileProgram {
            program,
            framebuffer_size_uniform,
            tile_size_uniform,
            stencil_texture_uniform,
            stencil_texture_size_uniform,
            view_box_origin_uniform,
        }
    }
}

struct AlphaTileMulticolorProgram
{
    alpha_tile_program: AlphaTileProgram,
    fill_colors_texture_uniform: D::Uniform,
    fill_colors_texture_size_uniform: D::Uniform,
}

impl AlphaTileMulticolorProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> AlphaTileMulticolorProgram {
        let alpha_tile_program = AlphaTileProgram::new(device, "tile_alpha_multicolor", resources);
        let fill_colors_texture_uniform =
            device.get_uniform(&alpha_tile_program.program, "FillColorsTexture");
        let fill_colors_texture_size_uniform =
            device.get_uniform(&alpha_tile_program.program, "FillColorsTextureSize");
        AlphaTileMulticolorProgram {
            alpha_tile_program,
            fill_colors_texture_uniform,
            fill_colors_texture_size_uniform,
        }
    }
}

struct AlphaTileMonochromeProgram
{
    alpha_tile_program: AlphaTileProgram,
    fill_color_uniform: D::Uniform,
}

impl AlphaTileMonochromeProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> AlphaTileMonochromeProgram {
        let alpha_tile_program = AlphaTileProgram::new(device, "tile_alpha_monochrome", resources);
        let fill_color_uniform = device.get_uniform(&alpha_tile_program.program, "FillColor");
        AlphaTileMonochromeProgram {
            alpha_tile_program,
            fill_color_uniform,
        }
    }
}

struct PostprocessProgram
{
    program: D::Program,
    source_uniform: D::Uniform,
    source_size_uniform: D::Uniform,
    framebuffer_size_uniform: D::Uniform,
    kernel_uniform: D::Uniform,
    gamma_lut_uniform: D::Uniform,
    gamma_correction_enabled_uniform: D::Uniform,
    fg_color_uniform: D::Uniform,
    bg_color_uniform: D::Uniform,
}

impl PostprocessProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> PostprocessProgram {
        let program = device.create_program(resources, "post");
        let source_uniform = device.get_uniform(&program, "Source");
        let source_size_uniform = device.get_uniform(&program, "SourceSize");
        let framebuffer_size_uniform = device.get_uniform(&program, "FramebufferSize");
        let kernel_uniform = device.get_uniform(&program, "Kernel");
        let gamma_lut_uniform = device.get_uniform(&program, "GammaLUT");
        let gamma_correction_enabled_uniform =
            device.get_uniform(&program, "GammaCorrectionEnabled");
        let fg_color_uniform = device.get_uniform(&program, "FGColor");
        let bg_color_uniform = device.get_uniform(&program, "BGColor");
        PostprocessProgram {
            program,
            source_uniform,
            source_size_uniform,
            framebuffer_size_uniform,
            kernel_uniform,
            gamma_lut_uniform,
            gamma_correction_enabled_uniform,
            fg_color_uniform,
            bg_color_uniform,
        }
    }
}

>>>>>>> master
struct PostprocessVertexArray
{
    vertex_array: D::VertexArray,
}

impl PostprocessVertexArray
{
    fn new(
        device: &pfgpu::PfDevice,
        postprocess_pipeline: &PostprocessProgram,
        quad_vertex_positions_buffer: &pfgpu::Buffer,
    ) -> PostprocessVertexArray {
        let vertex_array = device.create_vertex_buffer();
        let position_attr = device.get_vertex_attr(&postprocess_pipeline.program, "Position");

        device.bind_vertex_buffer(&vertex_array);
        device.use_pipeline(&postprocess_pipeline.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&position_attr, 2, VertexAttrType::U8, false, 0, 0, 0);

        PostprocessVertexArray { vertex_array }
    }
}

struct StencilProgram
{
    program: D::Program,
}

impl StencilProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> StencilProgram {
        let program = device.create_pipeline(resources, "stencil");
        StencilProgram { program }
    }
}

struct StencilVertexArray
{
    vertex_array: D::VertexArray,
    vertex_buffer: pfgpu::Buffer,
}

impl StencilVertexArray
{
    fn new(device: &pfgpu::PfDevice, stencil_pipeline: &StencilProgram) -> StencilVertexArray {
        let (vertex_array, vertex_buffer) = (device.create_vertex_buffer(), device.create_buffer());

        let position_attr = device.get_vertex_attr(&stencil_pipeline.program, "Position");

        device.bind_vertex_buffer(&vertex_array);
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

struct ReprojectionProgram
{
    program: D::Program,
    old_transform_uniform: D::Uniform,
    new_transform_uniform: D::Uniform,
    texture_uniform: D::Uniform,
}

impl ReprojectionProgram
{
    fn new(device: &pfgpu::PfDevice, resources: &dyn ResourceLoader) -> ReprojectionProgram {
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

struct ReprojectionVertexArray
{
    vertex_array: D::VertexArray,
}

impl ReprojectionVertexArray
{
    fn new(
        device: &pfgpu::PfDevice,
        reprojection_pipeline: &ReprojectionProgram,
        quad_vertex_positions_buffer: &pfgpu::Buffer,
    ) -> ReprojectionVertexArray {
        let vertex_array = device.create_vertex_buffer();

        let position_attr = device.get_vertex_attr(&reprojection_pipeline.program, "Position");

        device.bind_vertex_buffer(&vertex_array);
        device.use_pipeline(&reprojection_pipeline.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&position_attr, 2, VertexAttrType::U8, false, 0, 0, 0);

        ReprojectionVertexArray { vertex_array }
    }
}

#[derive(Clone)]
pub enum DestFramebuffer
{
    Default {
        viewport: pfgeom::basic::rect::RectI32,
        window_size: pfgeom::basic::point::Point2DI32,
    },
    Other(D::pfgpu::Framebuffer),
}

impl DestFramebuffer
{
    #[inline]
    pub fn full_window(window_size: Point2DI32) -> DestFramebuffer {
        let viewport = RectI32::new(Point2DI32::default(), window_size);
        DestFramebuffer::Default {
            viewport,
            window_size,
        }
    }

    fn window_size(&self, device: &pfgpu::PfDevice) -> pfgeom::basic::point::Point2DI32 {
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
    pub path_count: usize,
    pub fill_count: usize,
    pub alpha_tile_count: usize,
    pub solid_tile_count: usize,
}

impl Add<RenderStats> for RenderStats {
    type Output = RenderStats;
    fn add(self, other: RenderStats) -> RenderStats {
        RenderStats {
            path_count: self.path_count + other.path_count,
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
            path_count: self.path_count / divisor,
            solid_tile_count: self.solid_tile_count / divisor,
            alpha_tile_count: self.alpha_tile_count / divisor,
            fill_count: self.fill_count / divisor,
        }
    }
}

struct RenderTimers
{
    stage_0: Vec<D::TimerQuery>,
    stage_1: Option<D::TimerQuery>,
}

impl RenderTimers
{
    fn new() -> RenderTimers {
        RenderTimers {
            stage_0: vec![],
            stage_1: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RenderTime {
    pub stage_0: Duration,
    pub stage_1: Duration,
}

impl Default for RenderTime {
    #[inline]
    fn default() -> RenderTime {
        RenderTime {
            stage_0: Duration::new(0, 0),
            stage_1: Duration::new(0, 0),
        }
    }
}

impl Add<RenderTime> for RenderTime {
    type Output = RenderTime;

    #[inline]
    fn add(self, other: RenderTime) -> RenderTime {
        RenderTime {
            stage_0: self.stage_0 + other.stage_0,
            stage_1: self.stage_1 + other.stage_1,
        }
    }
}
