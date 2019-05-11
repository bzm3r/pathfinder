// pathfinder/demo/common/src/device.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! GPU rendering code specifically for the demo.

use pathfinder_gpu::resources::ResourceLoader;
use pathfinder_gpu::{BufferTarget, PfDevice, VertexAttrType};

pub struct GroundProgram<D>
where
    D: PfDevice,
{
    pub program: D::Program,
    pub transform_uniform: D::Uniform,
    pub gridline_count_uniform: D::Uniform,
    pub ground_color_uniform: D::Uniform,
    pub gridline_color_uniform: D::Uniform,
}

impl<D> GroundProgram<D>
where
    D: PfDevice,
{
    pub fn new(device: &D, resources: &dyn ResourceLoader) -> GroundProgram<D> {
        let program = device.create_program(resources, "demo_ground");
        let transform_uniform = device.get_uniform(&program, "Transform");
        let gridline_count_uniform = device.get_uniform(&program, "GridlineCount");
        let ground_color_uniform = device.get_uniform(&program, "GroundColor");
        let gridline_color_uniform = device.get_uniform(&program, "GridlineColor");
        GroundProgram {
            program,
            transform_uniform,
            gridline_count_uniform,
            ground_color_uniform,
            gridline_color_uniform,
        }
    }
}

pub struct GroundVertexArray<D>
where
    D: PfDevice,
{
    pub vertex_array: D::VertexArray,
}

impl<D> GroundVertexArray<D>
where
    D: PfDevice,
{
    pub fn new(
        device: &D,
        ground_program: &GroundProgram<D>,
        quad_vertex_positions_buffer: &D::Buffer,
    ) -> GroundVertexArray<D> {
        let vertex_array = device.create_vertex_array();

        let position_attr = device.get_vertex_attr(&ground_program.program, "Position");

        device.bind_vertex_array(&vertex_array);
        device.use_program(&ground_program.program);
        device.bind_buffer(quad_vertex_positions_buffer, BufferTarget::Vertex);
        device.configure_float_vertex_attr(&position_attr, 2, VertexAttrType::U8, false, 0, 0, 0);

        GroundVertexArray { vertex_array }
    }
}
