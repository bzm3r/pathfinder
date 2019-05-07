// pathfinder/renderer/src/gpu/renderer.rs
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

use back::Backend as Backend;

extern crate gfx_hal as hal;
extern crate shaderc;
extern crate log;
extern crate winit;

use crate::gpu_data::{AlphaTileBatchPrimitive, FillBatchPrimitive};
use crate::gpu_data::{RenderCommand, SolidTileBatchPrimitive};
use crate::post::DefringingKernel;
use crate::scene::ObjectShader;
use crate::tiles::{TILE_HEIGHT, TILE_WIDTH};
use pathfinder_geometry::basic::point::{Point2DI32, Point3DF32};
use pathfinder_geometry::basic::rect::RectI32;
use pathfinder_geometry::basic::transform3d::Transform3DF32;
use pathfinder_geometry::color::ColorF;
use pathfinder_gpu::resources::ResourceLoader;
use pathfinder_simd::default::{F32x4, I32x4};
use std::cmp;
use std::collections::VecDeque;
use std::mem;
use std::ops::{Add, Div};
use std::time::Duration;
use std::u32;
use hal;

static QUAD_VERTEX_POSITIONS: [u8; 8] = [0, 0, 1, 0, 1, 1, 0, 1];

// FIXME(pcwalton): Shrink this again!
const MASK_FRAMEBUFFER_WIDTH: i32 = TILE_WIDTH as i32 * 256;
const MASK_FRAMEBUFFER_HEIGHT: i32 = TILE_HEIGHT as i32 * 256;

// TODO(pcwalton): Replace with `mem::size_of` calls?
const FILL_INSTANCE_SIZE: usize = 8;
const SOLID_TILE_INSTANCE_SIZE: usize = 6;
const MASK_TILE_INSTANCE_SIZE: usize = 8;

const FILL_COLORS_TEXTURE_WIDTH: i32 = 256;
const FILL_COLORS_TEXTURE_HEIGHT: i32 = 256;

const MAX_FILLS_PER_BATCH: usize = 0x4000;

pub struct HalDevice {
    instance: back::Instance,
    surface: <Backend as hal::Backend>::Surface,
    device: <Backend as hal::Backend>::Device,
    adapter: hal::Adapter<Backend>,
    queue_group: hal::queue::QueueGroup<Backend, hal::Graphics>,
    swapchain: <Backend as hal::Backend>::Swapchain,
    extent: hal::window::Extent2D,
    backbuffer: hal::window::Backbuffer<Backend>,
    format: hal::format::Format,
    frames_in_flight: usize,
    image_available_semaphores: Vec<<Backend as hal::Backend>::Semaphore>,
    render_finished_semaphores: Vec<<Backend as hal::Backend>::Semaphore>,
    in_flight_fences: Vec<<Backend as hal::Backend>::Fence>,
    swapchain_image_views: Vec<(<Backend as hal::Backend>::ImageView)>,
}

impl HalDevice {
    unsafe fn new(window: &winit::Window, instance_name: &str) -> HalDevice {
        let instance = back::Instance::create(instance_name, 1);

        let mut surface = instance.create_surface(window);

        let adapter = HalDevice::pick_adapter(&instance, &surface);

        let (mut device, queue_group) = HalDevice::create_device_with_graphics_queues(&adapter, &surface);

        let (swapchain, extent, backbuffer, swapchain_framebuffer_format, frames_in_flight) = HalDevice::create_swapchain(&adapter, &device, &mut surface, None);

        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) = HalDevice::create_synchronizers(&device, frames_in_flight);

        let swapchain_image_views: Vec<_> = HalDevice::create_image_views();

        let swapchain_framebuffers = HalDevice::create_swapchain_framebuffers(&device, &render_pass, &swapchain_image_views, extent);

        let mut command_pool = device.create_command_pool_typed(&queue_group, hal::pool::CommandPoolCreateFlags::RESET_INDIVIDUAL).map_err(|_| "Could not create raw command pool.")?;

        let submission_command_buffers: Vec<_> = swapchain_framebuffers
            .iter()
            .map(|_| command_pool.acquire_command_buffer())
            .collect();
    }

    fn pick_adapter(instance: &back::Instance, surface: &<Backend as hal::Backend>::Surface) -> Result<hal::Adapter<Backend>, &'static str>{
        // pick appropriate physical device (adapter)
        instance
            .enumerate_adapters()
            .into_iter()
            .find(|a| {
                a.queue_families
                    .iter()
                    .any(|qf| qf.supports_graphics() && surface.supports_queue_family(qf))
            })
            .ok_or("No physical device available with queue families which support graphics and presentation to surface.")?
    }

    fn create_device_with_graphics_queues(
        adapter: &mut hal::adapter::Adapter<Backend>,
        surface: &<Backend as hal::Backend>::Surface,
    ) -> (
        <Backend as hal::Backend>::Device,
        hal::queue::QueueGroup<Backend, hal::Graphics>,
        hal::queue::QueueType,
        hal::queue::family::QueueFamilyId,
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

        let mut queue_group = queues
            .take::<hal::Graphics>(family.id())
            .expect("Could not take ownership of relevant queue group.");

        (device, queue_group, family.queue_type(), family.id())
    }

    fn create_swap_chain(
        adapter: &hal::adapter::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        surface: &mut <Backend as hal::Backend>::Surface,
        previous_swapchain: Option<<Backend as hal::Backend>::Swapchain>,
        window: &winit::Window,
    ) -> (
        <Backend as hal::Backend>::Swapchain,
        hal::window::Extent2D,
        hal::window::Backbuffer<Backend>,
        hal::format::Format,
        usize,
    ) {
        let (caps, compatible_formats, compatible_present_modes, composite_alphas) =
            surface.compatibility(&adapter.physical_device);

        let present_mode = {
            use hal::window::PresentMode::{Mailbox, Fifo, Relaxed, Immediate};
            [Mailbox, Fifo, Relaxed, Immediate]
                .iter()
                .cloned()
                .find(|pm| compatible_present_modes.contains(pm))
                .ok_or("Surface does not support any known presentation mode.")?
        };

        let composite_alpha = {
            hal::window::CompositeAlpha::all()
                .iter()
                .cloned()
                .find(|ca| composite_alphas.contains(ca))
                .ok_or("Surface does not support any known alpha composition mode.")?
        };

        let format = match compatible_formats {
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
                        .ok_or("Surface does not support any known format.")?,
                },
        };

        let extent = {
            let window_client_area = window
                .get_inner_size()
                .ok_or("Window doesn't exist!")?
                .to_physical(window.get_hidpi_factor());

            hal::window::Extent2D {
                width: caps.extents.end.width.min(window_client_area.width as u32),
                height: caps
                    .extents
                    .end
                    .height
                    .min(window_client_area.height as u32),
            }
        };

        let image_count = if present_mode == hal::window::PresentMode::Mailbox {
            (caps.image_count.end - 1).min(3)
        } else {
            (caps.image_count.end - 1).min(2)
        };

        let image_layers = 1;

        let image_usage = if caps.usage.contains(hal::image::Usage::COLOR_ATTACHMENT) {
            hal::image::Usage::COLOR_ATTACHMENT
        } else {
            Err("Surface does not support color attachments.")?
        };

        let swapchain_config = hal::window::SwapchainConfig {
            present_mode,
            composite_alpha,
            format,
            extent,
            image_count,
            image_layers,
            image_usage,
        };

        let (swapchain, backbuffer) = unsafe {
            device
                .create_swapchain(surface, swapchain_config, None)
                .map_err(|_| "Could not create swapchain.")?
        };

        (swapchain, extent, backbuffer, format, image_count as usize)
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
        backbuffer: hal::window::Backbuffer<Backend>,
        requested_format: hal::format::Format,
        device: &<Backend as hal::Backend>::Device,
    ) -> Vec<<Backend as hal::Backend>::ImageView> {
        match backbuffer {
            hal::window::Backbuffer::Images(images) => images
                .into_iter()
                .map(|image| {
                    let image_view = match device.create_image_view(
                        &image,
                        hal::image::ViewKind::D2,
                        requested_format,
                        hal::format::Swizzle::NO,
                        hal::image::SubresourceRange {
                            aspects: hal::format::Aspects::COLOR,
                            levels: 0..1,
                            layers: 0..1,
                        },
                    ) {
                        Ok(image_view) => image_view,
                        Err(_) => panic!("Error creating image view for an image."),
                    };

                    image_view
                })
                .collect(),
            _ => unimplemented!(),
        }
    }

    fn create_framebuffer(
        device: &<Backend as hal::Backend>::Device,
        render_pass: &<Backend as hal::Backend>::RenderPass,
        image_views: &[<Backend as hal::Backend>::ImageView],
        extent: hal::window::Extent2D,
    ) -> Vec<<Backend as hal::Backend>::Framebuffer> {
        let mut framebuffer: Vec<<Backend as hal::Backend>::Framebuffer> = Vec::new();

        unsafe {
            for image_view in image_views.iter() {
                swapchain_framebuffers.push(
                    device
                        .create_framebuffer(
                            render_pass,
                            vec![image_view],
                            hal::image::Extent {
                                width: extent.width as _,
                                height: extent.height as _,
                                depth: 1,
                            },
                        )
                        .expect("failed to create framebuffer!"),
                );
            }
        }

        framebuffer
    }

    fn create_shader_modules(resources: &dyn Resources) -> HalShaderSet {

    }
}

pub struct HalShaderSet {
    vertex: <Backend as hal::Backend>::ShaderModule,
    frag: Option<<Backend as hal::Backend>::ShaderModule>,
    hull: Option<<Backend as hal::Backend>::ShaderModule>,
    domain: Option<<Backend as hal::Backend>::ShaderModule>,
    geometry: Option<<Backend as hal::Backend>::ShaderModule>,
}
pub struct HalBuffer {
    size: Point2DI32,
    buffer: ManuallyDrop<<Backend as hal::Backend>::Buffer>,
    memory: ManuallyDrop<<Backend as hal::Backend>::Memory>,
    requirements: hal::memory::Requirements,
}

impl HalBuffer {
    unsafe fn new(adapter: &<Backend as hal::Backend>::Adapter, device: &<Backend as hal::Backend>::Device, size: Point2DI32, usage: hal::buffer::Usage) -> HalBuffer {
        let mut buffer = device
            .create_buffer(size, usage)
            .map_err(|_| format!("Unable to create buffer of size {} and usage type{}", size, usage))?;

        let requirements = device.get_buffer_requirements(&buffer);

        let memory_type_id = adapter
            .physical_device
            .memory_properties()
            .memory_types
            .iter()
            .enumerate()
            .find(|&(id, memory_type)| {
                requirements.type_mask & (1 << id) != 0
                    && memory_type.properties.contains(hal::memory::Properties::CPU_VISIBLE)
            })
            .map(|(id, _)| hal::adapter::MemoryTypeId(id))
            .ok_or("Adapter cannot supply required memory.")?;

        let memory = device
            .allocate_memory(memory_type_id, requirements.size)
            .map_err(|_| "Could not allocate memory on device.")?;

        device
            .bind_buffer_memory(&memory, 0, &mut buffer)
            .map_err(|_| "Could not bind memory to device.")?;

        HalBuffer { size, buffer, memory, requirements }
    }

    pub unsafe fn manually_drop(&self, device: &D) {
        use core::ptr::read;
        device.destroy_buffer(ManuallyDrop::into_inner(read(&self.buffer)));
        device.free_memory(ManuallyDrop::into_inner(read(&self.memory)));
    }
}

pub struct HalTexture {
    image: <Backend as hal::Backend>::Image,
    requirements: hal::memory::Requirements,
    memory: <Backend as hal::Backend>::Memory,
    image_view: <Backend as hal::Backend>::ImageView,
    sampler: <Backend as hal::Backend>::Sampler,
}

impl HalTexture {
    fn destroy(self, device: &<Backend as hal::Backend>::Device) {
        device.destroy_image_view(self.image_view);
        device.free_memory(self.memory);
    }
}

struct HalFramebuffer {
    framebuffers: Vec<<Backend as hal::Backend>::Framebuffer>,
}

struct FillPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

impl FillPipeline {
    fn new(device: &HalDevice, resources: &dyn ResourceLoader, extent: hal::window::Extent2D) -> Result<FillPipeline, &str> {
        let (vertex_shader_module, fragment_shader_module, _, _, _) = device.create_shader_modules("fill", resources);

        let (descriptor_set_layouts, pipeline_layout, graphics_pipeline) = {
            let (vs_entry, fs_entry) = (
                hal::pso::EntryPoint {
                    entry: "main",
                    module: &vertex_shader_module,
                    specialization: hal::pso::Specialization {
                        constants: &[],
                        data: &[],
                    },
                },
                hal::pso::EntryPoint {
                    entry: "main",
                    module: &fragment_shader_module,
                    specialization: hal::pso::Specialization {
                        constants: &[],
                        data: &[],
                    },
                },
            );

            let shaders = hal::pso::GraphicsShaderSet {
                vertex: vs_entry,
                hull: Some(),
                domain: None,
                geometry: None,
                fragment: Some(fs_entry),
            };

            let input_assembler = hal::pso::InputAssemblerDesc::new(hal::Primitive::TriangleList);

            let vertex_buffers: Vec<hal::pso::VertexBufferDesc> =
                vec![
                    // quad_vertex_positions_buffer
                    hal::pso::VertexBufferDesc {
                        binding: 0,
                        stride: 8,
                        rate: hal::pso::VertexInputRate::Vertex,
                    },
                    // fill_vertex_buffer
                    hal::pso::VertexBufferDesc {
                       binding: 1,
                       stride: 64,
                       rate: hal::pso::VertexInputRate::Vertex,
                   },
                ];

            let attributes: Vec<hal::pso::AttributeDesc> = vec![
                // tess_coord_attr
                hal::pso::AttributeDesc {
                    location: 0,
                    binding: 0,
                    element: Element {
                        format: R8Unorm,
                        offset: 0,
                    }
                },
                // from_px_attr
                AttributeDesc {
                    location: 0,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 0,
                    }
                },
                // to_px_attr
                AttributeDesc {
                    location: 1,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 1,
                    }
                },
                // from_subpx_attr
                AttributeDesc {
                    location: 2,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 2,
                    }
                },
                // to_subpx_attr
                AttributeDesc {
                    location: 3,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 2,
                    }
                },
                // tile_index_attr
                AttributeDesc {
                    location: 5,
                    binding: 0,
                    element: Element {
                        format: Rg8Unorm,
                        offset: 6,
                    }
                },
            ];

            let rasterizer = hal::pso::Rasterizer {
                depth_clamping: false,
                polygon_mode: hal::pso::PolygonMode::Fill,
                cull_face: hal::pso::Face::NONE,
                front_face: hal::pso::FrontFace::CounterClockwise,
                depth_bias: None,
                conservative: false,
            };

            let depth_stencil = hal::pso::DepthStencilDesc {
                depth: hal::pso::DepthTest::Off,
                depth_bounds: false,
                stencil: hal::pso::StencilTest::Off,
            };

            let blender = {
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
                hal::pso::BlendDesc {
                    logic_op: Some(hal::pso::LogicOp::Copy),
                    targets: vec![hal::pso::ColorBlendDesc(hal::pso::ColorMask::ALL, blend_state)],
                }
            };

            let baked_states = hal::pso::BakedStates {
                viewport: Some(hal::pso::Viewport {
                    rect: extent.to_extent().rect(),
                    depth: (0.0..1.0),
                }),
                scissor: Some(extent.to_extent().rect()),
                blend_color: None,
                depth_bounds: None,
            };

            let bindings = vec![
                hal::pso::DescriptorSetLayoutBinding {
                    binding: 0,
                    ty: hal::pso::DescriptorType::UniformBuffer,
                    count: 2,
                    stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                    immutable_samplers: false,
                },
            ];

            let immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

            let descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> =
                vec![unsafe {
                    device
                        .create_descriptor_set_layout(bindings, immutable_samplers)
                        .map_err(|_| "Couldn't make a DescriptorSetLayout")?
                }];

            let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

            let layout = unsafe {
                device
                    .create_pipeline_layout(&descriptor_set_layouts, push_constants)
                    .map_err(|_| "Couldn't create a pipeline layout")?
            };

            let pipeline = {
                let desc = hal::pso::GraphicsPipelineDesc {
                    shaders,
                    rasterizer,
                    vertex_buffers,
                    attributes,
                    input_assembler,
                    blender,
                    depth_stencil,
                    multisampling: None,
                    baked_states,
                    layout: &layout,
                    subpass: hal::pass::Subpass {
                        index: 0,
                        main_pass: render_pass,
                    },
                    flags: hal::pso::PipelineCreationFlags::empty(),
                    parent: hal::pso::BasePipeline::None,
                };

                unsafe {
                    device
                        .create_graphics_pipeline(&desc, None)
                        .map_err(|_| "Couldn't create a graphics pipeline!")?
                }
            };

            Ok(
                FillPipeline {
                    descriptor_set_layouts,
                    layout,
                    pipeline,
                }
            )
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        (descriptor_set_layouts, pipeline_layout, graphics_pipeline)
    }
}

struct SolidMulticolorPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

impl SolidMulticolorPipeline {
    fn new(device: &HalDevice, resources: &dyn ResourceLoader, extent: hal::window::Extent2D) -> Result<SolidMulticolorPipeline, &str> {
        let (vertex_shader_module, fragment_shader_module, _, _, _) = device.create_shader_modules("fill", resources);

        let (descriptor_set_layouts, pipeline_layout, graphics_pipeline) = {
            let (vs_entry, fs_entry) = (
                hal::pso::EntryPoint {
                    entry: "main",
                    module: &vertex_shader_module,
                    specialization: hal::pso::Specialization {
                        constants: &[],
                        data: &[],
                    },
                },
                hal::pso::EntryPoint {
                    entry: "main",
                    module: &fragment_shader_module,
                    specialization: hal::pso::Specialization {
                        constants: &[],
                        data: &[],
                    },
                },
            );

            let shaders = hal::pso::GraphicsShaderSet {
                vertex: vs_entry,
                hull: Some(),
                domain: None,
                geometry: None,
                fragment: Some(fs_entry),
            };

            let input_assembler = hal::pso::InputAssemblerDesc::new(hal::Primitive::TriangleList);

            let vertex_buffers: Vec<hal::pso::VertexBufferDesc> =
                vec![
                    // quad_vertex_positions_buffer
                    hal::pso::VertexBufferDesc {
                        binding: 0,
                        stride: 8,
                        rate: hal::pso::VertexInputRate::Vertex,
                    },
                    // fill_vertex_buffer
                    hal::pso::VertexBufferDesc {
                        binding: 1,
                        stride: 64,
                        rate: hal::pso::VertexInputRate::Vertex,
                    },
                ];

            let attributes: Vec<hal::pso::AttributeDesc> = vec![
                // tess_coord_attr
                hal::pso::AttributeDesc {
                    location: 0,
                    binding: 0,
                    element: Element {
                        format: R8Unorm,
                        offset: 0,
                    }
                },
                // from_px_attr
                AttributeDesc {
                    location: 0,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 0,
                    }
                },
                // to_px_attr
                AttributeDesc {
                    location: 1,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 1,
                    }
                },
                // from_subpx_attr
                AttributeDesc {
                    location: 2,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 2,
                    }
                },
                // to_subpx_attr
                AttributeDesc {
                    location: 3,
                    binding: 1,
                    element: Element {
                        format: R8Unorm,
                        offset: 2,
                    }
                },
                // tile_index_attr
                AttributeDesc {
                    location: 5,
                    binding: 0,
                    element: Element {
                        format: Rg8Unorm,
                        offset: 6,
                    }
                },
            ];

            let rasterizer = hal::pso::Rasterizer {
                depth_clamping: false,
                polygon_mode: hal::pso::PolygonMode::Fill,
                cull_face: hal::pso::Face::NONE,
                front_face: hal::pso::FrontFace::CounterClockwise,
                depth_bias: None,
                conservative: false,
            };

            let depth_stencil = hal::pso::DepthStencilDesc {
                depth: hal::pso::DepthTest::Off,
                depth_bounds: false,
                stencil: hal::pso::StencilTest::Off,
            };

            let blender = {
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
                hal::pso::BlendDesc {
                    logic_op: Some(hal::pso::LogicOp::Copy),
                    targets: vec![hal::pso::ColorBlendDesc(hal::pso::ColorMask::ALL, blend_state)],
                }
            };

            let baked_states = hal::pso::BakedStates {
                viewport: Some(hal::pso::Viewport {
                    rect: extent.to_extent().rect(),
                    depth: (0.0..1.0),
                }),
                scissor: Some(extent.to_extent().rect()),
                blend_color: None,
                depth_bounds: None,
            };

            let bindings = vec![
                hal::pso::DescriptorSetLayoutBinding {
                    binding: 0,
                    ty: hal::pso::DescriptorType::UniformBuffer,
                    count: 2,
                    stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                    immutable_samplers: false,
                },
            ];

            let immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

            let descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> =
                vec![unsafe {
                    device
                        .create_descriptor_set_layout(bindings, immutable_samplers)
                        .map_err(|_| "Couldn't make a DescriptorSetLayout")?
                }];

            let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

            let layout = unsafe {
                device
                    .create_pipeline_layout(&descriptor_set_layouts, push_constants)
                    .map_err(|_| "Couldn't create a pipeline layout")?
            };

            let pipeline = {
                let desc = hal::pso::GraphicsPipelineDesc {
                    shaders,
                    rasterizer,
                    vertex_buffers,
                    attributes,
                    input_assembler,
                    blender,
                    depth_stencil,
                    multisampling: None,
                    baked_states,
                    layout: &layout,
                    subpass: hal::pass::Subpass {
                        index: 0,
                        main_pass: render_pass,
                    },
                    flags: hal::pso::PipelineCreationFlags::empty(),
                    parent: hal::pso::BasePipeline::None,
                };

                unsafe {
                    device
                        .create_graphics_pipeline(&desc, None)
                        .map_err(|_| "Couldn't create a graphics pipeline!")?
                }
            };

            Ok(
                FillPipeline {
                    descriptor_set_layouts,
                    layout,
                    pipeline,
                }
            )
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        (descriptor_set_layouts, pipeline_layout, graphics_pipeline)
    }
}

struct AlphaMulticolorPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

struct SolidMonochromePipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

struct PostprocessPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

struct StencilPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

struct ReprojectionPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pipeline_layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

pub struct HalRenderer {
    // Device
    pub device: HalDevice,

    dest_framebuffers: HalFramebuffer,
    fill_pipeline: FillPipeline,
    solid_multicolor_pipeline: SolidMulticolorPipeline,
    alpha_multicolor_pipeline: AlphaMulticolorPipeline,
    solid_monochrome_pipeline: SolidMonochromePipeline,
    alpha_monochrome_pipeline: AlphaMonochromePipeline,

    area_lut_texture: HalTexture,
    quad_vertex_positions_buffer: HalTexture,
    fill_vertex_array: HalTexture,
    mask_framebuffer: HalTexture,
    fill_colors_texture: HalTexture,

    // Postprocessing shader
    postprocess_source_framebuffer: HalFramebuffer,
    postprocess_pipeline: HalPipeline,
    gamma_lut_texture: HalTexture,

    // Stencil shader
    stencil_pipeline: HalPipeline,

    // Reprojection shader
    reprojection_pipeline: HalPipeline,

    // Rendering state
    mask_framebuffer_cleared: bool,
    buffered_fills: Vec<FillBatchPrimitive>,

    // Extra info
    use_depth: bool,
}

impl Renderer {
    pub fn new(
        window: &winit::Window, 
        instance_name: &str,
        resources: &dyn ResourceLoader,
        dest_framebuffer: HalFramebuffer,
    ) -> Renderer {
        let device = Device::new(window, instance_name);
        
        let fill_pipeline = Renderer::create_fill_pipeline(&device);

        let solid_multicolor_pipeline = SolidTileMulticolorProgram::new(&device, resources);
        let alpha_multicolor_pipeline = AlphaTileMulticolorProgram::new(&device, resources);
        let solid_monochrome_pipeline = SolidTileMonochromeProgram::new(&device, resources);
        let alpha_monochrome_pipeline = AlphaTileMonochromeProgram::new(&device, resources);

        let postprocess_pipeline = PostprocessProgram::new(&device, resources);
        let stencil_pipeline = StencilProgram::new(&device, resources);
        let reprojection_pipeline = ReprojectionProgram::new(&device, resources);

        let area_lut_texture = device.create_texture_from_png(resources, "area-lut");
        let gamma_lut_texture = device.create_texture_from_png(resources, "gamma-lut");

        let quad_vertex_positions_buffer = device.create_buffer();
        device.allocate_buffer(
            &quad_vertex_positions_buffer,
            BufferData::Memory(&QUAD_VERTEX_POSITIONS),
            BufferTarget::Vertex,
            BufferUploadMode::Static,
        );

        let fill_vertex_array =
            FillVertexArray::new(&device, &fill_pipeline, &quad_vertex_positions_buffer);
        let alpha_multicolor_tile_vertex_array = AlphaTileVertexArray::new(
            &device,
            &alpha_multicolor_pipeline.alpha_pipeline,
            &quad_vertex_positions_buffer,
        );
        let solid_multicolor_tile_vertex_array = SolidTileVertexArray::new(
            &device,
            &solid_multicolor_pipeline.solid_pipeline,
            &quad_vertex_positions_buffer,
        );
        let alpha_monochrome_tile_vertex_array = AlphaTileVertexArray::new(
            &device,
            &alpha_monochrome_pipeline.alpha_pipeline,
            &quad_vertex_positions_buffer,
        );
        let solid_monochrome_tile_vertex_array = SolidTileVertexArray::new(
            &device,
            &solid_monochrome_pipeline.solid_pipeline,
            &quad_vertex_positions_buffer,
        );
        let postprocess_vertex_array = PostprocessVertexArray::new(
            &device,
            &postprocess_pipeline,
            &quad_vertex_positions_buffer,
        );
        let stencil_vertex_array = StencilVertexArray::new(&device, &stencil_pipeline);
        let reprojection_vertex_array = ReprojectionVertexArray::new(
            &device,
            &reprojection_pipeline,
            &quad_vertex_positions_buffer,
        );

        let mask_framebuffer_size =
            Point2DI32::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT);
        let mask_framebuffer_texture =
            device.create_texture(TextureFormat::R16F, mask_framebuffer_size);
        let mask_framebuffer = device.create_framebuffer(mask_framebuffer_texture);

        let fill_colors_size =
            Point2DI32::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT);
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

    pub fn render_command(&mut self, command: &RenderCommand) {
        match *command {
            RenderCommand::Start { bounding_quad, object_count } => {
                if self.use_depth {
                    self.draw_stencil(&bounding_quad);
                }
                self.stats.object_count = object_count;
            }
            RenderCommand::AddShaders(ref shaders) => self.upload_shaders(shaders),
            RenderCommand::AddFills(ref fills) => self.add_fills(fills),
            RenderCommand::FlushFills => self.draw_buffered_fills(),
            RenderCommand::SolidTile(ref solid_tiles) => {
                let count = solid_tiles.len();
                self.stats.solid_tile_count += count;
                self.upload_solid_tiles(solid_tiles);
                self.draw_solid_tiles(count as u32);
            }
            RenderCommand::AlphaTile(ref alpha_tiles) => {
                let count = alpha_tiles.len();
                self.stats.alpha_tile_count += count;
                self.upload_alpha_tiles(alpha_tiles);
                self.draw_alpha_tiles(count as u32);
            }
            RenderCommand::Finish { .. } => {}
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
    pub fn set_main_framebuffer_size(&mut self, new_framebuffer_size: Point2DI32) {
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

    fn upload_shaders(&mut self, shaders: &[ObjectShader]) {
        let size = Point2DI32::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT);
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

    fn upload_solid_tiles(&mut self, solid_tiles: &[SolidTileBatchPrimitive]) {
        self.device.allocate_buffer(
            &self.solid_tile_vertex_array().vertex_buffer,
            BufferData::Memory(&solid_tiles),
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );
    }

    fn upload_alpha_tiles(&mut self, alpha_tiles: &[AlphaTileBatchPrimitive]) {
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
            color: Some(ColorF::transparent_black()),
            ..ClearParams::default()
        });
    }

    fn add_fills(&mut self, mut fills: &[FillBatchPrimitive]) {
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
                I32x4::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT, 0, 0).to_f32x4(),
            ),
        );
        self.device.set_uniform(
            &self.fill_pipeline.tile_size_uniform,
            UniformData::Vec2(I32x4::new(TILE_WIDTH as i32, TILE_HEIGHT as i32, 0, 0).to_f32x4()),
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
            UniformData::Vec2(I32x4::new(TILE_WIDTH as i32, TILE_HEIGHT as i32, 0, 0).to_f32x4()),
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
                I32x4::new(MASK_FRAMEBUFFER_WIDTH, MASK_FRAMEBUFFER_HEIGHT, 0, 0).to_f32x4(),
            ),
        );

        match self.render_mode {
            RenderMode::Multicolor => {
                self.device.bind_texture(&self.fill_colors_texture, 1);
                self.device.set_uniform(
                    &self
                        .alpha_multicolor_pipeline
                        .fill_colors_texture_uniform,
                    UniformData::TextureUnit(1),
                );
                self.device.set_uniform(
                    &self
                        .alpha_multicolor_pipeline
                        .fill_colors_texture_size_uniform,
                    UniformData::Vec2(
                        I32x4::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT, 0, 0)
                            .to_f32x4(),
                    ),
                );
            }
            RenderMode::Monochrome { .. } if self.postprocessing_needed() => {
                self.device.set_uniform(
                    &self.alpha_monochrome_pipeline.fill_color_uniform,
                    UniformData::Vec4(F32x4::splat(1.0)),
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
            UniformData::Vec2(F32x4::default()),
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
            UniformData::Vec2(I32x4::new(TILE_WIDTH as i32, TILE_HEIGHT as i32, 0, 0).to_f32x4()),
        );

        match self.render_mode {
            RenderMode::Multicolor => {
                self.device.bind_texture(&self.fill_colors_texture, 0);
                self.device.set_uniform(
                    &self
                        .solid_multicolor_pipeline
                        .fill_colors_texture_uniform,
                    UniformData::TextureUnit(0),
                );
                self.device.set_uniform(
                    &self
                        .solid_multicolor_pipeline
                        .fill_colors_texture_size_uniform,
                    UniformData::Vec2(
                        I32x4::new(FILL_COLORS_TEXTURE_WIDTH, FILL_COLORS_TEXTURE_HEIGHT, 0, 0)
                            .to_f32x4(),
                    ),
                );
            }
            RenderMode::Monochrome { .. } if self.postprocessing_needed() => {
                self.device.set_uniform(
                    &self.solid_monochrome_pipeline.fill_color_uniform,
                    UniformData::Vec4(F32x4::splat(1.0)),
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
            UniformData::Vec2(F32x4::default()),
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
                    UniformData::Vec4(F32x4::from_slice(&kernel.0)),
                );
            }
            None => {
                self.device.set_uniform(
                    &self.postprocess_pipeline.kernel_uniform,
                    UniformData::Vec4(F32x4::default()),
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

    fn draw_stencil(&self, quad_positions: &[Point3DF32]) {
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
        texture: &HalTexture,
        old_transform: &Transform3DF32,
        new_transform: &Transform3DF32,
    ) {
        self.bind_draw_framebuffer();

        self.device
            .bind_vertex_array(&self.reprojection_vertex_array.vertex_array);
        self.device.use_pipeline(&self.reprojection_pipeline.program);
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
            color: Some(ColorF::transparent_black()),
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

    fn stencil_state(&self) -> Option<StencilState> {
        if !self.use_depth {
            return None;
        }

        Some(StencilState {
            func: StencilFunc::Equal,
            reference: 1,
            mask: 1,
            write: false,
        })
    }

    fn draw_viewport(&self) -> RectI32 {
        let main_viewport = self.main_viewport();
        match self.render_mode {
            RenderMode::Monochrome {
                defringing_kernel: Some(..),
                ..
            } => {
                let scale = Point2DI32::new(3, 1);
                RectI32::new(Point2DI32::default(), main_viewport.size().scale_xy(scale))
            }
            _ => main_viewport,
        }
    }

    fn main_viewport(&self) -> RectI32 {
        match self.dest_framebuffer {
            DestFramebuffer::Default { viewport, .. } => viewport,
            DestFramebuffer::Other(ref framebuffer) => {
                let size = self
                    .device
                    .texture_size(self.device.framebuffer_texture(framebuffer));
                RectI32::new(Point2DI32::default(), size)
            }
        }
    }
}

struct FillVertexArray<D>
    where
        D: Device,
{
    vertex_array: D::VertexArray,
    vertex_buffer: D::Buffer,
}

impl<D> FillVertexArray<D>
    where
        D: Device,
{
    fn new(
        device: &D,
        fill_pipeline: &FillProgram<D>,
        quad_vertex_positions_buffer: &D::Buffer,
    ) -> FillVertexArray<D> {
        let vertex_array = device.create_vertex_array();

        let vertex_buffer = device.create_buffer();
        let vertex_buffer_data: BufferData<FillBatchPrimitive> =
            BufferData::Uninitialized(MAX_FILLS_PER_BATCH);
        device.allocate_buffer(
            &vertex_buffer,
            vertex_buffer_data,
            BufferTarget::Vertex,
            BufferUploadMode::Dynamic,
        );

        let tess_coord_attr = device.get_vertex_attr(&fill_pipeline.program, "TessCoord");
        let from_px_attr = device.get_vertex_attr(&fill_pipeline.program, "FromPx");
        let to_px_attr = device.get_vertex_attr(&fill_pipeline.program, "ToPx");
        let from_subpx_attr = device.get_vertex_attr(&fill_pipeline.program, "FromSubpx");
        let to_subpx_attr = device.get_vertex_attr(&fill_pipeline.program, "ToSubpx");
        let tile_index_attr = device.get_vertex_attr(&fill_pipeline.program, "TileIndex");

        device.bind_vertex_array(&vertex_array);
        device.use_pipeline(&fill_pipeline.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&tess_coord_attr, 2, VertexAttrType::U8, false, 0, 0, 0);
        device.bind_buffer(&vertex_buffer, BufferTarget::Vertex);
        device.configure_int_vertex_attr(
            &from_px_attr,
            1,
            VertexAttrType::U8,
            FILL_INSTANCE_SIZE,
            0,
            1,
        );
        device.configure_int_vertex_attr(
            &to_px_attr,
            1,
            VertexAttrType::U8,
            FILL_INSTANCE_SIZE,
            1,
            1,
        );
        device.configure_float_vertex_attr(
            &from_subpx_attr,
            2,
            VertexAttrType::U8,
            true,
            FILL_INSTANCE_SIZE,
            2,
            1,
        );
        device.configure_float_vertex_attr(
            &to_subpx_attr,
            2,
            VertexAttrType::U8,
            true,
            FILL_INSTANCE_SIZE,
            4,
            1,
        );
        device.configure_int_vertex_attr(
            &tile_index_attr,
            1,
            VertexAttrType::U16,
            FILL_INSTANCE_SIZE,
            6,
            1,
        );

        FillVertexArray {
            vertex_array,
            vertex_buffer,
        }
    }
}

struct AlphaTileVertexArray<D>
    where
        D: Device,
{
    vertex_array: D::VertexArray,
    vertex_buffer: D::Buffer,
}

impl<D> AlphaTileVertexArray<D>
    where
        D: Device,
{
    fn new(
        device: &D,
        alpha_pipeline: &AlphaTileProgram<D>,
        quad_vertex_positions_buffer: &D::Buffer,
    ) -> AlphaTileVertexArray<D> {
        let (vertex_array, vertex_buffer) = (device.create_vertex_array(), device.create_buffer());

        let tess_coord_attr = device.get_vertex_attr(&alpha_pipeline.program, "TessCoord");
        let tile_origin_attr = device.get_vertex_attr(&alpha_pipeline.program, "TileOrigin");
        let backdrop_attr = device.get_vertex_attr(&alpha_pipeline.program, "Backdrop");
        let object_attr = device.get_vertex_attr(&alpha_pipeline.program, "Object");
        let tile_index_attr = device.get_vertex_attr(&alpha_pipeline.program, "TileIndex");

        // NB: The object must be of type `I16`, not `U16`, to work around a macOS Radeon
        // driver bug.
        device.bind_vertex_array(&vertex_array);
        device.use_pipeline(&alpha_pipeline.program);
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

struct SolidTileVertexArray<D>
    where
        D: Device,
{
    vertex_array: D::VertexArray,
    vertex_buffer: D::Buffer,
}

impl<D> SolidTileVertexArray<D>
    where
        D: Device,
{
    fn new(
        device: &D,
        solid_pipeline: &SolidTileProgram<D>,
        quad_vertex_positions_buffer: &D::Buffer,
    ) -> SolidTileVertexArray<D> {
        let (vertex_array, vertex_buffer) = (device.create_vertex_array(), device.create_buffer());

        let tess_coord_attr = device.get_vertex_attr(&solid_pipeline.program, "TessCoord");
        let tile_origin_attr = device.get_vertex_attr(&solid_pipeline.program, "TileOrigin");
        let object_attr = device.get_vertex_attr(&solid_pipeline.program, "Object");

        // NB: The object must be of type short, not unsigned short, to work around a macOS
        // Radeon driver bug.
        device.bind_vertex_array(&vertex_array);
        device.use_pipeline(&solid_pipeline.program);
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

struct FillProgram<D>
    where
        D: Device,
{
    program: D::Program,
    framebuffer_size_uniform: D::Uniform,
    tile_size_uniform: D::Uniform,
    area_lut_uniform: D::Uniform,
}

impl<D> FillProgram<D>
    where
        D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> FillProgram<D> {
        let program = device.create_pipeline(resources, "fill");
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

struct SolidTileProgram<D>
    where
        D: Device,
{
    program: D::Program,
    framebuffer_size_uniform: D::Uniform,
    tile_size_uniform: D::Uniform,
    view_box_origin_uniform: D::Uniform,
}

impl<D> SolidTileProgram<D>
    where
        D: Device,
{
    fn new(device: &D, program_name: &str, resources: &dyn ResourceLoader) -> SolidTileProgram<D> {
        let program = device.create_pipeline_from_shader_names(
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

struct SolidTileMulticolorProgram<D>
    where
        D: Device,
{
    solid_pipeline: SolidTileProgram<D>,
    fill_colors_texture_uniform: D::Uniform,
    fill_colors_texture_size_uniform: D::Uniform,
}

impl<D> SolidTileMulticolorProgram<D>
    where
        D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> SolidTileMulticolorProgram<D> {
        let solid_pipeline = SolidTileProgram::new(device, "tile_solid_multicolor", resources);
        let fill_colors_texture_uniform =
            device.get_uniform(&solid_pipeline.program, "FillColorsTexture");
        let fill_colors_texture_size_uniform =
            device.get_uniform(&solid_pipeline.program, "FillColorsTextureSize");
        SolidTileMulticolorProgram {
            solid_pipeline,
            fill_colors_texture_uniform,
            fill_colors_texture_size_uniform,
        }
    }
}

struct SolidTileMonochromeProgram<D>
    where
        D: Device,
{
    solid_pipeline: SolidTileProgram<D>,
    fill_color_uniform: D::Uniform,
}

impl<D> SolidTileMonochromeProgram<D>
    where
        D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> SolidTileMonochromeProgram<D> {
        let solid_pipeline = SolidTileProgram::new(device, "tile_solid_monochrome", resources);
        let fill_color_uniform = device.get_uniform(&solid_pipeline.program, "FillColor");
        SolidTileMonochromeProgram {
            solid_pipeline,
            fill_color_uniform,
        }
    }
}

struct AlphaTileProgram<D>
    where
        D: Device,
{
    program: D::Program,
    framebuffer_size_uniform: D::Uniform,
    tile_size_uniform: D::Uniform,
    stencil_texture_uniform: D::Uniform,
    stencil_texture_size_uniform: D::Uniform,
    view_box_origin_uniform: D::Uniform,
}

impl<D> AlphaTileProgram<D>
    where
        D: Device,
{
    fn new(device: &D, program_name: &str, resources: &dyn ResourceLoader) -> AlphaTileProgram<D> {
        let program = device.create_pipeline_from_shader_names(
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

struct AlphaTileMulticolorProgram<D>
    where
        D: Device,
{
    alpha_pipeline: AlphaTileProgram<D>,
    fill_colors_texture_uniform: D::Uniform,
    fill_colors_texture_size_uniform: D::Uniform,
}

impl<D> AlphaTileMulticolorProgram<D>
    where
        D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> AlphaTileMulticolorProgram<D> {
        let alpha_pipeline = AlphaTileProgram::new(device, "tile_alpha_multicolor", resources);
        let fill_colors_texture_uniform =
            device.get_uniform(&alpha_pipeline.program, "FillColorsTexture");
        let fill_colors_texture_size_uniform =
            device.get_uniform(&alpha_pipeline.program, "FillColorsTextureSize");
        AlphaTileMulticolorProgram {
            alpha_pipeline,
            fill_colors_texture_uniform,
            fill_colors_texture_size_uniform,
        }
    }
}

struct AlphaTileMonochromeProgram<D>
    where
        D: Device,
{
    alpha_pipeline: AlphaTileProgram<D>,
    fill_color_uniform: D::Uniform,
}

impl<D> AlphaTileMonochromeProgram<D>
    where
        D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> AlphaTileMonochromeProgram<D> {
        let alpha_pipeline = AlphaTileProgram::new(device, "tile_alpha_monochrome", resources);
        let fill_color_uniform = device.get_uniform(&alpha_pipeline.program, "FillColor");
        AlphaTileMonochromeProgram {
            alpha_pipeline,
            fill_color_uniform,
        }
    }
}

struct PostprocessProgram<D>
    where
        D: Device,
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

impl<D> PostprocessProgram<D>
    where
        D: Device,
{
    fn new(device: &D, resources: &dyn ResourceLoader) -> PostprocessProgram<D> {
        let program = device.create_pipeline(resources, "post");
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
        viewport: RectI32,
        window_size: Point2DI32,
    },
    Other(D::Framebuffer),
}

impl<D> DestFramebuffer<D>
    where
        D: Device,
{
    #[inline]
    pub fn full_window(window_size: Point2DI32) -> DestFramebuffer<D> {
        let viewport = RectI32::new(Point2DI32::default(), window_size);
        DestFramebuffer::Default { viewport, window_size }
    }

    fn window_size(&self, device: &D) -> Point2DI32 {
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
        fg_color: ColorF,
        bg_color: ColorF,
        defringing_kernel: Option<DefringingKernel>,
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
