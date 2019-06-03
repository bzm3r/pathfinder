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

pub fn mask_pipeline_descriptor_set_layout_bindings() -> Vec<hal::pso::DescriptorSetLayoutBinding> {
    vec![
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
        // uAreaLUT
        hal::pso::DescriptorSetLayoutBinding {
            binding: 2,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        }
    ]
}

pub fn draw_pipeline_descriptor_set_layout_bindings() -> Vec<hal::pso::DescriptorSetLayoutBinding> {
    vec![
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
        // uStencilTextureSize;
        hal::pso::DescriptorSetLayoutBinding {
            binding: 3,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
    ]
}

pub fn create_postprocess_pipeline_descriptor_set_layout_bindings() -> Vec<hal::pso::DescriptorSetLayoutBinding> {
    vec![
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
        // uSource (sampler2D)
        hal::pso::DescriptorSetLayoutBinding {
            binding: 4,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        },
        // uGammaLUT (sampler2D)
        hal::pso::DescriptorSetLayoutBinding {
            binding: 5,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        },
        // uKernel
        hal::pso::DescriptorSetLayoutBinding {
            binding: 6,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        },
    ]
}
