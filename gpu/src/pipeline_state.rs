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
use takeable_option::Takeable;

pub struct DrawPipelineState<'a> {
    adapter: &'a mut hal::Adapter<Backend>,
    device: &'a <Backend as hal::Backend>::Device,
    window: &'a winit::Window,
    max_frames_in_flight: usize,
    surface: <Backend as hal::Backend>::Surface,
    swapchain_image_format: hal::format::Format,
    swapchain_images: Takeable<Vec<<Backend as hal::Backend>::Image>>,
    swapchain_image_views: Takeable<Vec<<Backend as hal::Backend>::ImageView>>,
    swapchain_framebuffers: Takeable<Vec<<Backend as hal::Backend>::Framebuffer>>,
    swapchain: Takeable<<Backend as hal::Backend>::Swapchain>,
    in_flight_fences: Takeable<<Backend as hal::Backend>::Fence>,
    draw_pipeline_render_pass_desc: crate::render_pass::RenderPassDesc,
    draw_pipeline_layout_state: Takeable<PipelineLayoutState>,
    postprocess_pipeline_layout_state: Takeable<PipelineLayoutState>,
    solid_tile_multicolor_pipeline: Takeable<<Backend as hal::Backend>::Pipeline>,
    solid_tile_monochrome_pipeline: Takeable<<Backend as hal::Backend>::Pipeline>,
    alpha_tile_multicolor_pipeline: Takeable<<Backend as hal::Backend>::Pipeline>,
    alpha_tile_monochrome_pipeline: Takeable<<Backend as hal::Backend>::Pipeline>,
    stencil_pipeline: Takeable<<Backend as hal::Backend>::Pipeline>,
    postprocess_pipeline: Takeable<<Backend as hal::Backend>::Pipeline>,
    quad_positions_vertex_buffer: &'a crate::Buffer<'a>,
    solid_tile_vertex_buffer: VertexBufferPool,
    alpha_tile_vertex_buffer: VertexBufferPool,
    stencil_vertex_buffer: VertexBufferPool,
    fill_pipeline_state: FillPipelineState<'a>,
    monochrome: bool,
    command_queue: <Backend as hal::Backend>::CommandQueue,
    command_pool: <Backend as hal::Backend>::CommandPool,
    current_frame_index: usize,
}

impl<'a> DrawPipelineState<'a> {
    pub unsafe fn new<'a>(
        adapter: &'a mut hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        surface: &mut <Backend as hal::Backend>::Surface,
        window: &'a winit::Window,
        command_pool: <Backend as hal::Backend>::CommandPool,
        draw_render_pass_desc: crate::render_pass::RenderPassDesc,
        fill_render_pass_desc: crate::render_pass::RenderPassDesc,
        postprocess_render_pass_desc: crate::render_pass::RenderPassDesc,
        fill_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        postprocess_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
    ) -> DrawPipelineState {
        let current_frame_index: usize = 0;
        
        let fill_pipeline_state = FillPipelineState::new(adapter, device, resources, &command_queue, &command_pool, &quad_vertex_positions_buffer, fill_framebuffer_size, max_fill_vertex_buffer_size, &current_frame_index);

        let (swapchain_image_format,
            swapchain_images,
            swapchain_image_views,
            swapchain_framebuffers,
            swapchain,
            draw_pipeline_layout_state,
            solid_tile_multicolor_pipeline,
            solid_tile_monochrome_pipeline,
            alpha_tile_multicolor_pipeline,
            alpha_tile_monochrome_pipeline,
            stencil_pipeline,
            postprocess_pipeline_layout_state,
            postprocess_pipeline,
            submission_command_buffers
        ) = DrawPipelineState::create_state(adapter,
                                                device,
                                                surface,
                                                window,
                                                command_pool,
                                                draw_render_pass_desc);

        DrawPipelineState {
            adapter,
            device,
            window,
            max_frames_in_flight,
            surface,
            swapchain_image_format,
            swapchain_images,
            swapchain_image_views,
            swapchain_framebuffers,
            swapchain,
            in_flight_fences,
            draw_pipeline_render_pass_desc,
            draw_pipeline_layout_state,
            postprocess_pipeline_layout_state,
            solid_tile_multicolor_pipeline,
            solid_tile_monochrome_pipeline,
            alpha_tile_multicolor_pipeline,
            alpha_tile_monochrome_pipeline,
            stencil_pipeline,
            postprocess_pipeline,
            quad_positions_vertex_buffer,
            solid_tile_vertex_buffer,
            alpha_tile_vertex_buffer,
            stencil_vertex_buffer,
            fill_pipeline_state,
            monochrome,
            command_pool,
            command_queue,
            submission_command_buffers,
            current_frame_index: usize,
        }
    }

    pub unsafe fn get_framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.swapchain_framebuffers[self.current_index]
    }

    pub fn request_free_frame_index(&mut self) -> Option<usize> {
        self.device.wait_for_fences(self.in_flight_fences.iter(), hal::device::WaitFor::Any, core::u64::max);

        for (i, f) in self.in_flight_fences.iter().enumerate() {
            if self.device.get_fence_status(f).unwrap() {
                return Some(i);
            }
        }

        None
    }

    pub unsafe fn create_state(
        adapter: &mut hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        surface: &mut <Backend as hal::Backend>::Surface,
        command_pool: &<Backend as hal::Backend>::CommanPool,
        draw_render_pass_desc: crate::render_pass::RenderPassDesc,
        draw_dsl_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        post_dsl_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>)
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

        let swapchain_config = hal::window::SwapchainConfig::from_caps(&capabilities, draw_image_format, extent);

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
                    )
                )
                .collect();

        let max_frames_in_flight = swapchain_images.len();

        let mut swapchain_framebuffers: Vec<<Backend as hal::Backend>::Framebuffer> =
            swapchain_image_views
                .iter()
                .map(|iv| device.create_framebuffer(iv, draw_render_pass, vec![iv], extent))
                .collect();

        let in_flight_fences: Vec<<Backend as hal::Backend>::Fence> = (0..max_frames_in_flight).into_iter().map(|_| device.create_fence().unwrap()).collect();

        let submission_command_buffers: Vec<_> = swapchain_framebuffers
            .iter()
            .map(|_| command_pool.acquire_command_buffer())
            .collect();

        let RenderPassDesc {attachments: mut attachments, subpass_colors: subpoass_colors, subpass_inputs: subpass_inputs} = draw_render_pass_desc;
        let hal::pass::Attachment{samples: samples, ops: ops, stencil_ops: stencil_ops, layouts: layouts, ..} = attachments.pop().unwrap();
        let attachments = vec![hal::pass::Attachment{
            format: Some(swapchain_image_format),
            samples,
            ops,
            stencil_ops,
            layouts,
        }];

        let draw_render_pass_desc = crate::render_pass::RenderPassDesc {
            attachments,
            subpass_colors,
            subpass_inputs,
        };

        let render_pass = DrawPipelineState::create_render_pass(device, draw_render_pass_desc);
        let draw_pipeline_layout_state = PipelineLayoutState::new(device, descriptor_set_layout_bindings, &render_pass);

        let solid_tile_multicolor_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resources, solid_tile_multicolor_pipeline_description);
        let solid_tile_monochrome_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resources, solid_tile_monochrome_pipeline_description);
        let alpha_tile_multicolor_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resources, alpha_tile_multicolor_pipeline_description);
        let alpha_tile_monochrome_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resources, alpha_tile_monochrome_pipeline_description);
        let stencil_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resources, stencil_pipeline_description);
        let postprocess_pipeline = crate::pipeline::create_pipeline(device, &draw_pipeline_layout_state, resources, postprocess_pipeline_description);
    }

    pub unsafe fn present(&mut self, command_queue: &mut <Backend as hal::Backend>::CommandQueue) -> Result<Option<hal::window::Suboptimal>, hal::window::PresentError>  {
        self.current_frame_index = self.request_free_frame_index();

        let (image_index, _) = swapchain.acquire_image(core::u64::MAX, None, Some(&self.in_flight_fences[current_frame_index])).unwrap();

        let present_result = self.swapchain.present_no_semaphores(command_queue, image_index);

        match  present_result {
            Ok(Some(_)) => {
                self.recreate_swapchain();
            },
            _ => { }
        }

        present_result
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

    pub unsafe fn destroy_draw_pipeline_state(device: &<Backend as hal::Backend>::Device, draw_pipeline_state: DrawPipelineState) {
        let DrawPipelineState { swapchain_framebuffers: sfbs, swapchain: sc, image_available_semaphores: ias, render_finished_semaphores: rfs} = swapchain_state;

        for s in ias.into_iter() {
            device.destroy_semaphore(s);
        }

        for s in rfs.into_iter() {
            device.destroy_semaphore(s);
        }

        for fb in sfbs.into_iter() {
            crate::Framebuffer::destroy_framebuffer(device, fb);
        }

        device.destroy_swapchain(sc);
    }
}

pub struct FillPipelineState<'a> {
    device: &'a <Backend as hal::Backend>::Backend,
    pipeline: <Backend as hal::Backend>::Pipeline,
    pipeline_layout_state: PipelineLayoutState,
    command_queue: &'a <Backend as hal::Backend>::CommandQueue,
    command_pool: &'a <Backend as hal::Backend>::CommandPool,
    quad_positions_vertex_buffer: &'a crate::Buffer<'a>,
    framebuffer: Framebuffer,
    fill_vertex_buffer_pool: VertexBufferPool<'a>,
    mask_framebuffer_size: pfgeom::basic::point::Point2DI32,
}

impl<'a> FillPipelineState<'a> {
    unsafe fn new(adapter: &hal::Adapter<Backend>,
                  device: &'a <Backend as hal::Backend>::Device,
                  resources: &dyn resources::ResourceLoader,
                  command_queue: &'a <Backend as hal::Backend>::CommandQueue,
                  command_pool: &'a <Backend as hal::Backend>::CommandPool,
                  quad_vertex_positions_buffer: &'a crate::Buffer,
                  mask_framebuffer_size: pfgeom::basic::point::Point2DI32,
                  max_fill_vertex_buffer_size: u64,
                  current_frame_index: &'a usize) -> FillPipelineState<'a>
    {
        let fill_render_pass = create_render_pass(&device, crate::render_pass::create_fill_render_pass_desc());
        let pipeline_layout_state = PipelineLayoutState::new(&device, fill_descriptor_set_layout_bindings, fill_render_pass);

        let framebuffer = Framebuffer::new(adapter, device, hal::format::Format::R16Sfloat, mask_framebuffer_size, pipeline_layout_state.render_pass());

        let fill_vertex_buffer = VertexBufferPool::new(adapter, device, max_fill_vertex_buffer_size, fill_vertex_buffer_pool_size, hal::buffer::Usage::VERTEX, current_frame_index, in_flight_fill_fences);

        let pipeline = pipeline::create_fill_pipeline(device, pipeline_layout_state.pipeline_layout(), resources, mask_framebuffer_size);

        FillPipelineState {
            device,
            pipeline,
            pipeline_layout_state,
            command_queue,
            command_pool,
            quad_positions_vertex_buffer,
            framebuffer,
            fill_vertex_buffer_pool,
            mask_framebuffer_size,
        }
    }

    fn framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.framebuffer.unwrap().framebuffer()
    }

    fn pipeline(&self) -> &<Backend as hal::Backend>::Pipeline {
        &self.pipeline
    }

    pub unsafe fn upload_vertex_buffer_data<T>(&mut self, data: &[crate::gpu_data::FillBatchPrimitive], vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        self.fill_vertex_buffer_pool.submit_data_to_buffer(data, first_vertex..vertex_count, first_instance..instance_count);
    }

    pub unsafe fn submit_fill_draws(&mut self) {
        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::Oneshot>();
        let fence = self.device.create_fence();

        cmd_buffer.begin();

        cmdbuffer.bind_graphics_pipeline(self.pipeline());
        cmd_buffer.bind_graphics_descriptor_sets(self.pipeline_layout_state.pipeline_layout(), 0, Some(self.pipeline_layout_state.descriptor_set_layout()), &[]);

        cmd_buffer.begin_render_pass(self.pipeline_layout_state.render_pass(),
                                     self.framebuffer(),
                                     hal::pso::Rect {
                                         x: 0,
                                         y: 0,
                                         w: self.mask_framebuffer_size.x() as i16,
                                         h: self.mask_framebuffer_size.y() as i16,
                                     },
                                     &[]);

        for (vertex_count, instance_count, buf) in self.fill_vertex_buffer_pool.submission_list.iter() {
            cmd_buffer.bind_vertex_buffer(0, [(buf.buffer(), 0)]);
            cmd_buffer.draw(vertex_count, instance_count);
            fences.push(buf.fence());
        }

        cmd_buffer.end_render_pass();
        cmd_buffer.finish();

        let submission = hal::queue::Submission {
            command_buffers: [&cmd_buffer],
            wait_semaphores: None,
            signal_semaphores: None,
        };

        self.command_queue.submit(submission, fences);

    }


    pub unsafe fn destroy_fill_pipeline_state(device: &<Backend as hal::Backend>::Device, fill_renderer: FillPipelineState) {
        let FillPipelineState { fill_vertex_buffer_pool: fvb, .. } = fill_renderer;

        for f in [cf, ff] {
            device.destroy_fence(f);
        }

        device.destroy_semaphore(fs);

        RawBufferPool::destroy_buffer(device, idb);
        RawBufferPool::destroy_buffer(device, fvb);
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

