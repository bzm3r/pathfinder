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

pub fn create_fill_render_pass_desc() -> pfgpu::render_pass::RenderPassDescription {
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


    pfgpu::render_pass::RenderPassDescription {
        attachments: vec![mask_texture],
        num_subpasses: 1,
        colors_per_subpass: vec![vec![(0, hal::image::Layout::ColorAttachmentOptimal)],],
        inputs_per_subpass: vec![Vec::<hal::pass::AttachmentRef>::new(),],
        preserves_per_subpass: vec![Vec::<hal::pass::AttachmentId>::new(),],
    }
}

pub fn create_draw_pass_desc() -> pfgpu::render_pass::RenderPassDescription {
    let fill_texture = hal::pass::Attachment {
        format: Some(hal::format::Format::R16Sfloat),
        samples: 0,
        ops: hal::pass::AttachmentOps {
            load: hal::pass::AttachmentLoadOp::Load,
            store: hal::pass::AttachmentStoreOp::Store,
        },
        stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
        layouts: hal::image::Layout::ShaderReadOnlyOptimal..hal::image::Layout::ShaderReadOnlyOptimal,
    };

    // format field will be will be filled out by GpuState based on swapchain image format
    let transient = hal::pass::Attachment {
        format: None,
        samples: 0,
        ops: hal::pass::AttachmentOps {
            load: hal::pass::AttachmentLoadOp::Clear,
            store: hal::pass::AttachmentStoreOp::Store,
        },
        stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
        layouts: hal::image::Layout::Undefined..hal::image::Layout::ShaderReadOnlyOptimal,
    };

    // format field will be will be filled out by GpuState based on swapchain image format
    let dest = hal::pass::Attachment {
        format: None,
        samples: 0,
        ops: hal::pass::AttachmentOps {
            load: hal::pass::AttachmentLoadOp::Clear,
            store: hal::pass::AttachmentStoreOp::Store,
        },
        stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
        layouts: hal::image::Layout::Undefined..hal::image::Layout::Present,
    };

    pfgpu::render_pass::RenderPassDescription {
        attachments: vec![fill_texture, transient, dest],
        num_subpasses: 2,
        inputs_per_subpass: vec![vec![(0, hal::image::Layout::ShaderReadOnlyOptimal)], vec![(1, hal::image::Layout::ShaderReadOnlyOptimal)],],
        colors_per_subpass: vec![vec![(1, hal::image::Layout::ColorAttachmentOptimal)], vec![(2, hal::image::Layout::Present)],],
        preserves_per_subpass: vec![Vec::<hal::pass::AttachmentId>::new(), Vec::<hal::pass::AttachmentId>::new()],
    }
}

pub fn create_draw_pass_no_postprocess_desc() -> pfgpu::render_pass::RenderPassDescription {
    let fill_texture = hal::pass::Attachment {
        format: Some(hal::format::Format::R16Sfloat),
        samples: 0,
        ops: hal::pass::AttachmentOps {
            load: hal::pass::AttachmentLoadOp::Load,
            store: hal::pass::AttachmentStoreOp::Store,
        },
        stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
        layouts: hal::image::Layout::ShaderReadOnlyOptimal..hal::image::Layout::ShaderReadOnlyOptimal,
    };

    // format field will be will be filled out by GpuState based on swapchain image format
    let dest = hal::pass::Attachment {
        format: None,
        samples: 0,
        ops: hal::pass::AttachmentOps {
            load: hal::pass::AttachmentLoadOp::Clear,
            store: hal::pass::AttachmentStoreOp::Store,
        },
        stencil_ops: hal::pass::AttachmentOps::DONT_CARE,
        layouts: hal::image::Layout::Undefined..hal::image::Layout::Present,
    };

    pfgpu::render_pass::RenderPassDescription {
        attachments: vec![fill_texture, dest],
        num_subpasses: 1,
        inputs_per_subpass: vec![vec![(0, hal::image::Layout::ShaderReadOnlyOptimal)],],
        colors_per_subpass: vec![vec![(1, hal::image::Layout::Present)],],
        preserves_per_subpass: vec![Vec::<hal::pass::AttachmentId>::new(), Vec::<hal::pass::AttachmentId>::new()],
    }
}

