// pathfinder/renderer/src/gpu_data.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use pathfinder_geometry::basic::line_segment::{LineSegmentU4, LineSegmentU8};
use pathfinder_geometry::basic::point::Point2DI32;

#[derive(Clone, Debug)]
pub struct PaintData {
    pub size: Point2DI32,
    pub texels: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct FillObjectPrimitive {
    pub px: LineSegmentU4,
    pub subpx: LineSegmentU8,
    pub tile_x: i16,
    pub tile_y: i16,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct TileObjectPrimitive {
    /// If `u16::MAX`, then this is a solid tile.
    pub alpha_tile_index: u16,
    pub backdrop: i8,
}

// FIXME(pcwalton): Move `subpx` before `px` and remove `repr(packed)`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(packed)]
pub struct FillBatchPrimitive {
    pub px: LineSegmentU4,
    pub subpx: LineSegmentU8,
    pub alpha_tile_index: u16,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SolidTileBatchPrimitive {
    pub tile_x: i16,
    pub tile_y: i16,
    pub origin_u: u16,
    pub origin_v: u16,
    pub object_index: u16,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct AlphaTileBatchPrimitive {
    pub tile_x_lo: u8,
    pub tile_y_lo: u8,
    pub tile_hi: u8,
    pub backdrop: i8,
    pub object_index: u16,
    pub tile_index: u16,
    pub origin_u: u16,
    pub origin_v: u16,
}