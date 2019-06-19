// pathfinder/examples/canvas_glutin_minimal/src/main.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Demonstrates how to use the Pathfinder canvas API with `glutin`.

use glutin::dpi::PhysicalSize;
use glutin::{ContextBuilder, ControlFlow, Event, EventsLoop, GlProfile, GlRequest, KeyboardInput};
use glutin::{VirtualKeyCode,  WindowBuilder, WindowEvent};
use pathfinder_canvas::{CanvasFontContext, CanvasRenderingContext2D, Path2D};
use pathfinder_geometry::basic::vector::{Vector2F, Vector2I};
use pathfinder_geometry::basic::rect::RectF;
use pathfinder_geometry::color::ColorF;
use pathfinder_gl::{GLDevice, GLVersion};
use pathfinder_gpu::resources::FilesystemResourceLoader;
use pathfinder_gpu::{ClearParams, Device};
use pathfinder_renderer::concurrent::rayon::RayonExecutor;
use pathfinder_renderer::concurrent::scene_proxy::SceneProxy;
use pathfinder_renderer::gpu::renderer::{DestFramebuffer, Renderer};
use pathfinder_renderer::options::RenderOptions;

fn main() {
    // Calculate the right logical size of the window.
    let mut event_loop = EventsLoop::new();
    let hidpi_factor = event_loop.get_primary_monitor().get_hidpi_factor();
    let window_size = Vector2I::new(640, 480);
    let physical_window_size = PhysicalSize::new(window_size.x() as f64, window_size.y() as f64);
    let logical_window_size = physical_window_size.to_logical(hidpi_factor);

    // Open a window.
    let window_builder = WindowBuilder::new().with_title("Minimal example")
                                             .with_dimensions(logical_window_size);

    // Create an OpenGL 3.x context for Pathfinder to use.
    let gl_context = ContextBuilder::new().with_gl(GlRequest::Latest)
                                          .with_gl_profile(GlProfile::Core)
                                          .build_windowed(window_builder, &event_loop)
                                          .unwrap();

    // Load OpenGL, and make the context current.
    let gl_context = unsafe { gl_context.make_current().unwrap() };
    gl::load_with(|name| gl_context.get_proc_address(name) as *const _);

    // Create a Pathfinder renderer.
    let mut renderer = Renderer::new(GLDevice::new(GLVersion::GL3, 0),
                                     &FilesystemResourceLoader::locate(),
                                     DestFramebuffer::full_window(window_size));

    // Clear to white.
    renderer.device.clear(&ClearParams { color: Some(ColorF::white()), ..ClearParams::default() });

    // Make a canvas. We're going to draw a house.
    let mut canvas = CanvasRenderingContext2D::new(CanvasFontContext::new(), window_size.to_f32());

    // Set line width.
    canvas.set_line_width(10.0);

    // Draw walls.
    canvas.stroke_rect(RectF::new(Vector2F::new(75.0, 140.0), Vector2F::new(150.0, 110.0)));

    // Draw door.
    canvas.fill_rect(RectF::new(Vector2F::new(130.0, 190.0), Vector2F::new(40.0, 60.0)));

    // Draw roof.
    let mut path = Path2D::new();
    path.move_to(Vector2F::new(50.0, 140.0));
    path.line_to(Vector2F::new(150.0, 60.0));
    path.line_to(Vector2F::new(250.0, 140.0));
    path.close_path();
    canvas.stroke_path(path);

    // Render the canvas to screen.
    let scene = SceneProxy::from_scene(canvas.into_scene(), RayonExecutor);
    scene.build_and_render(&mut renderer, RenderOptions::default());
    gl_context.swap_buffers().unwrap();

    // Wait for a keypress.
    event_loop.run_forever(|event| {
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } |
            Event::WindowEvent {
                event: WindowEvent::KeyboardInput {
                    input: KeyboardInput { virtual_keycode: Some(VirtualKeyCode::Escape), .. },
                    ..
                },
                ..
            } => ControlFlow::Break,
            _ => ControlFlow::Continue,
        }
    })
}
