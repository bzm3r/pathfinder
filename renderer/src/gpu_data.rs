// pathfinder/renderer/src/gpu_data.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Packed data ready to be sent to the GPU.

use crate::options::BoundingQuad;
use crate::tile_map::DenseTileMap;
use pathfinder_geometry::basic::line_segment::{LineSegmentU4, LineSegmentU8};
use pathfinder_geometry::basic::point::Point2DI32;
use pathfinder_geometry::basic::rect::RectF32;
use std::fmt::{Debug, Formatter, Result as DebugResult};
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct BuiltObject {
    pub bounds: RectF32,
    pub fills: Vec<FillBatchPrimitive>,
    pub alpha_tiles: Vec<AlphaTileBatchPrimitive>,
    pub tiles: DenseTileMap<TileObjectPrimitive>,
}

pub enum RenderCommand {
    Start { path_count: usize, bounding_quad: BoundingQuad },
    AddPaintData(PaintData),
    AddFills(Vec<FillBatchPrimitive>),
    FlushFills,
    AlphaTile(Vec<AlphaTileBatchPrimitive>),
    SolidTile(Vec<SolidTileBatchPrimitive>),
    Finish { build_time: Duration },
}

impl Debug for RenderCommand {
    fn fmt(&self, formatter: &mut Formatter) -> DebugResult {
        match *self {
            RenderCommand::Start { .. } => write!(formatter, "Start"),
            RenderCommand::AddPaintData(ref paint_data) => {
                write!(formatter, "AddPaintData({}x{})", paint_data.size.x(), paint_data.size.y())
            }
            RenderCommand::AddFills(ref fills) => write!(formatter, "AddFills(x{})", fills.len()),
            RenderCommand::FlushFills => write!(formatter, "FlushFills"),
            RenderCommand::AlphaTile(ref tiles) => {
                write!(formatter, "AlphaTile(x{})", tiles.len())
            }
            RenderCommand::SolidTile(ref tiles) => {
                write!(formatter, "SolidTile(x{})", tiles.len())
            }
            RenderCommand::Finish { .. } => write!(formatter, "Finish"),
        }
    }
}