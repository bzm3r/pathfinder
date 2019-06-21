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
use crate::gpu::pipeline_descriptions::create_fill_pipeline_description;

static QUAD_VERTEX_POSITIONS: [u8; 8] = [0, 0, 1, 0, 1, 1, 0, 1];
static QUAD_VERTEX_INDICES: [u32; 6] = [0, 1, 3, 1, 2, 3];

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

pub struct Renderer<'a> {
    pub gpu_state: pfgpu::GpuState<'a>,

    area_lut_texture: pfgpu::Image,
    gamma_lut_texture: pfgpu::Image,

    // Rendering state
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
        let mut gpu_state = pfgpu::GpuState::new(window, resource_laoder, "renderer", fill_render_pass_description, draw_render_pass_description, postprocess_render_pass_description, fill_descriptor_set_layout_bindings, draw_descriptor_set_layout_bindings, postprocess_descriptor_set_layout_bindings, fill_pipeline_description, tile_solid_monochrome_pipeline_description, tile_solid_multicolor_pipeline_description, tile_alpha_monochrome_pipeline_description, tile_alpha_multicolor_pipeline_description, postprocess_pipeline_description, stencil_pipeline_description, fill_framebuffer_size, max_quad_vertex_positions_buffer_size, max_fill_vertex_buffer_size, max_tile_vertex_buffer_size, monochrome);

        let area_lut_texture = gpu_state.create_texture_from_png(resources, "area-lut");
        let gamma_lut_texture = gpu_state.create_texture_from_png(resources, "gamma-lut");

        let quad_vertex_positions_buffer = device.create_vertex_buffer(QUAD_VERTEX_POSITIONS.len() as u64);
        device.upload_data(quad_vertex_positions_buffer, &QUAD_VERTEX_POSITIONS);

        Renderer {
            gpu_state,
            area_lut_texture,
            gamma_lut_texture,
            buffered_fills: vec![],
            buffered_alpha_tiles: vec![],
            buffered_solid_tiles: vec![],
            use_depth: false,
        }
    }

    pub unsafe fn begin_scene(&mut self) {
        // initialize postprocessing framebuffer
        // clear postprocessing framebuffer
    }

    unsafe fn init_postprocessing_framebuffer(&mut self) {
        // if postprocessing is needed, create relevant framebuffer/support

        let source_framebuffer_size = self.draw_viewport().size();

        let clear_params = pfgpu::ClearParams {
            color: Some(pfgeom::basic::ColorF::transparent_black()),
            ..pfgpu::ClearParams::default()
        };


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
