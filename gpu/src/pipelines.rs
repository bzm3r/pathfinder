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

use crate::BlendState;
use resources;

// TODO(pcwalton): Replace with `mem::size_of` calls?
const FILL_INSTANCE_SIZE: u32 = 8;
const SOLID_TILE_INSTANCE_SIZE: u32 = 6;
const MASK_TILE_INSTANCE_SIZE: u32 = 8;

fn generate_tess_coord_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R8Uint,
                offset,
            },
        },
    )
}

fn generate_stencil_position_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R32Sfloat,
                offset,
            },
        },
    )
}

fn generate_px_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R8Uint,
                offset,
            },
        },
    )
}

fn generate_subpx_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R8Unorm,
                offset,
            },
        },
    )
}

fn generate_tile_index_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + 2 * num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R16Uint,
                offset,
            },
        },
    )
}

fn generate_solid_tile_origin_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + 2 * num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R16Sint,
                offset,
            },
        },
    )
}

fn generate_alpha_tile_origin_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R8Uint,
                offset,
            },
        },
    )
}

fn generate_object_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + 2 * num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R16Sint,
                offset,
            },
        },
    )
}

fn generate_backdrop_attribute_desc(
    binding: u32,
    location: u32,
    offset: u32,
    num_elements: u32,
) -> (u32, hal::pso::AttributeDesc) {
    (
        offset + num_elements,
        hal::pso::AttributeDesc {
            location,
            binding,
            element: hal::pso::Element {
                format: hal::format::Format::R8Sint,
                offset,
            },
        },
    )
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
            let blend_state = hal::pso::BlendState::Off;
            return hal::pso::BlendDesc {
                logic_op: None,
                targets: vec![],
            };
        }
    }
}

fn generate_depth_test_for_stencil_shader() -> hal::pso::DepthTest {
    hal::pso::DepthTest {
        fun: hal::pso::Comparison::Less,
        write: true,
    }
}

struct FillPipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
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
                stride: 0, // tightly packed
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // fill_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: FILL_INSTANCE_SIZE,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let quad_vertex_positions_buffer_cursor: u32 = 0;
            let fill_vertex_buffer_cursor: u32 = 0;

            let (quad_vertex_positions_buffer_cursor, tess_coord_attribute_desc) =
                generate_tess_coord_attribute_desc(0, 0, quad_vertex_positions_buffer_cursor, 2);
            let (fill_vertex_buffer_cursor, from_px_attribute_desc) =
                generate_px_attribute_desc(1, 1, fill_vertex_buffer_cursor, 1);
            let (fil_vertex_buffer_cursor, to_px_attribute_desc) =
                generate_px_attribute_desc(1, 2, fill_vertex_buffer_cursor, 1);
            let (fill_vertex_buffer_cursor, from_subpx_attribute_desc) =
                generate_subpx_attribute_desc(1, 3, fill_vertex_buffer_cursor, 2);
            let (fill_vertex_buffer_cursor, to_subpx_attribute_desc) =
                generate_subpx_attribute_desc(1, 4, fill_vertex_buffer_cursor, 2);
            let (fill_vertex_buffer_cursor, tile_index_attribute_desc) =
                generate_tile_index_attribute_desc(1, 5, fill_vertex_buffer_cursor, 1);

            vec![
                tess_coord_attribute_desc,
                from_px_attribute_desc,
                to_px_attribute_desc,
                from_subpx_attribute_desc,
                to_subpx_attribute_desc,
                tile_index_attribute_desc,
            ]
        };

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

        let blender = generate_blend_desc(BlendState::RGBOneAlphaOne);

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
        ];

        let fragment_shader_bindings = vec![
            // uAreaLUT
            hal::pso::DescriptorSetLayoutBinding {
            binding: 0,
            ty: hal::pso::DescriptorType::Sampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
            }
        ];

        let vertex_shader_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let fragment_shader_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();

        let pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    vertex_shader_bindings,
                    vertex_shader_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    fragment_shader_bindings,
                    fragment_shader_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
        ];

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = unsafe {
            device
                .create_pipeline_layout(&descriptor_set_layouts, push_constants)
                .map_err(|_| "Could not create pipeline layout.")?
        };

        let render_pass = FillPipelineRenderPass::new();

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
                    main_pass: &render_pass,
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
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
            render_pass,
        })
    }
}

struct FillPipelineRenderPass(<Backend as hal::Backend>::RenderPass);

impl FillPipelineRenderPass {
    fn new() -> FillPipelineRenderPass {
        let mask_texture = hal::pass::Attachment {
            format: Some(hal::format::Format::R16SFloat),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Clear,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::ColorAttachmentOptimal..hal::image::Layout::ShaderReadOnlyOptimal,
        };

        let subpass = hal::pass::SubpassDesc {
            colors: &[(0, hal::image::Layout::ColorAttachmentOptimal)],
            depth_stencil: None,
            inputs: &[],
            resolves: &[],
            preserves: &[],
        };

        FillPipelineRenderPass(
            device
                .create_render_pass(&[mask_texture], &[subpass], &[])
                .map_err(|_| "Could not create render pass.")?
        )
    }
}

struct SolidMulticolorPipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
}

impl SolidMulticolorPipeline {
    unsafe fn new(
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
                stride: 0,
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // solid_multicolor_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: SOLID_TILE_INSTANCE_SIZE,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let quad_vertex_positions_buffer_cursor: u32 = 0;
            let solid_multicolor_vertex_buffer_cursor: u32 = 0;

            let (quad_vertex_positions_buffer_cursor, tess_coord_attribute_desc) =
                generate_tess_coord_attribute_desc(0, 0, quad_vertex_positions_buffer_cursor, 2);
            let (solid_multicolor_vertex_buffer_cursor, tile_origin_attribute_desc) =
                generate_solid_tile_origin_attribute_desc(
                    1,
                    1,
                    solid_multicolor_vertex_buffer_cursor,
                    2,
                );
            let (solid_multicolor_vertex_buffer_cursor, object_attribute_desc) =
                generate_object_attribute_desc(1, 2, solid_multicolor_vertex_buffer_cursor, 1);

            vec![
                tess_coord_attribute_desc,
                tile_origin_attribute_desc,
                object_attribute_desc,
            ]
        };

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

        let blender = generate_blend_desc(BlendState::Off);

        let baked_states = hal::pso::BakedStates {
            viewport: Some(hal::pso::Viewport {
                rect: extent.to_extent().rect(),
                depth: (0.0..1.0),
            }),
            scissor: Some(extent.to_extent().rect()),
            blend_color: None,
            depth_bounds: None,
        };

        let tile_solid_vertex_shader_uniform_inputs = vec![
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

        let tile_multicolor_vertex_shader_uniform_inputs = vec![
            // uFillColorsTexture (sampler2D)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::Sampler,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uFillColorsTexture (texture)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::SampledImage,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let tile_solid_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let tile_multicolor_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    tile_solid_vertex_shader_uniform_inputs,
                    tile_solid_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    tile_multicolor_vertex_shader_uniform_inputs,
                    tile_multicolor_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
        ];

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = device
            .create_pipeline_layout(&descriptor_set_layouts, push_constants)
            .map_err(|_| "Could not create pipeline layout.")?;

        let render_pass = SolidPipelineRenderPass::new();

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
                    main_pass: &render_pass,
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
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
            render_pass,
        })
    }
}

struct SolidMonochromePipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
}

impl SolidMonochromePipeline {
    unsafe fn new(
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
                stride: 0,
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // solid_multicolor_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: SOLID_TILE_INSTANCE_SIZE,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let quad_vertex_positions_buffer_cursor: u32 = 0;
            let solid_multicolor_vertex_buffer_cursor: u32 = 0;

            let (quad_vertex_positions_buffer_cursor, tess_coord_attribute_desc) =
                generate_tess_coord_attribute_desc(0, 0, quad_vertex_positions_buffer_cursor, 2);
            let (solid_multicolor_vertex_buffer_cursor, tile_origin_attribute_desc) =
                generate_solid_tile_origin_attribute_desc(
                    1,
                    1,
                    solid_multicolor_vertex_buffer_cursor,
                    2,
                );
            let (solid_multicolor_vertex_buffer_cursor, object_attribute_desc) =
                generate_object_attribute_desc(1, 2, solid_multicolor_vertex_buffer_cursor, 1);

            vec![
                tess_coord_attribute_desc,
                tile_origin_attribute_desc,
                object_attribute_desc,
            ]
        };

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

        let blender = generate_blend_desc(BlendState::Off);

        let baked_states = hal::pso::BakedStates {
            viewport: Some(hal::pso::Viewport {
                rect: extent.to_extent().rect(),
                depth: (0.0..1.0),
            }),
            scissor: Some(extent.to_extent().rect()),
            blend_color: None,
            depth_bounds: None,
        };

        let tile_solid_vertex_shader_uniform_inputs = vec![
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

        let tile_monochrome_vertex_shader_uniform_inputs = vec![
            // uFillColor (vec4)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let tile_solid_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let tile_monochrome_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    tile_solid_vertex_shader_uniform_inputs,
                    tile_solid_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    tile_monochrome_vertex_shader_uniform_inputs,
                    tile_monochrome_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
        ];

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = device
            .create_pipeline_layout(&descriptor_set_layouts, push_constants)
            .map_err(|_| "Could not create pipeline layout.")?;

        let render_pass = SolidPipelineRenderPass::new();

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
                    main_pass: &render_pass,
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
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
            render_pass,
        })
    }
}

struct SolidPipelineRenderPass(<Backend as hal::Backend>::RenderPass);

impl SolidPipelineRenderPass {
    fn new() -> SolidPipelineRenderPass {
        let input_attachment = hal::pass::Attachment {
            format: Some(hal::format::Format::R16SFloat),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Clear,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::ShaderReadOnlyOptimal..hal::image::Layout::ShaderReadOnlyOptimal,
        };

        let dest = hal::pass::Attachment {
            format: Some(hal::format::Format::Rgba8Srgb),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Clear,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::Undefined..hal::image::Layout::ColorAttachmentOptimal,
        };

        let subpass = hal::pass::SubpassDesc {
            colors: &[(1, hal::image::Layout::ColorAttachmentOptimal)],
            depth_stencil: None,
            inputs: &[(0, hal::image::Layout::ShaderReadOnlyOptimal)],
            resolves: &[],
            preserves: &[],
        };

        SolidPipelineRenderPass(
            device
                .create_render_pass(&[input_attachment, dest], &[subpass], &[])
                .map_err(|_| "Could not create render pass.")?
        )
    }
}

struct AlphaMulticolorPipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
}

impl AlphaMulticolorPipeline {
    unsafe fn new(
        device: &device::Device,
        pf_resources: &dyn resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<AlphaMulticolorPipeline, &str> {
        let vertex_shader_module = device.compose_shader_module(
            resources,
            "tile_alpha_multicolor",
            device::ShaderKind::Vertex,
        );
        let fragment_shader_module =
            device.compose_shader_module(resources, "tile_alpha", device::ShaderKind::Fragment);

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
                stride: 0,
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // alpha_multicolor_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: MASK_TILE_INSTANCE_SIZE,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let quad_vertex_positions_buffer_cursor: u32 = 0;
            let alpha_multicolor_vertex_buffer_cursor: u32 = 0;

            let (quad_vertex_positions_buffer_cursor, tess_coord_attribute_desc) =
                generate_tess_coord_attribute_desc(0, 0, quad_vertex_positions_buffer_cursor, 2);
            let (alpha_multicolor_vertex_buffer_cursor, tile_origin_attribute_desc) =
                generate_alpha_tile_origin_attribute_desc(
                    1,
                    1,
                    alpha_multicolor_vertex_buffer_cursor,
                    3,
                );
            let (alpha_multicolor_vertex_buffer_cursor, backdrop_attribute_desc) =
                generate_backdrop_attribute_desc(1, 1, alpha_multicolor_vertex_buffer_cursor, 1);
            let (alpha_multicolor_vertex_buffer_cursor, object_attribute_desc) =
                generate_object_attribute_desc(1, 1, alpha_multicolor_vertex_buffer_cursor, 2);
            let (alpha_multicolor_vertex_buffer_cursor, tile_index_attribute_desc) =
                generate_tile_index_attribute_desc(1, 2, alpha_multicolor_vertex_buffer_cursor, 2);

            vec![
                tess_coord_attribute_desc,
                tile_origin_attribute_desc,
                backdrop_attribute_desc,
                object_attribute_desc,
                tile_index_attribute_desc,
            ]
        };

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

        let blender = generate_blend_desc(BlendState::RGBOneAlphaOneMinusSrcAlpha);

        let baked_states = hal::pso::BakedStates {
            viewport: Some(hal::pso::Viewport {
                rect: extent.to_extent().rect(),
                depth: (0.0..1.0),
            }),
            scissor: Some(extent.to_extent().rect()),
            blend_color: None,
            depth_bounds: None,
        };

        let tile_alpha_vertex_shader_uniform_inputs = vec![
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
            // uStencilTextureSize;
            hal::pso::DescriptorSetLayoutBinding {
                binding: 2,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            //uViewBoxOrigin
            hal::pso::DescriptorSetLayoutBinding {
                binding: 3,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let tile_multicolor_vertex_shader_uniform_inputs = vec![
            // uFillColorsTexture (sampler2D)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::Sampler,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            // uFillColorsTexture (texture)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::SampledImage,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let tile_alpha_fragment_shader_uniform_inputs = vec![
            // uStencilTexture (sampler2D)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::Sampler,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
        ];

        let pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    tile_alpha_vertex_shader_uniform_inputs,
                    Vec::<<Backend as hal::Backend>::Sampler>::new(),
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    tile_multicolor_vertex_shader_uniform_inputs,
                    Vec::<<Backend as hal::Backend>::Sampler>::new(),
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    tile_alpha_fragment_shader_uniform_inputs,
                    Vec::<<Backend as hal::Backend>::Sampler>::new(),
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
        ];

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = device
            .create_pipeline_layout(&descriptor_set_layouts, push_constants)
            .map_err(|_| "Could not create pipeline layout.")?;

        let render_pass = AlphaPipelineRenderPass::new();

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
                    main_pass: &render_pass,
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
            }
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(AlphaMulticolorPipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
            render_pass,
        })
    }
}

struct AlphaMonochromePipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
}

impl AlphaMonochromePipeline {
    unsafe fn new(
        device: &device::Device,
        pf_resources: &dyn resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<AlphaMonochromePipeline, &str> {
        let vertex_shader_module = device.compose_shader_module(
            resources,
            "tile_alpha_monochrome",
            device::ShaderKind::Vertex,
        );
        let fragment_shader_module =
            device.compose_shader_module(resources, "tile_alpha", device::ShaderKind::Fragment);

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
                stride: 0,
                rate: hal::pso::VertexInputRate::Vertex,
            },
            // alpha_multicolor_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 1,
                stride: MASK_TILE_INSTANCE_SIZE,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let quad_vertex_positions_buffer_cursor: u32 = 0;
            let alpha_multicolor_vertex_buffer_cursor: u32 = 0;

            let (quad_vertex_positions_buffer_cursor, tess_coord_attribute_desc) =
                generate_tess_coord_attribute_desc(0, 0, quad_vertex_positions_buffer_cursor, 2);
            let (alpha_multicolor_vertex_buffer_cursor, tile_origin_attribute_desc) =
                generate_alpha_tile_origin_attribute_desc(
                    1,
                    1,
                    alpha_multicolor_vertex_buffer_cursor,
                    3,
                );
            let (alpha_multicolor_vertex_buffer_cursor, backdrop_attribute_desc) =
                generate_backdrop_attribute_desc(1, 1, alpha_multicolor_vertex_buffer_cursor, 1);
            let (alpha_multicolor_vertex_buffer_cursor, object_attribute_desc) =
                generate_object_attribute_desc(1, 1, alpha_multicolor_vertex_buffer_cursor, 1);
            let (alpha_multicolor_vertex_buffer_cursor, tile_index_attribute_desc) =
                generate_tile_index_attribute_desc(1, 2, alpha_multicolor_vertex_buffer_cursor, 1);

            vec![
                tess_coord_attribute_desc,
                tile_origin_attribute_desc,
                backdrop_attribute_desc,
                object_attribute_desc,
                tile_index_attribute_desc,
            ]
        };

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

        let blender = generate_blend_desc(BlendState::RGBOneAlphaOneMinusSrcAlpha);

        let baked_states = hal::pso::BakedStates {
            viewport: Some(hal::pso::Viewport {
                rect: extent.to_extent().rect(),
                depth: (0.0..1.0),
            }),
            scissor: Some(extent.to_extent().rect()),
            blend_color: None,
            depth_bounds: None,
        };

        let tile_alpha_vertex_shader_uniform_inputs = vec![
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
            // uStencilTextureSize;
            hal::pso::DescriptorSetLayoutBinding {
                binding: 2,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
            //uViewBoxOrigin
            hal::pso::DescriptorSetLayoutBinding {
                binding: 3,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let tile_monochrome_vertex_shader_uniform_inputs = vec![
            // uFillColor (vec4)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::VERTEX,
                immutable_samplers: false,
            },
        ];

        let tile_alpha_fragment_shader_uniform_inputs = vec![
            // uStencilTexture (sampler2D)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::Sampler,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
        ];

        let tile_alpha_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let tile_monochrome_immutable_samplers = Vec::<<Backend as hal::Backend>::Sampler>::new();
        let tile_alpha_fragment_shader_immutable_samplers =
            Vec::<<Backend as hal::Backend>::Sampler>::new();
        let pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    tile_alpha_vertex_shader_uniform_inputs,
                    tile_alpha_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    tile_monochrome_vertex_shader_uniform_inputs,
                    tile_monochrome_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    tile_alpha_fragment_shader_uniform_inputs,
                    tile_alpha_fragment_shader_immutable_samplers,
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
        ];

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = device
            .create_pipeline_layout(&descriptor_set_layouts, push_constants)
            .map_err(|_| "Could not create pipeline layout.")?;

        let render_pass = AlphaPipelineRenderPass::new();

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
                    main_pass: &render_pass,
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
            }
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(AlphaMonochromePipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
            render_pass,
        })
    }
}

struct AlphaPipelineRenderPass(<Backend as hal::Backend>::RenderPass);

impl AlphaPipelineRenderPass {
    fn new() -> AlphaPipelineRenderPass {
        let mask_texture = hal::pass::Attachment {
            format: Some(hal::format::Format::R16SFloat),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Clear,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::ShaderReadOnlyOptimal..hal::image::Layout::ShaderReadOnlyOptimal,
        };

        let dest = hal::pass::Attachment {
            format: Some(hal::format::Format::Rgba8Srgb),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Clear,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::Undefined..hal::image::Layout::ColorAttachmentOptimal,
        };

        let subpass = hal::pass::SubpassDesc {
            colors: &[(1, hal::image::Layout::ColorAttachmentOptimal)],
            depth_stencil: None,
            inputs: &[(0, hal::image::Layout::ShaderReadOnlyOptimal)],
            resolves: &[],
            preserves: &[],
        };

        AlphaPipelineRenderPass(
            device
                .create_render_pass(&[mask_texture, dest], &[subpass], &[])
                .map_err(|_| "Could not create render pass.")?
        )
    }
}

struct PostprocessPipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
}

impl PostprocessPipeline {
    unsafe fn new(
        device: &device::Device,
        pf_resources: &dyn resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<PostprocessPipeline, &str> {
        let vertex_shader_module =
            device.compose_shader_module(resources, "post", device::ShaderKind::Vertex);
        let fragment_shader_module =
            device.compose_shader_module(resources, "post", device::ShaderKind::Fragment);

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
                stride: 0,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let quad_vertex_positions_buffer_cursor: u32 = 0;

            let (quad_vertex_positions_buffer_cursor, tess_coord_attribute_desc) =
                generate_tess_coord_attribute_desc(0, 0, quad_vertex_positions_buffer_cursor, 2);

            vec![
                // called aPositions in shader, but has the same form
                tess_coord_attribute_desc,
            ]
        };

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

        let blender = generate_blend_desc(BlendState::Off);

        let baked_states = hal::pso::BakedStates {
            viewport: Some(hal::pso::Viewport {
                rect: extent.to_extent().rect(),
                depth: (0.0..1.0),
            }),
            scissor: Some(extent.to_extent().rect()),
            blend_color: None,
            depth_bounds: None,
        };

        let post_fragment_shader_uniform_inputs = vec![
            // uSourceSize
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
            // uFGColor
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
            // uBGColor
            hal::pso::DescriptorSetLayoutBinding {
                binding: 2,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
            // uGammaCorrectionEnabled
            hal::pso::DescriptorSetLayoutBinding {
                binding: 3,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
        ];

        let post_fragment_samplers = vec![
            // uSource (sampler2D)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::Sampler,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
            // uGammaLUT (sampler2D)
            hal::pso::DescriptorSetLayoutBinding {
                binding: 1,
                ty: hal::pso::DescriptorType::Sampler,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
        ];

        let post_convolve_fragment_shader_uniform_inputs = vec![
            // uKernel
            hal::pso::DescriptorSetLayoutBinding {
                binding: 0,
                ty: hal::pso::DescriptorType::UniformBuffer,
                count: 1,
                stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
                immutable_samplers: false,
            },
        ];

        let pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout> = vec![
            device
                .create_descriptor_set_layout(
                    post_fragment_shader_uniform_inputs,
                    Vec::<<Backend as hal::Backend>::Sampler>::new(),
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    post_gamma_correct_fragment_shader_uniform_inputs,
                    Vec::<<Backend as hal::Backend>::Sampler>::new(),
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
            device
                .create_descriptor_set_layout(
                    post_convolve_fragment_shader_uniform_inputs,
                    Vec::<<Backend as hal::Backend>::Sampler>::new(),
                )
                .map_err(|_| "Could not make descriptor set layout.")?,
        ];

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = device
            .create_pipeline_layout(&descriptor_set_layouts, push_constants)
            .map_err(|_| "Could not create pipeline layout.")?;

        let render_pass = PostprocessPipelineRenderPass::new();

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
                    main_pass: PostprocessPipelineRenderPass::new(),
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
            }
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(PostprocessPipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
            render_pass,
        })
    }
}

struct PostprocessPipelineRenderPass(<Backend as hal::Backend>::RenderPass);

impl PostprocessPipelineRenderPass {
    fn new() -> PostprocessPipelineRenderPass {
        let dest = hal::pass::Attachment {
            format: Some(hal::format::Format::Rgba8Srgb),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Load,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::ColorAttachmentOptimal..hal::image::Layout::ColorAttachmentOptimal,
        };


        let subpass = hal::pass::SubpassDesc {
            colors: &[(0, hal::image::Layout::ColorAttachmentOptimal)],
            depth_stencil: None,
            inputs: &[],
            resolves: &[],
            preserves: &[],
        };

        PostprocessPipelineRenderPass(
            device
                .create_render_pass(&[dest], &[subpass], &[])
                .map_err(|_| "Could not create render pass.")?
        )
    }
}

struct StencilPipeline {
    pub descriptor_set_layouts: Vec<<Backend as hal::Backend>::DescriptorSetLayout>,
    pub layout: <Backend as hal::Backend>::PipelineLayout,
    pub pipeline: <Backend as hal::Backend>::GraphicsPipeline,
    pub render_pass: <Backend as hal::Backend>::RenderPass,
}

impl StencilPipeline {
    unsafe fn new(
        device: &device::Device,
        pf_resources: &dyn resources::ResourceLoader,
        extent: hal::window::Extent2D,
    ) -> Result<StencilPipeline, &str> {
        let vertex_shader_module =
            device.compose_shader_module(resources, "stencil", device::ShaderKind::Vertex);
        let fragment_shader_module =
            device.compose_shader_module(resources, "stencil", device::ShaderKind::Fragment);

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
            // stencil_vertex_buffer
            hal::pso::VertexBufferDesc {
                binding: 0,
                stride: 16,
                rate: hal::pso::VertexInputRate::Vertex,
            },
        ];

        let attributes: Vec<hal::pso::AttributeDesc> = {
            let stencil_vertex_buffer_cursor: u32 = 0;

            let (stencil_vertex_buffer_cursor, position_attribute_desc) =
                generate_stencil_position_attribute_desc(
                    0,
                    0,
                    quad_vertex_positions_buffer_cursor,
                    3,
                );

            vec![
                // called aPositions in shader, but has the same form
                position_attribute_desc,
            ]
        };

        let rasterizer = hal::pso::Rasterizer {
            depth_clamping: false,
            polygon_mode: hal::pso::PolygonMode::Fill,
            cull_face: hal::pso::Face::NONE,
            front_face: hal::pso::FrontFace::CounterClockwise,
            depth_bias: None,
            conservative: false,
        };

        let depth_stencil = hal::pso::DepthStencilDesc {
            depth: generate_depth_test_for_stencil_shader(),
            depth_bounds: false,
            stencil: generate_stencil_test(hal::pso::Comparison::Always, 1, 1, true),
        };

        let blender = generate_blend_desc(BlendState::Off);

        let baked_states = hal::pso::BakedStates {
            viewport: Some(hal::pso::Viewport {
                rect: extent.to_extent().rect(),
                depth: (0.0..1.0),
            }),
            scissor: Some(extent.to_extent().rect()),
            blend_color: None,
            depth_bounds: None,
        };

        let descriptor_set_layouts = Vec::<<Backend as hal::Backend>::DescriptorSetLayout>::new();

        let push_constants = Vec::<(hal::pso::ShaderStageFlags, core::ops::Range<u32>)>::new();

        let layout = device
            .create_pipeline_layout(&descriptor_set_layouts, push_constants)
            .map_err(|_| "Could not create pipeline layout.")?;

        let render_pass = StencilPipelineRenderPass::new();

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
                    main_pass: &render_pass,
                },
                flags: hal::pso::PipelineCreationFlags::empty(),
                parent: hal::pso::BasePipeline::None,
            };

            unsafe {
                device
                    .create_graphics_pipeline(&desc, None)
                    .map_err(|_| "Could not create graphics pipeline.")?
            }
        };

        unsafe {
            device.destroy_shader_module(vertex_shader_module);
            device.destroy_shader_module(fragment_shader_module);
        }

        Ok(StencilPipeline {
            descriptor_set_layouts,
            layout,
            pipeline,
            render_pass,
        })
    }
}

struct StencilPipelineRenderPass(<Backend as hal::Backend>::RenderPass);

impl StencilPipelineRenderPass {
    fn new() -> StencilPipelineRenderPass {
        let dest = hal::pass::Attachment {
            format: Some(hal::format::Format::Rgba8Srgb),
            samples: 0,
            ops: hal::pass::AttachmentOps {
                load: hal::pass::AttachmentLoadOp::Load,
                store: hal::pass::AttachmentStoreOp::Store,
            },
            stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
            layouts: hal::image::Layout::ColorAttachmentOptimal..hal::image::Layout::ColorAttachmentOptimal,
        };


        let subpass = hal::pass::SubpassDesc {
            colors: &[(0, hal::image::Layout::ColorAttachmentOptimal)],
            depth_stencil: None,
            inputs: &[],
            resolves: &[],
            preserves: &[],
        };

        StencilPipelineRenderPass(
            device
                .create_render_pass(&[dest], &[subpass], &[])
                .map_err(|_| "Could not create render pass.")?
        )
    }
}
