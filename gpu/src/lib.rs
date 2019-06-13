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
use image as img_crate;
use pathfinder_geometry as pfgeom;
use pathfinder_simd as pfsimd;
use takeable_option::Takeable;

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
    command_queue: hal::CommandQueue<Backend, hal::Graphics>,
    command_pool: hal::CommandPool<Backend, hal::Graphics>,
    draw_pipeline_state: crate::pipeline_state::DrawPipelineState<'a>,
}

impl<'a> GpuState<'a> {
    pub unsafe fn new(window: &'a winit::Window,
                      resource_loader: &'a dyn resources::ResourceLoader,
                      instance_name: &str,
                      fill_render_pass_description: crate::render_pass::RenderPassDescription,
                      draw_render_pass_description: crate::render_pass::RenderPassDescription,
                      postprocess_render_pass_description: crate::render_pass::RenderPassDescription,
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
                      max_quad_vertex_positions_buffer_size: u64,
                      max_fill_vertex_buffer_size: u64,
                      max_tile_vertex_buffer_size: u64,
                      monochrome: bool,
    ) -> GpuState<'a> {

        let instance = back::Instance::create(instance_name, 1);

        let mut surface = instance.create_surface(window);

        let mut adapter = GpuState::pick_adapter(&instance, &surface).unwrap();

        let (device, mut queue_group) =
            GpuState::create_device_with_graphics_queues(&mut adapter, &surface);

        let command_queue = queue_group.queues.drain(0..1).next().unwrap();

        let command_pool = device
            .create_command_pool_typed(
                &queue_group,
                hal::pool::CommandPoolCreateFlags::RESET_INDIVIDUAL,
            )
            .unwrap();

        let draw_pipeline_state = crate::pipeline_state::DrawPipelineState::new(&mut adapter, &device, &mut surface, window, resource_loader, &command_queue, command_pool, max_quad_vertex_positions_buffer_size as u64, draw_render_pass_description, fill_render_pass_description, postprocess_render_pass_description, fill_descriptor_set_layout_bindings, draw_descriptor_set_layout_bindings, postprocess_descriptor_set_layout_bindings, fill_pipeline_description, tile_solid_multicolor_pipeline_description, tile_solid_monochrome_pipeline_description, tile_alpha_multicolor_pipeline_description, tile_alpha_monochrome_pipeline_description, stencil_pipeline_description, postprocess_pipeline_description, fill_framebuffer_size, max_fill_vertex_buffer_size, max_tile_vertex_buffer_size, monochrome);

        GpuState {
            instance,
            surface,
            device,
            adapter,
            command_queue,

            command_pool,
            draw_pipeline_state,
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

    pub unsafe fn create_texture_from_png(&mut self, resources: &dyn resources::ResourceLoader, name: &str)  -> Image {
        let data = resources.slurp(&format!("textures/{}.png", name)).unwrap();
        let image = img_crate::load_from_memory_with_format(&data, img_crate::ImageFormat::PNG).unwrap().to_luma();
        let pixel_size= std::mem::size_of::<img_crate::Luma<u8>>();
        let size = pfgeom::basic::point::Point2DI32::new(image.width() as i32, image.height() as i32);

        Image::new_from_data(&self.adapter, &self.device, &mut self.command_pool, &mut self.command_queue, size, pixel_size, &data)
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
    pub fn from_transform_3d(transform: &pfgeom::basic::transform3d::Transform3DF32) -> UniformData {
        UniformData::Mat4([transform.c0, transform.c1, transform.c2, transform.c3])
    }
}

pub enum Memory<'a> {
    Reference(u64, hal::MemoryTypeId, &'a <Backend as hal::Backend>::Memory),
    Direct(<Backend as hal::Backend>::Memory),
}

pub struct Buffer<'a>{
    device: &'a <Backend as hal::Backend>::Device,
    usage: hal::buffer::Usage,
    buffer_size: u64,
    memory: Memory<'a>,
    requirements: hal::memory::Requirements,
    buffer: <Backend as hal::Backend>::Buffer,
    fence: Takeable<&'a <Backend as hal::Backend>::Fence>,
}

impl<'a> Buffer<'a> {
    unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        buffer_size: u64,
        usage: hal::buffer::Usage,
        memory: Option<Memory<'a>>,
        fence: Option<&'a <Backend as hal::Backend>::Fence>,
    ) -> Buffer<'a> {
        let mut buffer = device.create_buffer(buffer_size, usage).unwrap();
        let requirements = device.get_buffer_requirements(&mut buffer);

        let memory = match memory {
            Some(Memory::Reference(offset, mid, mref)) => {
                let memory_type = adapter.physical_device.memory_properties().memory_types[mid.0];
                assert!(requirements.type_mask & (1 << mid.0) != 0 && memory_type.properties.contains(hal::memory::Properties::CPU_VISIBLE));
                device.bind_buffer_memory(mref, offset, &mut buffer).unwrap();
                Memory::Reference(offset, mid, mref)
            },
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

                let mem = device
                    .allocate_memory(memory_type_id, buffer_size)
                    .unwrap();

                device.bind_buffer_memory(&mem, 0, &mut buffer).unwrap();
                Memory::Direct(mem)
            }
        };

        Buffer {
            device,
            usage,
            buffer_size,
            memory,
            requirements,
            buffer,
            fence: match fence {
                Some(f) => Takeable::new(f),
                _ => Takeable::new_empty(),
            },
        }
    }

    pub fn buffer(&self) -> &<Backend as hal::Backend>::Buffer {
        &self.buffer
    }

    pub unsafe fn upload_data<T>(&mut self, device: &<Backend as hal::Backend>::Device, data: &[T], fence: Option<&'a <Backend as hal::Backend>::Fence>) where T: Copy {
        assert!(data.len() <= self.buffer_size as usize);

        let mref = match self.memory {
            Memory::Direct(mem) => { &mem },
            Memory::Reference(_, _, mref) => { mref },
        };

        let mut writer = device
            .acquire_mapping_writer::<T>(mref, 0..self.requirements.size)
            .unwrap();
        writer[0..data.len()].copy_from_slice(data);
        device.release_mapping_writer(writer).unwrap();

        match Takeable::try_take(&mut self.fence) {
            Some(f) => {
                assert!(self.device.wait_for_fence(f, 20).unwrap());
            },
            _ => {},
        }

        match fence {
            Some(f) => {
                Takeable::insert(&mut self.fence, f);
            },
            _ => {},
        }
    }

    pub fn is_free(&mut self, timeout_ns: u64) -> bool {
        match Takeable::try_take(&mut self.fence) {
            Some(f) => {
                self.device.wait_for_fence(f, timeout_ns).unwrap()
            },
            _ => true,
        }
    }

    pub fn memory_ref(&self) -> &<Backend as hal::Backend>::Memory {
        match self.memory {
            Memory::Reference(_, _, mref) => {
                mref
            },
            Memory::Direct(mem) => {
                &mem
            }
        }
    }

    unsafe fn destroy_buffer(device: &<Backend as hal::Backend>::Device, buffer: Buffer){
        let Buffer { memory: mem, buffer: buf, .. } = buffer;
        device.destroy_buffer(buf);

        match mem {
            Memory::Reference(_, _, _) => {},
            Memory::Direct(m) => { device.free_memory(m); }
        }
    }
}

struct BufferPool<'a> {
    device: &'a <Backend as hal::Backend>::Device,
    usage: hal::buffer::Usage,
    pool: Vec<Buffer<'a>>,
    num_buffers: u8,
    buffer_size: u64,
    memory: <Backend as hal::Backend>::Memory,
    requirements: hal::memory::Requirements,
    pub submission_list: Vec<(std::ops::Range<hal::VertexCount>, std::ops::Range<hal::InstanceCount>, usize)>,
}

impl<'a> BufferPool<'a> {
    pub unsafe fn new(
        adapter: &hal::Adapter<Backend>,
        device: &'a <Backend as hal::Backend>::Device,
        buffer_size: u64,
        num_buffers: u8,
        usage: hal::buffer::Usage,
    ) -> BufferPool<'a> {
        let mut pool: Vec<Buffer> = vec![];

        let dummy_buffer = device.create_buffer(buffer_size*(num_buffers as u64), usage).unwrap();
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
            pool.push(Buffer::new(adapter, device, buffer_size, usage, Some(Memory::Reference(n*buffer_size, memory_type_id, &memory)), None));
        }

        BufferPool {
            device,
            usage,
            pool,
            num_buffers,
            buffer_size,
            memory,
            requirements,
            submission_list: vec![],
        }
    }

    pub unsafe fn get_free_buffer_index(&mut self) -> Option<usize> {
        for (ix, buf) in self.pool.iter_mut().enumerate() {
            if buf.is_free(5) {
                return Some(ix);
            }
        }

        None
    }

    pub unsafe fn upload_data<T>(&mut self, data: &[T], vertices: std::ops::Range<hal::VertexCount>, instances: std::ops::Range<hal::InstanceCount>, fence: Option<&'a <Backend as hal::Backend>::Fence>) -> bool where T: Copy  {
        if self.submission_list.len() == self.pool.len() {
            false
        } else {
            loop {
                match self.get_free_buffer_index() {
                    Some(ix) => {
                        self.pool[ix].upload_data(self.device, data, fence);
                        self.submission_list.push((vertices, instances, ix));
                        break;
                    }
                    _ => { continue; }
                }
            }
            true
        }
    }

    pub fn get_buffer(&self, ix: usize) -> &'a Buffer {
        &(self.pool[ix])
    }

    pub fn clear_submission_list(&mut self) {
        self.submission_list.clear();
    }

    unsafe fn destroy_buffer_pool(device: &<Backend as hal::Backend>::Device, buffer: BufferPool){
        let BufferPool { pool: p, memory: mem, .. } = buffer;
        for b in p.into_iter() {
            Buffer::destroy_buffer(device, b);
        }
        device.free_memory(mem);
    }
}

pub struct Image {
    image: <Backend as hal::Backend>::Image,
    requirements: hal::memory::Requirements,
    memory: <Backend as hal::Backend>::Memory,
    size: pfgeom::basic::point::Point2DI32,
    format: hal::format::Format,
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

    unsafe fn new_from_data(adapter: &hal::Adapter<Backend>, device: &<Backend as hal::Backend>::Device, command_pool: &mut hal::CommandPool<back::Backend, hal::Graphics>, command_queue: &mut hal::CommandQueue<back::Backend, hal::Graphics>, size: pfgeom::basic::point::Point2DI32, texel_size: usize, data: &[u8]) -> Image {
        let texture = Image::new(adapter, device, hal::format::Format::R8Uint, size);

        let staging_buffer =
            Buffer::new(adapter, device, (size.x() * size.y()) as u64, hal::buffer::Usage::TRANSFER_SRC, None, None);

        let mut writer = device
            .acquire_mapping_writer::<u8>(&staging_buffer.memory_ref(), 0..staging_buffer.requirements.size)
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
        self.size
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

        let subresource_range = hal::image::SubresourceRange {
            aspects: hal::format::Aspects::COLOR,
            levels: 0..1,
            layers: 0..1,
        };
        let image_view =
            device.create_image_view(
                &(image.image),
                hal::image::ViewKind::D2,
                texture_format,
                hal::format::Swizzle::NO,
                subresource_range).unwrap();

        let framebuffer = device.create_framebuffer(render_pass, vec![&image_view], hal::image::Extent { width: size.x() as u32, height: size.y() as u32, depth: 1 }).unwrap();

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

    pub unsafe fn destroy_framebuffer(device: &<Backend as hal::Backend>::Device, framebuffer: Framebuffer) {
        let Framebuffer { framebuffer: fb, image: img, image_view: imv} = framebuffer;
        device.destroy_image_view(imv);
        Image::destroy_image(device,img);
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

