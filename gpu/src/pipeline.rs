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
use crate::{StencilFunc, BlendState};
use pathfinder_geometry as pfgeom;

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
            op_pass: op_pass,
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
            op_pass: op_pass,
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
            return hal::pso::BlendDesc {
                logic_op: None,
                targets: vec![hal::pso::ColorBlendDesc::EMPTY],
            };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderKind {
    Vertex,
    Fragment,
}


unsafe fn compose_shader_module(
    device: &<Backend as hal::Backend>::Device,
    resource_loader: &dyn crate::resources::ResourceLoader,
    name: &str,
    shader_kind: ShaderKind,
) -> <Backend as hal::Backend>::ShaderModule {
    let shader_kind_char = match shader_kind {
        ShaderKind::Vertex => 'v',
        ShaderKind::Fragment => 'f',
    };

    let source = resource_loader
        .slurp(&format!("shaders/{}.{}s.glsl", name, shader_kind_char))
        .unwrap();

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

#[derive(Clone)]
pub struct PipelineDescription {
    pub size: pfgeom::basic::point::Point2DI32,
    pub shader_name: String,
    pub vertex_buffer_descriptions: Vec<hal::pso::VertexBufferDesc>,
    pub attribute_descriptions: Vec<hal::pso::AttributeDesc>,
    pub rasterizer: hal::pso::Rasterizer,
    pub depth_stencil: hal::pso::DepthStencilDesc,
    pub blend_state: crate::BlendState,
    pub baked_states: hal::pso::BakedStates,
}

pub unsafe fn create_pipeline<'a>(
    device: &<Backend as hal::Backend>::Device,
    pipeline_layout_state: &crate::pipeline_state::PipelineLayoutState,
    resource_loader: &dyn crate::resources::ResourceLoader,
    pipeline_description: PipelineDescription,
) -> <Backend as hal::Backend>::GraphicsPipeline {
    let vertex_shader_module: <Backend as hal::Backend>::ShaderModule =
        compose_shader_module(device, resource_loader, &pipeline_description.shader_name, ShaderKind::Vertex);
    let fragment_shader_module: <Backend as hal::Backend>::ShaderModule =
        compose_shader_module(device, resource_loader, &pipeline_description.shader_name, ShaderKind::Fragment);

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

    let blender = generate_blend_desc(pipeline_description.blend_state);

    let pipeline = {
        let PipelineDescription { rasterizer, vertex_buffer_descriptions, attribute_descriptions, depth_stencil, baked_states, ..} = pipeline_description;
        let desc = hal::pso::GraphicsPipelineDesc {
            shaders,
            rasterizer,
            vertex_buffers: vertex_buffer_descriptions,
            attributes: attribute_descriptions,
            input_assembler,
            blender,
            depth_stencil,
            multisampling: None,
            baked_states,
            layout: pipeline_layout_state.pipeline_layout(),
            subpass: hal::pass::Subpass {
                index: 0,
                main_pass: pipeline_layout_state.render_pass(),
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

