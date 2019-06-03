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

pub struct RenderPassDesc {
    attachments: Vec<hal::pass::Attachment>,
    subpass_colors: Vec<hal::pass::AttachmentRef>,
    subpass_inputs: Vec<hal::pass::AttachmentRef>,
}

pub fn create_fill_render_pass_desc() -> RenderPassDesc {
    let mask_texture = hal::pass::Attachment {
        format: Some(hal::format::Format::R16Sfloat),
        samples: 0,
        ops: hal::pass::AttachmentOps {
            load: hal::pass::AttachmentLoadOp::Clear,
            store: hal::pass::AttachmentStoreOp::Store,
        },
        stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
        layouts: hal::image::Layout::Undefined..hal::image::Layout::ShaderReadOnlyOptimal,
    };

    RenderPassDesc {
        attachments: vec![mask_texture],
        subpass_colors: vec![(0, hal::image::Layout::ColorAttachmentOptimal)],
        subpass_inputs: vec![],
    }
}

pub fn create_draw_pass_desc() -> RenderPassDesc {
    let mask_texture = hal::pass::Attachment {
        format: Some(hal::format::Format::R16Sfloat),
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

    RenderPassDesc {
        attachments: vec![dest],
        subpass_colors: vec![(1, hal::image::Layout::ColorAttachmentOptimal)],
        subpass_inputs: vec![(0, hal::image::Layout::ShaderReadOnlyOptimal)],
    }
}

pub fn create_postprocess_pass_desc() -> RenderPassDesc {
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

    RenderPassDesc {
        attachments: vec![dest],
        subpass_colors: vec![(0, hal::image::Layout::ColorAttachmentOptimal)],
        subpass_inputs: vec![(0, hal::image::Layout::ShaderReadOnlyOptimal)],
    }
}


