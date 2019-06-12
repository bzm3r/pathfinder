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

use hal::{Surface, Device, Swapchain};
use pathfinder_geometry as pfgeom;
use takeable_option::Takeable;

pub struct SwapchainState {
    swapchain_image_format: hal::format::Format,
    swapchain_images: Vec<<Backend as hal::Backend>::Image>,
    swapchain_image_views: Vec<<Backend as hal::Backend>::ImageView>,
    swapchain_framebuffers: Vec<<Backend as hal::Backend>::Framebuffer>,
    swapchain: <Backend as hal::Backend>::Swapchain,
    in_flight_fences: Vec<<Backend as hal::Backend>::Fence>,
    draw_pipeline_layout_state: PipelineLayoutState,
    postprocess_pipeline_layout_state: PipelineLayoutState,
    tile_solid_multicolor_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    tile_solid_monochrome_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    tile_alpha_multicolor_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    tile_alpha_monochrome_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    stencil_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    postprocess_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    // submission_command_buffers: Vec<hal::command::CommandBuffer<back::Backend, hal::Graphics, _, hal::command::Primary, back::command::CommandBuffer>>,
}

impl SwapchainState {
    unsafe fn new(
        adapter: &mut hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        window: &winit::Window,
        surface: &mut <Backend as hal::Backend>::Surface,
        resource_loader: &dyn crate::resources::ResourceLoader,
        command_pool: &mut hal::CommandPool<back::Backend, hal::Graphics>,
        draw_render_pass_description: crate::render_pass::RenderPassDescription,
        postprocess_render_pass_description: crate::render_pass::RenderPassDescription,
        draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        postprocess_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        tile_solid_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
        tile_solid_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
        tile_alpha_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
        tile_alpha_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
        stencil_pipeline_description: crate::pipeline::PipelineDescription,
        postprocess_pipeline_description: crate::pipeline::PipelineDescription,
    ) -> SwapchainState
    {
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

        let swapchain_config = hal::window::SwapchainConfig::from_caps(&capabilities, swapchain_image_format, extent);

        let (swapchain, swapchain_images) = device
            .create_swapchain(surface, swapchain_config, None)
            .unwrap();

        let swapchain_image_views: Vec<<Backend as hal::Backend>::ImageView> =
            swapchain_images
                .iter()
                .map(|i| device
                    .create_image_view(
                        i,
                        hal::image::ViewKind::D2,
                        swapchain_image_format,
                        hal::format::Swizzle::NO,
                        hal::image::SubresourceRange {
                            aspects: hal::format::Aspects::COLOR,
                            levels: 0..1,
                            layers: 0..1,
                        },
                    ).unwrap()
                )
                .collect();

        let max_frames_in_flight = swapchain_images.len();

        let crate::render_pass::RenderPassDescription {attachments: mut attachments, subpass_colors: subpass_colors, subpass_inputs: subpass_inputs} = draw_render_pass_description;
        let hal::pass::Attachment{samples: samples, ops: ops, stencil_ops: stencil_ops, layouts: layouts, ..} = attachments.pop().unwrap();
        let attachments = vec![hal::pass::Attachment{
            format: Some(swapchain_image_format),
            samples,
            ops,
            stencil_ops,
            layouts,
        }];

        let draw_render_pass_description = crate::render_pass::RenderPassDescription {
            attachments,
            subpass_colors,
            subpass_inputs,
        };

        let draw_render_pass = crate::render_pass::create_render_pass(device, draw_render_pass_description);
        let draw_pipeline_layout_state = PipelineLayoutState::new(device, draw_descriptor_set_layout_bindings, draw_render_pass);

        let mut swapchain_framebuffers: Vec<<Backend as hal::Backend>::Framebuffer> =
            swapchain_image_views
                .iter()
                .map(|iv| device.create_framebuffer(&draw_render_pass, vec![iv], hal::image::Extent { width: extent.width, height: extent.height, depth: 1 }).unwrap())
                .collect();

//        let submission_command_buffers: Vec<_> = swapchain_framebuffers
//            .iter()
//            .map(|_| command_pool.acquire_command_buffer())
//            .collect();

        let tile_solid_multicolor_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resource_loader, tile_solid_multicolor_pipeline_description);
        let tile_solid_monochrome_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resource_loader, tile_solid_monochrome_pipeline_description);
        let tile_alpha_multicolor_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resource_loader, tile_alpha_multicolor_pipeline_description);
        let tile_alpha_monochrome_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resource_loader, tile_alpha_monochrome_pipeline_description);
        let stencil_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resource_loader, stencil_pipeline_description);

        let postprocess_render_pass = crate::render_pass::create_render_pass(device, postprocess_render_pass_description);
        let postprocess_pipeline_layout_state = PipelineLayoutState::new(device, postprocess_descriptor_set_layout_bindings, postprocess_render_pass);
        let postprocess_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resource_loader, postprocess_pipeline_description);

        let in_flight_fences: Vec<<Backend as hal::Backend>::Fence> = (0..max_frames_in_flight).map(|_| device.create_fence(true).unwrap()).collect();

        SwapchainState {
            swapchain_image_format,
            swapchain_images,
            swapchain_image_views,
            swapchain_framebuffers,
            swapchain,
            in_flight_fences,
            draw_pipeline_layout_state,
            postprocess_pipeline_layout_state,
            tile_solid_multicolor_pipeline,
            tile_solid_monochrome_pipeline,
            tile_alpha_multicolor_pipeline,
            tile_alpha_monochrome_pipeline,
            stencil_pipeline,
            postprocess_pipeline,
            //submission_command_buffers,
        }
    }

    unsafe fn destroy_swapchain_state(device: &<Backend as hal::Backend>::Device, command_pool: &hal::CommandPool<back::Backend, hal::Graphics>, swapchain_state: SwapchainState) {
        let SwapchainState {
            in_flight_fences,
            swapchain_image_format,
            swapchain_images,
            swapchain_image_views,
            swapchain_framebuffers,
            swapchain,
            draw_pipeline_layout_state,
            tile_solid_multicolor_pipeline,
            tile_solid_monochrome_pipeline,
            tile_alpha_multicolor_pipeline,
            tile_alpha_monochrome_pipeline,
            stencil_pipeline,
            postprocess_pipeline_layout_state,
            postprocess_pipeline,
            //submission_command_buffers
        } = swapchain_state;

        for f in in_flight_fences.into_iter() {
            device.destroy_fence(f);
        }

        for iv in swapchain_image_views.into_iter() {
            device.destroy_image_view(iv);
        }

        for i in swapchain_images.into_iter() {
            device.destroy_image(i);
        }

        for fb in swapchain_framebuffers.into_iter() {
            device.destroy_framebuffer(fb)
        }

        for pl in vec![tile_solid_multicolor_pipeline, tile_solid_monochrome_pipeline, tile_alpha_multicolor_pipeline, tile_alpha_monochrome_pipeline, stencil_pipeline, postprocess_pipeline].into_iter() {
            device.destroy_graphics_pipeline(pl);
        }

        for pl_s in vec![draw_pipeline_layout_state, postprocess_pipeline_layout_state].into_iter() {
            PipelineLayoutState::destroy_pipeline_layout_state(device, pl_s);
        }
    }
}

pub struct DrawPipelineState<'a> {
    adapter: &'a mut hal::Adapter<Backend>,
    device: &'a <Backend as hal::Backend>::Device,
    window: &'a winit::Window,
    surface: &'a mut <Backend as hal::Backend>::Surface,
    resource_loader: &'a dyn crate::resources::ResourceLoader,
    draw_render_pass_description: crate::render_pass::RenderPassDescription,
    postprocess_render_pass_description: crate::render_pass::RenderPassDescription,
    draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
    postprocess_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
    tile_solid_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
    tile_solid_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
    tile_alpha_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
    tile_alpha_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
    stencil_pipeline_description: crate::pipeline::PipelineDescription,
    postprocess_pipeline_description: crate::pipeline::PipelineDescription,
    swapchain_state: Takeable<SwapchainState>,
    quad_vertex_positions_buffer_pool: crate::VertexBufferPool<'a>,
    tile_solid_vertex_buffer_pool: crate::VertexBufferPool<'a>,
    tile_alpha_vertex_buffer_pool: crate::VertexBufferPool<'a>,
    stencil_vertex_buffer_pool: crate::VertexBufferPool<'a>,
    fill_pipeline_state: FillPipelineState<'a>,
    monochrome: bool,
    command_queue: &'a hal::CommandQueue<back::Backend, hal::Graphics>,
    command_pool: hal::CommandPool<back::Backend, hal::Graphics>,
    current_frame_index: usize,
}

impl<'a> DrawPipelineState<'a> {
    pub unsafe fn new(
        adapter: &'a mut hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        surface: &'a mut <Backend as hal::Backend>::Surface,
        window: &'a winit::Window,
        resource_loader: &'a dyn crate::resources::ResourceLoader,
        command_queue: &'a hal::CommandQueue<back::Backend, hal::Graphics>,
        command_pool: hal::CommandPool<back::Backend, hal::Graphics>,
        max_quad_vertex_positions_buffer_size: u64,
        draw_render_pass_description: crate::render_pass::RenderPassDescription,
        fill_render_pass_description: crate::render_pass::RenderPassDescription,
        postprocess_render_pass_description: crate::render_pass::RenderPassDescription,
        fill_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        postprocess_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        fill_pipeline_description: crate::pipeline::PipelineDescription,
        tile_solid_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
        tile_solid_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
        tile_alpha_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
        tile_alpha_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
        stencil_pipeline_description: crate::pipeline::PipelineDescription,
        postprocess_pipeline_description: crate::pipeline::PipelineDescription,
        fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
        max_fill_vertex_buffer_size: u64,
        max_tile_vertex_buffer_size: u64,
        monochrome: bool,
    ) -> DrawPipelineState<'a> {
        let current_frame_index: usize = 0;

        let mut command_pool = command_pool;

        let swapchain_state = SwapchainState::new(adapter,
                                            device,
                                            window,
                                            surface,
                                            resource_loader,
                                            &mut command_pool,
                                            draw_render_pass_description,
                                            postprocess_render_pass_description,
                                            draw_descriptor_set_layout_bindings,
                                            postprocess_descriptor_set_layout_bindings,
                                            tile_solid_multicolor_pipeline_description,
                                            tile_solid_monochrome_pipeline_description,
                                            tile_alpha_multicolor_pipeline_description,
                                            tile_alpha_monochrome_pipeline_description,
                                            stencil_pipeline_description,
                                            postprocess_pipeline_description);

        let quad_vertex_positions_buffer_pool= crate::VertexBufferPool::new(adapter, device, max_quad_vertex_positions_buffer_size, 1);
        let fill_pipeline_state = FillPipelineState::new(adapter, device, resource_loader, command_queue, &command_pool, &quad_vertex_positions_buffer_pool, fill_render_pass_description, fill_descriptor_set_layout_bindings, fill_pipeline_description, fill_framebuffer_size, max_fill_vertex_buffer_size, swapchain_state.in_flight_fences.len() as u8);

        let tile_solid_vertex_buffer_pool = crate::VertexBufferPool::new(adapter, device, max_tile_vertex_buffer_size, swapchain_state.in_flight_fences.len() as u8);
        let tile_alpha_vertex_buffer_pool = crate::VertexBufferPool::new(adapter, device, max_tile_vertex_buffer_size, swapchain_state.in_flight_fences.len() as u8);
        let stencil_vertex_buffer_pool = crate::VertexBufferPool::new(adapter, device, quad_vertex_positions_buffer_pool.buffer_size, swapchain_state.in_flight_fences.len() as u8);

        DrawPipelineState {
            adapter,
            device,
            window,
            surface,
            resource_loader,
            draw_render_pass_description,
            postprocess_render_pass_description,
            draw_descriptor_set_layout_bindings,
            postprocess_descriptor_set_layout_bindings,
            tile_solid_multicolor_pipeline_description,
            tile_solid_monochrome_pipeline_description,
            tile_alpha_multicolor_pipeline_description,
            tile_alpha_monochrome_pipeline_description,
            stencil_pipeline_description,
            postprocess_pipeline_description,
            swapchain_state: Takeable::new(swapchain_state),
            quad_vertex_positions_buffer_pool,
            tile_solid_vertex_buffer_pool,
            tile_alpha_vertex_buffer_pool,
            stencil_vertex_buffer_pool,
            fill_pipeline_state,
            monochrome,
            command_queue,
            command_pool,
            current_frame_index,
        }
    }

    pub unsafe fn get_framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.swapchain_state.swapchain_framebuffers[self.current_frame_index]
    }

    pub fn request_free_frame_index(&mut self) -> Option<usize> {
        self.device.wait_for_fences(self.swapchain_state.in_flight_fences.iter(), hal::device::WaitFor::Any, core::u64::MAX);

        for (i, f) in self.swapchain_state.in_flight_fences.iter().enumerate() {
            if self.device.get_fence_status(f).unwrap() {
                return Some(i);
            }
        }

        None
    }

    unsafe fn recreate_swapchain(&mut self) {
        match Takeable::try_take(&mut self.swapchain_state) {
            Some(ss) => {
                SwapchainState::destroy_swapchain_state(self.device, &self.command_pool, ss);
            },
            _ => {},
        }

        self.swapchain_state = Takeable::new(SwapchainState::new(self.adapter,
                                                                 self.device,
                                                                 self.window,
                                                                 &mut self.surface,
                                                                 self.resource_loader,
                                                                 &mut self.command_pool,
                                                                 self.draw_render_pass_description,
                                                                 self.postprocess_render_pass_description,
                                                                 self.draw_descriptor_set_layout_bindings,
                                                                 self.postprocess_descriptor_set_layout_bindings,
                                                                 self.tile_solid_multicolor_pipeline_description,
                                                                 self.tile_solid_monochrome_pipeline_description,
                                                                 self.tile_alpha_multicolor_pipeline_description,
                                                                 self.tile_alpha_monochrome_pipeline_description,
                                                                 self.stencil_pipeline_description,
                                                                 self.postprocess_pipeline_description,))
    }

    pub unsafe fn present(&mut self, command_queue: &mut <Backend as hal::Backend>::CommandQueue) -> Result<Option<hal::window::Suboptimal>, hal::window::PresentError>  {
        self.current_frame_index = self.request_free_frame_index().unwrap();

        let (image_index, _) = self.swapchain_state.swapchain.acquire_image(core::u64::MAX, None, Some(&self.swapchain_state.in_flight_fences[self.current_frame_index])).unwrap();

        let present_result = self.swapchain_state.swapchain.present_nosemaphores(command_queue, image_index);

        match  present_result {
            Ok(Some(_)) => {
                self.recreate_swapchain();
            },
            _ => { }
        }

        present_result
    }

    pub unsafe fn destroy_draw_pipeline_state(device: &<Backend as hal::Backend>::Device, draw_pipeline_state: DrawPipelineState) {
        unimplemented!()
    }
}

pub struct FillPipelineState<'a> {
    device: &'a <Backend as hal::Backend>::Device,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pipeline_layout_state: PipelineLayoutState,
    command_queue: &'a hal::CommandQueue<back::Backend, hal::Graphics>,
    command_pool: &'a hal::CommandPool<back::Backend, hal::Graphics>,
    quad_vertex_positions_buffer_pool: &'a crate::VertexBufferPool<'a>,
    framebuffer: crate::Framebuffer,
    fill_vertex_buffer_pool: crate::VertexBufferPool<'a>,
    fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
}

impl<'a> FillPipelineState<'a> {
    pub unsafe fn new(adapter: &hal::Adapter<Backend>,
                      device: &'a <Backend as hal::Backend>::Device,
                      resource_loader: &dyn crate::resources::ResourceLoader,
                      command_queue: &'a hal::CommandQueue<back::Backend, hal::Graphics>,
                      command_pool: &'a hal::CommandPool<back::Backend, hal::Graphics>,
                      quad_vertex_positions_buffer_pool: &'a crate::VertexBufferPool,
                      fill_render_pass_description: crate::render_pass::RenderPassDescription,
                      fill_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
                      fill_pipeline_description: crate::pipeline::PipelineDescription,
                      fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
                      max_fill_vertex_buffer_size: u64,
                      fill_vertex_buffer_pool_size: u8) -> FillPipelineState<'a>
    {
        let fill_render_pass = crate::render_pass::create_render_pass(&device, fill_render_pass_description);

        let pipeline_layout_state = PipelineLayoutState::new(&device, fill_descriptor_set_layout_bindings, fill_render_pass);

        let framebuffer = crate::Framebuffer::new(adapter, device, hal::format::Format::R16Sfloat, fill_framebuffer_size, pipeline_layout_state.render_pass());

        let fill_vertex_buffer_pool = crate::VertexBufferPool::new(adapter, device, max_fill_vertex_buffer_size, fill_vertex_buffer_pool_size);

        let pipeline = crate::pipeline::create_pipeline(device,
                                                        &pipeline_layout_state,
                                                        resource_loader,
                                                        fill_pipeline_description);

        FillPipelineState {
            device,
            pipeline,
            pipeline_layout_state,
            command_queue,
            command_pool,
            quad_vertex_positions_buffer_pool,
            framebuffer,
            fill_vertex_buffer_pool,
            fill_framebuffer_size,
        }
    }

    fn framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.framebuffer.framebuffer()
    }

    fn pipeline(&self) -> &<Backend as hal::Backend>::GraphicsPipeline {
        &self.pipeline
    }

    pub unsafe fn upload_vertex_buffer_data<T>(&mut self, data: &[crate::batch_primitives::FillBatchPrimitive], vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32, fence: Option<&<Backend as hal::Backend>::Fence>) {
        self.fill_vertex_buffer_pool.submit_data_to_buffer(data, first_vertex..vertex_count, first_instance..instance_count, fence);
    }

    pub unsafe fn submit_fill_draws(&mut self) {
        let mut cmd_buffer = self.command_pool.acquire_command_buffer::<hal::command::OneShot>();

        cmd_buffer.begin();

        cmd_buffer.bind_graphics_pipeline(self.pipeline());
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, self.pipeline_layout_state.descriptor_sets(), &[]);

        cmd_buffer.begin_render_pass(self.pipeline_layout_state.render_pass(),
                                     self.framebuffer(),
                                     hal::pso::Rect {
                                         x: 0,
                                         y: 0,
                                         w: self.fill_framebuffer_size.x() as i16,
                                         h: self.fill_framebuffer_size.y() as i16,
                                     },
                                     &[]);

        // TODO: quad vertex positions buffer pool
        for (vertex_count, instance_count, buf) in self.fill_vertex_buffer_pool.submission_list.iter() {
            cmd_buffer.bind_vertex_buffer(0, [(buf.buffer(), 0)]);
            cmd_buffer.draw(vertex_count, instance_count);
        }

        cmd_buffer.end_render_pass();
        cmd_buffer.finish();

        let submission = hal::queue::Submission {
            command_buffers: [&cmd_buffer],
            wait_semaphores: None,
            signal_semaphores: None,
        };

        self.command_queue.submit(submission, None);

    }


    pub unsafe fn destroy_fill_pipeline_state(device: &<Backend as hal::Backend>::Device, fill_pipeline_state: FillPipelineState) {
        let FillPipelineState { fill_vertex_buffer_pool: fvb, framebuffer: fb, pipeline: pl, pipeline_layout_state: pls, .. } = fill_pipeline_state;
        crate::Framebuffer::destroy_framebuffer(device, fb);
        device.destroy_graphics_pipeline(pl);
        PipelineLayoutState::destroy_pipeline_layout_state(device, pls);
        crate::VertexBufferPool::destroy_vertex_buffer_pool(device, fvb);
    }
}

pub struct PipelineLayoutState {
    descriptor_set_layout: <Backend as hal::Backend>::DescriptorSetLayout,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    render_pass: Option<<Backend as hal::Backend>::RenderPass>,
}

impl PipelineLayoutState {
    pub fn new(device: &<Backend as hal::Backend>::Device, descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>, render_pass: <Backend as hal::Backend>::RenderPass) -> PipelineLayoutState {
        let immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let descriptor_set_layout  = device.create_descriptor_set_layout(descriptor_set_layout_bindings, immutable_samplers).unwrap();

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let pipeline_layout = unsafe {
            device
                .create_pipeline_layout([&descriptor_set_layout], push_constants)
                .unwrap()
        };

        PipelineLayoutState {
            pipeline_layout,
            descriptor_set_layout,
            render_pass,
        }
    }

    pub fn pipeline_layout(&self) -> &<Backend as hal::Backend>::PipelineLayout {
        &self.pipeline_layout
    }

    pub fn render_pass(&self) -> &<Backend as hal::Backend>::RenderPass {
        &self.render_pass
    }

    fn descriptor_set_layout(&self) -> &<Backend as hal::Backend>::DescriptorSetLayout {
        &self.descriptor_set_layout
    }

    unsafe fn destroy_pipeline_layout_state(device: &<Backend as hal::Backend>::Device, pl_state: PipelineLayoutState){
        let PipelineLayoutState { descriptor_set_layout: dsl, render_pass: rp, pipeline_layout: pl} = pl_state;

        device.destroy_pipeline_layout(pl);
        device.destroy_render_pass(rp);
        device.destroy_descriptor_set_layout(dsl);
    }
}

