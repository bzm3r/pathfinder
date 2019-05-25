// pathfinder/geometry/src/color.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use pathfinder_simd::default::F32x4;
use std::fmt::{self, Debug, Formatter};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ColorU {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorU {
    #[inline]
    pub fn black() -> ColorU {
        ColorU {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        }
    }

    #[inline]
    pub fn to_f32(&self) -> ColorF {
        let color = F32x4::new(self.r as f32, self.g as f32, self.b as f32, self.a as f32);
        ColorF(color * F32x4::splat(1.0 / 255.0))
    }
}

impl Debug for ColorU {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        if self.a == 255 {
            write!(formatter, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(
                formatter,
                "rgba({}, {}, {}, {})",
                self.r,
                self.g,
                self.b,
                self.a as f32 / 255.0
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct ColorF(pub F32x4);

impl ColorF {
    #[inline]
    pub fn transparent_black() -> ColorF {
        ColorF(F32x4::default())
    }

    #[inline]
    pub fn white() -> ColorF {
        ColorF(F32x4::splat(1.0))
    }

    #[inline]
    pub fn r(&self) -> f32 {
        self.0[0]
    }

    #[inline]
    pub fn g(&self) -> f32 {
        self.0[1]
    }

    #[inline]
    pub fn b(&self) -> f32 {
        self.0[2]
    }

    #[inline]
    pub fn a(&self) -> f32 {
        self.0[3]
    }

    pub fn to_rgba_array(&self) -> [f32;4] {
        [self.r(), self.g(), self.b(), self.a()]
    }
}
