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

use hal::{Device};
use crate::StencilFunc;
use crate::BlendState;
use crate::resources as pf_resources;
use crate::pipeline_layouts;

// TODO(pcwalton): Replace with `mem::size_of` calls?
const FILL_INSTANCE_SIZE: u32 = 8;
const SOLID_TILE_INSTANCE_SIZE: u32 = 6;
const MASK_TILE_INSTANCE_SIZE: u32 = 8;

pub unsafe fn create_fill_pipeline(
    pf_device: &crate::PfDevice,
    pipeline_layout: pipeline_layouts::MaskPipelineLayout,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module =
        pf_device.compose_shader_module(resources, "fill", crate::ShaderKind::Vertex);
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "fill", crate::ShaderKind::Fragment);

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

    let render_pass = pipeline_layout.get_render_pass();
    let layout = pipeline_layout.get_layout();

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
            layout: pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

    unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline)
}

pub unsafe fn create_solid_multicolor_pipeline(
    pf_device: &crate::PfDevice,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
    pipeline_layout: pipeline_layouts::DrawPipelineLayout,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module = pf_device.compose_shader_module(
        resources,
        "tile_solid_multicolor",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "tile_solid", crate::ShaderKind::Fragment);

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
        stencil: generate_stencil_test(crate::StencilFunc::Equal, 1, 1, false),
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
            layout: &pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: &pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

        unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline)
}


pub unsafe fn create_solid_monochrome_pipeline(
    pf_device: &crate::PfDevice,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
    pipeline_layout: pipeline_layouts::DrawPipelineLayout,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module = pf_device.compose_shader_module(
        resources,
        "tile_solid_monochrome",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "tile_solid", crate::ShaderKind::Fragment);

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
            layout: pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

    unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline)
}

pub unsafe fn create_alpha_multicolor_pipeline(
    pf_device: &crate::PfDevice,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module = pf_device.compose_shader_module(
        resources,
        "tile_alpha_multicolor",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "tile_alpha", crate::ShaderKind::Fragment);

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
            layout: pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

    unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline_layout)
}


pub unsafe fn create_alpha_monochrome_pipeline(
    pf_device: &crate::PfDevice,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module = pf_device.compose_shader_module(
        resources,
        "tile_alpha_monochrome",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "tile_alpha", crate::ShaderKind::Fragment);

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
            layout: pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

    unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline)
}

pub unsafe fn create_postprocess_pipeline(
    pf_device: &crate::PfDevice,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module =
        pf_device.compose_shader_module(resources, "post", crate::ShaderKind::Vertex);
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "post", crate::ShaderKind::Fragment);

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
            layout: pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

    unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline)
}


pub unsafe fn create_stencil_pipeline(
    pf_device: &crate::PfDevice,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn pf_resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> Result<<Backend as hal::Backend>::GraphicsPipeline, &'static str> {
    let vertex_shader_module =
        pf_device.compose_shader_module(resources, "stencil", crate::ShaderKind::Vertex);
    let fragment_shader_module =
        pf_device.compose_shader_module(resources, "stencil", crate::ShaderKind::Fragment);

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
                stencil_vertex_buffer_cursor,
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
            layout: pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout.get_render_pass(),
            },
            flags: hal::pso::PipelineCreationFlags::empty(),
            parent: hal::pso::BasePipeline::None,
        };

        unsafe {
            pf_device
                .device
                .create_graphics_pipeline(&desc, None)
                .unwrap()
        }
    };

    unsafe {
        pf_device.device.destroy_shader_module(vertex_shader_module);
        pf_device.device.destroy_shader_module(fragment_shader_module);
    }

    Ok(pipeline)
}


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
    func: crate::StencilFunc,
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
                crate::StencilFunc::Always => hal::pso::Comparison::Always,
                crate::StencilFunc::Equal => hal::pso::Comparison::Equal,
                crate::StencilFunc::NotEqual => hal::pso::Comparison::NotEqual,
            },
            mask_read: hal::pso::State::Static(mask),
            mask_write: mask_write,
            op_fail: hal::pso::StencilOp::Keep,
            op_depth_fail: hal::pso::StencilOp::Keep,
            op_pass: hal::pso::StencilOp::Keep,
            reference: hal::pso::State::Static(reference),
        },
        back: hal::pso::StencilFace {
            fun: match func {
                crate::StencilFunc::Always => hal::pso::Comparison::Always,
                crate::StencilFunc::Equal => hal::pso::Comparison::Equal,
                crate::StencilFunc::NotEqual => hal::pso::Comparison::NotEqual,
            },
            mask_read: hal::pso::State::Static(mask),
            mask_write: mask_write,
            op_fail: hal::pso::StencilOp::Keep,
            op_depth_fail: hal::pso::StencilOp::Keep,
            op_pass: hal::pso::StencilOp::Keep,
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
    hal::pso::DepthTest::On {
        fun: hal::pso::Comparison::Less,
        write: true,
    }
}
