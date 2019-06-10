// pathfinder/gpu/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

extern crate gfx_hal as hal;

use pathfinder_gpu as pfgpu;

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

fn generate_depth_test_for_stencil_shader() -> hal::pso::DepthTest {
    hal::pso::DepthTest::On {
        fun: hal::pso::Comparison::Less,
        write: true,
    }
}

pub unsafe fn create_fill_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("fill");

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

    let blend_state = pfgpu::pfgpu::BlendStateRGBOneAlphaOne;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: mask_framebuffer_size_rect,
            depth: (0.0..1.0),
        }),
        scissor: Some(mask_framebuffer_size_rect),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}

pub unsafe fn create_solid_tile_multicolor_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("tile_solid_multicolor");

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
        stencil: generate_stencil_test(pfgpu::StencilFuncEqual, 1, 1, false),
    };

    let blend_state = pfgpu::BlendStateOff;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: extent.to_extent().rect(),
            depth: (0.0..1.0),
        }),
        scissor: Some(extent.to_extent().rect()),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}

pub unsafe fn create_solid_tile_monochrome_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("tile_solid_monochrome");

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
        stencil: generate_stencil_test(pfgpu::StencilFuncEqual, 1, 1, false),
    };

    let blend_state = pfgpu::BlendStateOff;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: extent.to_extent().rect(),
            depth: (0.0..1.0),
        }),
        scissor: Some(extent.to_extent().rect()),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}

pub unsafe fn create_alpha_tile_multicolor_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("tile_alpha_multicolor");

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
        stencil: generate_stencil_test(pfgpu::StencilFuncEqual, 1, 1, false),
    };

    let blend_state = pfgpu::BlendStateRGBOneAlphaOneMinusSrcAlpha;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: extent.to_extent().rect(),
            depth: (0.0..1.0),
        }),
        scissor: Some(extent.to_extent().rect()),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}

pub unsafe fn create_alpha_tile_monochrome_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("tile_alpha_monochrome");

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
        stencil: generate_stencil_test(pfgpu::StencilFuncEqual, 1, 1, false),
    };

    let blend_state = pfgpu::BlendStateRGBOneAlphaOneMinusSrcAlpha;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: extent.to_extent().rect(),
            depth: (0.0..1.0),
        }),
        scissor: Some(extent.to_extent().rect()),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}

pub unsafe fn create_postprocess_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("post");

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

    let blend_state = pfgpu::BlendStateOff;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: extent.to_extent().rect(),
            depth: (0.0..1.0),
        }),
        scissor: Some(extent.to_extent().rect()),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}

pub unsafe fn create_stencil_pipeline_description(
    size: pfgeom::basic::point::Point2DI32,
) -> pfgpu::pipeline::PipelineDesc {
    let shader_name = String::from("stencil");

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
            generate_stencil_position_attribute_desc(0, 0, stencil_vertex_buffer_cursor, 3);

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
        stencil: generate_stencil_test(pfgpu::StencilFuncAlways, 1, 1, true),
    };

    let blend_state = pfgpu::BlendStateOff;

    let baked_states = hal::pso::BakedStates {
        viewport: Some(hal::pso::Viewport {
            rect: extent.to_extent().rect(),
            depth: (0.0..1.0),
        }),
        scissor: Some(extent.to_extent().rect()),
        blend_color: None,
        depth_bounds: None,
    };

    pfgpu::pipeline::PipelineDesc {
        size,
        shader_name,
        vertex_buffer_descriptions,
        attribute_descriptions,
        rasterizer,
        depth_stencil,
        blend_state,
        baked_states,
    }
}
