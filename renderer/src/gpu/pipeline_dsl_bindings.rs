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

pub fn fill_pipeline_descriptor_set_layout_bindings() -> Vec<hal::pso::DescriptorSetLayoutBinding> {
    vec![
        // UniformStructA
        hal::pso::DescriptorSetLayoutBinding {
            binding: 0,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
        // uAreaLUT
        hal::pso::DescriptorSetLayoutBinding {
            binding: 1,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        }
    ]
}

pub fn draw_pipeline_descriptor_set_layout_bindings() -> Vec<hal::pso::DescriptorSetLayoutBinding> {
    vec![
        // UniformStructA
        hal::pso::DescriptorSetLayoutBinding {
            binding: 0,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
        // UniformStructB
        hal::pso::DescriptorSetLayoutBinding {
            binding: 1,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
        // UniformStructC
        hal::pso::DescriptorSetLayoutBinding {
            binding: 2,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
        // postprocessing related descriptor sets
        // UniformStructD;
        hal::pso::DescriptorSetLayoutBinding {
            binding: 3,
            ty: hal::pso::DescriptorType::UniformBuffer,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
        // uPaintTexture
        hal::pso::DescriptorSetLayoutBinding {
            binding: 4,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::VERTEX,
            immutable_samplers: false,
        },
        // uStencilTexture
        hal::pso::DescriptorSetLayoutBinding {
            binding: 5,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        },
        // uSource
        hal::pso::DescriptorSetLayoutBinding {
            binding: 6,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        },
        // uGammaLUT
        hal::pso::DescriptorSetLayoutBinding {
            binding: 7,
            ty: hal::pso::DescriptorType::CombinedImageSampler,
            count: 1,
            stage_flags: hal::pso::ShaderStageFlags::FRAGMENT,
            immutable_samplers: false,
        },
    ]
}

