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

use resources;

struct FillPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

impl FillPipeline {
    unsafe fn new(
        device: &crate::PfDevice,
        pf_resources: &dyn gpu_resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<FillPipeline, &str> {
        let vertex_shader_module =
            device.compose_shader_module(resources, "fill", device::ShaderKind::Vertex);
        let fragment_shader_module =
            device.compose_shader_module(resources, "fill", device::ShaderKind::Fragment);

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
            hull: None,
            domain: None,
            geometry: None,
            fragment: Some(fs_entry),
        };

        let input_assembler = hal::pso::InputAssemblerDesc::new(hal::Primitive::TriangleList);

        let vertex_buffers: Vec<hal::pso::VertexBufferDesc> = vec![
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
            // aTessCoord
            hal::pso::AttributeDesc {
                location: 0,
                binding: 0,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 0,
                },
            },
            // from_px_attr
            hal::pso::AttributeDesc {
                location: 1,
                binding: 1,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 0,
                },
            },
            // to_px_attr
            hal::pso::AttributeDesc {
                location: 2,
                binding: 1,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 1,
                },
            },
            // from_subpx_attr
            hal::pso::AttributeDesc {
                location: 3,
                binding: 1,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 2,
                },
            },
            // to_subpx_attr
            hal::pso::AttributeDesc {
                location: 4,
                binding: 1,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 4,
                },
            },
            // tile_index_attr
            hal::pso::AttributeDesc {
                location: 5,
                binding: 0,
                element: hal::pso::Element {
                    format: R16Unorm,
                    offset: 6,
                },
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
                targets: vec![hal::pso::ColorBlendDesc(
                    hal::pso::ColorMask::ALL,
                    blend_state,
                )],
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

        // why multiple descriptor sets instead of one descriptor set with multiple bindings?
        // descriptor sets should group attachments by usage frequency
        let vertex_shader_bindings = vec![
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let fragment_shader_bindings = vec![hal::pso::DescriptorSetLayoutBinding {
            binding: 0,
            ty: hal::pso::DescriptorType::Sampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        }];

        let vertex_shader_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let fragment_shader_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    vertex_shader_bindings,
                    vertex_shader_immutable_samplers,
                )
                .map_err(|_| "Couldn't make a DescriptorSetLayout")?,
            device
                .create_descriptor_set_layout(
                    fragment_shader_bindings,
                    fragment_shader_immutable_samplers,
                )
                .map_err(|_| "Couldn't make a DescriptorSetLayout")?,
        ];

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

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(FillPipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
        })
    }
}

struct SolidMulticolorPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
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
            op_pass: StencilOp,
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
            op_pass: StencilOp,
            reference: hal::pso::State::Static(reference),
        },
    }
}

impl SolidMulticolorPipeline {
    fn new(
        device: &device::Device,
        pf_resources: &dyn resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<SolidMulticolorPipeline, &str> {
        let vertex_shader_module = device.compose_shader_module(
            resources,
            "tile_solid_multicolor",
            device::ShaderKind::Vertex,
        );
        let fragment_shader_module =
            device.compose_shader_module(resources, "tile_solid", device::ShaderKind::Fragment);

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
            hull: Some(None),
            domain: None,
            geometry: None,
            fragment: Some(fs_entry),
        };

        let input_assembler = hal::pso::InputAssemblerDesc::new(hal::Primitive::TriangleList);

        let vertex_buffers: Vec<hal::pso::VertexBufferDesc> = vec![
            // quad_vertex_positions_buffer
            hal::pso::VertexBufferDesc {
                binding: 0,
                stride: 1, // 8 bits
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // solid_multicolor_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: 8, // 64 bits
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = vec![
            // aTessCoord
            hal::pso::AttributeDesc {
                location: 0,
                binding: 0,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 0,
                },
            },
            // aTileOrigin
            hal::pso::AttributeDesc {
                location: 1,
                binding: 1,
                element: hal::pso::Element {
                    format: R16Int,
                    offset: 0,
                },
            },
            // aTileOrigin
            hal::pso::AttributeDesc {
                location: 2,
                binding: 1,
                element: hal::pso::Element {
                    format: R16Int,
                    offset: 4,
                },
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
            stencil: generate_stencil_test(StencilFunc::Equal, 1, 1, false),
        };

        let blender = {
            let blend_state = hal::pso::BlendState::Off;
            hal::pso::BlendDesc {
                logic_op: None,
                targets: vec![],
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

        let vertex_shader_bindings = vec![
            // uFramebufferSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uTileSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uViewboxOrigin
            hal::pso::DescriptorSetLayoutBinding {
                binding: 2,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let vertex_shader_uniform_inputs = vec![
            // uFramebufferSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uTileSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uViewboxOrigin
            hal::pso::DescriptorSetLayoutBinding {
                binding: 2,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let vertex_shader_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> =
            vec![unsafe {
                device
                    .create_descriptor_set_layout(
                        vertex_shader_bindings,
                        vertex_shader_immutable_samplers,
                    )
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

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(SolidMulticolorPipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
        })
    }
}

struct AlphaMulticolorPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

struct SolidMonochromePipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

impl SolidMonochromePipeline {
    fn new(
        device: &device::Device,
        pf_resources: &dyn resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<SolidMonochromePipeline, &str> {
        let vertex_shader_module = device.compose_shader_module(
            resources,
            "tile_solid_monochrome",
            device::ShaderKind::Vertex,
        );
        let fragment_shader_module =
            device.compose_shader_module(resources, "tile_solid", device::ShaderKind::Fragment);

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
            hull: Some(None),
            domain: None,
            geometry: None,
            fragment: Some(fs_entry),
        };

        let input_assembler = hal::pso::InputAssemblerDesc::new(hal::Primitive::TriangleList);

        let vertex_buffers: Vec<hal::pso::VertexBufferDesc> = vec![
            // quad_vertex_positions_buffer
            hal::pso::VertexBufferDesc {
                binding: 0,
                stride: 1, // 8 bits
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // solid_monochrome_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: 8, // 64 bits
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = vec![
            // aTessCoord
            hal::pso::AttributeDesc {
                location: 0,
                binding: 0,
                element: hal::pso::Element {
                    format: R8Unorm,
                    offset: 0,
                },
            },
            // aTileOrigin
            hal::pso::AttributeDesc {
                location: 1,
                binding: 1,
                element: hal::pso::Element {
                    format: R16Int,
                    offset: 0,
                },
            },
            // aTileOrigin
            hal::pso::AttributeDesc {
                location: 2,
                binding: 1,
                element: hal::pso::Element {
                    format: R16Int,
                    offset: 4,
                },
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
            stencil: generate_stencil_test(StencilFunc::Equal, 1, 1, false),
        };

        let blender = {
            let blend_state = hal::pso::BlendState::Off;
            hal::pso::BlendDesc {
                logic_op: None,
                targets: vec![],
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

        let vertex_shader_bindings = vec![
            // uFramebufferSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uTileSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uViewboxOrigin
            hal::pso::DescriptorSetLayoutBinding {
                binding: 2,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let vertex_shader_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> =
            vec![unsafe {
                device
                    .create_descriptor_set_layout(
                        vertex_shader_bindings,
                        vertex_shader_immutable_samplers,
                    )
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

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(SolidMonochromePipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
        })
    }
}

struct PostprocessPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
}

struct StencilPipeline {
    descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    layout: <Backend as hal::Backend>::PipelineLayout,
    pipeline: <Backend as hal::Backend>::GraphicsPipeline,
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
