// pathfinder/gl/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! An OpenGL implementation of the device abstraction.

#[cfg(feature = "dx12")]
extern crate gfx_backend_dx12 as back;
#[cfg(feature = "metal")]
extern crate gfx_backend_metal as back;
#[cfg(feature = "vulkan")]
extern crate gfx_backend_vulkan as back;
extern crate gfx_hal as hal;
use pathfinder_geometry::basic::point::Point2DI32;
use pathfinder_geometry::basic::rect::RectI32;
use pathfinder_gpu::{BlendState, BufferTarget, BufferUploadMode, DepthFunc, Device, Primitive};
use pathfinder_gpu::{RenderState, ShaderKind, StencilFunc, TextureFormat};
use pathfinder_gpu::{UniformData, VertexAttrType};
use pathfinder_simd::default::F32x4;
use rustache::{HashBuilder, Render};
use std::ffi::CString;
use std::io::Cursor;
use std::mem;
use std::ptr;
use std::str;
use std::time::Duration;

use gfx_hal::{
    adapter::{Adapter, MemoryTypeId, PhysicalDevice},
    buffer::Usage as BufferUsage,
    command::{ClearColor, ClearValue, CommandBuffer, MultiShot, Primary},
    device::Device,
    format::{Aspects, ChannelType, Format, Swizzle},
    image::{Extent, Layout, SubresourceRange, Usage, ViewKind},
    memory::{Properties, Requirements},
    pass::{Attachment, AttachmentLoadOp, AttachmentOps, AttachmentStoreOp, Subpass, SubpassDesc},
    pool::{CommandPool, CommandPoolCreateFlags},
    pso::{
        AttributeDesc, BakedStates, BasePipeline, BlendDesc, BlendOp, BlendState, ColorBlendDesc,
        ColorMask, DepthStencilDesc, DepthTest, DescriptorSetLayoutBinding, Element, EntryPoint, Face,
        Factor, FrontFace, GraphicsPipelineDesc, GraphicsShaderSet, InputAssemblerDesc, LogicOp,
        PipelineCreationFlags, PipelineStage, PolygonMode, Rasterizer, Rect, ShaderStageFlags,
        Specialization, StencilTest, VertexBufferDesc, Viewport,
    },
    queue::{family::QueueGroup, Submission},
    window::{Backbuffer, Extent2D, FrameSync, PresentMode, Swapchain, SwapchainConfig},
    Backend, Gpu, Graphics, Instance, Primitive, QueueFamily, Surface,
    query,
};

pub struct HalDevice {
    buffer: ManuallyDrop<<back::Backend as Backend>::Buffer>,
    memory: ManuallyDrop<<back::Backend as Backend>::Memory>,
    descriptor_set_layouts: Vec<<back::Backend as Backend>::DescriptorSetLayout>,
    pipeline_layout: ManuallyDrop<<back::Backend as Backend>::PipelineLayout>,
    gfx_pipeline: ManuallyDrop<<back::Backend as Backend>::GraphicsPipeline>,
    requirements: Requirements,
    current_frame: usize,
    frames_in_flight: usize,
    in_flight_fences: Vec<<back::Backend as Backend>::Fence>,
    render_finished_semaphores: Vec<<back::Backend as Backend>::Semaphore>,
    image_available_semaphores: Vec<<back::Backend as Backend>::Semaphore>,
    submission_command_buffers: Vec<CommandBuffer<back::Backend, Graphics, MultiShot, Primary>>,
    command_pool: ManuallyDrop<CommandPool<back::Backend, Graphics>>,
    swapchain_framebuffers: Vec<<back::Backend as Backend>::Framebuffer>,
    image_views: Vec<(<back::Backend as Backend>::ImageView)>,
    render_pass: ManuallyDrop<<back::Backend as Backend>::RenderPass>,
    render_area: Rect,
    queue_group: QueueGroup<back::Backend, Graphics>,
    swapchain: ManuallyDrop<<back::Backend as Backend>::Swapchain>,
    device: ManuallyDrop<back::Device>,
    _adapter: Adapter<back::Backend>,
    _surface: <back::Backend as Backend>::Surface,
    _instance: ManuallyDrop<back::Instance>
}
//pub struct HalDevice {
//    hal_state: HalState
//}
//
//impl HalDevice {
//    unsafe fn init_hal(window: &Window) -> HalState {
//        let instance = HalState::create_instance();
//        let mut adapter = HalState::pick_adapter(&instance);
//        let mut surface = HalState::create_surface(&instance, window);
//        let (device, queue_group, queue_type, qf_id) =
//            HalState::create_device_with_graphics_queues(&mut adapter, &surface);
//        let (swapchain, extent, backbuffer, format) =
//            HalState::create_swap_chain(&adapter, &device, &mut surface, None);
//        let image_views =
//            HalState::create_image_views(backbuffer, format, &device);
//        let render_pass = HelloTriangleApplication::create_render_pass(&device, Some(format));
//        let (descriptor_set_layouts, pipeline_layout, gfx_pipeline) =
//            HalState::create_graphics_pipeline(&device, extent, &render_pass);
//        let swapchain_framebuffers = HalState::create_framebuffers(
//            &device,
//            &render_pass,
//            &frame_images,
//            extent,
//        );
//        let mut command_pool =
//            HalState::create_command_pool(&device, queue_type, qf_id);
//        let submission_command_buffers = HalState::create_command_buffers(
//            &mut command_pool,
//            &render_pass,
//            &swapchain_framebuffers,
//            extent,
//            &gfx_pipeline,
//        );
//        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) =
//            HalState::create_sync_objects(&device);
//
//        HalState {
//            in_flight_fences,
//            render_finished_semaphores,
//            image_available_semaphores,
//            submission_command_buffers,
//            command_pool,
//            swapchain_framebuffers,
//            gfx_pipeline,
//            descriptor_set_layouts,
//            pipeline_layout,
//            render_pass,
//            image_views,
//            swapchain,
//            queue_group,
//            device,
//            _surface: surface,
//            _adapter: adapter,
//            _instance: instance,
//        }
//    }
//}
//pub struct HalState {
//    buffer: ManuallyDrop<<back::Backend as Backend>::Buffer>,
//    memory: ManuallyDrop<<back::Backend as Backend>::Memory>,
//    descriptor_set_layouts: Vec<<back::Backend as Backend>::DescriptorSetLayout>,
//    pipeline_layout: ManuallyDrop<<back::Backend as Backend>::PipelineLayout>,
//    gfx_pipeline: ManuallyDrop<<back::Backend as Backend>::GraphicsPipeline>,
//    requirements: Requirements,
//    current_frame: usize,
//    frames_in_flight: usize,
//    in_flight_fences: Vec<<back::Backend as Backend>::Fence>,
//    render_finished_semaphores: Vec<<back::Backend as Backend>::Semaphore>,
//    image_available_semaphores: Vec<<back::Backend as Backend>::Semaphore>,
//    submission_command_buffers: Vec<CommandBuffer<back::Backend, Graphics, MultiShot, Primary>>,
//    command_pool: ManuallyDrop<CommandPool<back::Backend, Graphics>>,
//    swapchain_framebuffers: Vec<<back::Backend as Backend>::Framebuffer>,
//    image_views: Vec<(<back::Backend as Backend>::ImageView)>,
//    render_pass: ManuallyDrop<<back::Backend as Backend>::RenderPass>,
//    render_area: Rect,
//    queue_group: QueueGroup<back::Backend, Graphics>,
//    swapchain: ManuallyDrop<<back::Backend as Backend>::Swapchain>,
//    device: ManuallyDrop<back::Device>,
//    _adapter: Adapter<back::Backend>,
//    _surface: <back::Backend as Backend>::Surface,
//    _instance: ManuallyDrop<back::Instance>,
//}
//
//impl HalState {
//
//}
//
//impl core::ops::Drop for HalState {
//    fn drop(&mut self) {
//        let _ = self.device.wait_idle();
//
//        unsafe {
//            for descriptor_set_layout in self.descriptor_set_layouts.drain(..) {
//                self
//                    .device
//                    .destroy_descriptor_set_layout(descriptor_set_layout)
//            }
//
//            for fence in self.in_flight_fences.drain(..) {
//                self.device.destroy_fence(fence)
//            }
//
//            for semaphore in self.render_finished_semaphores.drain(..) {
//                self.device.destroy_semaphore(semaphore)
//            }
//
//            for semaphore in self.image_available_semaphores.drain(..) {
//                self.device.destroy_semaphore(semaphore)
//            }
//
//            for framebuffer in self.framebuffers.drain(..) {
//                self.device.destroy_framebuffer(framebuffer);
//            }
//
//            for image_view in self.image_views.drain(..) {
//                self.device.destroy_image_view(image_view);
//            }
//
//            // very unsure if this is the right way to do things
//            use core::ptr::read;
//            self
//                .device
//                .destroy_buffer(ManuallyDrop::into_inner(read(&self.buffer)));
//            self
//                .device
//                .free_memory(ManuallyDrop::into_inner(read(&self.memory)));
//            self
//                .device
//                .destroy_pipeline_layout(ManuallyDrop::into_inner(read(&self.pipeline_layout)));
//            self
//                .device
//                .destroy_graphics_pipeline(ManuallyDrop::into_inner(read(&self.graphics_pipeline)));
//            self
//                .device
//                .destroy_command_pool(ManuallyDrop::into_inner(read(&self.command_pool)).into_raw());
//            self
//                .device
//                .destroy_render_pass(ManuallyDrop::into_inner(read(&self.render_pass)));
//            self
//                .device
//                .destroy_swapchain(ManuallyDrop::into_inner(read(&self.swapchain)));
//
//            ManuallyDrop::drop(&mut self.device);
//            ManuallyDrop::drop(&mut self._instance);
//        }
//    }
//}
//
//
//impl HalDevice {
//    fn pick_adapter(instance: &back::Instance, surface: &<back::Backend as Backend>::Surface) -> Result<Adapter<back::Backend>, &'static str>{
//        // pick appropriate physical device (adapter)
//        instance
//            .enumerate_adapters()
//            .into_iter()
//            .find(|a| {
//                a.queue_families
//                    .iter()
//                    .any(|qf| qf.supports_graphics() && surface.supports_queue_family(qf))
//            })
//            .ok_or("No physical device available with queue families which support graphics and presentation to surface.")?
//    }
//
//
//    fn create_device_with_graphics_queues(
//        adapter: &mut Adapter<back::Backend>,
//        surface: &<back::Backend as Backend>::Surface,
//    ) -> (
//        <back::Backend as Backend>::Device,
//        QueueGroup<back::Backend, Graphics>,
//        queue::QueueType,
//        queue::family::QueueFamilyId,
//    ) {
//        let family = adapter
//            .queue_families
//            .iter()
//            .find(|family| {
//                Graphics::supported_by(family.queue_type())
//                    && family.max_queues() > 0
//                    && surface.supports_queue_family(family)
//            })
//            .expect("Could not find a queue family supporting graphics.");
//
//        let priorities = vec![1.0; 1];
//        let families = [(family, priorities.as_slice())];
//
//        let Gpu { device, mut queues } = unsafe {
//            adapter
//                .physical_device
//                .open(&families, Features::empty())
//                .expect("Could not create device.")
//        };
//
//        let mut queue_group = queues
//            .take::<Graphics>(family.id())
//            .expect("Could not take ownership of relevant queue group.");
//
//        (device, queue_group, family.queue_type(), family.id())
//    }
//
//    fn create_swap_chain(
//        adapter: &Adapter<back::Backend>,
//        device: &<back::Backend as Backend>::Device,
//        surface: &mut <back::Backend as Backend>::Surface,
//        previous_swapchain: Option<<back::Backend as Backend>::Swapchain>,
//    ) -> (
//        <back::Backend as Backend>::Swapchain,
//        window::Extent2D,
//        Backbuffer<back::Backend>,
//        format::Format,
//        usize,
//    ) {
//        let (caps, compatible_formats, compatible_present_modes, composite_alphas) =
//            surface.compatibility(&adapter.physical_device);
//
//        let present_mode = {
//            use gfx_hal::window::PresentMode::*;
//            [Mailbox, Fifo, Relaxed, Immediate]
//                .iter()
//                .cloned()
//                .find(|pm| compatible_present_modes.contains(pm))
//                .ok_or("Surface does not support any known presentation mode.")?
//        };
//
//        let composite_alpha = {
//            use gfx_hal::window::CompositeAlpha::*;
//            [Opaque, Inherit, PreMultiplied, PostMultiplied]
//                .iter()
//                .cloned()
//                .find(|ca| composite_alphas.contains(ca))
//                .ok_or("Surface does not support any known alpha composition mode.")?
//        };
//
//        let format = match compatible_formats {
//            None => Format::Rgba8Srgb,
//            Some(formats) => match formats
//                .iter()
//                .find(|format| format.base_format().1 == ChannelType::Srgb)
//                .cloned()
//                {
//                    Some(srgb_format) => srgb_format,
//                    None => formats
//                        .get(0)
//                        .cloned()
//                        .ok_or("Surface does not support any known format.")?,
//                },
//        };
//
//        let extent = {
//            let window_client_area = window
//                .get_inner_size()
//                .ok_or("Window doesn't exist!")?
//                .to_physical(window.get_hidpi_factor());
//
//            Extent2D {
//                width: caps.extents.end.width.min(window_client_area.width as u32),
//                height: caps
//                    .extents
//                    .end
//                    .height
//                    .min(window_client_area.height as u32),
//            }
//        };
//
//        let image_count = if present_mode == PresentMode::Mailbox {
//            (caps.image_count.end - 1).min(3)
//        } else {
//            (caps.image_count.end - 1).min(2)
//        };
//
//        let image_layers = 1;
//
//        let image_usage = if caps.usage.contains(Usage::COLOR_ATTACHMENT) {
//            Usage::COLOR_ATTACHMENT
//        } else {
//            Err("Surface does not support color attachments.")?
//        };
//
//        let swapchain_config = SwapchainConfig {
//            present_mode,
//            composite_alpha,
//            format,
//            extent,
//            image_count,
//            image_layers,
//            image_usage,
//        };
//
//        let (swapchain, backbuffer) = unsafe {
//            device
//                .create_swapchain(surface, swapchain_config, None)
//                .map_err(|_| "Could not create swapchain.")?
//        };
//
//        (swapchain, extent, backbuffer, format, image_count as usize)
//    }
//
//    fn create_synchronizers(
//        device: &<back::Backend as Backend>::Device,
//    ) -> (
//        Vec<<back::Backend as Backend>::Semaphore>,
//        Vec<<back::Backend as Backend>::Semaphore>,
//        Vec<<back::Backend as Backend>::Fence>,
//    ) {
//        let mut image_available_semaphores: Vec<<back::Backend as Backend>::Semaphore> = Vec::new();
//        let mut render_finished_semaphores: Vec<<back::Backend as Backend>::Semaphore> = Vec::new();
//        let mut in_flight_fences: Vec<<back::Backend as Backend>::Fence> = Vec::new();
//
//        for _ in 0..MAX_FRAMES_IN_FLIGHT {
//            image_available_semaphores.push(device.create_semaphore().unwrap());
//            render_finished_semaphores.push(device.create_semaphore().unwrap());
//            in_flight_fences.push(device.create_fence(true).unwrap());
//        }
//
//        (
//            image_available_semaphores,
//            render_finished_semaphores,
//            in_flight_fences,
//        )
//    }
//
//    fn create_render_pass(
//        device: &<back::Backend as Backend>::Device,
//        format: Option<format::Format>,
//    ) -> <back::Backend as Backend>::RenderPass {
//        let samples: u8 = 1;
//
//        let ops = pass::AttachmentOps {
//            load: pass::AttachmentLoadOp::Clear,
//            store: pass::AttachmentStoreOp::Store,
//        };
//
//        let stencil_ops = pass::AttachmentOps::DONT_CARE;
//
//        let layouts = image::Layout::Undefined..image::Layout::Present;
//
//        let color_attachment = pass::Attachment {
//            format,
//            samples,
//            ops,
//            stencil_ops,
//            layouts,
//        };
//
//        let color_attachment_ref: pass::AttachmentRef = (0, image::Layout::ColorAttachmentOptimal);
//
//        // hal assumes pipeline bind point is GRAPHICS
//        let subpass = pass::SubpassDesc {
//            colors: &[color_attachment_ref],
//            depth_stencil: None,
//            inputs: &[],
//            resolves: &[],
//            preserves: &[],
//        };
//
//        unsafe {
//            device
//                .create_render_pass(&[color_attachment], &[subpass], &[])
//                .unwrap()
//        }
//    }
//
//    unsafe fn create_image_views(
//        backbuffer: Backbuffer<back::Backend>,
//        format: format::Format,
//        device: &<back::Backend as Backend>::Device,
//    ) -> Vec<<back::Backend as Backend>::ImageView> {
//        match backbuffer {
//            window::Backbuffer::Images(images) => images
//                .into_iter()
//                .map(|image| {
//                    let image_view = match device.create_image_view(
//                        &image,
//                        image::ViewKind::D2,
//                        format,
//                        format::Swizzle::NO,
//                        image::SubresourceRange {
//                            aspects: format::Aspects::COLOR,
//                            levels: 0..1,
//                            layers: 0..1,
//                        },
//                    ) {
//                        Ok(image_view) => image_view,
//                        Err(_) => panic!("Error creating image view for an image."),
//                    };
//
//                    image_view
//                })
//                .collect(),
//            _ => unimplemented!(),
//        }
//    }
//
//    fn create_framebuffers(
//        device: &<back::Backend as Backend>::Device,
//        render_pass: &<back::Backend as Backend>::RenderPass,
//        image_views: &[<back::Backend as Backend>::ImageView],
//        extent: window::Extent2D,
//    ) -> Vec<<back::Backend as Backend>::Framebuffer> {
//        let mut swapchain_framebuffers: Vec<<back::Backend as Backend>::Framebuffer> = Vec::new();
//
//        unsafe {
//            for image_view in image_views.iter() {
//                swapchain_framebuffers.push(
//                    device
//                        .create_framebuffer(
//                            render_pass,
//                            vec![image_view],
//                            image::Extent {
//                                width: extent.width as _,
//                                height: extent.height as _,
//                                depth: 1,
//                            },
//                        )
//                        .expect("failed to create framebuffer!"),
//                );
//            }
//        }
//
//        swapchain_framebuffers
//    }
//
//    pub fn new(window: &Window, window_name: &str) -> Result<Self, &'static str> {
//        let instance = back::Instance::create(window_name, 1);
//
//        let mut surface = instance.create_surface(window);
//
//        let adapter = HalState::pick_adapter(&instance, &surface);
//
//        let (mut device, queue_group) = HalState::create_device_with_graphics_queues(&adapter, &surface);
//
//        // initialize swapchain, this is extra long
//        let (swapchain, extent, backbuffer, format, frames_in_flight) = HalState::create_swapchain(&adapter, &device, &mut surface, None);
//
//        // create synchronization objects
//        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) = HalState::create_synchronizers(&device);
//
//        // create render pass
//        let render_pass = HalState::create_renderpass(&device, Some(format));
//
//        // create image views
//        let image_views: Vec<_> = HalState::create_image_views();
//
//        let framebuffers = HalState::create_framebuffers(&device, &render_pass, &image_views, extent);
//
//        let mut command_pool = unsafe {
//            device
//                .create_command_pool_typed(&queue_group, CommandPoolCreateFlags::RESET_INDIVIDUAL)
//                .map_err(|_| "Could not create raw command pool.")?
//        };
//
//        let command_buffers: Vec<_> = framebuffers
//            .iter()
//            .map(|_| command_pool.acquire_command_buffer())
//            .collect();
//
//        // Build our pipeline and vertex buffer
//        let (descriptor_set_layouts, pipeline_layout, graphics_pipeline) =
//            Self::create_pipeline(&mut device, extent, &render_pass)?;
//        let (buffer, memory, requirements) = unsafe {
//            const F32_XY_TRIANGLE: u64 = (size_of::<f32>() * 2 * 3) as u64;
//            let mut buffer = device
//                .create_buffer(F32_XY_TRIANGLE, BufferUsage::VERTEX)
//                .map_err(|_| "Couldn't create a buffer for the vertices")?;
//            let requirements = device.get_buffer_requirements(&buffer);
//            let memory_type_id = adapter
//                .physical_device
//                .memory_properties()
//                .memory_types
//                .iter()
//                .enumerate()
//                .find(|&(id, memory_type)| {
//                    requirements.type_mask & (1 << id) != 0
//                        && memory_type.properties.contains(Properties::CPU_VISIBLE)
//                })
//                .map(|(id, _)| MemoryTypeId(id))
//                .ok_or("Couldn't find a memory type to support the vertex buffer!")?;
//            let memory = device
//                .allocate_memory(memory_type_id, requirements.size)
//                .map_err(|_| "Couldn't allocate vertex buffer memory")?;
//            device
//                .bind_buffer_memory(&memory, 0, &mut buffer)
//                .map_err(|_| "Couldn't bind the buffer memory!")?;
//            (buffer, memory, requirements)
//        };
//
//        Ok(Self {
//            requirements,
//            buffer: ManuallyDrop::new(buffer),
//            memory: ManuallyDrop::new(memory),
//            _instance: ManuallyDrop::new(instance),
//            _surface: surface,
//            _adapter: adapter,
//            device: ManuallyDrop::new(device),
//            queue_group,
//            swapchain: ManuallyDrop::new(swapchain),
//            render_area: extent.to_extent().rect(),
//            render_pass: ManuallyDrop::new(render_pass),
//            image_views,
//            framebuffers,
//            command_pool: ManuallyDrop::new(command_pool),
//            command_buffers,
//            image_available_semaphores,
//            render_finished_semaphores,
//            in_flight_fences,
//            frames_in_flight,
//            current_frame: 0,
//            descriptor_set_layouts,
//            pipeline_layout: ManuallyDrop::new(pipeline_layout),
//            graphics_pipeline: ManuallyDrop::new(graphics_pipeline),
//        })
//    }
//
//    #[allow(clippy::type_complexity)]
//    fn create_pipeline(
//        device: &mut back::Device, extent: Extent2D,
//        render_pass: &<back::Backend as Backend>::RenderPass,
//    ) -> Result<
//        (
//            Vec<<back::Backend as Backend>::DescriptorSetLayout>,
//            <back::Backend as Backend>::PipelineLayout,
//            <back::Backend as Backend>::GraphicsPipeline,
//        ),
//        &'static str,
//    > {
//        let mut compiler = shaderc::Compiler::new().ok_or("shaderc not found!")?;
//        let vertex_compile_artifact = compiler
//            .compile_into_spirv(
//                VERTEX_SOURCE,
//                shaderc::ShaderKind::Vertex,
//                "vertex.vert",
//                "main",
//                None,
//            )
//            .map_err(|_| "Couldn't compile vertex shader!")?;
//        let fragment_compile_artifact = compiler
//            .compile_into_spirv(
//                FRAGMENT_SOURCE,
//                shaderc::ShaderKind::Fragment,
//                "fragment.frag",
//                "main",
//                None,
//            )
//            .map_err(|e| {
//                error!("{}", e);
//                "Couldn't compile fragment shader!"
//            })?;
//        let vertex_shader_module = unsafe {
//            device
//                .create_shader_module(vertex_compile_artifact.as_binary_u8())
//                .map_err(|_| "Couldn't make the vertex module")?
//        };
//        let fragment_shader_module = unsafe {
//            device
//                .create_shader_module(fragment_compile_artifact.as_binary_u8())
//                .map_err(|_| "Couldn't make the fragment module")?
//        };
//        let (descriptor_set_layouts, pipeline_layout, gfx_pipeline) = {
//            let (vs_entry, fs_entry) = (
//                EntryPoint {
//                    entry: "main",
//                    module: &vertex_shader_module,
//                    specialization: Specialization {
//                        constants: &[],
//                        data: &[],
//                    },
//                },
//                EntryPoint {
//                    entry: "main",
//                    module: &fragment_shader_module,
//                    specialization: Specialization {
//                        constants: &[],
//                        data: &[],
//                    },
//                },
//            );
//            let shaders = GraphicsShaderSet {
//                vertex: vs_entry,
//                hull: None,
//                domain: None,
//                geometry: None,
//                fragment: Some(fs_entry),
//            };
//
//            let input_assembler = InputAssemblerDesc::new(Primitive::TriangleList);
//
//            let vertex_buffers: Vec<VertexBufferDesc> = vec![VertexBufferDesc {
//                binding: 0,
//                stride: (size_of::<f32>() * 2) as u32,
//                rate: 0,
//            }];
//            let attributes: Vec<AttributeDesc> = vec![AttributeDesc {
//                location: 0,
//                binding: 0,
//                element: Element {
//                    format: Format::Rg32Float,
//                    offset: 0,
//                },
//            }];
//
//            let rasterizer = Rasterizer {
//                depth_clamping: false,
//                polygon_mode: PolygonMode::Fill,
//                cull_face: Face::NONE,
//                front_face: FrontFace::Clockwise,
//                depth_bias: None,
//                conservative: false,
//            };
//
//            let depth_stencil = DepthStencilDesc {
//                depth: DepthTest::Off,
//                depth_bounds: false,
//                stencil: StencilTest::Off,
//            };
//
//            let blender = {
//                let blend_state = BlendState::On {
//                    color: BlendOp::Add {
//                        src: Factor::One,
//                        dst: Factor::Zero,
//                    },
//                    alpha: BlendOp::Add {
//                        src: Factor::One,
//                        dst: Factor::Zero,
//                    },
//                };
//                BlendDesc {
//                    logic_op: Some(LogicOp::Copy),
//                    targets: vec![ColorBlendDesc(ColorMask::ALL, blend_state)],
//                }
//            };
//
//            let baked_states = BakedStates {
//                viewport: Some(Viewport {
//                    rect: extent.to_extent().rect(),
//                    depth: (0.0..1.0),
//                }),
//                scissor: Some(extent.to_extent().rect()),
//                blend_color: None,
//                depth_bounds: None,
//            };
//
//            let bindings = Vec::<DescriptorSetLayoutBinding>::new();
//            let immutable_samplers = Vec::<<back::Backend as Backend>::Sampler>::new();
//            let descriptor_set_layouts: Vec<<back::Backend as Backend>::DescriptorSetLayout> =
//                vec![unsafe {
//                    device
//                        .create_descriptor_set_layout(bindings, immutable_samplers)
//                        .map_err(|_| "Couldn't make a DescriptorSetLayout")?
//                }];
//            let push_constants = Vec::<(ShaderStageFlags, core::ops::Range<u32>)>::new();
//            let layout = unsafe {
//                device
//                    .create_pipeline_layout(&descriptor_set_layouts, push_constants)
//                    .map_err(|_| "Couldn't create a pipeline layout")?
//            };
//
//            let gfx_pipeline = {
//                let desc = GraphicsPipelineDesc {
//                    shaders,
//                    rasterizer,
//                    vertex_buffers,
//                    attributes,
//                    input_assembler,
//                    blender,
//                    depth_stencil,
//                    multisampling: None,
//                    baked_states,
//                    layout: &layout,
//                    subpass: Subpass {
//                        index: 0,
//                        main_pass: render_pass,
//                    },
//                    flags: PipelineCreationFlags::empty(),
//                    parent: BasePipeline::None,
//                };
//
//                unsafe {
//                    device
//                        .create_graphics_pipeline(&desc, None)
//                        .map_err(|_| "Couldn't create a graphics pipeline!")?
//                }
//            };
//
//            (descriptor_set_layouts, layout, gfx_pipeline)
//        };
//
//        unsafe {
//            device.destroy_shader_module(vertex_shader_module);
//            device.destroy_shader_module(fragment_shader_module);
//        }
//
//        Ok((descriptor_set_layouts, pipeline_layout, gfx_pipeline))
//    }
//}
//
//impl core::ops::Drop for HalState {
//    /// We have to clean up "leaf" elements before "root" elements. Basically, we
//    /// clean up in reverse of the order that we created things.
//    fn drop(&mut self) {
//        let _ = self.device.wait_idle();
//        unsafe {
//            for descriptor_set_layout in self.descriptor_set_layouts.drain(..) {
//                self
//                    .device
//                    .destroy_descriptor_set_layout(descriptor_set_layout)
//            }
//            for fence in self.in_flight_fences.drain(..) {
//                self.device.destroy_fence(fence)
//            }
//            for semaphore in self.render_finished_semaphores.drain(..) {
//                self.device.destroy_semaphore(semaphore)
//            }
//            for semaphore in self.image_available_semaphores.drain(..) {
//                self.device.destroy_semaphore(semaphore)
//            }
//            for framebuffer in self.framebuffers.drain(..) {
//                self.device.destroy_framebuffer(framebuffer);
//            }
//            for image_view in self.image_views.drain(..) {
//                self.device.destroy_image_view(image_view);
//            }
//            // LAST RESORT STYLE CODE, NOT TO BE IMITATED LIGHTLY
//            use core::ptr::read;
//            self
//                .device
//                .destroy_buffer(ManuallyDrop::into_inner(read(&self.buffer)));
//            self
//                .device
//                .free_memory(ManuallyDrop::into_inner(read(&self.memory)));
//            self
//                .device
//                .destroy_pipeline_layout(ManuallyDrop::into_inner(read(&self.pipeline_layout)));
//            self
//                .device
//                .destroy_graphics_pipeline(ManuallyDrop::into_inner(read(&self.graphics_pipeline)));
//            self
//                .device
//                .destroy_command_pool(ManuallyDrop::into_inner(read(&self.command_pool)).into_raw());
//            self
//                .device
//                .destroy_render_pass(ManuallyDrop::into_inner(read(&self.render_pass)));
//            self
//                .device
//                .destroy_swapchain(ManuallyDrop::into_inner(read(&self.swapchain)));
//            ManuallyDrop::drop(&mut self.device);
//            ManuallyDrop::drop(&mut self._instance);
//        }
//    }
//}
//
//impl HalDevice {
//    #[inline]
//    pub fn new(window, window_name) -> HalDevice {
//        let hal_state = HalState::new();
//        HalDevice {
//            hal_state,
//        }
//    }
//
//    fn set_texture_parameters(&self, texture: &GLTexture) {
//        self.bind_texture(texture, 0);
//        unsafe {
//            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as GLint); ck();
//            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as GLint); ck();
//            gl::TexParameteri(gl::TEXTURE_2D,
//                              gl::TEXTURE_WRAP_S,
//                              gl::CLAMP_TO_EDGE as GLint); ck();
//            gl::TexParameteri(gl::TEXTURE_2D,
//                              gl::TEXTURE_WRAP_T,
//                              gl::CLAMP_TO_EDGE as GLint); ck();
//        }
//    }
//
//    fn set_render_state(&self, render_state: &RenderState) {
//        unsafe {
//            // Set blend.
//            match render_state.blend {
//                BlendState::Off => {
//                    gl::Disable(gl::BLEND); ck();
//                }
//                BlendState::RGBOneAlphaOne => {
//                    gl::BlendEquation(gl::FUNC_ADD); ck();
//                    gl::BlendFunc(gl::ONE, gl::ONE); ck();
//                    gl::Enable(gl::BLEND); ck();
//                }
//                BlendState::RGBOneAlphaOneMinusSrcAlpha => {
//                    gl::BlendEquation(gl::FUNC_ADD); ck();
//                    gl::BlendFuncSeparate(gl::ONE,
//                                          gl::ONE_MINUS_SRC_ALPHA,
//                                          gl::ONE,
//                                          gl::ONE); ck();
//                    gl::Enable(gl::BLEND); ck();
//                }
//                BlendState::RGBSrcAlphaAlphaOneMinusSrcAlpha => {
//                    gl::BlendEquation(gl::FUNC_ADD); ck();
//                    gl::BlendFuncSeparate(gl::SRC_ALPHA,
//                                          gl::ONE_MINUS_SRC_ALPHA,
//                                          gl::ONE,
//                                          gl::ONE); ck();
//                    gl::Enable(gl::BLEND); ck();
//                }
//            }
//
//            // Set depth.
//            match render_state.depth {
//                None => {
//                    gl::Disable(gl::DEPTH_TEST); ck();
//                }
//                Some(ref state) => {
//                    gl::DepthFunc(state.func.to_gl_depth_func()); ck();
//                    gl::DepthMask(state.write as GLboolean); ck();
//                    gl::Enable(gl::DEPTH_TEST); ck();
//                }
//            }
//
//            // Set stencil.
//            match render_state.stencil {
//                None => {
//                    gl::Disable(gl::STENCIL_TEST); ck();
//                }
//                Some(ref state) => {
//                    gl::StencilFunc(state.func.to_gl_stencil_func(),
//                                    state.reference as GLint,
//                                    state.mask); ck();
//                    let (pass_action, write_mask) = if state.write {
//                        (gl::REPLACE, state.mask)
//                    } else {
//                        (gl::KEEP, 0)
//                    };
//                    gl::StencilOp(gl::KEEP, gl::KEEP, pass_action); ck();
//                    gl::StencilMask(write_mask);
//                    gl::Enable(gl::STENCIL_TEST); ck();
//                }
//            }
//
//            // Set color mask.
//            let color_mask = render_state.color_mask as GLboolean;
//            gl::ColorMask(color_mask, color_mask, color_mask, color_mask); ck();
//        }
//    }
//
//    fn reset_render_state(&self, render_state: &RenderState) {
//        unsafe {
//            match render_state.blend {
//                BlendState::Off => {}
//                BlendState::RGBOneAlphaOneMinusSrcAlpha |
//                BlendState::RGBOneAlphaOne |
//                BlendState::RGBSrcAlphaAlphaOneMinusSrcAlpha => {
//                    gl::Disable(gl::BLEND); ck();
//                }
//            }
//
//            if render_state.depth.is_some() {
//                gl::Disable(gl::DEPTH_TEST); ck();
//            }
//
//            if render_state.stencil.is_some() {
//                gl::StencilMask(!0); ck();
//                gl::Disable(gl::STENCIL_TEST); ck();
//            }
//
//            gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE); ck();
//        }
//    }
//}
//
//impl Device for GLDevice {
//    type Buffer = <back::Backend as Backend>::Buffer;
//    type Framebuffer = <back::Backend as Backend>::Framebuffer;
//    type Program = <back::Backend as Backend>::GraphicsPipeline;
//    type Shader = <back::Backend as Backend>::ShaderModule;
//    type Texture = <back::Backend as Backend>::Image;
//    type TimerQuery = query::Query; // query
//    type Uniform = <back::Backend as Backend>::Buffer; // staging resource?
//    type VertexArray = <back::Backend as Backend>::Buffer; // buffer
//    type VertexAttr = usize; //usize
//
//    fn create_texture(&self, format: TextureFormat, size: Point2DI32) -> GLTexture {
//        let (gl_internal_format, gl_format, gl_type);
//        match format {
//            TextureFormat::R8 => {
//                gl_internal_format = gl::R8 as GLint;
//                gl_format = gl::RED;
//                gl_type = gl::UNSIGNED_BYTE;
//            }
//            TextureFormat::R16F => {
//                gl_internal_format = gl::R16F as GLint;
//                gl_format = gl::RED;
//                gl_type = gl::HALF_FLOAT;
//            }
//            TextureFormat::RGBA8 => {
//                gl_internal_format = gl::RGBA as GLint;
//                gl_format = gl::RGBA;
//                gl_type = gl::UNSIGNED_BYTE;
//            }
//        }
//
//        let mut texture = GLTexture { gl_texture: 0, size };
//        unsafe {
//            gl::GenTextures(1, &mut texture.gl_texture); ck();
//            self.bind_texture(&texture, 0);
//            gl::TexImage2D(gl::TEXTURE_2D,
//                           0,
//                           gl_internal_format,
//                           size.x() as GLsizei,
//                           size.y() as GLsizei,
//                           0,
//                           gl_format,
//                           gl_type,
//                           ptr::null()); ck();
//        }
//
//        self.set_texture_parameters(&texture);
//        texture
//    }
//
//    fn create_texture_from_data(&self, size: Point2DI32, data: &[u8]) -> GLTexture {
//        assert!(data.len() >= size.x() as usize * size.y() as usize);
//
//        let mut texture = GLTexture { gl_texture: 0, size };
//        unsafe {
//            gl::GenTextures(1, &mut texture.gl_texture); ck();
//            self.bind_texture(&texture, 0);
//            gl::TexImage2D(gl::TEXTURE_2D,
//                           0,
//                           gl::R8 as GLint,
//                           size.x() as GLsizei,
//                           size.y() as GLsizei,
//                           0,
//                           gl::RED,
//                           gl::UNSIGNED_BYTE,
//                           data.as_ptr() as *const GLvoid); ck();
//        }
//
//        self.set_texture_parameters(&texture);
//        texture
//    }
//
//    fn create_shader_from_source(&self,
//                                 name: &str,
//                                 source: &[u8],
//                                 kind: ShaderKind,
//                                 mut template_input: HashBuilder)
//                                 -> GLShader {
//        // FIXME(pcwalton): Do this once and cache it.
//        let glsl_version_spec = self.version.to_glsl_version_spec();
//        template_input = template_input.insert("version", glsl_version_spec);
//
//        let mut output = Cursor::new(vec![]);
//        template_input.render(str::from_utf8(source).unwrap(), &mut output).unwrap();
//        let source = output.into_inner();
//
//        let gl_shader_kind = match kind {
//            ShaderKind::Vertex => gl::VERTEX_SHADER,
//            ShaderKind::Fragment => gl::FRAGMENT_SHADER,
//        };
//
//        unsafe {
//            let gl_shader = gl::CreateShader(gl_shader_kind); ck();
//            gl::ShaderSource(gl_shader,
//                             1,
//                             [source.as_ptr() as *const GLchar].as_ptr(),
//                             [source.len() as GLint].as_ptr()); ck();
//            gl::CompileShader(gl_shader); ck();
//
//            let mut compile_status = 0;
//            gl::GetShaderiv(gl_shader, gl::COMPILE_STATUS, &mut compile_status); ck();
//            if compile_status != gl::TRUE as GLint {
//                let mut info_log_length = 0;
//                gl::GetShaderiv(gl_shader, gl::INFO_LOG_LENGTH, &mut info_log_length); ck();
//                let mut info_log = vec![0; info_log_length as usize];
//                gl::GetShaderInfoLog(gl_shader,
//                                     info_log.len() as GLint,
//                                     ptr::null_mut(),
//                                     info_log.as_mut_ptr() as *mut GLchar); ck();
//                eprintln!("Shader info log:\n{}", String::from_utf8_lossy(&info_log));
//                panic!("{:?} shader '{}' compilation failed", kind, name);
//            }
//
//            GLShader { gl_shader }
//        }
//    }
//
//    fn create_program_from_shaders(&self,
//                                   name: &str,
//                                   vertex_shader: GLShader,
//                                   fragment_shader: GLShader)
//                                   -> GLProgram {
//        let gl_program;
//        unsafe {
//            gl_program = gl::CreateProgram(); ck();
//            gl::AttachShader(gl_program, vertex_shader.gl_shader); ck();
//            gl::AttachShader(gl_program, fragment_shader.gl_shader); ck();
//            gl::LinkProgram(gl_program); ck();
//
//            let mut link_status = 0;
//            gl::GetProgramiv(gl_program, gl::LINK_STATUS, &mut link_status); ck();
//            if link_status != gl::TRUE as GLint {
//                let mut info_log_length = 0;
//                gl::GetProgramiv(gl_program, gl::INFO_LOG_LENGTH, &mut info_log_length); ck();
//                let mut info_log = vec![0; info_log_length as usize];
//                gl::GetProgramInfoLog(gl_program,
//                                      info_log.len() as GLint,
//                                      ptr::null_mut(),
//                                      info_log.as_mut_ptr() as *mut GLchar); ck();
//                eprintln!("Program info log:\n{}", String::from_utf8_lossy(&info_log));
//                panic!("Program '{}' linking failed", name);
//            }
//        }
//
//        GLProgram { gl_program, vertex_shader, fragment_shader }
//    }
//
//    #[inline]
//    fn create_vertex_array(&self) -> GLVertexArray {
//        unsafe {
//            let mut array = GLVertexArray { gl_vertex_array: 0 };
//            gl::GenVertexArrays(1, &mut array.gl_vertex_array); ck();
//            array
//        }
//    }
//
//    fn get_vertex_attr(&self, program: &Self::Program, name: &str) -> GLVertexAttr {
//        let name = CString::new(format!("a{}", name)).unwrap();
//        let attr = unsafe {
//            gl::GetAttribLocation(program.gl_program, name.as_ptr() as *const GLchar) as GLuint
//        }; ck();
//        GLVertexAttr { attr }
//    }
//
//    fn get_uniform(&self, program: &GLProgram, name: &str) -> GLUniform {
//        let name = CString::new(format!("u{}", name)).unwrap();
//        let location = unsafe {
//            gl::GetUniformLocation(program.gl_program, name.as_ptr() as *const GLchar)
//        }; ck();
//        GLUniform { location }
//    }
//
//    fn use_program(&self, program: &Self::Program) {
//        unsafe {
//            gl::UseProgram(program.gl_program); ck();
//        }
//    }
//
//    fn configure_float_vertex_attr(&self,
//                                   attr: &GLVertexAttr,
//                                   size: usize,
//                                   attr_type: VertexAttrType,
//                                   normalized: bool,
//                                   stride: usize,
//                                   offset: usize,
//                                   divisor: u32) {
//        unsafe {
//            gl::VertexAttribPointer(attr.attr,
//                                    size as GLint,
//                                    attr_type.to_gl_type(),
//                                    if normalized { gl::TRUE } else { gl::FALSE },
//                                    stride as GLint,
//                                    offset as *const GLvoid); ck();
//            gl::VertexAttribDivisor(attr.attr, divisor); ck();
//            gl::EnableVertexAttribArray(attr.attr); ck();
//        }
//    }
//
//    fn configure_int_vertex_attr(&self,
//                                 attr: &GLVertexAttr,
//                                 size: usize,
//                                 attr_type: VertexAttrType,
//                                 stride: usize,
//                                 offset: usize,
//                                 divisor: u32) {
//        unsafe {
//            gl::VertexAttribIPointer(attr.attr,
//                                    size as GLint,
//                                    attr_type.to_gl_type(),
//                                    stride as GLint,
//                                    offset as *const GLvoid); ck();
//            gl::VertexAttribDivisor(attr.attr, divisor); ck();
//            gl::EnableVertexAttribArray(attr.attr); ck();
//        }
//    }
//
//    fn set_uniform(&self, uniform: &Self::Uniform, data: UniformData) {
//        unsafe {
//            match data {
//                UniformData::Int(value) => {
//                    gl::Uniform1i(uniform.location, value); ck();
//                }
//                UniformData::Mat4(data) => {
//                    assert_eq!(mem::size_of::<[F32x4; 4]>(), 4 * 4 * 4);
//                    let data_ptr: *const F32x4 = data.as_ptr();
//                    gl::UniformMatrix4fv(uniform.location,
//                                         1,
//                                         gl::FALSE,
//                                         data_ptr as *const GLfloat);
//                }
//                UniformData::Vec2(data) => {
//                    gl::Uniform2f(uniform.location, data.x(), data.y()); ck();
//                }
//                UniformData::Vec4(data) => {
//                    gl::Uniform4f(uniform.location, data.x(), data.y(), data.z(), data.w()); ck();
//                }
//                UniformData::TextureUnit(unit) => {
//                    gl::Uniform1i(uniform.location, unit as GLint); ck();
//                }
//            }
//        }
//    }
//
//    fn create_framebuffer(&self, texture: GLTexture) -> GLFramebuffer {
//        let mut gl_framebuffer = 0;
//        unsafe {
//            gl::GenFramebuffers(1, &mut gl_framebuffer); ck();
//            gl::BindFramebuffer(gl::FRAMEBUFFER, gl_framebuffer); ck();
//            self.bind_texture(&texture, 0);
//            gl::FramebufferTexture2D(gl::FRAMEBUFFER,
//                                     gl::COLOR_ATTACHMENT0,
//                                     gl::TEXTURE_2D,
//                                     texture.gl_texture,
//                                     0); ck();
//            assert_eq!(gl::CheckFramebufferStatus(gl::FRAMEBUFFER), gl::FRAMEBUFFER_COMPLETE);
//        }
//
//        GLFramebuffer { gl_framebuffer, texture }
//    }
//
//    fn create_buffer(&self) -> GLBuffer {
//        unsafe {
//            let mut gl_buffer = 0;
//            gl::GenBuffers(1, &mut gl_buffer); ck();
//            GLBuffer { gl_buffer }
//        }
//    }
//
//    fn upload_to_buffer<T>(&self,
//                           buffer: &GLBuffer,
//                           data: &[T],
//                           target: BufferTarget,
//                           mode: BufferUploadMode) {
//        let target = match target {
//            BufferTarget::Vertex => gl::ARRAY_BUFFER,
//            BufferTarget::Index => gl::ELEMENT_ARRAY_BUFFER,
//        };
//        let mode = match mode {
//            BufferUploadMode::Static => gl::STATIC_DRAW,
//            BufferUploadMode::Dynamic => gl::DYNAMIC_DRAW,
//        };
//        unsafe {
//            gl::BindBuffer(target, buffer.gl_buffer); ck();
//            gl::BufferData(target,
//                           (data.len() * mem::size_of::<T>()) as GLsizeiptr,
//                           data.as_ptr() as *const GLvoid,
//                           mode); ck();
//        }
//    }
//
//    #[inline]
//    fn framebuffer_texture<'f>(&self, framebuffer: &'f Self::Framebuffer) -> &'f Self::Texture {
//        &framebuffer.texture
//    }
//
//    #[inline]
//    fn texture_size(&self, texture: &Self::Texture) -> Point2DI32 {
//        texture.size
//    }
//
//    fn upload_to_texture(&self, texture: &Self::Texture, size: Point2DI32, data: &[u8]) {
//        assert!(data.len() >= size.x() as usize * size.y() as usize * 4);
//        unsafe {
//            self.bind_texture(texture, 0);
//            gl::TexImage2D(gl::TEXTURE_2D,
//                           0,
//                           gl::RGBA as GLint,
//                           size.x() as GLsizei,
//                           size.y() as GLsizei,
//                           0,
//                           gl::RGBA,
//                           gl::UNSIGNED_BYTE,
//                           data.as_ptr() as *const GLvoid); ck();
//        }
//
//        self.set_texture_parameters(texture);
//    }
//
//    fn read_pixels_from_default_framebuffer(&self, size: Point2DI32) -> Vec<u8> {
//        let mut pixels = vec![0; size.x() as usize * size.y() as usize * 4];
//        unsafe {
//            gl::BindFramebuffer(gl::FRAMEBUFFER, self.default_framebuffer); ck();
//            gl::ReadPixels(0,
//                           0,
//                           size.x() as GLsizei,
//                           size.y() as GLsizei,
//                           gl::RGBA,
//                           gl::UNSIGNED_BYTE,
//                           pixels.as_mut_ptr() as *mut GLvoid); ck();
//        }
//
//        // Flip right-side-up.
//        let stride = size.x() as usize * 4;
//        for y in 0..(size.y() as usize / 2) {
//            let (index_a, index_b) = (y * stride, (size.y() as usize - y - 1) * stride);
//            for offset in 0..stride {
//                pixels.swap(index_a + offset, index_b + offset);
//            }
//        }
//
//        pixels
//    }
//
//    // TODO(pcwalton): Switch to `ColorF`!
//    fn clear(&self, color: Option<F32x4>, depth: Option<f32>, stencil: Option<u8>) {
//        unsafe {
//            let mut flags = 0;
//            if let Some(color) = color {
//                gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE); ck();
//                gl::ClearColor(color.x(), color.y(), color.z(), color.w()); ck();
//                flags |= gl::COLOR_BUFFER_BIT;
//            }
//            if let Some(depth) = depth {
//                gl::DepthMask(gl::TRUE); ck();
//                gl::ClearDepthf(depth as _); ck(); // FIXME(pcwalton): GLES
//                flags |= gl::DEPTH_BUFFER_BIT;
//            }
//            if let Some(stencil) = stencil {
//                gl::StencilMask(!0); ck();
//                gl::ClearStencil(stencil as GLint); ck();
//                flags |= gl::STENCIL_BUFFER_BIT;
//            }
//            if flags != 0 {
//                gl::Clear(flags); ck();
//            }
//        }
//    }
//
//    fn draw_arrays(&self, primitive: Primitive, index_count: u32, render_state: &RenderState) {
//        self.set_render_state(render_state);
//        unsafe {
//            gl::DrawArrays(primitive.to_gl_primitive(), 0, index_count as GLsizei); ck();
//        }
//        self.reset_render_state(render_state);
//    }
//
//    fn draw_elements(&self, primitive: Primitive, index_count: u32, render_state: &RenderState) {
//        self.set_render_state(render_state);
//        unsafe {
//            gl::DrawElements(primitive.to_gl_primitive(),
//                             index_count as GLsizei,
//                             gl::UNSIGNED_INT,
//                             ptr::null()); ck();
//        }
//        self.reset_render_state(render_state);
//    }
//
//    fn draw_arrays_instanced(&self,
//                             primitive: Primitive,
//                             index_count: u32,
//                             instance_count: u32,
//                             render_state: &RenderState) {
//        self.set_render_state(render_state);
//        unsafe {
//            gl::DrawArraysInstanced(primitive.to_gl_primitive(),
//                                    0,
//                                    index_count as GLsizei,
//                                    instance_count as GLsizei); ck();
//        }
//        self.reset_render_state(render_state);
//    }
//
//    #[inline]
//    fn create_timer_query(&self) -> GLTimerQuery {
//        let mut query = GLTimerQuery { gl_query: 0 };
//        unsafe {
//            gl::GenQueries(1, &mut query.gl_query); ck();
//        }
//        query
//    }
//
//    #[inline]
//    fn begin_timer_query(&self, query: &Self::TimerQuery) {
//        unsafe {
//            gl::BeginQuery(gl::TIME_ELAPSED, query.gl_query); ck();
//        }
//    }
//
//    #[inline]
//    fn end_timer_query(&self, _: &Self::TimerQuery) {
//        unsafe {
//            gl::EndQuery(gl::TIME_ELAPSED); ck();
//        }
//    }
//
//    #[inline]
//    fn timer_query_is_available(&self, query: &Self::TimerQuery) -> bool {
//        unsafe {
//            let mut result = 0;
//            gl::GetQueryObjectiv(query.gl_query, gl::QUERY_RESULT_AVAILABLE, &mut result); ck();
//            result != gl::FALSE as GLint
//        }
//    }
//
//    #[inline]
//    fn get_timer_query(&self, query: &Self::TimerQuery) -> Duration {
//        unsafe {
//            let mut result = 0;
//            gl::GetQueryObjectui64v(query.gl_query, gl::QUERY_RESULT, &mut result); ck();
//            Duration::from_nanos(result)
//        }
//    }
//
//    #[inline]
//    fn bind_vertex_array(&self, vertex_array: &GLVertexArray) {
//        unsafe {
//            gl::BindVertexArray(vertex_array.gl_vertex_array); ck();
//        }
//    }
//
//    #[inline]
//    fn bind_buffer(&self, buffer: &GLBuffer, target: BufferTarget) {
//        unsafe {
//            gl::BindBuffer(target.to_gl_target(), buffer.gl_buffer); ck();
//        }
//    }
//
//    #[inline]
//    fn bind_default_framebuffer(&self, viewport: RectI32) {
//        unsafe {
//            gl::BindFramebuffer(gl::FRAMEBUFFER, self.default_framebuffer); ck();
//            gl::Viewport(viewport.origin().x(),
//                         viewport.origin().y(),
//                         viewport.size().x(),
//                         viewport.size().y()); ck();
//        }
//    }
//
//    #[inline]
//    fn bind_framebuffer(&self, framebuffer: &GLFramebuffer) {
//        unsafe {
//            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer.gl_framebuffer); ck();
//            gl::Viewport(0, 0, framebuffer.texture.size.x(), framebuffer.texture.size.y()); ck();
//        }
//    }
//
//    #[inline]
//    fn bind_texture(&self, texture: &GLTexture, unit: u32) {
//        unsafe {
//            gl::ActiveTexture(gl::TEXTURE0 + unit); ck();
//            gl::BindTexture(gl::TEXTURE_2D, texture.gl_texture); ck();
//        }
//    }
//}
//
//pub struct GLVertexArray {
//    pub gl_vertex_array: GLuint,
//}
//
//impl Drop for GLVertexArray {
//    #[inline]
//    fn drop(&mut self) {
//        unsafe {
//            gl::DeleteVertexArrays(1, &mut self.gl_vertex_array); ck();
//        }
//    }
//}
//
//pub struct GLVertexAttr {
//    attr: GLuint,
//}
//
//impl GLVertexAttr {
//    pub fn configure_float(&self,
//                           size: GLint,
//                           gl_type: GLuint,
//                           normalized: bool,
//                           stride: GLsizei,
//                           offset: usize,
//                           divisor: GLuint) {
//        unsafe {
//            gl::VertexAttribPointer(self.attr,
//                                    size,
//                                    gl_type,
//                                    if normalized { gl::TRUE } else { gl::FALSE },
//                                    stride,
//                                    offset as *const GLvoid); ck();
//            gl::VertexAttribDivisor(self.attr, divisor); ck();
//            gl::EnableVertexAttribArray(self.attr); ck();
//        }
//    }
//
//    pub fn configure_int(&self,
//                         size: GLint,
//                         gl_type: GLuint,
//                         stride: GLsizei,
//                         offset: usize,
//                         divisor: GLuint) {
//        unsafe {
//            gl::VertexAttribIPointer(self.attr,
//                                     size,
//                                     gl_type,
//                                     stride,
//                                     offset as *const GLvoid); ck();
//            gl::VertexAttribDivisor(self.attr, divisor); ck();
//            gl::EnableVertexAttribArray(self.attr); ck();
//        }
//    }
//}
//
//pub struct GLFramebuffer {
//    pub gl_framebuffer: GLuint,
//    pub texture: GLTexture,
//}
//
//impl Drop for GLFramebuffer {
//    fn drop(&mut self) {
//        unsafe {
//            gl::DeleteFramebuffers(1, &mut self.gl_framebuffer); ck();
//        }
//    }
//}
//
//pub struct GLBuffer {
//    pub gl_buffer: GLuint,
//}
//
//impl Drop for GLBuffer {
//    fn drop(&mut self) {
//        unsafe {
//            gl::DeleteBuffers(1, &mut self.gl_buffer); ck();
//        }
//    }
//}
//
//#[derive(Debug)]
//pub struct GLUniform {
//    pub location: GLint,
//}
//
//pub struct GLProgram {
//    pub gl_program: GLuint,
//    #[allow(dead_code)]
//    vertex_shader: GLShader,
//    #[allow(dead_code)]
//    fragment_shader: GLShader,
//}
//
//impl Drop for GLProgram {
//    fn drop(&mut self) {
//        unsafe {
//            gl::DeleteProgram(self.gl_program); ck();
//        }
//    }
//}
//
//pub struct GLShader {
//    gl_shader: GLuint,
//}
//
//impl Drop for GLShader {
//    fn drop(&mut self) {
//        unsafe {
//            gl::DeleteShader(self.gl_shader); ck();
//        }
//    }
//}
//
//pub struct GLTexture {
//    gl_texture: GLuint,
//    pub size: Point2DI32,
//}
//
//pub struct GLTimerQuery {
//    gl_query: GLuint,
//}
//
//impl Drop for GLTimerQuery {
//    #[inline]
//    fn drop(&mut self) {
//        unsafe {
//            gl::DeleteQueries(1, &mut self.gl_query); ck();
//        }
//    }
//}
//
//trait BufferTargetExt {
//    fn to_gl_target(self) -> GLuint;
//}
//
//impl BufferTargetExt for BufferTarget {
//    fn to_gl_target(self) -> GLuint {
//        match self {
//            BufferTarget::Vertex => gl::ARRAY_BUFFER,
//            BufferTarget::Index => gl::ELEMENT_ARRAY_BUFFER,
//        }
//    }
//}
//
//trait DepthFuncExt {
//    fn to_gl_depth_func(self) -> GLenum;
//}
//
//impl DepthFuncExt for DepthFunc {
//    fn to_gl_depth_func(self) -> GLenum {
//        match self {
//            DepthFunc::Less => gl::LESS,
//            DepthFunc::Always => gl::ALWAYS,
//        }
//    }
//}
//
//trait PrimitiveExt {
//    fn to_gl_primitive(self) -> GLuint;
//}
//
//impl PrimitiveExt for Primitive {
//    fn to_gl_primitive(self) -> GLuint {
//        match self {
//            Primitive::Triangles => gl::TRIANGLES,
//            Primitive::TriangleFan => gl::TRIANGLE_FAN,
//            Primitive::Lines => gl::LINES,
//        }
//    }
//}
//
//trait StencilFuncExt {
//    fn to_gl_stencil_func(self) -> GLenum;
//}
//
//impl StencilFuncExt for StencilFunc {
//    fn to_gl_stencil_func(self) -> GLenum {
//        match self {
//            StencilFunc::Always => gl::ALWAYS,
//            StencilFunc::Equal => gl::EQUAL,
//            StencilFunc::NotEqual => gl::NOTEQUAL,
//        }
//    }
//}
//
//trait VertexAttrTypeExt {
//    fn to_gl_type(self) -> GLuint;
//}
//
//impl VertexAttrTypeExt for VertexAttrType {
//    fn to_gl_type(self) -> GLuint {
//        match self {
//            VertexAttrType::F32 => gl::FLOAT,
//            VertexAttrType::I16 => gl::SHORT,
//            VertexAttrType::U16 => gl::UNSIGNED_SHORT,
//            VertexAttrType::U8  => gl::UNSIGNED_BYTE,
//        }
//    }
//}
//
///// The version/dialect of OpenGL we should render with.
//pub enum GLVersion {
//    /// OpenGL 3.0+, core profile.
//    GL3,
//    /// OpenGL ES 3.0+.
//    GLES3,
//}
//
//impl GLVersion {
//    fn to_glsl_version_spec(&self) -> &'static str {
//        match *self {
//            GLVersion::GL3 => "330",
//            GLVersion::GLES3 => "300 es",
//        }
//    }
//}
//
