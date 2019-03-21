// pathfinder/demo/common/src/window.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A minimal cross-platform windowing layer.

use pathfinder_geometry::basic::point::Point2DI32;
use pathfinder_geometry::distortion::BarrelDistortionCoefficients;
use pathfinder_gl::GLVersion;
use pathfinder_gpu::resources::ResourceLoader;
use std::path::PathBuf;

pub trait Window {
    fn gl_version(&self) -> GLVersion;
    fn present(&self);
    fn resource_loader(&self) -> &dyn ResourceLoader;
    fn create_user_event_id(&self) -> u32;
    fn push_user_event(message_type: u32, message_data: u32);
    fn present_open_svg_dialog(&mut self);
    fn run_save_dialog(&self, extension: &str) -> Result<PathBuf, ()>;

    #[inline]
    fn barrel_distortion_coefficients(&self) -> BarrelDistortionCoefficients {
        BarrelDistortionCoefficients::default()
    }
}

pub enum Event {
    Quit,
    WindowResized(WindowSize),
    User { message_type: u32, message_data: u32 },
}

#[derive(Clone, Copy)]
pub enum Keycode {
    Alphanumeric(u8),
    Escape,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowSize {
    pub logical_size: Point2DI32,
    pub backing_scale_factor: f32,
}

impl WindowSize {
    #[inline]
    pub fn device_size(&self) -> Point2DI32 {
        self.logical_size.to_f32().scale(self.backing_scale_factor).to_i32()
    }
}
