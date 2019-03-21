// pathfinder/demo/src/ui.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::{Mode, Options};
use pathfinder_geometry::basic::point::Point2DI32;
use pathfinder_geometry::basic::rect::RectI32;
use pathfinder_gpu::Device;
use pathfinder_renderer::gpu::debug::DebugUI;
use pathfinder_ui::{FONT_ASCENT, PADDING};
use pathfinder_ui::{TOOLTIP_HEIGHT, WINDOW_COLOR};

pub struct DemoUI {
    // FIXME(pcwalton): Factor the below out into a model class.

    pub mode: Mode,
    pub dark_background_enabled: bool,
    pub gamma_correction_effect_enabled: bool,
    pub stem_darkening_effect_enabled: bool,
    pub subpixel_aa_effect_enabled: bool,
    pub message: String,
    pub show_text_effects: bool,
}

impl DemoUI {
    pub fn new(options: Options) -> DemoUI {
        DemoUI {
            mode: options.mode,
            dark_background_enabled: false,
            gamma_correction_effect_enabled: true,
            stem_darkening_effect_enabled: true,
            subpixel_aa_effect_enabled: true,
            message: String::new(),
            show_text_effects: true,
        }
    }

    pub fn update<D>(&mut self,
                     device: &D,
                     debug_ui: &mut DebugUI<D>)
                     where D: Device {
        // Draw message text.

        self.draw_message_text(device, debug_ui);
    }

    fn draw_message_text<D>(&mut self, device: &D, debug_ui: &mut DebugUI<D>) where D:Device {
        if self.message.is_empty() {
            return;
        }

        let message_size = debug_ui.ui.measure_text(&self.message);
        let window_origin = Point2DI32::new(PADDING, PADDING);
        let window_size = Point2DI32::new(PADDING * 2 + message_size, TOOLTIP_HEIGHT);
        debug_ui.ui.draw_solid_rounded_rect(device,
                                            RectI32::new(window_origin, window_size),
                                            WINDOW_COLOR);
        debug_ui.ui.draw_text(device,
                              &self.message,
                              window_origin + Point2DI32::new(PADDING, PADDING + FONT_ASCENT),
                              false);
    }
}
