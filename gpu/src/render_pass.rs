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
use hal::Device;

#[derive(Clone)]
pub struct RenderPassDescription {
    attachments: Vec<hal::pass::Attachment>,
    subpass_colors: Vec<hal::pass::AttachmentRef>,
    subpass_inputs: Vec<hal::pass::AttachmentRef>,
}

pub unsafe fn create_render_pass(device: &<Backend as hal::Backend>::Device, render_pass_desc: crate::render_pass::RenderPassDescription) -> <Backend as hal::Backend>::RenderPass {
    let subpass = hal::pass::SubpassDesc {
        colors: &render_pass_desc.subpass_colors,
        inputs: &render_pass_desc.subpass_inputs,
        depth_stencil: None,
        resolves: &[],
        preserves: &[],
    };

    device.create_render_pass(&render_pass_desc.attachments, &[subpass], &[]).unwrap()
}