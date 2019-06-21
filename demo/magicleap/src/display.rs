// pathfinder/demo/immersive/display.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::error::Error;
use std::io;
use pathfinder_geometry::basic::vector::Vector2I;
use pathfinder_geometry::basic::rect::RectI32;
use pathfinder_geometry::basic::transform3d::Perspective;
use pathfinder_geometry::basic::transform3d::Transform3DF32;
use pathfinder_gl::GLVersion;
use pathfinder_gpu::resources::ResourceLoader;

pub trait Display: Sized {
    type Error: DisplayError;
    type Camera: DisplayCamera<Error = Self::Error>;

    fn resource_loader(&self) -> &dyn ResourceLoader;
    fn gl_version(&self) -> GLVersion;
    fn make_current(&mut self) -> Result<(), Self::Error>;

    fn running(&self) -> bool;
    fn size(&self) -> Point2DI32;

    fn begin_frame(&mut self) -> Result<&mut[Self::Camera], Self::Error>;
    fn end_frame(&mut self) -> Result<(), Self::Error>;
}

pub trait DisplayCamera {
    type Error: DisplayError;

    fn bounds(&self) -> RectI32;
    fn view(&self) -> Transform3DF32;
    fn perspective(&self) -> Perspective;

    fn make_current(&mut self) -> Result<(), Self::Error>;
}

pub trait DisplayError: Error + From<usvg::Error> + From<io::Error>{
}
