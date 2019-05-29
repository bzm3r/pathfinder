// pathfinder/gpu/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Minimal abstractions over GPU device capabilities.
#[cfg(feature = "dx12")]
extern crate gfx_backend_dx12 as back;
#[cfg(feature = "metal")]
extern crate gfx_backend_metal as back;
#[cfg(feature = "vulkan")]
extern crate gfx_backend_vulkan as back;

use back::Backend;

extern crate gfx_hal as hal;
extern crate log;
extern crate shaderc;
extern crate winit;

use hal::{Instance, Surface, Capability, Device, QueueFamily, PhysicalDevice, };
use crate::resources::ResourceLoader;
use image as img_crate;
use pathfinder_geometry as pfgeom;
use pathfinder_simd as pfsimd;
use rustache::HashBuilder;
use rustache::Render;

pub mod resources;
pub mod pipeline_layout_descs;
pub mod pipelines;
pub mod render_pass_descs;

struct IndirectDrawData {
    vertex_count: u32,
    instance_count: u32,
    first_vertex: u32,
    first_instance: u32,
}

pub trait FillData {
}

pub trait AlphaTileData {
}

pub trait SolidTileData {
}

pub struct FillRenderer<'a> {
    device: &'a <Backend as hal::Backend>::Backend,
    pipeline: <Backend as hal::Backend>::Pipeline,
    pipeline_layout_state: &'a PipelineLayoutState,
    command_queue: &'a <Backend as hal::Backend>::CommandQueue,
    quad_positions_vertex_buffer: &'a Buffer,
    framebuffer: &'a Framebuffer,
    cleared: bool,
    clear_fence: <Backend as hal::Backend>::Fence,
    clear_image_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    fill_fence: <Backend as hal::Backend>::Fence,
    fill_semaphore: <Backend as hal::Backend>::Semaphore,
    fill_vertex_buffer: Buffer,
    indirect_draw_buffer: Buffer,
    fill_command_buffer: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
}

impl<'a> FillRenderer<'a> {
    unsafe fn new(adapter: &hal::Adapter<Backend>,
                  device: &'a <Backend as hal::Backend>::Device,
                  pipeline_layout_state: &'a PipelineLayoutState,
                  resources: &dyn resources::ResourceLoader,
                  command_queue: &<Backend as hal::Backend>::CommandQueue,
                  command_pool: &<Backend as hal::Backend>::CommandPool,
                  quad_vertex_positions_buffer: &'a Buffer,
                  mask_framebuffer_size: pfgeom::basic::point::Point2DI32,
                  max_fill_vertex_buffer_size: u64) -> FillRenderer<'a>
    {
        let framebuffer = Framebuffer::new(adapter, device, hal::format::Format::R16Sfloat, mask_framebuffer_size, pipeline_layout_state.render_pass());

        let cleared = false;
        let clear_fence = device.create_fence().unwrap();
        clear_fence.reset().unwrap();

        let clear_image_command = FillRenderer::create_clear_image_command_buffer(device, command_pool, clear_params);

        let fill_fence = device.create_fence().unwrap();
        fill_fence.reset_fence().unwrap();
        let fill_semaphore = device.create_semaphore().unwrap();

        let fill_vertex_buffer = Buffer::new(adapter, device, max_fill_vertex_buffer_size, hal::buffer::Usage::VERTEX);
        let indirect_draw_buffer = Buffer::new(adapter, device, std::mem::size_of::<IndirectDrawData>() as u64, hal::buffer::Usage::INDIRECT);

        let fill_command_buffer = FillRenderer::create_fill_command_buffer(device, command_pool, viewport);

        let pipeline = pipelines::create_fill_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, mask_framebuffer_size);

        FillRenderer {
            device,
            pipeline,
            pipeline_layout_state,
            command_queue,
            quad_positions_vertex_buffer,
            framebuffer,
            cleared,
            clear_fence,
            clear_image_command,
            fill_fence,
            fill_semaphore,
            indirect_draw_buffer,
            fill_vertex_buffer,
            fill_command_buffer,
        }
    }

    fn framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.framebuffer.unwrap().framebuffer()
    }

    fn create_clear_image_command_buffer(&self, command_pool: &<Backend as hal::Backend>::CommandPool, clear_params: ClearParams) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        let clear_color: [f32;4] = {
            match clear_params.color {
                Some(color) => color.to_rgba_array(),
                _ => {
                    let color = pfgeom::color::ColorF::transparent_black();
                    color.to_rgba_array()
                }
            }
        };

        let depth_stencil: hal::command::ClearDepthStencil = {
            let depth = clear_params.depth.unwrap_or(0.0);
            let stencil = clear_params.stencil.unwrap_or(0) as u32;

            hal::command::ClearDepthStencil(depth, stencil)
        };

        cmd_buffer.clear_image(self.framebuffer.image(), hal::image::Layout::ColorAttachmentOptimal, clear_color, depth_stencil);

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn create_fill_command_buffer(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.bind_graphics_pipeline(self.pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.indirect_draw_buffer.buffer()), (self.fill_vertex_buffer.buffer(), 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.render_pass,
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw_indirect(&self.indirect_command_buffer(), 0, 1, 0);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn submit_clear_image_command(&self, render_complete_semaphore: &<Backend as hal::Backend>::Semaphore) {
        let submission = hal::queue::Submission {
            command_buffers: [&self.clear_image_command],
            wait_semaphores: [(render_complete_semaphore, hal::pso::PipelineStage::COLOR_ATTACHMENT_OUTPUT)],
            signal_semaphores: [&self.clear_complete_semaphore],
        };
        self.command_queue.submit(submission, &self.clear_complete_fence);
    }

    unsafe fn upload_to_fill_vertex_buffer<T>(&self, data: &[T], indirect_draw_data: IndirectDrawData) where T: FillData {
        self.indirect_draw_buffer.upload_data(self.device, indirect_draw_data);
        self.fill_vertex_buffer.upload_data(self.device, data);
    }

    unsafe fn submit_fill_command(&self) {
        self.indirect_draw_buffer.upload_data(self.device, indirect_draw_command);

        let submission = hal::queue::Submission {
            command_buffers: [&self.fill_command_buffer],
            wait_semaphores: [(&self.clear_complete_semaphore, hal::pso::PiplineStage::COLOR_ATTACHMENT_OUTPUT)],
            signal_semaphores: [&self.fill_drawing_complete_semaphore],
        };
        self.command_queue.submit(submission, &self.fill_drawing_complete_fence);
    }

    pub unsafe fn draw_fills<T>(&self, data: &[T], vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) where T: FillData {
        let indirect_draw_data = IndirectDrawData {
            vertex_count,
            instance_count,
            first_vertex,
            first_instance,
        };

        self.upload_to_fill_vertex_buffer(data, indirect_draw_data);
        self.submit_fill_command();
    }

    pub fn reset_viewport(&self, viewport: hal::pso::Viewport) {
        self.fill_max_command.set_viewport(0, &[viewport.clone()]);
    }

    pub unsafe fn destroy_fill_renderer(device: &<Backend as hal::Backend>::Device, fill_renderer: FillRenderer) {
        let FillRenderer { clear_fence: cf, fill_fence: ff, fill_semaphore: fs, indirect_draw_buffer: idb, fill_vertex_buffer: fvb, .. } = fill_renderer;

        for f in [cf, ff] {
            device.destroy_fence(f);
        }

        device.destroy_semaphore(fs);

        Buffer::destroy_buffer(device, idb);
        Buffer::destroy_buffer(device, fvb);
    }
}

pub struct DrawRenderer<'a> {
    device: &'a <Backend as hal::Backend>::Backend,
    solid_tile_multicolor_pipeline: <Backend as hal::Backend>::Pipeline,
    solid_tile_monochrome_pipeline: <Backend as hal::Backend>::Pipeline,
    alpha_tile_multicolor_pipeline: <Backend as hal::Backend>::Pipeline,
    alpha_tile_monochrome_pipeline: <Backend as hal::Backend>::Pipeline,
    stencil_pipeline: <Backend as hal::Backend>::Pipeline,
    postprocess_pipeline: <Backend as hal::Backend>::Pipeline,
    pipeline_layout_state: &'a PipelineLayoutState,
    command_queue: &'a <Backend as hal::Backend>::CommandQueue,
    quad_positions_vertex_buffer: &'a Buffer,
    swapchain_state: SwapchainState,
    draw_solid_tiles_monochrome_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    draw_solid_tiles_multicolor_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    draw_alpha_tiles_monochrome_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    draw_alpha_tiles_multicolor_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    stencil_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    postprocess_command: hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>,
    solid_tile_indirect_draw_buffer: Buffer,
    alpha_tile_indirect_draw_buffer: Buffer,
    solid_tile_vertex_buffer: Buffer,
    alpha_tile_vertex_buffer: Buffer,
    stencil_vertex_buffer: Buffer,
    monochrome: bool,
}

impl<'a> DrawRenderer<'a> {
    unsafe fn new(adapter: &hal::Adapter<Backend>,
                  device: &'a <Backend as hal::Backend>::Device,
                  pipeline_layout_state: &'a PipelineLayoutState,
                  resources: &dyn resources::ResourceLoader,
                  extent: hal::window::Extent2D,
                  command_queue: &<Backend as hal::Backend>::CommandQueue,
                  command_pool: &<Backend as hal::Backend>::CommandPool,
                  frame_buffer: &'a Framebuffer,
                  quad_vertex_positions_buffer: &'a Buffer,
                  max_tile_buffer_size: u64,
                  monochrome: bool) -> DrawRenderer<'a>
    {
        let solid_tile_multicolor_pipeline = pipelines::create_solid_tile_multicolor_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, extent);
        let solid_tile_monochrome_pipeline = pipelines::create_solid_tile_monochrome_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, extent);
        let alpha_tile_multicolor_pipeline = pipelines::create_alpha_tile_multicolor_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, extent);
        let alpha_tile_monochrome_pipeline = pipelines::create_solid_tile_monochrome_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, extent);
        let stencil_pipeline = pipelines::create_stencil_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, extent);
        let postprocess_pipeline = pipelines::create_postprocess_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, extent);
        
        DrawRenderer {
            device,
            solid_tile_multicolor_pipeline,
            solid_tile_monochrome_pipeline,
            alpha_tile_multicolor_pipeline,
            alpha_tile_monochrome_pipeline,
            stencil_pipeline,
            postprocess_pipeline,
            pipeline_layout_state,
            command_queue,
            quad_positions_vertex_buffer,
            swapchain_state,
            draw_solid_tiles_monochrome_command,
            draw_solid_tiles_multicolor_command,
            draw_alpha_tiles_monochrome_command,
            draw_alpha_tiles_multicolor_command,
            stencil_command,
            postprocess_command,
            solid_tile_indirect_draw_buffer,
            alpha_tile_indirect_draw_buffer,
            solid_tile_vertex_buffer,
            alpha_tile_vertex_buffer,
            stencil_vertex_buffer,
            monochrome,
        }
    }

    unsafe fn framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        self.swapchain_state.get_framebuffer()
    }

    unsafe fn create_solid_tile_monochrome_command(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.set_viewports(0, &[viewport.clone()]);
        cmd_buffer.set_scissors(0, &[viewport.rect]);
        cmd_buffer.bind_graphics_pipeline(self.solid_tile_monochrome_pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.solid_tile_indirect_draw_buffer.buffer()), (self.solid_tile_vertex_buffer.buffer(), 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.pipeline_layout_state.render_pass(),
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw_indirect(&self.indirect_command_buffer(), 0, 1, 0);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn create_solid_tile_multicolor_command(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.set_viewports(0, &[viewport.clone()]);
        cmd_buffer.set_scissors(0, &[viewport.rect]);
        cmd_buffer.bind_graphics_pipeline(self.solid_tile_multicolor_pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.solid_tile_indirect_draw_buffer.buffer()), (self.solid_tile_vertex_buffer.buffer(), 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.pipeline_layout_state.render_pass(),
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw_indirect(&self.indirect_command_buffer(), 0, 1, 0);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn create_alpha_tile_monochrome_command(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.set_viewports(0, &[viewport.clone()]);
        cmd_buffer.set_scissors(0, &[viewport.rect]);
        cmd_buffer.bind_graphics_pipeline(self.alpha_tile_monochrome_pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.alpha_tile_indirect_draw_buffer.buffer()), (self.alpha_tile_vertex_buffer.buffer(), 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.pipeline_layout_state.render_pass(),
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw_indirect(&self.indirect_command_buffer(), 0, 1, 0);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn create_alpha_tile_multicolor_command(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.set_viewports(0, &[viewport.clone()]);
        cmd_buffer.set_scissors(0, &[viewport.rect]);
        cmd_buffer.bind_graphics_pipeline(self.alpha_tile_multicolor_pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.alpha_tile_indirect_draw_buffer.buffer()), (self.alpha_tile_vertex_buffer.buffer(), 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.pipeline_layout_state.render_pass(),
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw_indirect(&self.indirect_command_buffer(), 0, 1, 0);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn create_stencil_command(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.set_viewports(0, &[viewport.clone()]);
        cmd_buffer.set_scissors(0, &[viewport.rect]);
        cmd_buffer.bind_graphics_pipeline(self.stencil_pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.stencil_vertex_buffer.buffer(), 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.pipeline_layout_state.render_pass(),
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw(0..4, 0..1);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn postprocess_pipeline(&self, command_pool: &<Backend as hal::Backend>::CommandPool, viewport: hal::pso::Viewport) -> hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::MultiShot>();

        let allow_pending_resubmit = false;
        cmd_buffer.begin(allow_pending_resubmit);

        cmd_buffer.set_viewports(0, &[viewport.clone()]);
        cmd_buffer.set_scissors(0, &[viewport.rect]);
        cmd_buffer.bind_graphics_pipeline(self.postprocess_pipeline);
        cmd_buffer.bind_vertex_buffers(0, [(self.quad_positions_vertex_buffer, 0)]);
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        {
            let mut encoder = cmd_buffer.begin_render_pass_inline(
                self.pipeline_layout_state.render_pass(),
                self.framebuffer(),
                viewport.rect,
                &[],
            );
            encoder.draw(0..4, 0..1);
        }

        cmd_buffer.finish();

        cmd_buffer
    }

    unsafe fn upload_to_solid_tile_vertex_buffer<T>(&self, data: &[T], indirect_draw_data: IndirectDrawData) where T: SolidTileData {
        self.indirect_draw_buffer.upload_data(self.device, indirect_draw_data);
        self.solid_tile_vertex_buffer.upload_data(self.device, data);
    }

    unsafe fn upload_to_alpha_tile_vertex_buffer<T>(&self, data: &[T], indirect_draw_data: IndirectDrawData) where T: AlphaTileData {
        self.indirect_draw_buffer.upload_data(self.device, indirect_draw_data);
        self.alpha_tile_vertex_buffer.upload_data(self.device, data);
    }

    unsafe fn upload_to_stencil_vertex_buffer(&self, data: &[T])  {
        self.indirect_draw_buffer.upload_data(self.device, indirect_draw_data);
        self.alpha_tile_vertex_buffer.upload_data(self.device, data);
    }

    unsafe fn draw_solid_tile_command(&self) -> &hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        if self.monochrome {
            &self.draw_solid_tiles_monochrome_command
        } else {
            &self.draw_solid_tiles_multicolor_command
        }
    }

    unsafe fn draw_alpha_tile_command(&self) -> &hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary> {
        if self.monochrome {
            &self.draw_alpha_tiles_monochrome_command
        } else {
            &self.draw_alpha_tiles_multicolor_command
        }
    }

    unsafe fn submit_solid_tile_command(&self) {
        let submission = hal::queue::Submission {
            command_buffers: [self.draw_solid_tile_command()],
            wait_semaphores: None,
            signal_semaphores: None,
        };
        self.command_queue.submit(submission, &self.solid_tile_render_finished_fence);
    }

    unsafe fn submit_alpha_tile_command(&self) {
        let submission = hal::queue::Submission {
            command_buffers: [self.draw_alpha_tile_command()],
            wait_semaphores: None,
            signal_semaphores: None,
        };

        self.command_queue.submit(submission, &self.alpha_tile_render_finished_fence);
    }

    pub unsafe fn draw_solid_tiles<T>(&self, data: &[T], vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) where T: SolidTileData {
        let indirect_draw_data = IndirectDrawData {
            vertex_count,
            instance_count,
            first_vertex,
            first_instance,
        };

        self.upload_to_solid_tile_vertex_buffer(data, indirect_draw_data);
        self.submit_solid_tile_command();
    }

    pub unsafe fn draw_alpha_tiles<T>(&self, data: &[T], vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) where T: SolidTileData {
        let indirect_draw_data = IndirectDrawData {
            vertex_count,
            instance_count,
            first_vertex,
            first_instance,
        };

        self.upload_to_solid_tile_vertex_buffer(data, indirect_draw_data);
        self.submit_solid_tile_command();
    }

    pub fn reset_viewport(&self, viewport: hal::pso::Viewport) {
        self.draw_solid_tiles_monochrome_command.set_viewport(0, &[viewport.clone()]);
        self.draw_solid_tiles_multicolor_command.set_viewport(0, &[viewport.clone()]);
        self.draw_alpha_tiles_monochrome_command.set_viewport(0, &[viewport.clone()]);
        self.draw_alpha_tiles_multicolor_command.set_viewport(0, &[viewport.clone()]);
        self.stencil_command.set_viewport(0, &[viewport.clone()]);
        self.postprocess_command.set_viewport(0, &[viewport.clone()]);
    }

    pub unsafe fn destroy_draw_renderer(device: &<Backend as hal::Backend>::Device, draw_renderer: DrawRenderer) {
        let FillRenderer { clear_fence: cf, fill_fence: ff, fill_semaphore: fs, indirect_draw_buffer: idb, fill_vertex_buffer: fvb, .. } = fill_renderer;

        for f in [cf, ff] {
            device.destroy_fence(f);
        }

        device.destroy_semaphore(fs);

        Buffer::destroy_buffer(device, idb);
        Buffer::destroy_buffer(device, fvb);
    }
}

pub struct SwapchainState {
    max_frames_in_flight: usize,
    swapchain_image_format: hal::format::Format,
    swapchain_framebuffers: Vec<Framebuffer>,
    swapchain: <Backend as hal::Backend>::Swapchain,
    image_available_semaphores: Vec<<Backend as hal::Backend>::Semaphore>,
    render_finished_semaphores: Vec<<Backend as hal::Backend>::Semaphore>,
    submission_command_buffers: Vec<hal::command::CommandBuffer<Backend, hal::Graphics, hal::command::MultiShot, hal::command::Primary>>,
    current_index: usize,
}

impl SwapchainState {
    pub unsafe fn new(adapter: &mut hal::Adapter<Backend>, device: &<Backend as hal::Backend>::Device, surface: &mut <Backend as hal::Backend>::Surface, draw_render_pass: &<Backend as hal::Backend>::RenerPass, max_frames_in_flight: usize, command_pool: &<Backend as hal::Backend>::CommandPool) -> SwapchainState {
        let (capabilities, compatible_formats, _compatible_present_modes) =
            surface.compatibility(&mut adapter.physical_device);

        let swapchain_image_format = match compatible_formats {
            None => hal::format::Format::Rgba8Srgb,
            Some(formats) => match formats
                .iter()
                .find(|format| format.base_format().1 == hal::format::ChannelType::Srgb)
                .cloned()
                {
                    Some(srgb_format) => srgb_format,
                    None => formats
                        .get(0)
                        .cloned()
                        .ok_or("Surface does not support any known format.")
                        .unwrap(),
                },
        };

        let extent = {
            let window_client_area = window
                .get_inner_size()
                .unwrap()
                .to_physical(window.get_hidpi_factor());

            hal::window::Extent2D {
                width: capabilities.extents.end.width.min(window_client_area.width as u32),
                height: capabilities
                    .extents
                    .end
                    .height
                    .min(window_client_area.height as u32),
            }
        };

        let swapchain_config = hal::window::SwapchainConfig::from_caps(&capabilities, draw_image_format, extent);

        let (swapchain, swapchain_images) = device
            .create_swapchain(surface, swapchain_config, previous_swapchain)
            .unwrap();


        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) =
            PfDevice::create_synchronizers(&device, max_frames_in_flight);

        let mut swapchain_framebuffers: Vec<Framebuffer> = Vec::new();

        for image_view in swapchain_image_views.iter() {
            swapchain_framebuffers.push(Framebuffer::new(adapter, device, swapchain_format, pfgeom::basic::point::Point2DI32::new(extent.width as i32, extent.height as i32), draw_render_pass));
        }

        let image_available_semaphores: Vec<<Backend as hal::Backend>::Semaphore> = (0..max_frames_in_flight).into_iter().map(|_| device.create_semaphore().unwrap()).collect();
        let render_finished_semaphores: Vec<<Backend as hal::Backend>::Semaphore> = (0..max_frames_in_flight).into_iter().map(|_| device.create_semaphore().unwrap()).collect();

        let submission_command_buffers: Vec<_> = swapchain_framebuffers
            .iter()
            .map(|_| command_pool.acquire_command_buffer())
            .collect();

        SwapchainState {
            max_frames_in_flight,
            swapchain_image_format,
            swapchain_framebuffers,
            swapchain,
            image_available_semaphores,
            render_finished_semaphores,
            submission_command_buffers,
            current_index: 0,
        }
    }

    pub unsafe fn get_framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        self.swapchain_framebuffers[self.current_index].framebuffer()
    }

    pub unsafe fn recreate_swapchain(&self) {
        unimplemented!();
    }

    pub unsafe fn destroy_swapchain_state(device: &<Backend as hal::Backend>::Device, swapchain_state: SwapchainState) {
        let SwapchainState { swapchain_framebuffers: sfbs, swapchain: sc, image_available_semaphores: ias, render_finished_semaphores: rfs} = swapchain_state;

        for s in ias.into_iter() {
            device.destroy_semaphore(s);
        }

        for s in rfs.into_iter() {
            device.destroy_semaphore(s);
        }

        for fb in sfbs.into_iter() {
            Framebuffer::destroy_framebuffer(device, fb);
        }

        device.destroy_swapchain(sc);
    }
}

pub struct PfDevice<'a> {
    instance: back::Instance,
    surface: <Backend as hal::Backend>::Surface,
    pub device: <Backend as hal::Backend>::Device,
    adapter: hal::Adapter<Backend>,
    queue_group: hal::queue::QueueGroup<Backend, hal::Graphics>,
    pub extent: hal::window::Extent2D,

    command_pool: hal::CommandPool<Backend, hal::Graphics>,

    fill_pipeline_layout: PipelineLayoutState,
    draw_pipeline_layout: PipelineLayoutState,
    postprocess_pipeline_layout: PipelineLayoutState,

    fill_renderer: FillRenderer<'a>,
    tile_renderer: TileRenderer<'a>,
}

impl<'a> PfDevice<'a> {
    pub unsafe fn new(window: &winit::Window, 
                      instance_name: &str, 
                      fill_render_pass_desc: RenderPassDesc, 
                      draw_render_pass_desc: RenderPassDesc, 
                      postprocess_render_pass_desc: RenderPassDesc, 
                      fill_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>, 
                      draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>, 
                      postprocess_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>
                      mask_framebuffer_size: pfgeom::basic::point::Point2DI32,
                      max_quad_vertex_positions_buffer_size: usize,
                      max_fill_vertex_buffer_size: usize,
                      max_solid_tile_vertex_buffer_size: usize,
                      mask_alpha_tile_vertex_buffer_size: usize) -> PfDevice {
        let instance = back::Instance::create(instance_name, 1);

        let mut surface = instance.create_surface(window);

        let mut adapter = PfDevice::pick_adapter(&instance, &surface).unwrap();

        let (device, queue_group) =
            PfDevice::create_device_with_graphics_queues(&mut adapter, &surface);

        let fill_render_pass = PfDevice::create_render_pass(&device, fill_render_pass_desc);
        let draw_render_pass = PfDevice::create_render_pass(&device, draw_render_pass_desc);
        let postprocess_render_pass = PfDevice::create_render_pass(&device, postprocess_render_pass_desc);

        let max_frames_in_flight = 1;
        let swapchain_state= SwapchainState::new(&mut adapter, &device, &surface, &draw_render_pass, max_frames_in_flight, &command_pool);
        let in_flight_fences = (0..max_frames_in_flight).into_iter().map(|_| device.create_fence().unwrap()).collect();
        
        let fill_pipeline_layout = PipelineLayoutState::new(&device, fill_descriptor_set_layout_bindings, fill_render_pass);
        let draw_pipeline_layout = PipelineLayoutState::new(&device, draw_descriptor_set_layout_bindings, draw_render_pass);
        let postprocess_pipeline_layout = PipelineLayoutState::new(&device, postprocess_descriptor_set_layout_bindings, postprocess_render_pass);

        let mask_framebuffer = Framebuffer::new(&adapter, &device, hal::format::Format::R16Sfloat, mask_framebuffer_size, &draw_render_pass);

        let mut command_pool = device
            .create_command_pool_typed(
                &queue_group,
                hal::pool::CommandPoolCreateFlags::RESET_INDIVIDUAL,
            )
            .unwrap();

        adapter: &hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        pipeline: &'a <Backend as hal::Backend>::Pipeline,
        pipeline_layout_state: &'a PipelineLayoutState,
        command_queue: &<Backend as hal::Backend>::CommandQueue,
        command_pool: &<Backend as hal::Backend>::CommandPool,
        frame_buffer: &'a Framebuffer,
        quad_vertex_positions_buffer: &'a Buffer,
        max_fill_vertex_buffer_size: u64

        pf_device: &crate::PfDevice,
        pipeline_layout: pipeline_layouts::MaskPipelineLayout,
        resources: &dyn pf_resources::ResourceLoader,
        extent: hal::window::Extent2D
        let fill_renderer = FillRenderer::new(&adapter, &device, pipelines::create_fill_pipeline());

        PfDevice {
            instance,
            surface,
            device,
            adapter,
            queue_group,
            extent,

            command_pool,

            fill_pipeline_layout,
            draw_pipeline_layout,
            postprocess_pipeline_layout,

            fill_renderer,
            draw_renderer,
        }
    }

    pub unsafe fn create_render_pass(device: &<Backend as hal::Backend>::Device, render_pass_desc: RenderPassDesc) -> <Backend as hal::Backend>::RenderPass {
        let subpass = hal::pass::SubpassDesc {
            colors: &render_pass_desc.subpass_colors,
            inputs: &render_pass_desc.subpass_inputs,
            depth_stencil: None,
            resolves: &[],
            preserves: &[],
        };

        device.create_render_pass(&render_pass_desc.attachments, &[subpass], &[]).unwrap()
    }

    fn pick_adapter(
        instance: &back::Instance,
        surface: &<Backend as hal::Backend>::Surface,
    ) -> Result<hal::Adapter<Backend>, &'static str> {
        // pick appropriate physical device (physical_device)
        instance
            .enumerate_adapters()
            .into_iter()
            .find(|a| {
                a.queue_families
                    .iter()
                    .any(|qf| qf.supports_graphics() && surface.supports_queue_family(qf))
            })
            .ok_or("No physical device available with queue families which support graphics and presentation to surface.")
    }

    fn create_device_with_graphics_queues(
        adapter: &mut hal::Adapter<Backend>,
        surface: &<Backend as hal::Backend>::Surface,
    ) -> (
        <Backend as hal::Backend>::Device,
        hal::queue::QueueGroup<Backend, hal::Graphics>,
    ) {
        let family = adapter
            .queue_families
            .iter()
            .find(|family| {
                hal::Graphics::supported_by(family.queue_type())
                    && family.max_queues() > 0
                    && surface.supports_queue_family(family)
            })
            .expect("Could not find a queue family supporting graphics.");

        let priorities = vec![1.0; 1];
        let families = [(family, priorities.as_slice())];

        let hal::Gpu { device, mut queues } = unsafe {
            adapter
                .physical_device
                .open(&families, hal::Features::empty())
                .expect("Could not create device.")
        };

        let queue_group = queues
            .take::<hal::Graphics>(family.id())
            .expect("Could not take ownership of relevant queue group.");

        (device, queue_group)
    }

    unsafe fn create_swapchain(
        adapter: &mut hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        surface: &mut <Backend as hal::Backend>::Surface,
        previous_swapchain: Option<<Backend as hal::Backend>::Swapchain>,
        window: &winit::Window,
    ) -> (
        <Backend as hal::Backend>::Swapchain,
        hal::window::Extent2D,
        Vec<<Backend as hal::Backend>::Image>,
        hal::format::Format,
        usize,
    ) {
        let (capabilities, compatible_formats, _compatible_present_modes) =
            surface.compatibility(&mut adapter.physical_device);

        let draw_image_format = match compatible_formats {
            None => hal::format::Format::Rgba8Srgb,
            Some(formats) => match formats
                .iter()
                .find(|format| format.base_format().1 == hal::format::ChannelType::Srgb)
                .cloned()
            {
                Some(srgb_format) => srgb_format,
                None => formats
                    .get(0)
                    .cloned()
                    .ok_or("Surface does not support any known format.")
                    .unwrap(),
            },
        };

        let extent = {
            let window_client_area = window
                .get_inner_size()
                .unwrap()
                .to_physical(window.get_hidpi_factor());

            hal::window::Extent2D {
                width: capabilities.extents.end.width.min(window_client_area.width as u32),
                height: capabilities
                    .extents
                    .end
                    .height
                    .min(window_client_area.height as u32),
            }
        };

        let swapchain_config = hal::window::SwapchainConfig::from_caps(&capabilities, draw_image_format, extent);

        let (swapchain, draw_images) = device
            .create_swapchain(surface, swapchain_config, previous_swapchain)
            .unwrap();

        (swapchain, extent, draw_images, draw_image_format, (capabilities.image_count.end - 1) as usize)
    }

    pub fn extent(&self) -> pfgeom::basic::rect::RectI32 {
        let origin = pfgeom::basic::point::Point2DI32::default();
        let size = pfgeom::basic::point::Point2DI32::new(self.extent.width as i32, self.extent.height as i32);
        pfgeom::basic::rect::RectI32::new(origin, size)
    }

    fn create_synchronizers(
        device: &<Backend as hal::Backend>::Device,
        max_frames_in_flight: usize,
    ) -> (
        Vec<<Backend as hal::Backend>::Semaphore>,
        Vec<<Backend as hal::Backend>::Semaphore>,
        Vec<<Backend as hal::Backend>::Fence>,
    ) {
        let mut image_available_semaphores: Vec<<Backend as hal::Backend>::Semaphore> = Vec::new();
        let mut render_finished_semaphores: Vec<<Backend as hal::Backend>::Semaphore> = Vec::new();
        let mut in_flight_fences: Vec<<Backend as hal::Backend>::Fence> = Vec::new();

        for _ in 0..max_frames_in_flight {
            image_available_semaphores.push(device.create_semaphore().unwrap());
            render_finished_semaphores.push(device.create_semaphore().unwrap());
            in_flight_fences.push(device.create_fence(true).unwrap());
        }

        (
            image_available_semaphores,
            render_finished_semaphores,
            in_flight_fences,
        )
    }

    unsafe fn create_image_views(
        device: &<Backend as hal::Backend>::Device,
        images: &Vec<<Backend as hal::Backend>::Image>,
        requested_format: hal::format::Format,
    ) -> Vec<<Backend as hal::Backend>::ImageView> {
        images
            .into_iter()
            .map(|image| {
                device
                    .create_image_view(
                        &image,
                        hal::img_crate::ViewKind::D2,
                        requested_format,
                        hal::format::Swizzle::NO,
                        hal::img_crate::SubresourceRange {
                            aspects: hal::format::Aspects::COLOR,
                            levels: 0..1,
                            layers: 0..1,
                        },
                    )
                    .unwrap()
            })
            .collect()
    }

    pub unsafe fn create_framebuffer(&mut self,
        render_pass: &<Backend as hal::Backend>::RenderPass,
        image: Image,
    ) -> Framebuffer {
        let image_view = self.device
            .create_image_view(
                texture,
                hal::img_crate::ViewKind::D2,
                TextureFormat::to_hal_format(texture_format),
                hal::format::Swizzle::NO,
                hal::img_crate::SubresourceRange {
                    aspects: hal::format::Aspects::COLOR,
                    levels: 0..1,
                    layers: 0..1,
                },
            );

        let framebuffer = self
            .device
            .create_framebuffer(
                render_pass,
                vec![image_view],
                hal::img_crate::Extent {
                    width: texture_size.x() as u32,
                    height: texture_size.y() as u32,
                    depth: 1,
                },
            );

        Framebuffer {
            framebuffer,
            image_view,
            image,
        }
    }

    pub unsafe fn create_vertex_buffer(&self, size: u64) -> Buffer {
        Buffer::new(&self.adapter, &self.device, size, hal::buffer::Usage::VERTEX)
    }

    pub unsafe fn create_texture_from_png(&mut self, resources: &dyn ResourceLoader, name: &str)  -> Image {
        let data = resources.slurp(&format!("textures/{}.png", name)).unwrap();
        let image = img_crate::load_from_memory_with_format(&data, img_crate::ImageFormat::PNG).unwrap().to_luma();
        let pixel_size= std::mem::size_of::<img_crate::Luma<u8>>();
        let size = pfgeom::Point2DI32::new(image.width() as i32, image.height() as i32);

        img_crate::new_from_data(&self.adapter, &self.device, &mut self.command_pool, &mut self.queue_group.queues[0], size, pixel_size, &data)
    }

    pub unsafe fn upload_data_to_buffer<T>(&self, data: &[T]) where T: Copy {
        buffer.upload_data::<T>(&self.device, data);
    }
}

fn load_shader_include(resources: &dyn ResourceLoader, include_name: &str) -> String {
    let resource = resources
        .slurp(&format!("shaders/{}.inc.glsl", include_name))
        .unwrap();
    String::from_utf8_lossy(&resource).to_string()
}

#[derive(Clone, Copy, Debug)]
pub enum StencilFunc {
    Always,
    Equal,
    NotEqual,
}

#[derive(Clone, Copy, Debug)]
pub enum TextureFormat {
    R8,
    R16F,
    RGBA8,
}

impl TextureFormat {
    pub fn to_hal_format(texture_format: TextureFormat) -> hal::format::Format {
        match texture_format {
            TextureFormat::R8 => hal::format::Format::R8Uint,
            TextureFormat::R16F => hal::format::Format::R16Sfloat,
            TextureFormat::RGBA8 => hal::format::Format::Rgba8Srgb,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum VertexAttrType {
    F32,
    I16,
    I8,
    U16,
    U8,
}

#[derive(Clone, Copy, Debug)]
pub enum BufferData<'a, T> {
    Uninitialized(usize),
    Memory(&'a [T]),
}

#[derive(Clone, Copy, Debug)]
pub enum BufferTarget {
    Vertex,
    Index,
}

#[derive(Clone, Copy, Debug)]
pub enum BufferUploadMode {
    Static,
    Dynamic,
}

pub enum GlslStyle {
    Spirv,
    OpenGL,
}

#[derive(Clone, Copy)]
pub enum UniformData {
    Int(i32),
    Mat2(pfsimd::default::F32x4),
    Mat4([pfsimd::default::F32x4; 4]),
    Vec2(pfsimd::default::F32x4),
    Vec4(pfsimd::default::F32x4),
    TextureUnit(u32),
}

#[derive(Clone, Copy)]
pub enum Primitive {
    Triangles,
    TriangleFan,
    Lines,
}

#[derive(Clone, Copy, Default)]
pub struct ClearParams {
    pub color: Option<pfgeom::color::ColorF>,
    pub rect: Option<pfgeom::basic::RectI32>,
    pub depth: Option<f32>,
    pub stencil: Option<u8>,
}

#[derive(Clone, Copy, Debug)]
pub enum BlendState {
    Off,
    RGBOneAlphaOne,
    RGBOneAlphaOneMinusSrcAlpha,
    RGBSrcAlphaAlphaOneMinusSrcAlpha,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct DepthState {
    pub func: DepthFunc,
    pub write: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum DepthFunc {
    Less,
    Always,
}

impl Default for DepthFunc {
    #[inline]
    fn default() -> DepthFunc {
        DepthFunc::Less
    }
}

impl UniformData {
    #[inline]
    pub fn from_transform_3d(transform: &pfgeom::basic::Transform3DF32) -> UniformData {
        UniformData::Mat4([transform.c0, transform.c1, transform.c2, transform.c3])
    }
}

pub struct Buffer {
    usage: hal::buffer::Usage,
    buffer: <Backend as hal::Backend>::Buffer,
    memory: <Backend as hal::Backend>::Memory,
    requirements: hal::memory::Requirements,
}

impl Buffer {
    pub unsafe fn upload_data<T>(&self, device: <Backend as hal::Backend>::Device, data: &[T]) where T: Copy {
        // should we assert!(data.len() < self.requirements.size)?
        let mut writer = device
            .acquire_mapping_writer::<T>(&self.memory, 0..self.requirements.size)
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        device.release_mapping_writer(writer).unwrap();
    }

    unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        size: u64,
        usage: hal::buffer::Usage,
    ) -> Buffer {
        let mut buffer = device.create_buffer(size, usage)
            .unwrap();;

        let requirements = device.get_buffer_requirements(&buffer);

        let memory_type_id = adapter
            .physical_device
            .memory_properties()
            .memory_types
            .iter()
            .enumerate()
            .find(|&(id, memory_type)| {
                requirements.type_mask & (1 << id) != 0
                    && memory_type
                        .properties
                        .contains(hal::memory::Properties::CPU_VISIBLE)
            })
            .map(|(id, _)| hal::adapter::MemoryTypeId(id))
            .ok_or("PhysicalDevice cannot supply required memory.")
            .unwrap();;

        let memory = device
            .allocate_memory(memory_type_id, requirements.size)
            .unwrap();;

        device
            .bind_buffer_memory(&memory, 0, &mut buffer)
            .unwrap();;

        Buffer {
            usage,
            buffer,
            memory,
            requirements,
        }
    }

    pub fn buffer(&self) -> &<Backend as hal::Backend>::Buffer {
        &self.buffer
    }
    
    unsafe fn destroy_buffer(device: &<Backend as hal::Backend>::Device, buffer: Buffer){
        let Buffer { buffer: buff, memory: mem, .. } = buffer;
        device.destroy_buffer(buff);
        device.free_memory(mem);
    }
}

pub struct Image {
    image: <Backend as hal::Backend>::Image,
    requirements: hal::memory::Requirements,
    memory: <Backend as hal::Backend>::Memory,
    size: pfgeom::Point2DI32,
    format: hal::format::Format,
}

impl Image {
    unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        texture_format: hal::format::Format,
        size: pfgeom::Point2DI32,
    ) -> Image {
        // 3. Make an image with transfer_dst and SAMPLED usage
        let mut image = device
            .create_image(
                hal::img_crate::Kind::D2(size.x() as u32, size.y() as u32, 1, 0),
                1,
                texture_format,
                hal::img_crate::Tiling::Optimal,
                hal::img_crate::Usage::TRANSFER_DST | hal::img_crate::Usage::SAMPLED,
                hal::img_crate::ViewCapabilities::empty(),
            )
            .unwrap();;

        // 4. allocate memory for the image and bind it
        let requirements = device.get_image_requirements(&image);

        let upload_type = adapter
            .physical_device
            .memory_properties()
            .memory_types
            .iter()
            .enumerate()
            .position(|(id, mem_type)| {
                requirements.type_mask & (1 << id) != 0
                    && mem_type.properties.contains(hal::memory::Properties::DEVICE_LOCAL)
            })
            .unwrap()
            .into();

        let memory = device
            .allocate_memory(upload_type, requirements.size)
            .unwrap();;

        device
            .bind_image_memory(&memory, 0, &mut image)
            .unwrap();;

        Image {
            image,
            requirements,
            memory,
            size,
            format: texture_format,
        }
    }

    unsafe fn new_from_data(adapter: &hal::Adapter<Backend>, device: &<Backend as hal::Backend>::Device, command_pool: &mut hal::CommandPool<back::Backend, hal::Graphics>, command_queue: &mut hal::CommandQueue<back::Backend, hal::Graphics>, size: pfgeom::Point2DI32, texel_size: usize, data: &[u8]) -> Image {
        let texture = img_crate::new(adapter, device, TextureFormat::R8, size);

        let staging_buffer =
            Buffer::new(adapter, device, (size.x() * size.y()) as u64, hal::buffer::Usage::TRANSFER_SRC);

        let mut writer = device
            .acquire_mapping_writer::<u8>(&staging_buffer.memory, 0..staging_buffer.requirements.size)
            .unwrap();;
        writer[0..data.len()].copy_from_slice(data);
        device
            .release_mapping_writer(writer)
            .unwrap();;

        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::OneShot>();
        cmd_buffer.begin();

        // 7. Use a pipeline barrier to transition the image from empty/undefined
        //    to TRANSFER_WRITE/TransferDstOptimal
        let image_barrier = hal::memory::Barrier::Image {
            states: (hal::img_crate::Access::empty(), hal::img_crate::Layout::Undefined)
                ..(
                hal::img_crate::Access::TRANSFER_WRITE,
                hal::img_crate::Layout::TransferDstOptimal,
            ),
            target: &texture.image,
            families: None,
            range: hal::img_crate::SubresourceRange {
                aspects: hal::format::Aspects::COLOR,
                levels: 0..1,
                layers: 0..1,
            },
        };

        cmd_buffer.pipeline_barrier(
            hal::pso::PipelineStage::TOP_OF_PIPE..hal::pso::PipelineStage::TRANSFER,
            hal::memory::Dependencies::empty(),
            &[image_barrier],
        );

        let row_pitch = texel_size * (size.x() as usize);

        // 8. perform copy from staging buffer to image
        cmd_buffer.copy_buffer_to_image(
            &staging_buffer.buffer,
            &texture.image,
            hal::img_crate::Layout::TransferDstOptimal,
            &[hal::command::BufferImageCopy {
                buffer_offset: 0,
                buffer_width: (row_pitch / texel_size) as u32,
                buffer_height: size.y() as u32,
                image_layers: hal::img_crate::SubresourceLayers {
                    aspects: hal::format::Aspects::COLOR,
                    level: 0,
                    layers: 0..1,
                },
                image_offset: hal::img_crate::Offset { x: 0, y: 0, z: 0 },
                image_extent: hal::img_crate::Extent {
                    width: size.x() as u32,
                    height: size.y() as u32,
                    depth: 1,
                },
            }],
        );

        // 9. use pipeline barrier to transition the image to SHADER_READ access/
        //    ShaderReadOnlyOptimal layout
        let image_barrier = hal::memory::Barrier::Image {
            states: (
                hal::img_crate::Access::TRANSFER_WRITE,
                hal::img_crate::Layout::TransferDstOptimal,
            )
                ..(
                hal::img_crate::Access::SHADER_READ,
                hal::img_crate::Layout::ShaderReadOnlyOptimal,
            ),
            target: &texture.image,
            families: None,
            range: hal::img_crate::SubresourceRange {
                aspects: hal::format::Aspects::COLOR,
                levels: 0..1,
                layers: 0..1,
            },
        };

        cmd_buffer.pipeline_barrier(
            hal::pso::PipelineStage::TRANSFER..hal::pso::PipelineStage::FRAGMENT_SHADER,
            hal::memory::Dependencies::empty(),
            &[image_barrier],
        );

        // 10. Submit the cmd buffer to queue and wait for it
        cmd_buffer.finish();

        let upload_fence = device
            .create_fence(false)
            .unwrap();;

        command_queue.submit_nosemaphores(Some(&cmd_buffer), Some(&upload_fence));

        device
            .wait_for_fence(&upload_fence, core::u64::MAX)
            .unwrap();;

        device.destroy_fence(upload_fence);

        texture
    }

    unsafe fn destroy_image(device: &<Backend as hal::Backend>::Device, image: Image) {
        let Image { image: img, memory: mem, .. } = image;
        device.destroy_image(img);
        device.free_memory(mem);
    }

    pub fn size(&self) -> pfgeom::basic::point::Point2DI32 {
        &self.size
    }
}

pub struct Framebuffer {
    framebuffer: <Backend as hal::Backend>::Framebuffer,
    image: Image,
    image_view: <Backend as hal::Backend>::ImageView,
}

impl Framebuffer {
    pub unsafe fn new(adapter: &hal::Adapter<Backend>, device: &<Backend as hal::Backend>::Device, texture_format: hal::format::Format, size: pfgeom::basic::point::Point2DI32, render_pass: &<Backend as hal::Backend>::RenderPass) -> Framebuffer {
        let image = Image::new(adapter, device, texture_format, size);
        let framebuffer = device.create_framebuffer(image, render_pass);
        let subresource_range = hal::img_crate::SubresourceRange {
            aspects: hal::format::Aspects::COLOR,
            levels: 0..1,
            layers: 0..1,
        };
        let image_view = device.create_image_view(&image, hal::image::ViewKind::D2, texture_format, hal::format::Swizzle::NO, subresource_range);

        Framebuffer {
            framebuffer,
            image,
            image_view,
        }
    }
    pub fn image(&self) -> &Image {
        self.image.as_ref().unwrap()
    }

    pub fn framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        self.framebuffer.as_ref().unwrap()
    }

    pub unsafe fn destroy_framebuffer(device: &<Backend as hal::Backend>::Device, framebuffer: Framebuffer) {
        let Framebuffer { framebuffer: fb, image: img, image_view: imv} = framebuffer;
        device.destroy_image_view(imv);
        Image::destroy_image(device,img);
        device.destroy_framebuffeR(fb);
    }
}

pub enum RenderPassVariants {
    Mask,
    Draw,
    Postprocess,
}

pub enum PipelineVariant {
    Stencil,
    Fill,
    SolidMonochrome,
    SolidMulticolor,
    AlphaMonochrome,
    AlphaMulticolor,
    Postprocess,
}

pub struct RenderPassDesc {
    attachments: Vec<hal::pass::Attachment>,
    subpass_colors: Vec<hal::pass::AttachmentRef>,
    subpass_inputs: Vec<hal::pass::AttachmentRef>,
}

pub struct PipelineLayoutState {
    descriptor_set_layout: <Backend as hal::Backend>::DescriptorSetLayout,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    render_pass: <Backend as hal::Backend>::RenderPass,
}

impl PipelineLayoutState {
    pub fn new(device: &<Backend as hal::Backend>::Device, descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>, render_pass: <Backend as hal::Backend>::RenderPass) -> PipelineLayoutState {
        let immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let descriptor_set_layout:  = device.create_descriptor_set_layout(descriptor_set_layout_bindings, immutable_samplers).unwrap();

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout([&descriptor_set_layouts], push_constants)
                .unwrap()
        };

        PipelineLayoutState {
            pipeline_layout,
            descriptor_set_layout,
            render_pass,
        }
    }

    fn pipeline_layout(&self) -> &<Backend as hal::Backend>::PipelineLayout {
        &self.pipeline_layout
    }

    fn render_pass(&self) -> &<Backend as hal::Backend>::RenderPass {
        &self.render_pass
    }

    fn descriptor_set_layout(&self) -> &<Backend as hal::Backend>::DescriptorSetLayout {
        &self.descriptor_set_layout
    }

    unsafe fn destroy_pipeline_layout_state(device: &<Backend as hal::Backend>::Device, pl_state: PipelineLayoutState){
        let PipelineLayoutState { descriptor_set_layout: dsl, render_pass: rp, pipeline_layout: pl} = pl_state;

        device.destroy_pipeline_layout(pl);
        device.destroy_render_pass(rp);
        destroy.descriptor_set_layout(dsl);
    }
}
