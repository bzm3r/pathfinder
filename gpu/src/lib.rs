// pathfinder/gpu/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

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

use hal::command::{IntoRawCommandBuffer, RawCommandBuffer};
use hal::queue::RawCommandQueue;
use hal::{
    Capability, DescriptorPool, Device, Instance, PhysicalDevice, QueueFamily, Surface, Swapchain,
};
use image as img_crate;
use pathfinder_geometry as pfgeom;
use pathfinder_simd as pfsimd;
use takeable_option::Takeable;

use pathfinder_geometry::basic::line_segment::{LineSegmentU4, LineSegmentU8};
use pathfinder_geometry::basic::point::Point2DI32;

pub mod resources;

#[derive(Clone)]
pub struct RenderPassDescription {
    attachments: Vec<hal::pass::Attachment>,
    num_subpasses: usize,
    colors_per_subpass: Vec<Vec<hal::pass::AttachmentRef>>,
    inputs_per_subpass: Vec<Vec<hal::pass::AttachmentRef>>,
    preserves_per_subpass: Vec<Vec<hal::pass::AttachmentId>>
}

impl RenderPassDescription {
    fn update_attachment_format(&mut self, attachment_index: usize, new_format: hal::format::Format) {
        Option::replace(&mut self.attachments[attachment_index].format, new_format);
    }
}

pub unsafe fn create_render_pass(
    device: &<Backend as hal::Backend>::Device,
    render_pass_desc: RenderPassDescription,
) -> <Backend as hal::Backend>::RenderPass {

    let subpasses: Vec<hal::pass::SubpassDesc> = (0..render_pass_desc.num_subpasses).into_iter().map(|i| hal::pass::SubpassDesc {
        colors: &render_pass_desc.colors_per_subpass[i],
        inputs: &render_pass_desc.inputs_per_subpass[i],
        depth_stencil: None,
        resolves: &[],
        preserves: &render_pass_desc.preserves_per_subpass[i],
    }).collect();

    device
        .create_render_pass(&render_pass_desc.attachments, subpasses, &[])
        .unwrap()
}

pub struct SwapchainState {
    swapchain_images: Vec<<Backend as hal::Backend>::Image>,
    swapchain_image_views: Vec<<Backend as hal::Backend>::ImageView>,
    swapchain_framebuffers: Vec<<Backend as hal::Backend>::Framebuffer>,
    swapchain: <Backend as hal::Backend>::Swapchain,
    in_flight_fences: Vec<<Backend as hal::Backend>::Fence>,
    draw_pipeline_layout_state: PipelineLayoutState,
    tile_solid_multicolor_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    tile_solid_monochrome_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    tile_alpha_multicolor_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    tile_alpha_monochrome_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    stencil_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    postprocess_pipeline: Option<<Backend as hal::Backend>::GraphicsPipeline>,
    acquire_image_fence: <Backend as hal::Backend>::Fence,
    extent: hal::pso::Rect,
}

impl SwapchainState {
    unsafe fn new(
        adapter: &mut hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        window: &winit::Window,
        surface: &mut <Backend as hal::Backend>::Surface,
        resource_loader: &dyn crate::resources::ResourceLoader,
        mut draw_render_pass_description: RenderPassDescription,
        indices_of_attachments_without_format: Vec<usize>,
        draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        tile_solid_multicolor_pipeline_description: PipelineDescription,
        tile_solid_monochrome_pipeline_description: PipelineDescription,
        tile_alpha_multicolor_pipeline_description: PipelineDescription,
        tile_alpha_monochrome_pipeline_description: PipelineDescription,
        stencil_pipeline_description: PipelineDescription,
        postprocess_pipeline_description: Option<PipelineDescription>,
    ) -> SwapchainState {
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
                width: capabilities
                    .extents
                    .end
                    .width
                    .min(window_client_area.width as u32),
                height: capabilities
                    .extents
                    .end
                    .height
                    .min(window_client_area.height as u32),
            }
        };

        let extent_rect = hal::pso::Rect {
            x: 0,
            y: 0,
            w: extent.width as i16,
            h: extent.height as i16,
        };

        let swapchain_config =
            hal::window::SwapchainConfig::from_caps(&capabilities, swapchain_image_format, extent);

        let (swapchain, swapchain_images) = device
            .create_swapchain(surface, swapchain_config, None)
            .unwrap();

        let swapchain_image_views: Vec<<Backend as hal::Backend>::ImageView> = swapchain_images
            .iter()
            .map(|i| {
                device
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
                    .unwrap()
            })
            .collect();

        let max_frames_in_flight = swapchain_images.len();

        for ix in indices_of_attachments_without_format.into_iter() {
            draw_render_pass_description.update_attachment_format(ix, swapchain_image_format);
        }

        let draw_render_pass = create_render_pass(device, draw_render_pass_description);

        let draw_pipeline_layout_state = PipelineLayoutState::new(
            device,
            draw_descriptor_set_layout_bindings,
            draw_render_pass,
        );

        let swapchain_framebuffers: Vec<<Backend as hal::Backend>::Framebuffer> =
            swapchain_image_views
                .iter()
                .map(|iv| {
                    device
                        .create_framebuffer(
                            &draw_pipeline_layout_state.render_pass(),
                            vec![iv],
                            hal::image::Extent {
                                width: extent.width,
                                height: extent.height,
                                depth: 1,
                            },
                        )
                        .unwrap()
                })
                .collect();

        let tile_solid_multicolor_pipeline = create_pipeline(
            device,
            &draw_pipeline_layout_state,
            resource_loader,
            tile_solid_multicolor_pipeline_description,
        );

        let tile_solid_monochrome_pipeline = create_pipeline(
            device,
            &draw_pipeline_layout_state,
            resource_loader,
            tile_solid_monochrome_pipeline_description,
        );

        let tile_alpha_multicolor_pipeline = create_pipeline(
            device,
            &draw_pipeline_layout_state,
            resource_loader,
            tile_alpha_multicolor_pipeline_description,
        );

        let tile_alpha_monochrome_pipeline = create_pipeline(
            device,
            &draw_pipeline_layout_state,
            resource_loader,
            tile_alpha_monochrome_pipeline_description,
        );

        let stencil_pipeline = create_pipeline(
            device,
            &draw_pipeline_layout_state,
            resource_loader,
            stencil_pipeline_description,
        );

        let postprocess_pipeline = match postprocess_pipeline_description {
            Some(ppd) => {
                Some(create_pipeline(
                    device,
                    &draw_pipeline_layout_state,
                    resource_loader,
                    ppd,
                ))
            },
            _ => { None },
        };

        let in_flight_fences: Vec<<Backend as hal::Backend>::Fence> = (0..max_frames_in_flight)
            .map(|_| device.create_fence(true).unwrap())
            .collect();

        let acquire_image_fence = device.create_fence(false).unwrap();

        SwapchainState {
            swapchain_images,
            swapchain_image_views,
            swapchain_framebuffers,
            swapchain,
            in_flight_fences,
            draw_pipeline_layout_state,
            tile_solid_multicolor_pipeline,
            tile_solid_monochrome_pipeline,
            tile_alpha_multicolor_pipeline,
            tile_alpha_monochrome_pipeline,
            stencil_pipeline,
            postprocess_pipeline,
            extent: extent_rect,
            acquire_image_fence
        }
    }

    fn swapchain(&self) -> &<Backend as hal::Backend>::Swapchain {
        &self.swapchain
    }

    unsafe fn acquire_image(
        &mut self,
        device: &<Backend as hal::Backend>::Device,
        timeout_ns: u64,
    ) -> (u32, bool)
    {
        let (ix, suboptimal) = self.swapchain
            .acquire_image(timeout_ns, None, &self.acquire_image_fence).unwrap();

        device.wait_for_fence(&self.acquire_image_fence).unwrap();
        device.reset_fence(&self.acquire_iamge_fence);

        (ix, suboptimal.is_some())
    }

    unsafe fn destroy_swapchain_state(
        device: &<Backend as hal::Backend>::Device,
        command_pool: &mut hal::CommandPool<back::Backend, hal::Graphics>,
        swapchain_state: SwapchainState,
    ) {
        let SwapchainState {
            in_flight_fences,
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
            postprocess_pipeline,
            extent,
            acquire_image_fence
        } = swapchain_state;

        for f in in_flight_fences.into_iter() {
            device.destroy_fence(f);
        }

        device.destroy_fence(acquire_image_fence);

        for iv in swapchain_image_views.into_iter() {
            device.destroy_image_view(iv);
        }

        for i in swapchain_images.into_iter() {
            device.destroy_image(i);
        }

        for fb in swapchain_framebuffers.into_iter() {
            device.destroy_framebuffer(fb)
        }

        for pl in vec![
            tile_solid_multicolor_pipeline,
            tile_solid_monochrome_pipeline,
            tile_alpha_multicolor_pipeline,
            tile_alpha_monochrome_pipeline,
            stencil_pipeline,
        ]
        .into_iter()
        {
            device.destroy_graphics_pipeline(pl);
        }

        match postprocess_pipeline {
            Some(pp) => {
                device.destroy_graphics_pipeline(pp);
            },
            _ => {},
        };

        PipelineLayoutState::destroy_pipeline_layout_state(device, draw_pipeline_layout_state);

        device.destroy_swapchain(swapchain);

        command_pool.reset();
    }
}

pub struct GpuState<'a> {
    _instance: back::Instance,
    window: &'a winit::Window,
    resource_loader: &'a dyn resources::ResourceLoader,
    surface: <Backend as hal::Backend>::Surface,
    pub device: <Backend as hal::Backend>::Device,
    adapter: hal::Adapter<Backend>,
    command_queue: <Backend as hal::Backend>::CommandQueue,
    command_pool: hal::CommandPool<Backend, hal::Graphics>,
    draw_render_pass_description: RenderPassDescription,
    indices_of_attachments_without_format: Vec<usize>,
    draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
    tile_solid_multicolor_pipeline_description: PipelineDescription,
    tile_solid_monochrome_pipeline_description: PipelineDescription,
    tile_alpha_multicolor_pipeline_description: PipelineDescription,
    tile_alpha_monochrome_pipeline_description: PipelineDescription,
    stencil_pipeline_description: PipelineDescription,
    postprocess_pipeline_description: Option<PipelineDescription>,
    swapchain_state: Takeable<SwapchainState>,
    quad_vertex_positions_buffer_pool: BufferPool,
    quad_vertex_indices_buffer_pool: BufferPool,
    tile_solid_vertex_buffer_pool: BufferPool,
    tile_alpha_vertex_buffer_pool: BufferPool,
    stencil_vertex_buffer_pool: BufferPool,
    transient_buffer_pool: BufferPool,
    fill_pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    fill_pipeline_layout_state: PipelineLayoutState,
    fill_framebuffer: Framebuffer,
    fill_vertex_buffer_pool: BufferPool,
    fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
    area_lut_texture: Image,
    gamma_lut_texture: Image,
    paint_texture: Image,
    stencil_texture: Image,
    monochrome: bool,
    current_frame_index: usize,
}

impl<'a> GpuState<'a> {
    pub unsafe fn new(
        window: &'a winit::Window,
        resource_loader: &'a dyn resources::ResourceLoader,
        instance_name: &str,
        fill_render_pass_description: RenderPassDescription,
        draw_render_pass_description: RenderPassDescription,
        fill_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        fill_pipeline_description: PipelineDescription,
        tile_solid_monochrome_pipeline_description: PipelineDescription,
        tile_solid_multicolor_pipeline_description: PipelineDescription,
        tile_alpha_monochrome_pipeline_description: PipelineDescription,
        tile_alpha_multicolor_pipeline_description: PipelineDescription,
        stencil_pipeline_description: PipelineDescription,
        postprocess_pipeline_description: Option<PipelineDescription>,
        fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
        max_quad_vertex_positions_buffer_size: u64,
        max_quad_vertex_indices_buffer_size: u64,
        max_fill_vertex_buffer_size: u64,
        max_tile_vertex_buffer_size: u64,
        monochrome: bool,
    ) -> GpuState<'a> {
        let instance = back::Instance::create(instance_name, 1);

        let mut surface = instance.create_surface(window);

        let mut adapter = GpuState::pick_adapter(&instance, &surface).unwrap();

        let (device, mut queue_group) =
            GpuState::create_device_with_graphics_queues(&mut adapter, &surface);

        let command_queue = queue_group.queues.drain(0..1).next().unwrap().into_raw();

        let command_pool = device
            .create_command_pool_typed(
                &queue_group,
                hal::pool::CommandPoolCreateFlags::RESET_INDIVIDUAL,
            )
            .unwrap();

        let current_frame_index: usize = 0;

        let indices_of_attachments_without_format: Vec<usize>= if postprocess_pipeline_description.is_some() {
            vec![1, 2]
        } else {
            vec![1,]
        };

        let swapchain_state = Takeable::new(SwapchainState::new(
            &mut adapter,
            &device,
            window,
            &mut surface,
            resource_loader,
            draw_render_pass_description.clone(),
            indices_of_attachments_without_format.clone(),
            draw_descriptor_set_layout_bindings.clone(),
            tile_solid_multicolor_pipeline_description.clone(),
            tile_solid_monochrome_pipeline_description.clone(),
            tile_alpha_multicolor_pipeline_description.clone(),
            tile_alpha_monochrome_pipeline_description.clone(),
            stencil_pipeline_description.clone(),
                postprocess_pipeline_description.clone(),
        ));

        let quad_vertex_positions_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            max_quad_vertex_positions_buffer_size,
            1,
            hal::buffer::Usage::VERTEX,
        );

        let quad_vertex_indices_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            max_quad_vertex_indices_buffer_size,
            1,
            hal::buffer::Usage::INDEX,
        );

        let fill_render_pass = create_render_pass(&device, fill_render_pass_description);

        let fill_pipeline_layout_state = PipelineLayoutState::new(
            &device,
            fill_descriptor_set_layout_bindings,
            fill_render_pass,
        );

        let fill_framebuffer = Framebuffer::new(
            &mut adapter,
            &device,
            hal::format::Format::R16Sfloat,
            fill_framebuffer_size,
            fill_pipeline_layout_state.render_pass(),
        );

        let fill_vertex_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            max_fill_vertex_buffer_size,
            swapchain_state.in_flight_fences.len() as u8,
            hal::buffer::Usage::VERTEX,
        );

        let fill_pipeline = create_pipeline(
            &device,
            &fill_pipeline_layout_state,
            resource_loader,
            fill_pipeline_description,
        );

        let tile_solid_vertex_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            max_tile_vertex_buffer_size,
            swapchain_state.in_flight_fences.len() as u8,
            hal::buffer::Usage::VERTEX,
        );

        let tile_alpha_vertex_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            max_tile_vertex_buffer_size,
            swapchain_state.in_flight_fences.len() as u8,
            hal::buffer::Usage::VERTEX,
        );

        let stencil_vertex_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            quad_vertex_positions_buffer_pool.buffer_size,
            swapchain_state.in_flight_fences.len() as u8,
            hal::buffer::Usage::VERTEX,
        );

        let transient_buffer_pool = BufferPool::new(
            &mut adapter,
            &device,
            max_quad_vertex_positions_buffer_size,
            swapchain_state.in_flight_fences.len() as u8,
            hal::buffer::Usage::TRANSIENT,
        );

        let area_lut_texture = GpuState::create_texture_from_png(&mut adapter, &device, &command_pool, &command_queue, "area-lut");
        let gamma_lut_texture = GpuState::create_texture_from_png(&mut adapter, &device, &command_pool, &command_queue, "gamma-lut");
        let stencil_texture = Image::new(&adapter, &device, stencil_texture_format, stencil_texture_size);
        let paint_texture = Image::new(&adapter, &device, paint_texture_format, paint_texture_size);

        GpuState {
            _instance: instance,
            window,
            resource_loader,
            surface,
            device,
            adapter,
            command_queue,
            command_pool,
            draw_render_pass_description,
            indices_of_attachments_without_format,
            draw_descriptor_set_layout_bindings,
            tile_solid_multicolor_pipeline_description,
            tile_solid_monochrome_pipeline_description,
            tile_alpha_multicolor_pipeline_description,
            tile_alpha_monochrome_pipeline_description,
            stencil_pipeline_description,
            postprocess_pipeline_description,
            swapchain_state,
            quad_vertex_positions_buffer_pool,
            quad_vertex_indices_buffer_pool,
            tile_solid_vertex_buffer_pool,
            tile_alpha_vertex_buffer_pool,
            stencil_vertex_buffer_pool,
            fill_pipeline,
            fill_pipeline_layout_state,
            fill_framebuffer,
            fill_vertex_buffer_pool,
            fill_framebuffer_size,
            monochrome,
            current_frame_index,
            transient_buffer_pool,
            area_lut_texture,
            gamma_lut_texture,
            stencil_texture,
            paint_texture,
        }
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

    unsafe fn create_texture_from_png(
        adapter: &mut hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        command_pool: &hal::CommandPool<Backend, hal::Graphics>,
        command_queue: &<Backend as hal::Backend>::CommandQueue,
        resources: &dyn resources::ResourceLoader,
        name: &str,
    ) -> Image {
        let data = resources.slurp(&format!("textures/{}.png", name)).unwrap();
        let image = img_crate::load_from_memory_with_format(&data, img_crate::ImageFormat::PNG)
            .unwrap()
            .to_luma();
        let pixel_size = std::mem::size_of::<img_crate::Luma<u8>>();
        let size =
            pfgeom::basic::point::Point2DI32::new(image.width() as i32, image.height() as i32);

        Image::new_from_data(
            adapter,
            device,
            command_pool,
            command_queue,
            size,
            pixel_size,
            &data,
        )
    }

    pub unsafe fn get_framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.swapchain_state.swapchain_framebuffers[self.current_frame_index]
    }

    pub unsafe fn request_free_frame_index(&mut self) -> Option<usize> {
        self.device
            .wait_for_fences(
                self.swapchain_state.in_flight_fences.iter(),
                hal::device::WaitFor::Any,
                core::u64::MAX,
            )
            .unwrap();

        for (i, f) in self.swapchain_state.in_flight_fences.iter().enumerate() {
            if self.device.get_fence_status(f).unwrap() {
                return Some(i);
            }
        }

        None
    }

    unsafe fn destroy_swapchain_state(&mut self) {
        match Takeable::try_take(&mut self.swapchain_state) {
            Some(ss) => {
                SwapchainState::destroy_swapchain_state(&self.device, &mut self.command_pool, ss);
            }
            _ => {}
        }
    }

    unsafe fn create_swapchain(&mut self) -> SwapchainState {
        SwapchainState::new(
            &mut self.adapter,
            &self.device,
            self.window,
            &mut self.surface,
            self.resource_loader,
            self.draw_render_pass_description.clone(),
            self.indices_of_attachments_without_format.clone(),
            self.draw_descriptor_set_layout_bindings.clone(),
            self.tile_solid_multicolor_pipeline_description.clone(),
            self.tile_solid_monochrome_pipeline_description.clone(),
            self.tile_alpha_multicolor_pipeline_description.clone(),
            self.tile_alpha_monochrome_pipeline_description.clone(),
            self.stencil_pipeline_description.clone(),
            self.postprocess_pipeline_description.clone(),
        )
    }

    unsafe fn recreate_swapchain(&mut self) {
        self.destroy_swapchain_state();

        let new_swapchain = self.create_swapchain();
        Takeable::insert(&mut self.swapchain_state, new_swapchain);
    }

    pub unsafe fn present(
        &mut self,
        solid: bool,
    ) -> Result<Option<hal::window::Suboptimal>, hal::window::PresentError> {
        self.current_frame_index = self.request_free_frame_index().unwrap();

        let image_index = match self
            .swapchain_state
            .acquire_image(core::u64::MAX) {
            (_, true) => {
                self.recreate_swapchain();
                let (ix, _) = self.swapchain_state.acquire_image(core::u64::MAX);
                ix
            },
            (ix, false) => {
                ix
            },
        };

        self.submit_draws(&self.swapchain_state.swapchain_framebuffers[image_index]);

        let present_result = self
            .command_queue
            .present::<_, _, <Backend as hal::Backend>::Semaphore, _>(
                std::iter::once((self.swapchain_state.swapchain(), image_index)),
                std::iter::empty(),
            );

        match present_result {
            Ok(Some(_)) => {
                self.recreate_swapchain();
            }
            _ => {}
        }

        present_result
    }

    fn fill_framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.fill_framebuffer.framebuffer()
    }

    fn fill_pipeline(&self) -> &<Backend as hal::Backend>::GraphicsPipeline {
        &self.fill_pipeline
    }

    pub unsafe fn upload_fill_vertex_buffer_data<T>(
        &mut self,
        data: &[FillBatchPrimitive],
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        self.fill_vertex_buffer_pool.upload_data(
            &self.device,
            data,
            first_vertex..vertex_count,
            first_instance..instance_count,
        );
    }

    pub unsafe fn upload_tile_solid_vertex_buffer_data<T>(
        &mut self,
        data: &[SolidTileBatchPrimitive],
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
        fence: Option<&'a <Backend as hal::Backend>::Fence>,
    ) {
        self.fill_vertex_buffer_pool.upload_data(
            &self.device,
            data,
            first_vertex..vertex_count,
            first_instance..instance_count,
        );
    }

    pub unsafe fn upload_tile_alpha_vertex_buffer_data<T>(
        &mut self,
        data: &[AlphaTileBatchPrimitive],
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
        fence: Option<&'a <Backend as hal::Backend>::Fence>,
    ) {
        self.fill_vertex_buffer_pool.upload_data(
            &self.device,
            data,
            first_vertex..vertex_count,
            first_instance..instance_count,
        );
    }

    pub unsafe fn submit_fills(&mut self) {
        let mut cmd_buffer = self
            .command_pool
            .acquire_command_buffer::<hal::command::OneShot>()
            .into_raw();

        cmd_buffer.begin(
            hal::command::CommandBufferFlags::ONE_TIME_SUBMIT,
            hal::command::CommandBufferInheritanceInfo::default(),
        );

        cmd_buffer.bind_graphics_pipeline(self.fill_pipeline());

        cmd_buffer.bind_graphics_descriptor_sets(
            self.fill_pipeline_layout_state.pipeline_layout(),
            0,
            self.fill_pipeline_layout_state.descriptor_sets(),
            &[],
        );

        cmd_buffer.begin_render_pass(
            self.fill_pipeline_layout_state.render_pass(),
            self.fill_framebuffer(),
            hal::pso::Rect {
                x: 0,
                y: 0,
                w: self.fill_framebuffer_size.x() as i16,
                h: self.fill_framebuffer_size.y() as i16,
            },
            &[],
            hal::command::SubpassContents::Inline,
        );

        for (vertex_count, instance_count, ix) in
            self.fill_vertex_buffer_pool.submission_list.iter()
            {
                cmd_buffer.bind_vertex_buffers(
                    0,
                    vec![(self.quad_vertex_positions_buffer_pool.get_buffer(0).buffer(), 0), (self.fill_vertex_buffer_pool.get_buffer(*ix).buffer(), 0)],
                );
                cmd_buffer.bind_index_buffer(hal::buffer::IndexBufferView{
                    buffer: self.quad_vertex_indices_buffer_pool.get_buffer(0).buffer(),
                    offset: 0,
                    index_type: hal::IndexType::U32,
                });
                cmd_buffer.draw(vertex_count.clone(), instance_count.clone());
            }

        cmd_buffer.end_render_pass();
        cmd_buffer.finish();

        let submission = hal::queue::Submission {
            command_buffers: vec![&cmd_buffer],
            wait_semaphores: None,
            signal_semaphores: None,
        };

        self.command_queue
            .submit::<_, _, <Backend as hal::Backend>::Semaphore, _, _>(submission, None);
    }

    pub fn submit_tiles(&mut self, draw_framebuffer: &<Backend as hal::Backend>::Framebuffer, solid: bool) {
        let mut cmd_buffer = self
            .command_pool
            .acquire_command_buffer::<hal::command::OneShot>()
            .into_raw();

        cmd_buffer.begin(
            hal::command::CommandBufferFlags::ONE_TIME_SUBMIT,
            hal::command::CommandBufferInheritanceInfo::default(),
        );
        cmd_buffer.begin_render_pass(
            self.swapchain_state.draw_pipeline_layout_state.render_pass(),
            draw_framebuffer,
            self.swapchain_state.extent,
            &[],
            hal::command::SubpassContents::Inline,
        );

        cmd_buffer.bind_graphics_descriptor_sets(
            self.swapchain_state.draw_pipeline_layout_state.pipeline_layout(),
            0,
            self.swapchain_state.draw_pipeline_layout_state.descriptor_sets(),
            &[],
        );

        match (self.monochrome, solid) {
            (true, true) => {
                cmd_buffer.bind_graphics_pipeline(&self.swapchain_state.tile_solid_monochrome_pipeline);
            },
            (true, false) => {
                cmd_buffer.bind_graphics_pipeline(&self.swapchain_state.tile_alpha_monochrome_pipeline);
            },
            (false, true) => {
                cmd_buffer.bind_graphics_pipeline(&self.swapchain_state.tile_solid_multicolor_pipeline);
            },
            (false, false) => {
                cmd_buffer.bind_graphics_pipeline(&self.swapchain_state.tile_alpha_multicolor_pipeline);
            }
        }

        let tile_buffer_pool = if solid {
            &self.tile_solid_vertex_buffer_pool
        } else {
            &self.tile_alpha_vertex_buffer_pool
        };

        for (vertex_count, instance_count, ix) in
            tile_buffer_pool.submission_list.iter()
            {
                cmd_buffer.bind_vertex_buffers(
                    0,
                    vec![(self.quad_vertex_positions_buffer_pool.get_buffer(0).buffer(), 0), (self.tile_buffer_pool.get_buffer(*ix).buffer(), 0)],
                );
                cmd_buffer.bind_index_buffer(hal::buffer::IndexBufferView{
                    buffer: self.quad_vertex_indices_buffer_pool.get_buffer(0).buffer(),
                    offset: 0,
                    index_type: hal::IndexType::U32,
                });

                cmd_buffer.draw(vertex_count.clone(), instance_count.clone());
            }

        if self.postprocessing_needed {
            cmd_buffer.next_subpass(hal::command::SubpassContents::Inline);

            cmd_buffer.bind_graphics_pipeline(self.swapchain_state.postprocess_pipeline.as_ref().unwrap());

            cmd_buffer.bind_vertex_buffers(0, vec![(self.quad_vertex_positions_buffer_pool.get_buffer(0), 0)]);
            cmd_buffer.bind_index_buffer(hal::buffer::IndexBufferView{
                buffer: self.quad_vertex_indices_buffer_pool.get_buffer(0),
                offset: 0,
                index_type: hal::IndexType::U32,
            });

            cmd_buffer.draw(4, 1);
        }

        cmd_buffer.end_render_pass();
        cmd_buffer.finish();

        let submission = hal::queue::Submission {
            command_buffers: vec![&cmd_buffer],
            wait_semaphores: None,
            signal_semaphores: None,
        };

        if solid {
            self.command_queue
                .submit::<_, _, <Backend as hal::Backend>::Semaphore, _, _>(submission, None);
        } else {
            self.command_queue
                .submit::<_, _, <Backend as hal::Backend>::Semaphore, _, _>(submission, &self.swapchain_state.in_flight_fences[self.current_frame_index]);
        }
    }
    
    unsafe fn submit_draws(&mut self, draw_framebuffer: &<Backend as hal::Backend>::draw_framebuffer) {
        self.submit_fills();
        self.submit_tiles(draw_framebuffer, true);
        self.submit_tiles(draw_framebuffer, false);
    }

    pub unsafe fn destroy_gpu_state(mut gpu_state: GpuState) {
        gpu_state.destroy_swapchain_state();

        let GpuState {
            device,
            quad_vertex_positions_buffer_pool,
            tile_solid_vertex_buffer_pool,
            tile_alpha_vertex_buffer_pool,
            stencil_vertex_buffer_pool,
            command_pool,
            fill_vertex_buffer_pool: fvb,
            fill_framebuffer: ffb,
            fill_pipeline: fpl,
            fill_pipeline_layout_state: fpls,
            ..
        } = gpu_state;

        Framebuffer::destroy_framebuffer(&device, ffb);
        device.destroy_graphics_pipeline(fpl);
        PipelineLayoutState::destroy_pipeline_layout_state(&device, fpls);

        BufferPool::destroy_buffer_pool(&device, fvb);
        BufferPool::destroy_buffer_pool(&device, quad_vertex_positions_buffer_pool);
        BufferPool::destroy_buffer_pool(&device, tile_solid_vertex_buffer_pool);
        BufferPool::destroy_buffer_pool(&device, tile_alpha_vertex_buffer_pool);
        BufferPool::destroy_buffer_pool(&device, stencil_vertex_buffer_pool);

        device.destroy_command_pool(command_pool.into_raw());
    }
}

fn load_shader_include(resources: &dyn resources::ResourceLoader, include_name: &str) -> String {
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
    pub rect: Option<pfgeom::basic::rect::RectI32>,
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
    pub fn from_transform_3d(
        transform: &pfgeom::basic::transform3d::Transform3DF32,
    ) -> UniformData {
        UniformData::Mat4([transform.c0, transform.c1, transform.c2, transform.c3])
    }
}

pub enum Memory {
    Reference(
        u64,
        hal::MemoryTypeId,
        std::rc::Rc<<Backend as hal::Backend>::Memory>,
    ),
    Direct(<Backend as hal::Backend>::Memory),
}

pub struct Buffer {
    usage: hal::buffer::Usage,
    buffer_size: u64,
    memory: Memory,
    requirements: hal::memory::Requirements,
    buffer: <Backend as hal::Backend>::Buffer,
}

impl Buffer {
    unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        buffer_size: u64,
        usage: hal::buffer::Usage,
        memory: Option<Memory>,
    ) -> Buffer {
        let mut buffer = device.create_buffer(buffer_size, usage).unwrap();
        let requirements = device.get_buffer_requirements(&mut buffer);

        let memory = match memory {
            Some(Memory::Reference(offset, mid, mem)) => {
                let memory_type = adapter.physical_device.memory_properties().memory_types[mid.0];
                assert!(
                    requirements.type_mask & (1 << mid.0) != 0
                        && memory_type
                            .properties
                            .contains(hal::memory::Properties::CPU_VISIBLE)
                );
                device
                    .bind_buffer_memory(&mem, offset, &mut buffer)
                    .unwrap();
                Memory::Reference(offset, mid, mem)
            }
            _ => {
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
                    .unwrap();

                let mem = device.allocate_memory(memory_type_id, buffer_size).unwrap();

                device.bind_buffer_memory(&mem, 0, &mut buffer).unwrap();
                Memory::Direct(mem)
            }
        };

        Buffer {
            usage,
            buffer_size,
            memory,
            requirements,
            buffer,
        }
    }

    pub fn usage(&self) -> hal::buffer::Usage {
        self.usage
    }

    pub fn buffer(&self) -> &<Backend as hal::Backend>::Buffer {
        &self.buffer
    }

    pub unsafe fn upload_data<T>(
        &mut self,
        device: &<Backend as hal::Backend>::Device,
        data: &[T],
    ) where
        T: Copy,
    {
        assert!(data.len() <= self.buffer_size as usize);

        let mref = self.memory_ref();

        let mut writer = device
            .acquire_mapping_writer::<T>(mref, 0..self.requirements.size)
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        device.release_mapping_writer(writer).unwrap();
    }

    pub fn memory_ref(&self) -> &<Backend as hal::Backend>::Memory {
        match &self.memory {
            Memory::Direct(mref) => mref,
            Memory::Reference(_, _, mref) => mref,
        }
    }

    unsafe fn destroy_buffer(device: &<Backend as hal::Backend>::Device, buffer: Buffer) {
        let Buffer {
            memory: mem,
            buffer: buf,
            ..
        } = buffer;
        device.destroy_buffer(buf);

        match mem {
            Memory::Reference(_, _, _) => {}
            Memory::Direct(m) => {
                device.free_memory(m);
            }
        }
    }
}

struct BufferPool {
    usage: hal::buffer::Usage,
    pool: Vec<Buffer>,
    buffer_size: u64,
    memory: std::rc::Rc<<Backend as hal::Backend>::Memory>,
    pub submission_list: Vec<(
        std::ops::Range<hal::VertexCount>,
        std::ops::Range<hal::InstanceCount>,
        usize,
    )>,
}

impl BufferPool {
    pub unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        buffer_size: u64,
        num_buffers: u8,
        usage: hal::buffer::Usage,
    ) -> BufferPool {
        let mut pool: Vec<Buffer> = vec![];

        let dummy_buffer = device
            .create_buffer(buffer_size * (num_buffers as u64), usage)
            .unwrap();
        let requirements = device.get_buffer_requirements(&dummy_buffer);
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
            .unwrap();

        let memory = std::rc::Rc::new(
            device
                .allocate_memory(memory_type_id, requirements.size)
                .unwrap(),
        );

        for n in 0..(num_buffers as u64) {
            pool.push(Buffer::new(
                adapter,
                &device,
                buffer_size,
                usage,
                Some(Memory::Reference(
                    n * buffer_size,
                    memory_type_id,
                    memory.clone(),
                )),
                None,
            ));
        }

        BufferPool {
            usage,
            pool,
            buffer_size,
            memory,
            submission_list: vec![],
        }
    }

    pub unsafe fn get_free_buffer_index(
        &mut self,
        device: &<Backend as hal::Backend>::Device,
    ) -> Option<usize> {
        for (ix, buf) in self.pool.iter_mut().enumerate() {
            if buf.signalled(device, 5) {
                return Some(ix);
            }
        }

        None
    }

    pub unsafe fn upload_data<T>(
        &mut self,
        device: &<Backend as hal::Backend>::Device,
        data: &[T],
        vertices: std::ops::Range<hal::VertexCount>,
        instances: std::ops::Range<hal::InstanceCount>,
    ) -> bool
    where
        T: Copy,
    {
        if self.submission_list.len() == self.pool.len() {
            false
        } else {
            loop {
                match self.get_free_buffer_index(device) {
                    Some(ix) => {
                        self.pool[ix].upload_data(device, data);
                        self.submission_list.push((vertices, instances, ix));
                        break;
                    }
                    _ => {
                        continue;
                    }
                }
            }
            true
        }
    }

    pub fn get_buffer(&self, ix: usize) -> &Buffer {
        &(self.pool[ix])
    }

    pub fn clear_submission_list(&mut self) {
        self.submission_list.clear();
    }

    unsafe fn destroy_buffer_pool(device: &<Backend as hal::Backend>::Device, buffer: BufferPool) {
        let BufferPool {
            pool: p,
            memory: mem,
            ..
        } = buffer;
        for b in p.into_iter() {
            Buffer::destroy_buffer(device, b);
        }
        device.free_memory(std::rc::Rc::try_unwrap(mem).unwrap());
    }
}

pub struct Image {
    image: <Backend as hal::Backend>::Image,
    memory: <Backend as hal::Backend>::Memory,
    size: pfgeom::basic::point::Point2DI32,
}

impl Image {
    unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        texture_format: hal::format::Format,
        size: pfgeom::basic::point::Point2DI32,
    ) -> Image {
        // 3. Make an image with transfer_dst and SAMPLED usage
        let mut image = device
            .create_image(
                hal::image::Kind::D2(size.x() as u32, size.y() as u32, 1, 0),
                1,
                texture_format,
                hal::image::Tiling::Optimal,
                hal::image::Usage::TRANSFER_DST | hal::image::Usage::SAMPLED,
                hal::image::ViewCapabilities::empty(),
            )
            .unwrap();

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
                    && mem_type
                        .properties
                        .contains(hal::memory::Properties::DEVICE_LOCAL)
            })
            .unwrap()
            .into();

        let memory = device
            .allocate_memory(upload_type, requirements.size)
            .unwrap();

        device.bind_image_memory(&memory, 0, &mut image).unwrap();

        Image {
            image,
            memory,
            size,
        }
    }

    unsafe fn new_from_data(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        command_pool: &mut hal::CommandPool<back::Backend, hal::Graphics>,
        command_queue: &mut <Backend as hal::Backend>::CommandQueue,
        size: pfgeom::basic::point::Point2DI32,
        texel_size: usize,
        data: &[u8],
    ) -> Image {
        let texture = Image::new(adapter, &device, hal::format::Format::R8Uint, size);

        let staging_buffer = Buffer::new(
            adapter,
            &device,
            (size.x() * size.y()) as u64,
            hal::buffer::Usage::TRANSFER_SRC,
            None,
        );

        let mut writer = device
            .acquire_mapping_writer::<u8>(
                &staging_buffer.memory_ref(),
                0..staging_buffer.requirements.size,
            )
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        device.release_mapping_writer(writer).unwrap();

        let mut cmd_buffer = command_pool.acquire_command_buffer::<hal::command::OneShot>();
        cmd_buffer.begin();

        // 7. Use a pipeline barrier to transition the image from empty/undefined
        //    to TRANSFER_WRITE/TransferDstOptimal
        let image_barrier = hal::memory::Barrier::Image {
            states: (hal::image::Access::empty(), hal::image::Layout::Undefined)
                ..(
                    hal::image::Access::TRANSFER_WRITE,
                    hal::image::Layout::TransferDstOptimal,
                ),
            target: &texture.image,
            families: None,
            range: hal::image::SubresourceRange {
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
            hal::image::Layout::TransferDstOptimal,
            &[hal::command::BufferImageCopy {
                buffer_offset: 0,
                buffer_width: (row_pitch / texel_size) as u32,
                buffer_height: size.y() as u32,
                image_layers: hal::image::SubresourceLayers {
                    aspects: hal::format::Aspects::COLOR,
                    level: 0,
                    layers: 0..1,
                },
                image_offset: hal::image::Offset { x: 0, y: 0, z: 0 },
                image_extent: hal::image::Extent {
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
                hal::image::Access::TRANSFER_WRITE,
                hal::image::Layout::TransferDstOptimal,
            )
                ..(
                    hal::image::Access::SHADER_READ,
                    hal::image::Layout::ShaderReadOnlyOptimal,
                ),
            target: &texture.image,
            families: None,
            range: hal::image::SubresourceRange {
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

        let submission = hal::queue::Submission {
            command_buffers: vec![&cmd_buffer],
            wait_semaphores: None,
            signal_semaphores: None,
        };

        command_queue.submit::<_, _, <Backend as hal::Backend>::Semaphore, _, _>(submission, None);

        let upload_fence = device.create_fence(false).unwrap();

        device
            .wait_for_fence(&upload_fence, core::u64::MAX)
            .unwrap();

        device.destroy_fence(upload_fence);

        texture
    }

    unsafe fn destroy_image(device: &<Backend as hal::Backend>::Device, image: Image) {
        let Image {
            image: img,
            memory: mem,
            ..
        } = image;
        device.destroy_image(img);
        device.free_memory(mem);
    }

    pub fn size(&self) -> pfgeom::basic::point::Point2DI32 {
        self.size
    }
}

pub struct Framebuffer {
    framebuffer: <Backend as hal::Backend>::Framebuffer,
    image: Image,
    image_view: <Backend as hal::Backend>::ImageView,
}

impl Framebuffer {
    pub unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        texture_format: hal::format::Format,
        size: pfgeom::basic::point::Point2DI32,
        render_pass: &<Backend as hal::Backend>::RenderPass,
    ) -> Framebuffer {
        let image = Image::new(adapter, device, texture_format, size);

        let subresource_range = hal::image::SubresourceRange {
            aspects: hal::format::Aspects::COLOR,
            levels: 0..1,
            layers: 0..1,
        };
        let image_view = device
            .create_image_view(
                &(image.image),
                hal::image::ViewKind::D2,
                texture_format,
                hal::format::Swizzle::NO,
                subresource_range,
            )
            .unwrap();

        let framebuffer = device
            .create_framebuffer(
                render_pass,
                vec![&image_view],
                hal::image::Extent {
                    width: size.x() as u32,
                    height: size.y() as u32,
                    depth: 1,
                },
            )
            .unwrap();

        Framebuffer {
            framebuffer,
            image,
            image_view,
        }
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    pub fn framebuffer(&self) -> &<Backend as hal::Backend>::Framebuffer {
        &self.framebuffer
    }

    pub unsafe fn destroy_framebuffer(
        device: &<Backend as hal::Backend>::Device,
        framebuffer: Framebuffer,
    ) {
        let Framebuffer {
            framebuffer: fb,
            image: img,
            image_view: imv,
        } = framebuffer;
        device.destroy_image_view(imv);
        Image::destroy_image(device, img);
        device.destroy_framebuffer(fb);
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

fn generate_stencil_test(
    func: StencilFunc,
    reference: u32,
    mask: u32,
    write: bool,
) -> hal::pso::StencilTest {
    let (op_pass, mask_write) = if write {
        (hal::pso::StencilOp::Replace, hal::pso::State::Static(mask))
    } else {
        (hal::pso::StencilOp::Keep, hal::pso::State::Static(0))
    };

    hal::pso::StencilTest::On {
        front: hal::pso::StencilFace {
            fun: match func {
                StencilFunc::Always => hal::pso::Comparison::Always,
                StencilFunc::Equal => hal::pso::Comparison::Equal,
                StencilFunc::NotEqual => hal::pso::Comparison::NotEqual,
            },
            mask_read: hal::pso::State::Static(mask),
            mask_write: mask_write,
            op_fail: hal::pso::StencilOp::Keep,
            op_depth_fail: hal::pso::StencilOp::Keep,
            op_pass: op_pass,
            reference: hal::pso::State::Static(reference),
        },
        back: hal::pso::StencilFace {
            fun: match func {
                StencilFunc::Always => hal::pso::Comparison::Always,
                StencilFunc::Equal => hal::pso::Comparison::Equal,
                StencilFunc::NotEqual => hal::pso::Comparison::NotEqual,
            },
            mask_read: hal::pso::State::Static(mask),
            mask_write: mask_write,
            op_fail: hal::pso::StencilOp::Keep,
            op_depth_fail: hal::pso::StencilOp::Keep,
            op_pass: op_pass,
            reference: hal::pso::State::Static(reference),
        },
    }
}

fn generate_blend_desc(blend_state: BlendState) -> hal::pso::BlendDesc {
    match blend_state {
        BlendState::RGBOneAlphaOne => {
            let blend_state = hal::pso::BlendState::On {
                color: hal::pso::BlendOp::Add {
                    src: hal::pso::Factor::One,
                    dst: hal::pso::Factor::One,
                },
                alpha: hal::pso::BlendOp::Add {
                    src: hal::pso::Factor::One,
                    dst: hal::pso::Factor::One,
                },
            };
            return hal::pso::BlendDesc {
                logic_op: Some(hal::pso::LogicOp::Copy),
                targets: vec![hal::pso::ColorBlendDesc(
                    hal::pso::ColorMask::ALL,
                    blend_state,
                )],
            };
        }
        BlendState::RGBOneAlphaOneMinusSrcAlpha => {
            let blend_state = hal::pso::BlendState::On {
                color: hal::pso::BlendOp::Add {
                    src: hal::pso::Factor::One,
                    dst: hal::pso::Factor::OneMinusSrcAlpha,
                },
                alpha: hal::pso::BlendOp::Add {
                    src: hal::pso::Factor::One,
                    dst: hal::pso::Factor::One,
                },
            };
            return hal::pso::BlendDesc {
                logic_op: Some(hal::pso::LogicOp::Copy),
                targets: vec![hal::pso::ColorBlendDesc(
                    hal::pso::ColorMask::ALL,
                    blend_state,
                )],
            };
        }
        BlendState::RGBSrcAlphaAlphaOneMinusSrcAlpha => {
            let blend_state = hal::pso::BlendState::On {
                color: hal::pso::BlendOp::Add {
                    src: hal::pso::Factor::SrcAlpha,
                    dst: hal::pso::Factor::OneMinusSrcAlpha,
                },
                alpha: hal::pso::BlendOp::Add {
                    src: hal::pso::Factor::One,
                    dst: hal::pso::Factor::One,
                },
            };
            return hal::pso::BlendDesc {
                logic_op: Some(hal::pso::LogicOp::Copy),
                targets: vec![hal::pso::ColorBlendDesc(
                    hal::pso::ColorMask::ALL,
                    blend_state,
                )],
            };
        }
        BlendState::Off => {
            return hal::pso::BlendDesc {
                logic_op: None,
                targets: vec![hal::pso::ColorBlendDesc::EMPTY],
            };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderKind {
    Vertex,
    Fragment,
}

unsafe fn compose_shader_module(
    device: &<Backend as hal::Backend>::Device,
    resource_loader: &dyn crate::resources::ResourceLoader,
    name: &str,
    shader_kind: ShaderKind,
) -> <Backend as hal::Backend>::ShaderModule {
    let shader_kind_char = match shader_kind {
        ShaderKind::Vertex => 'v',
        ShaderKind::Fragment => 'f',
    };

    let source = resource_loader
        .slurp(&format!("shaders/{}.{}s.glsl", name, shader_kind_char))
        .unwrap();

    let mut compiler = shaderc::Compiler::new()
        .ok_or("shaderc not found!")
        .unwrap();

    let artifact = compiler
        .compile_into_spirv(
            std::str::from_utf8(&source).unwrap(),
            match shader_kind {
                ShaderKind::Vertex => shaderc::ShaderKind::Vertex,
                ShaderKind::Fragment => shaderc::ShaderKind::Fragment,
            },
            "",
            "main",
            None,
        )
        .unwrap();

    let shader_module = device
        .create_shader_module(artifact.as_binary_u8())
        .unwrap();

    shader_module
}

#[derive(Clone)]
pub struct PipelineDescription {
    pub size: pfgeom::basic::point::Point2DI32,
    pub shader_name: String,
    pub vertex_buffer_descriptions: Vec<hal::pso::VertexBufferDesc>,
    pub attribute_descriptions: Vec<hal::pso::AttributeDesc>,
    pub rasterizer: hal::pso::Rasterizer,
    pub depth_stencil: hal::pso::DepthStencilDesc,
    pub blend_state: crate::BlendState,
    pub baked_states: hal::pso::BakedStates,
}

pub unsafe fn create_pipeline<'a>(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout_state: &PipelineLayoutState,
    resource_loader: &dyn crate::resources::ResourceLoader,
    pipeline_description: PipelineDescription,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module: <Backend as hal::Backend>::ShaderModule = compose_shader_module(
        device,
        resource_loader,
        &pipeline_description.shader_name,
        ShaderKind::Vertex,
    );
    let fragment_shader_module: <Backend as hal::Backend>::ShaderModule = compose_shader_module(
        device,
        resource_loader,
        &pipeline_description.shader_name,
        ShaderKind::Fragment,
    );

    let (vs_entry, fs_entry) = (
        hal::pso::EntryPoint {
            entry: "main",
            module: &vertex_shader_module,
            specialization: hal::pso::Specialization {
                constants: std::borrow::Cow::Borrowed(&[]),
                data: std::borrow::Cow::Borrowed(&[]),
            },
        },
        hal::pso::EntryPoint {
            entry: "main",
            module: &fragment_shader_module,
            specialization: hal::pso::Specialization {
                constants: std::borrow::Cow::Borrowed(&[]),
                data: std::borrow::Cow::Borrowed(&[]),
            },
        },
    );

    let shaders = hal::pso::GraphicsShaderSet {
        vertex: vs_entry,
        hull: None,
        domain: None,
        geometry: None,
        fragment: Some(fs_entry),
    };

    let input_assembler = hal::pso::InputAssemblerDesc::new(hal::Primitive::TriangleList);

    let blender = generate_blend_desc(pipeline_description.blend_state);

    let pipeline = {
        let PipelineDescription {
            rasterizer,
            vertex_buffer_descriptions,
            attribute_descriptions,
            depth_stencil,
            baked_states,
            ..
        } = pipeline_description;

        let desc = hal::pso::GraphicsPipelineDesc {
            shaders,
            rasterizer,
            vertex_buffers: vertex_buffer_descriptions,
            attributes: attribute_descriptions,
            input_assembler,
            blender,
            depth_stencil,
            multisampling: None,
            baked_states,
            layout: pipeline_layout_state.pipeline_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout_state.render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module);
    device.destroy_shader_module(fragment_shader_module);

    pipeline
}

pub struct PipelineLayoutState {
    descriptor_set_layout: <Backend as hal::Backend>::DescriptorSetLayout,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    render_pass: <Backend as hal::Backend>::RenderPass,
    descriptor_pool: <Backend as hal::Backend>::DescriptorPool,
    descriptor_sets: Vec<<Backend as hal::Backend>::DescriptorSet>,
}

impl PipelineLayoutState {
    pub unsafe fn new(
        device: &<Backend as hal::Backend>::Device,
        descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
        render_pass: <Backend as hal::Backend>::RenderPass,
    ) -> PipelineLayoutState {
        let immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let descriptor_set_layout = device
            .create_descriptor_set_layout(
                descriptor_set_layout_bindings.clone(),
                immutable_samplers,
            )
            .unwrap();

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let pipeline_layout = device
            .create_pipeline_layout(vec![&descriptor_set_layout], push_constants)
            .unwrap();

        let mut descriptor_pool = device
            .create_descriptor_pool(
                descriptor_set_layout_bindings.len(),
                PipelineLayoutState::generate_descriptor_range_descs_from_layout_bindings(
                    &descriptor_set_layout_bindings,
                ),
                hal::pso::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET,
            )
            .unwrap();

        let descriptor_sets = vec![descriptor_pool
            .allocate_set(&descriptor_set_layout)
            .unwrap()];

        PipelineLayoutState {
            pipeline_layout,
            descriptor_set_layout,
            render_pass,
            descriptor_pool,
            descriptor_sets,
        }
    }

    fn generate_descriptor_range_descs_from_layout_bindings(
        descriptor_set_layout_bindings: &[hal::pso::DescriptorSetLayoutBinding],
    ) -> Vec<hal::pso::DescriptorRangeDesc> {
        descriptor_set_layout_bindings
            .iter()
            .map(|dsl| hal::pso::DescriptorRangeDesc {
                ty: dsl.ty,
                count: dsl.count,
            })
            .collect::<Vec<hal::pso::DescriptorRangeDesc>>()
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

    fn descriptor_sets(&self) -> &Vec<<Backend as hal::Backend>::DescriptorSet> {
        &self.descriptor_sets
    }

    pub unsafe fn destroy_pipeline_layout_state(
        device: &<Backend as hal::Backend>::Device,
        pl_state: PipelineLayoutState,
    ) {
        let PipelineLayoutState {
            descriptor_set_layout: dsl,
            render_pass: rp,
            pipeline_layout: pl,
            descriptor_sets: dss,
            descriptor_pool: mut dp,
        } = pl_state;

        device.destroy_pipeline_layout(pl);
        device.destroy_render_pass(rp);
        dp.free_sets(dss);
        device.destroy_descriptor_pool(dp);
        device.destroy_descriptor_set_layout(dsl);
    }
}

#[derive(Clone, Debug)]
pub struct PaintData {
    pub size: Point2DI32,
    pub texels: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct FillObjectPrimitive {
    pub px: LineSegmentU4,
    pub subpx: LineSegmentU8,
    pub tile_x: i16,
    pub tile_y: i16,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct TileObjectPrimitive {
    /// If `u16::MAX`, then this is a solid tile.
    pub alpha_tile_index: u16,
    pub backdrop: i8,
}

// FIXME(pcwalton): Move `subpx` before `px` and remove `repr(packed)`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(packed)]
pub struct FillBatchPrimitive {
    pub px: LineSegmentU4,
    pub subpx: LineSegmentU8,
    pub alpha_tile_index: u16,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SolidTileBatchPrimitive {
    pub tile_x: i16,
    pub tile_y: i16,
    pub origin_u: u16,
    pub origin_v: u16,
    pub object_index: u16,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct AlphaTileBatchPrimitive {
    pub tile_x_lo: u8,
    pub tile_y_lo: u8,
    pub tile_hi: u8,
    pub backdrop: i8,
    pub object_index: u16,
    pub tile_index: u16,
    pub origin_u: u16,
    pub origin_v: u16,
}
