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
use crate::gpu_data;
use crate::post;
use crate::pipelines;
use core::borrow::BorrowMut;

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

pub struct Renderer {
    pub device: pfgpu::PfDevice,

    mask_render_pass: pfgpu::RenderPass,
    postprocess_render_pass: pfgpu::RenderPass,

    fill_pipeline: pfgpu::Pipeline,
    solid_multicolor_pipeline: pfgpu::Pipeline,
    alpha_multicolor_pipeline: pfgpu::Pipeline,
    solid_monochrome_pipeline: pfgpu::Pipeline,
    alpha_monochrome_pipeline: pfgpu::Pipeline,
    solid_monochrome_tile_vertex_buffer: SolidTileVertexBuffer,
    alpha_monochrome_tile_vertex_buffer: AlphaTileVertexBuffer,
    solid_multicolor_tile_vertex_buffer: SolidTileVertexBuffer,
    alpha_multicolor_tile_vertex_buffer: AlphaTileVertexBuffer,

    area_lut_texture: pfgpu::Image,
    quad_vertex_positions_buffer: pfgpu::Buffer,
    fill_vertex_buffer: pfgpu::Buffer,
    mask_framebuffer: pfgpu::Framebuffer,
    fill_colors_texture: pfgpu::Image,

    // Postprocessing shader
    postprocess_source_framebuffer: Option<pfgpu::Framebuffer>,
    postprocess_pipeline: pfgpu::Pipeline,
    postprocess_vertex_buffer: pfgpu::Buffer,
    gamma_lut_texture: pfgpu::Image,

    // Stencil shader
    stencil_pipeline: pfgpu::Pipeline,
    stencil_vertex_buffer: pfgpu::Buffer,

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
        let mut device = pfgpu::PfDevice::new(window, instance_name, crate::render_passes::mask_render_pass_desc(), crate::render_passes::create_draw_pass_desc(), crate::render_passes::postprocess_pass_desc());

        let mask_render_pass = pfgpu::create_render_pass(crate::render_passes::create_mask_pass_desc());
        let postprocess_render_pass = pfgpu::create_render_pass(crate::render_passes::create_postprocess_pass_desc());

        let fill_pipeline = ;
        let solid_multicolor_pipeline = SolidMulticolorPipeline::new(&device, resources);
        let alpha_multicolor_pipeline = AlphaMulticolorPipeline::new(&device, resources);
        let solid_monochrome_pipeline = SolidTileMonochromePipeline::new(&device, resources);
        let alpha_monochrome_pipeline = AlphaTileMonochromePipeline::new(&device, resources);
        let postprocess_pipeline = PostprocessPipeline::new(&device, resources);
        let stencil_pipeline = StencilProgram::new(&device, resources);
        //let reprojection_pipeline = ReprojectionProgram::new(&device, resources);

        let area_lut_texture = device.create_texture_from_png(resources, "area-lut");
        let gamma_lut_texture = device.create_texture_from_png(resources, "gamma-lut");

        let quad_vertex_positions_buffer = device.create_vertex_buffer(QUAD_VERTEX_POSITIONS.len() as u64);
        device.upload_data(quad_vertex_positions_buffer, &QUAD_VERTEX_POSITIONS);

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
        let mask_framebuffer_texture = device.create_texture(pfgpu::TextureFormat::R16F, mask_framebuffer_size);
        let mask_framebuffer = device.create_framebuffer(mask_framebuffer_texture, &fill_pipeline.render_pass);

        let fill_colors_size =
            pfgeom::basic::point::Point2DI32::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT);
        let fill_colors_texture = device.create_texture(hal::format::Format::Rgba8Srgb, fill_colors_size);

        Renderer {
            device,

            mask_render_pass,
            postprocess_render_pass,

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

    pub unsafe fn begin_scene(&mut self) {
        self.init_postprocessing_framebuffer();

        self.mask_framebuffer_cleared = false;
    }

    unsafe fn init_postprocessing_framebuffer(&mut self) {
        if !self.postprocessing_needed() {
            self.postprocess_source_framebuffer = None;
            return;
        }

        let source_framebuffer_size = self.draw_viewport().size();
        match self.postprocess_source_framebuffer {
            Some(ref framebuffer)
            if framebuffer.image().size() == source_framebuffer_size => {}
            _ => {
                if self.postprocess_source_framebuffer.is_some() {
                    let existing_framebuffer = self.postprocess_source_framebuffer.take();
                    self.device.destroy_framebuffer(existing_framebuffer);
                }

                let postprocess_texture = self.device.create_texture(pfgpu::TextureFormat::R8, source_framebuffer_size);
                self.postprocess_source_framebuffer = Some(self.device.create_framebuffer(&self.postprocess_render_pass, postprocess_texture));
            }
        };

        let clear_params = pfgpu::ClearParams {
            color: Some(pfgeom::basic::ColorF::transparent_black()),
            ..pfgpu::ClearParams::default()
        };

        self.device.clear_image(self.postprocess_source_framebuffer.unwrap().image(), clear_params);
    }

    pub unsafe fn render_command(&mut self, command: &gpu_data::RenderCommand) {
        use gpu_data::RenderCommand;

        match *command {
            RenderCommand::Start { bounding_quad, path_count } => {
                if self.use_depth {
                    self.draw_stencil(&bounding_quad);
                }
            }
            RenderCommand::AddPaintData(ref paint_data) => self.upload_paint_data(paint_data),
            RenderCommand::AddFills(ref fills) => self.add_fills(fills),
            RenderCommand::FlushFills => {
                self.draw_buffered_fills();
            }
            RenderCommand::AddSolidTiles(ref solid_tiles) => self.add_solid_tiles(solid_tiles),
            RenderCommand::FlushSolidTiles => {
                self.draw_buffered_solid_tiles();
            }
            RenderCommand::AddAlphaTiles(ref alpha_tiles) => self.add_alpha_tiles(alpha_tiles),
            RenderCommand::FlushAlphaTiles => {
                self.draw_buffered_alpha_tiles();
            }
            RenderCommand::Finish { .. } => {}
        }
    }

    fn main_viewport(&self) -> pfgeom::basic::rect::RectI32 {
        self.device.extent()
    }

    fn draw_viewport(&self) -> pfgeom::basic::rect::RectI32 {
        let main_viewport = self.main_viewport();
        match self.render_mode {
            RenderMode::Monochrome {
                defringing_kernel: Some(..),
                ..
            } => {
                let scale = pfgeom::basic::point::Point2DI32::new(3, 1);
                let origin = pfgeom::basic::point::Point2DI32::default();
                let size = main_viewport.size().scale_xy(scale);
                pfgeom::basic::rect::RectI32::new(origin, size)
            }
            _ => main_viewport,
        }
    }

    unsafe fn draw_stencil(&self, quad_positions: &[pfgeom::basic::point::Point3DF32]) {
        self.stencil_vertex_buffer.upload_data(&self.device, quad_positions);
        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_array(&self.stencil_vertex_array.vertex_array);
        self.device.use_program(&self.stencil_program.program);
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
}

#[derive(Clone, Copy)]
pub enum RenderMode {
    Multicolor,
    Monochrome {
        fg_color: pfgeom::basic::color::ColorF,
        bg_color: pfgeom::basic::color::ColorF,
        defringing_kernel: Option<crate::post::DefringingKernel>,
        gamma_correction: bool,
    },
}

impl Default for RenderMode {
    #[inline]
    fn default() -> RenderMode {
        RenderMode::Multicolor
    }
}
