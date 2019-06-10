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

use hal::{Instance, Surface, Capability, Device, QueueFamily, PhysicalDevice, };
use crate::resources::ResourceLoader;
use image as img_crate;
use pathfinder_geometry as pfgeom;
use pathfinder_simd as pfsimd;
use rustache::HashBuilder;
use rustache::Render;

pub mod resources;
pub mod pipeline;
pub mod render_pass;
mod pipeline_state;
mod batch_primitives;

pub struct GpuState<'a> {
    instance: back::Instance,
    surface: <Backend as hal::Backend>::Surface,
    pub device: <Backend as hal::Backend>::Device,
    adapter: hal::Adapter<Backend>,
    queue_group: hal::queue::QueueGroup<Backend, hal::Graphics>,
    pub extent: hal::window::Extent2D,

    command_pool: hal::CommandPool<Backend, hal::Graphics>,

    fill_renderer: crate::pipeline_state::FillPipelineState<'a>,
    draw_renderer: crate::pipeline_state::DrawRenderer<'a>,
}

impl<'a> GpuState<'a> {
    pub unsafe fn new(window: &winit::Window, 
                      instance_name: &str,
                      fill_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>, 
                      draw_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>, 
                      postprocess_descriptor_set_layout_bindings: Vec<hal::pso::DescriptorSetLayoutBinding>,
                      fill_pipeline_description: crate::pipeline::PipelineDescription,
                      tile_solid_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
                      tile_solid_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
                      tile_alpha_monochrome_pipeline_description: crate::pipeline::PipelineDescription,
                      tile_alpha_multicolor_pipeline_description: crate::pipeline::PipelineDescription,
                      postprocess_pipeline_description: crate::pipeline::PipelineDescription,
                      stencil_pipeline_description: crate::pipeline::PipelineDescription,
                      fill_framebuffer_size: pfgeom::basic::point::Point2DI32,
                      max_quad_vertex_positions_buffer_size: usize,
                      max_fill_vertex_buffer_size: usize,
                      max_solid_tile_vertex_buffer_size: usize,
                      mask_alpha_tile_vertex_buffer_size: usize) -> GpuState {

        let instance = back::Instance::create(instance_name, 1);

        let mut surface = instance.create_surface(window);

        let mut adapter = GpuState::pick_adapter(&instance, &surface).unwrap();

        let (device, queue_group) =
            GpuState::create_device_with_graphics_queues(&mut adapter, &surface);


        let draw_render_pass = GpuState::create_render_pass(&device, draw_render_pass_desc);
        let postprocess_render_pass = GpuState::create_render_pass(&device, postprocess_render_pass_desc);

        let swapchain_state= crate::pipeline_state::DrawPipelineState::new(&mut adapter, &device, &surface, &draw_render_pass, max_frames_in_flight, &command_pool);
        let max_frames_in_flight = swapchain_state.max_frames_in_flight();

        let in_flight_draw_fences: Vec<<Backend as hal::Backend>::Fence> = 0..max_frames_in_flight.iter().map(|_| device.create_fence().unwrap()).collect();
        let in_flight_fill_fences: Vec<<Backend as hal::Backend>::Fence> = 0..max_frames_in_flight.iter().map(|_| device.create_fence().unwrap()).collect();

        let draw_pipeline_layout = PipelineLayoutState::new(&device, draw_descriptor_set_layout_bindings, draw_render_pass);
        let postprocess_pipeline_layout = PipelineLayoutState::new(&device, postprocess_descriptor_set_layout_bindings, postprocess_render_pass);

        let mut command_pool = device
            .create_command_pool_typed(
                &queue_group,
                hal::pool::CommandPoolCreateFlags::RESET_INDIVIDUAL,
            )
            .unwrap();

        let fill_renderer = crate::pipeline_state::FillPipelineState::new(&adapter, &device, &fill_pipeline_layout, resources, &command_queue, &command_pool, &quad_vertex_positions_buffer, fill_framebuffer_size, max_fill_vertex_buffer_size as u64, &in_flight_fill_fences, swapchain_state.current_index_ref());
        let draw_renderer = crate::pipeline_state::DrawRenderer::new(&adapter, &device, &draw_pipeline_layout, resources, extent, &command_queue, &command_pool, &quad_vertex_positions_buffer, max_tile_vertex_buffer_size, monochrome);

        GpuState {
            instance,
            surface,
            device,
            adapter,
            queue_group,
            extent,

            command_pool,

            fill_renderer,
            draw_renderer,
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

pub enum Memory<'a> {
    Reference(&'a <Backend as hal::Backend>::Memory),
    Direct(<Backend as hal::Backend>::Memory),
}

pub struct Buffer<'a>{
    usage: hal::buffer::Usage,
    buffer_size: u64,
    memory: Memory<'a>,
    requirements: hal::memory::Requirements,
    buffer: <Backend as hal::Backend>::Buffer,
    fence: &'a <Backend as hal::Backend>::Fence,
}

impl<'a> Buffer<'a> {
    pub unsafe fn upload_data<T>(&self, device: &<Backend as hal::Backend>::Device, data: &[T]) where T: Copy {
        assert!(data.len() <= self.buffer_size as usize);
        let mut writer = device
            .acquire_mapping_writer::<T>(&self.memory, 0..self.requirements.size)
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        device.release_mapping_writer(writer).unwrap();
    }

    unsafe fn new<'a>(
        adapter: &hal::Adapter<Backend>,
        device: &<Backend as hal::Backend>::Device,
        buffer_size: u64,
        usage: hal::buffer::Usage,
        memory_ref: Option<(u64, &'a <Backend as hal::Backend>::Memory)>,
    ) -> Buffer<'a> {
        let buffer = device.create_buffer(buffer_size, usage).unwrap();
        let fence = device.create_fence().unwrap();

        let requirements = device.get_buffer_requirements(&pool[0]);

        let memory = match memory_ref {
            Some((offset, mref)) => {
                device.bind_buffer_memory(mref, offset, buffer).unwrap();
                Memory::Reference(mref)
            },
            None => {
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

                let mem = device
                    .allocate_memory(memory_type_id, (num_buffers as u64)*buffer_size)
                    .unwrap();

                device.bind_buffer_memory(&mem, 0, buffer).unwrap();
                Memory::Direct(mem)
            }
        };

        Buffer {
            usage,
            buffer_size,
            memory,
            requirements,
            buffer,
            fence,
        }
    }

    pub fn set_fence(&mut self, new_fence: &'a <Backend as hal::Backend>::Fence) {
        self.fence = new_fence;
    }

    pub fn buffer(&self) -> &<Backend as hal::Backend>::Buffer {
        &self.buffer
    }
    pub fn fence(&self) -> &<Backend as hal::Backend>::Fence {
        self.fence
    }

    unsafe fn destroy_buffer(device: &<Backend as hal::Backend>::Device, buffer: Buffer){
        let Buffer { memory: mem, buffer: buf, .. } = buffer;
        device.destroy_buffer(buf);

        match mem {
            Memory::Reference(_) => {},
            Memory::Direct(m) => { device.free_memory(m); }
        }
    }
}

struct RawBufferPool<'a> {
    device: &'a <Backend as hal::Backend>::Device,
    usage: hal::buffer::Usage,
    pool: Vec<Buffer<'a>>,
    num_buffers: u8,
    buffer_size: u64,
    memory: <Backend as hal::Backend>::Memory,
    requirements: hal::memory::Requirements,
    current_frame_index: &'a usize,
}

impl<'a> RawBufferPool<'a> {
    pub unsafe fn new<'a>(
        adapter: &hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        buffer_size: u64,
        num_buffers: u8,
        usage: hal::buffer::Usage,
        current_frame_index: &'a usize,
    ) -> RawBufferPool {
        let mut pool: Vec<<Backend as hal::Backend>::BufferPool> = vec![];
        let mut fences: Vec<<Backend as hal::Backend>::Fence> = vec![];

        let dummy_buffer = device.create_buffer(buffer_size*(num_buffers as u64), usage);
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

        let memory = device
            .allocate_memory(memory_type_id, requirements.size)
            .unwrap();

        for n in 0..(num_buffers as u64) {
            pool.push(Buffer::new(adapter, device, buffer_size, usage, Some((n*buffer_size, &memory))));
        }

        RawBufferPool {
            device,
            usage,
            pool,
            num_buffers,
            buffer_size,
            memory,
            requirements,
            current_frame_index,
        }
    }

    pub unsafe fn upload_data<T>(&mut self, data: &[T]) -> Option<&<Backend as hal::Backend>::Buffer> where T: Copy  {
        let buf = &self.pool[self.current_frame_index];
        buf.upload_data(self.device, data);
        buf.buffer()
    }

    unsafe fn destroy_buffer_pool(device: &<Backend as hal::Backend>::Device, buffer: RawBufferPool){
        let RawBufferPool { pool: p, memory: mem, .. } = buffer;
        for b in p.into_iter() {
            device.destroy_buffer(b);
        }
        device.free_memory(mem);
    }
}

pub struct VertexBufferPool<'a> {
    buffer_pool: RawBufferPool<'a>,
    pub submission_list: Vec<(std::ops::Range<hal::VertexCount>, std::ops::Range<hal::InstanceCount>, &'a <Backend as hal::Backend>::Buffer)>,
    current_frame_index: &'a usize,
}

impl<'a> VertexBufferPool<'a> {
    pub unsafe fn new<'a>(
        adapter: &hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        buffer_size: u64,
        num_buffers: u8,
        usage: hal::buffer::Usage,
        current_frame_index: &'a usize,
    ) -> VertexBufferPool<'a> {
        let buffer_pool = RawBufferPool::new(adapter, device, buffer_size, num_buffers, hal::buffer::Usage::VERTEX, current_frame_index);

        VertexBufferPool {
            buffer_pool,
            submission_list: vec![],
            current_frame_index
        }
    }

    pub unsafe fn submit_data_to_buffer<T>(&mut self, data: &[T], vertices: std::ops::Range<hal::VertexCount>, instances: std::ops::Range<hal::InstanceCount>) where T: Copy {
        let buf= self.buffer_pool.upload_data(data).unwrap();
        self.submission_list.push((vertices, instances, buf));
    }

    pub unsafe fn clear_submission_list(&mut self) {
        self.submission_list.clear();

    }

    pub unsafe fn destroy_vertex_buffer_pool(device: &<Backend as hal::Backend>::Device, vertex_buffer_pool: VertexBufferPool) {
        let VertexBufferPool { buffer_pool: buf, .. } = vertex_buffer_pool;
        RawBufferPool::destroy_buffer_pool(device, buf);
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
                    && mem_type.properties.contains(hal::memory::Properties::DEVICE_LOCAL)
            })
            .unwrap()
            .into();

        let memory = device
            .allocate_memory(upload_type, requirements.size)
            .unwrap();

        device
            .bind_image_memory(&memory, 0, &mut image)
            .unwrap();

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
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        device
            .release_mapping_writer(writer)
            .unwrap();

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
            .unwrap();

        command_queue.submit_nosemaphores(Some(&cmd_buffer), Some(&upload_fence));

        device
            .wait_for_fence(&upload_fence, core::u64::MAX)
            .unwrap();

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

