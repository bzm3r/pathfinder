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

use crate::resources::ResourceLoader;
use image::ImageFormat;
use pathfinder_geometry::basic::point::Point2DI32;
use pathfinder_geometry::basic::rect::RectI32;
use pathfinder_geometry::basic::transform3d::Transform3DF32;
use pathfinder_geometry::color::ColorF;
use pathfinder_simd::default::F32x4;
use rustache::HashBuilder;
use std::time::Duration;

pub mod pipelines;
pub mod resources;

pub struct PfDevice {
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

impl PfDevice {
    unsafe fn new(window: &winit::Window, instance_name: &str) -> HalDevice {
        let instance = back::Instance::create(instance_name, 1);

        let mut surface = instance.create_surface(window);

        let adapter = PfDevice::pick_adapter(&instance, &surface);

        let (mut device, queue_group) =
            PfDevice::create_device_with_graphics_queues(&adapter, &surface);

        let (swapchain, extent, backbuffer, swapchain_framebuffer_format, frames_in_flight) =
            PfDevice::create_swapchain(&adapter, &device, &mut surface, None);

        let (image_available_semaphores, render_finished_semaphores, in_flight_fences) =
            PfDevice::create_synchronizers(&device, frames_in_flight);

        let swapchain_image_views: Vec<_> =
            PfDevice::create_image_views(device, backbuffer, swapchain_framebuffer_format);

        let swapchain_framebuffer = PfDevice::create_framebuffer(
            &device,
            &render_pass,
            &swapchain_image_views,
            extent,
        );

        let mut command_pool = device
            .create_command_pool_typed(
                &queue_group,
                hal::pool::CommandPoolCreateFlags::RESET_INDIVIDUAL,
            )
            .map_err(|_| "Could not create raw command pool.")?;

        let submission_command_buffers: Vec<_> = swapchain_framebuffer
            .iter()
            .map(|_| command_pool.acquire_command_buffer())
            .collect();
    }

    fn pick_adapter(
        instance: &back::Instance,
        surface: &<Backend as hal::Backend>::Surface,
    ) -> Result<hal::Adapter<Backend>, &'static str> {
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
            use hal::window::PresentMode::{Fifo, Immediate, Mailbox, Relaxed};
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
        device: &<Backend as hal::Backend>::Device,
        backbuffer: hal::window::Backbuffer<Backend>,
        requested_format: hal::format::Format,
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

    pub fn create_framebuffer(
        device: &<Backend as hal::Backend>::Device,
        render_pass: &<Backend as hal::Backend>::RenderPass,
        image_views: &[<Backend as hal::Backend>::ImageView],
        extent: hal::window::Extent2D,
    ) -> Vec<<Backend as hal::Backend>::Framebuffer> {
        let mut framebuffer: Vec<<Backend as hal::Backend>::Framebuffer> = Vec::new();

        unsafe {
            for image_view in image_views.iter() {
                swapchain_framebuffer.push(
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

    pub fn compose_shader_module(
        &self,
        resources: &dyn ResourceLoader,
        name: &str,
        shader_kind: ShaderKind,
    ) -> <Backend as hal::Backend>::ShaderModule {
        let shader_kind_char = match kind {
            ShaderKind::Vertex => 'v',
            ShaderKind::Fragment => 'f',
        };

        let source = resources
            .slurp(&format!("shaders/{}.{}s.glsl", name, shader_kind_char))
            .unwrap();

        let mut load_include_tile_alpha_vertex =
            |_| load_shader_include(resources, "tile_alpha_vertex");
        let mut load_include_tile_monochrome =
            |_| load_shader_include(resources, "tile_monochrome");
        let mut load_include_tile_multicolor =
            |_| load_shader_include(resources, "tile_multicolor");
        let mut load_include_tile_solid_vertex =
            |_| load_shader_include(resources, "tile_solid_vertex");
        let mut load_include_post_convolve = |_| load_shader_include(resources, "post_convolve");
        let mut load_include_post_gamma_correct =
            |_| load_shader_include(resources, "post_gamma_correct");
        let template_input = HashBuilder::new()
            .insert_lambda(
                "include_tile_alpha_vertex",
                &mut load_include_tile_alpha_vertex,
            )
            .insert_lambda("include_tile_monochrome", &mut load_include_tile_monochrome)
            .insert_lambda("include_tile_multicolor", &mut load_include_tile_multicolor)
            .insert_lambda(
                "include_tile_solid_vertex",
                &mut load_include_tile_solid_vertex,
            )
            .insert_lambda("include_post_convolve", &mut load_include_post_convolve)
            .insert_lambda(
                "include_post_gamma_correct",
                &mut load_include_post_gamma_correct,
            );

        let mut compiler = shaderc::Compiler::new().ok_or("shaderc not found!")?;

        let artifact = compiler
            .compile_into_spirv(
                str::from_utf8(&source).unwrap(),
                match kind {
                    ShaderKind::Vertex => shaderc::ShaderKind::Vertex,
                    ShaderKind::Fragment => shaderc::ShaderKind::Fragment,
                },
                "",
                "main",
                None,
            )
            .map_err(|_| "Could not compile shader.")?;

        let shader_module = unsafe {
            self.device
                .create_shader_module(artifact.as_binary_u8())
                .map_err(|_| "Could not make shader_module")?
        };

        shader_module
    }

    pub unsafe fn create_vertex_buffer(&self, size: u64) -> Buffer {
        Buffer::new(&self.adapter, &self.device, size, hal::buffer::Usage::Vertex)
    }

    pub unsafe fn create_texture_from_png(&self, resources: &dyn ResourceLoader, name: &str)  -> Texture {
        let data = resources.slurp(&format!("textures/{}.png", name)).unwrap();
        let image = image::load_from_memory_with_format(&data, ImageFormat::PNG).unwrap().to_luma();
        let size = Point2DI32::new(image.width() as i32, image.height() as i32);

        Texture::new_from_data(&self.adapter, &self.device, size, &data)
    }
}

fn load_shader_include(resources: &dyn ResourceLoader, include_name: &str) -> String {
    let resource = resources
        .slurp(&format!("shaders/{}.inc.glsl", include_name))
        .unwrap();
    String::from_utf8_lossy(&resource).to_string()
}

#[derive(Clone, Copy, Debug)]
pub enum TextureFormat {
    R8,
    R16F,
    RGBA8,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderKind {
    Vertex,
    Fragment,
}

pub enum GlslStyle {
    Spirv,
    OpenGL,
}

#[derive(Clone, Copy)]
pub enum UniformData {
    Int(i32),
    Mat2(F32x4),
    Mat4([F32x4; 4]),
    Vec2(F32x4),
    Vec4(F32x4),
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
    pub color: Option<ColorF>,
    pub rect: Option<RectI32>,
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

impl UniformData {
    #[inline]
    pub fn from_transform_3d(transform: &Transform3DF32) -> UniformData {
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
    unsafe fn upload_data<T>(&self, device: <Backend as hal::Backend>::Device, data: &[T]) where T: Copy {
        // should we assert!(data.len() < self.requirements.size) ?
        let mut writer = device
            .acquire_mapping_writer::<T>(&self.memory, 0..&self.requirements.size)
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        writer.release_mapping_writer(vertices).unwrap();
    }

    unsafe fn new(
        adapter: &<Backend as hal::Backend>::Adapter,
        device: &<Backend as hal::Backend>::Device,
        size: u64,
        usage: hal::buffer::Usage,
    ) -> Buffer {
        let mut buffer = device.create_buffer(size, usage).map_err(|_| {
            format!(
                "Unable to create buffer of size {} and usage type{}",
                size, usage
            )
        })?;

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
            .ok_or("Adapter cannot supply required memory.")?;

        let memory = device
            .allocate_memory(memory_type_id, requirements.size)
            .map_err(|_| "Could not allocate memory on device.")?;

        device
            .bind_buffer_memory(&memory, 0, &mut buffer)
            .map_err(|_| "Could not bind memory to device.")?;

        Buffer {
            usage,
            buffer,
            memory,
            requirements,
        }
    }
    
    unsafe fn destroy_buffer(device: &<Backend as hal::Backend>::Device, buffer: Buffer){
        let Buffer { buffer: buff, memory: mem, .. } = buffer;
        device.destroy_buffer(buff);
        device.free_memory(mem);
    }
}

pub struct Texture {
    image: <Backend as hal::Backend>::Image,
    requirements: hal::memory::Requirements,
    memory: <Backend as hal::Backend>::Memory,
    image_view: <Backend as hal::Backend>::ImageView,
    sampler: <Backend as hal::Backend>::Sampler,
}

impl Texture {
    fn new(
        device: &<Backend as hal::Backend>::Device,
        format: TextureFormat,
        size: Point2DI32,
    ) -> Texture {
        // 3. Make an image with transfer_dst and SAMPLED usage
        let mut image = device
            .create_image(
                hal::image::Kind::D1(data.len(), 1),
                1,
                Format::Rgba8Srgb,
                hal::image::Tiling::Optimal,
                hal::image::Usage::TRANSFER_DST | hal::image::Usage::SAMPLED,
                hal::image::ViewCapabilities::empty(),
            )
            .map_err(|_| "Couldn't create the image!")?;

        // 4. allocate memory for the image and bind it
        let requirements = device.get_image_requirements(&image);

        let memory_type_id = adapter
            .physical_device
            .memory_properties()
            .memory_types
            .iter()
            .enumerate()
            .find(|&(id, memory_type)| {
                // BIG NOTE: THIS IS DEVICE LOCAL NOT CPU VISIBLE
                requirements.type_mask & (1 << id) != 0
                    && memory_type.properties.contains(Properties::DEVICE_LOCAL)
            })
            .map(|(id, _)| hal::adpater::MemoryTypeId(id))
            .ok_or("Couldn't find a memory type to support the image!")?;

        let memory = device
            .allocate_memory(memory_type_id, requirements.size)
            .map_err(|_| "Couldn't allocate image memory!")?;

        device
            .bind_image_memory(&memory, 0, &mut image)
            .map_err(|_| "Couldn't bind the image memory!")?;

        // 5. create image view and sampler
        let image_view = device
            .create_image_view(
                &image,
                hal::image::ViewKind::D2,
                Format::Rgba8Srgb,
                hal::format::Swizzle::NO,
                hal::image::SubresourceRange {
                    aspects: Aspects::COLOR,
                    levels: 0..1,
                    layers: 0..1,
                },
            )
            .map_err(|_| "Couldn't create the image view!")?;

        let sampler = device
            .create_sampler(hal::image::SamplerInfo::new(
                hal::image::Filter::Nearest,
                hal::image::WrapMode::Tile,
            ))
            .map_err(|_| "Couldn't create the sampler!")?;

        Texture {
            image,
            requirements,
            memory,
            image_view,
            sampler,
        }
    }

    unsafe fn new_from_data(adapter: &<Backend as hal::Backend>::Adapter, device: &<Backend as hal::Backend>::Device, size: Point2DI32, data: &[u8]) -> Texture {
        let texture = Texture::create_texture(hal::format::R8Unorm, size);

        let staging_buffer =
            Buffer::new(&adapter, device, (size.x * size.y) as u64, BufferUsage::TRANSFER_SRC)?;

        let mut writer = device
            .acquire_mapping_writer::<u8>(&staging_bundle.memory, 0..staging_buffer.requirements.size)
            .map_err(|_| "Could not acquire mapping writer.")?;
        writer[0..data.len()].copy_from_slice(data);
        device
            .release_mapping_writer(writer)
            .map_err(|_| "Couldn't release the mapping writer to the staging buffer!")?;

        let mut cmd_buffer = command_pool.acquire_command_buffer::<gfx_hal::command::OneShot>();
        cmd_buffer.begin();

        // 7. Use a pipeline barrier to transition the image from empty/undefined
        //    to TRANSFER_WRITE/TransferDstOptimal
        let image_barrier = gfx_hal::memory::Barrier::Image {
            states: (gfx_hal::image::Access::empty(), hal::image::Layout::Undefined)
                ..(
                gfx_hal::image::Access::TRANSFER_WRITE,
                hal::image::Layout::TransferDstOptimal,
            ),
            target: &texture.image,
            families: None,
            range: hal::image::SubresourceRange {
                aspects: Aspects::COLOR,
                levels: 0..1,
                layers: 0..1,
            },
        };

        cmd_buffer.pipeline_barrier(
            PipelineStage::TOP_OF_PIPE..PipelineStage::TRANSFER,
            gfx_hal::memory::Dependencies::empty(),
            &[image_barrier],
        );

        // 8. perform copy from staging buffer to image
        cmd_buffer.copy_buffer_to_image(
            &staging_bundle.buffer,
            &texture.image,
            hal::image::Layout::TransferDstOptimal,
            &[gfx_hal::command::BufferImageCopy {
                buffer_offset: 0,
                buffer_width: (row_pitch / pixel_size) as u32,
                buffer_height: img.height(),
                image_layers: gfx_hal::image::SubresourceLayers {
                    aspects: Aspects::COLOR,
                    level: 0,
                    layers: 0..1,
                },
                image_offset: gfx_hal::image::Offset { x: 0, y: 0, z: 0 },
                image_extent: gfx_hal::image::Extent {
                    width: img.width(),
                    height: img.height(),
                    depth: 1,
                },
            }],
        );

        // 9. use pipeline barrier to transition the image to SHADER_READ access/
        //    ShaderReadOnlyOptimal layout
        let image_barrier = gfx_hal::memory::Barrier::Image {
            states: (
                gfx_hal::image::Access::TRANSFER_WRITE,
                hal::image::Layout::TransferDstOptimal,
            )
                ..(
                gfx_hal::image::Access::SHADER_READ,
                hal::image::Layout::ShaderReadOnlyOptimal,
            ),
            target: &texture.image,
            families: None,
            range: hal::image::SubresourceRange {
                aspects: Aspects::COLOR,
                levels: 0..1,
                layers: 0..1,
            },
        };

        cmd_buffer.pipeline_barrier(
            PipelineStage::TRANSFER..PipelineStage::FRAGMENT_SHADER,
            gfx_hal::memory::Dependencies::empty(),
            &[image_barrier],
        );

        // 10. Submit the cmd buffer to queue and wait for it
        cmd_buffer.finish();

        let upload_fence = device
            .create_fence(false)
            .map_err(|_| "Couldn't create an upload fence!")?;
        command_queue.submit_nosemaphores(Some(&cmd_buffer), Some(&upload_fence));

        device
            .wait_for_fence(&upload_fence, core::u64::MAX)
            .map_err(|_| "Couldn't wait for the fence!")?;

        device.destroy_fence(upload_fence);

        texture
    }

    pub fn destroy_texture(device: &<Backend as hal::Backend>::Device, texture: Texture) {
        let Texture { image_view: imview, memory: mem, .. } = texture;
        device.destroy_image_view(imview);
        device.free_memory(mem);
    }
}

pub struct Framebuffer {
    framebuffers: Vec<<Backend as hal::Backend>::Framebuffer>,
}
