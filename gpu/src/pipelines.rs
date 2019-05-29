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

use crate::{resources, pipeline_layout_descs};
use crate::{StencilFunc, BlendState};
use pathfinder_geometry as pfgeom;

use rustache;

// TODO(pcwalton): Replace with `mem::size_of` calls?
const FILL_INSTANCE_SIZE: u32 = 8;
const SOLID_TILE_INSTANCE_SIZE: u32 = 6;
const MASK_TILE_INSTANCE_SIZE: u32 = 8;

unsafe fn compose_shader_module(
    device: &<Backend as hal::Backend>::Device,
    resources: &dyn resources::ResourceLoader,
    name: &str,
    shader_kind: ShaderKind,
) -> <Backend as hal::Backend>::ShaderModule {
    let shader_kind_char = match shader_kind {
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
    let template_input = rustache::HashBuilder::new()
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

    let mut output = std::io::Cursor::new(vec![]);
    template_input.render(std::str::from_utf8(&source).unwrap(), &mut output).unwrap();
    let source = output.into_inner();

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

    let shader_module = device.create_shader_module(artifact.as_binary_u8())
        .unwrap();

    shader_module
}

pub unsafe fn create_fill_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &<Backend as hal::Backend>::PipelineLayout,
    resources: &dyn resources::ResourceLoader,
    size: pfgeom::basic::point::Point2DI32,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module =
        compose_shader_module(device, resources, "fill", crate::ShaderKind::Vertex);
    let fragment_shader_module =
        compose_shader_module(device, resources, "fill", crate::ShaderKind::Fragment);

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

    let mask_framebuffer_size_rect = hal::pso::Rect {
        x: 0,
        y: 0,
        w: size.x() as i16,
        h: size.y() as i16,
    };
    
    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: mask_framebuffer_size_rect,
            depth: (0.0..1.0),
        }),
        scissor: Some(mask_framebuffer_size_rect),
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

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module); 
    device.destroy_shader_module(fragment_shader_module);

    pipeline
}

pub unsafe fn create_solid_tile_multicolor_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &<Backend as hal::Backend>::PipelineLayout,
    resources: &dyn resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module = compose_shader_module(
        device,
        resources,
        "tile_solid_multicolor",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        compose_shader_module(device, resources, "tile_solid", crate::ShaderKind::Fragment);

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
            layout: &pipeline_layout.get_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: &pipeline_layout.get_render_pass(),
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


pub unsafe fn create_solid_tile_monochrome_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &<Backend as hal::Backend>::PipelineLayout,
    resources: &dyn resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module = compose_shader_module(
        device,
        resources,
        "tile_solid_monochrome",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        compose_shader_module(device, resources, "tile_solid", crate::ShaderKind::Fragment);

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

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module);
    device.destroy_shader_module(fragment_shader_module);


    pipeline
}

pub unsafe fn create_alpha_tile_multicolor_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &<Backend as hal::Backend>::PipelineLayout,
    resources: &dyn resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module = compose_shader_module(device, 
        resources,
        "tile_alpha_multicolor",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        compose_shader_module(device, resources, "tile_alpha", crate::ShaderKind::Fragment);

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

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module);
    device.destroy_shader_module(fragment_shader_module);

    pipeline
}


pub unsafe fn create_alpha_tile_monochrome_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module = compose_shader_module(device, 
        resources,
        "tile_alpha_monochrome",
        crate::ShaderKind::Vertex,
    );
    let fragment_shader_module =
        compose_shader_module(device, resources, "tile_alpha", crate::ShaderKind::Fragment);

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

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module);
    device.destroy_shader_module(fragment_shader_module);

    pipeline
}

pub unsafe fn create_postprocess_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module =
        compose_shader_module(device, resources, "post", crate::ShaderKind::Vertex);
    let fragment_shader_module =
        compose_shader_module(device, resources, "post", crate::ShaderKind::Fragment);

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

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module);
    device.destroy_shader_module(fragment_shader_module);

    pipeline
}


pub unsafe fn create_stencil_pipeline(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout: &pipeline_layouts::DrawPipelineLayout,
    resources: &dyn resources::ResourceLoader,
    extent: hal::window::Extent2D,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module =
        compose_shader_module(device, resources, "stencil", crate::ShaderKind::Vertex);
    let fragment_shader_module =
        compose_shader_module(device, resources, "stencil", crate::ShaderKind::Fragment);

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
        stencil: generate_stencil_test(StencilFunc::Always, 1, 1, true),
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

        device.create_graphics_pipeline(&desc, None).unwrap()
    };

    device.destroy_shader_module(vertex_shader_module);
    device.destroy_shader_module(fragment_shader_module);

    pipeline
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
            op_pass: hal::pso::StencilOp::Keep,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderKind {
    Vertex,
    Fragment,
}
