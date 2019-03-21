// pathfinder/demo/common/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A demo app for Pathfinder.

use crate::device::{GroundLineVertexArray, GroundProgram, GroundSolidVertexArray};
use crate::ui::DemoUI;
use crate::window::{Event, Window, WindowSize};
use image::ColorType;
use pathfinder_geometry::basic::point::{Point2DF32, Point2DI32};
use pathfinder_geometry::basic::rect::{RectF32, RectI32};
use pathfinder_geometry::basic::transform2d::Transform2DF32;
use pathfinder_geometry::basic::transform3d::{Transform3DF32};
use pathfinder_geometry::color::ColorU;
use pathfinder_gl::GLDevice;
use pathfinder_gpu::resources::ResourceLoader;
use pathfinder_gpu::{DepthFunc, DepthState, Device, Primitive, RenderState, StencilFunc};
use pathfinder_gpu::{StencilState, UniformData};
use pathfinder_renderer::builder::{RenderOptions, RenderTransform, SceneBuilder};
use pathfinder_renderer::gpu::renderer::Renderer;
use pathfinder_renderer::gpu_data::{BuiltScene, Stats};
use pathfinder_renderer::post::{DEFRINGING_KERNEL_CORE_GRAPHICS, STEM_DARKENING_FACTORS};
use pathfinder_renderer::scene::Scene;
use pathfinder_renderer::z_buffer::ZBuffer;
use pathfinder_svg::BuiltSVG;
use pathfinder_ui::UIEvent;
use std::iter;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use usvg::{Options as UsvgOptions, Tree};

static DEFAULT_SVG_VIRTUAL_PATH: &'static str = "svg/Ghostscript_Tiger.svg";

const LIGHT_BG_COLOR:     ColorU = ColorU { r: 248, g: 248, b: 248, a: 255 };
const GROUND_SOLID_COLOR: ColorU = ColorU { r: 80,  g: 80,  b: 80,  a: 255 };
const GROUND_LINE_COLOR:  ColorU = ColorU { r: 127, g: 127, b: 127, a: 255 };

const APPROX_FONT_SIZE: f32 = 16.0;

const MESSAGE_TIMEOUT_SECS: u64 = 5;

pub const GRIDLINE_COUNT: u8 = 10;

pub mod window;

mod device;
mod ui;

pub struct DemoApp<W> where W: Window {
    pub window: W,
    pub should_exit: bool,

    window_size: WindowSize,

    scene_view_box: RectF32,
    scene_is_monochrome: bool,

    camera: Camera,
    frame_counter: u32,
    pending_screenshot_path: Option<PathBuf>,
    dirty: bool,
    expire_message_event_id: u32,
    message_epoch: u32,

    current_frame: Option<Frame>,

    ui: DemoUI,
    scene_thread_proxy: SceneThreadProxy,
    renderer: Renderer<GLDevice>,

    ground_program: GroundProgram<GLDevice>,
    ground_solid_vertex_array: GroundSolidVertexArray<GLDevice>,
    ground_line_vertex_array: GroundLineVertexArray<GLDevice>,
}

impl<W> DemoApp<W> where W: Window {
    pub fn new(window: W, window_size: WindowSize) -> DemoApp<W> {
        let expire_message_event_id = window.create_user_event_id();

        let device = GLDevice::new(window.gl_version());
        let resources = window.resource_loader();

        let view_box_size = window_size.device_size();

        let built_svg = load_scene(resources);
        let message = get_svg_building_message(&built_svg);
        let scene_view_box = built_svg.scene.view_box;
        let scene_is_monochrome = built_svg.scene.is_monochrome();

        let renderer = Renderer::new(device,
                                     resources,
                                     RectI32::new(Point2DI32::default(), view_box_size),
                                     window_size.device_size());
        let scene_thread_proxy = SceneThreadProxy::new(built_svg.scene);
        scene_thread_proxy.set_drawable_size(view_box_size);

        let camera = Camera::new_2d(scene_view_box, view_box_size);

        let ground_program = GroundProgram::new(&renderer.device, resources);
        let ground_solid_vertex_array =
            GroundSolidVertexArray::new(&renderer.device,
                                        &ground_program,
                                        &renderer.quad_vertex_positions_buffer());
        let ground_line_vertex_array = GroundLineVertexArray::new(&renderer.device,
                                                                  &ground_program);

        let mut ui = DemoUI::new();
        let mut message_epoch = 0;
        emit_message::<W>(&mut ui, &mut message_epoch, expire_message_event_id, message);

        DemoApp {
            window,
            should_exit: false,

            window_size,

            scene_view_box,
            scene_is_monochrome,

            camera,
            frame_counter: 0,
            pending_screenshot_path: None,
            dirty: true,
            expire_message_event_id,
            message_epoch,

            current_frame: None,

            ui,
            scene_thread_proxy,
            renderer,

            ground_program,
            ground_solid_vertex_array,
            ground_line_vertex_array,
        }
    }

    pub fn prepare_frame(&mut self, events: Vec<Event>) -> u32 {
        // Update the scene.
        self.build_scene();

        // Handle events.
        let ui_events = self.handle_events(events);

        // Get the render message, and determine how many scenes it contains.
        let render_msg = self.scene_thread_proxy.receiver.recv().unwrap();
        let render_scene_count = render_msg.render_scenes.len() as u32;

        // Save the frame.
        self.current_frame = Some(Frame::new(render_msg, ui_events));

        // Begin drawing the scene.
        self.renderer.device.clear(Some(self.background_color().to_f32().0), Some(1.0), Some(0));

        render_scene_count
    }

    fn build_scene(&mut self) {
        let render_transform = match self.camera {
            Camera::TwoD(transform) => RenderTransform::Transform2D(transform),
        };

        let is_first_frame = self.frame_counter == 0;
        let frame_count = if is_first_frame { 2 } else { 1 };

        for _ in 0..frame_count {
            let viewport_count = 1;
            let render_transforms = iter::repeat(render_transform.clone()).take(viewport_count)
                                                                          .collect();
            self.scene_thread_proxy.sender.send(MainToSceneMsg::Build(BuildOptions {
                render_transforms,
                stem_darkening_font_size: if self.ui.stem_darkening_effect_enabled {
                    Some(APPROX_FONT_SIZE * self.window_size.backing_scale_factor)
                } else {
                    None
                },
            })).unwrap();
        }

        if is_first_frame {
            self.dirty = true;
        }
    }

    fn handle_events(&mut self, events: Vec<Event>) -> Vec<UIEvent> {
        let ui_events = vec![];
        self.dirty = false;

        for event in events {
            match event {
                Event::Quit { .. } => {
                    self.should_exit = true;
                    self.dirty = true;
                }
                Event::WindowResized(new_size) => {
                    self.window_size = new_size;
                    let view_box_size = self.window_size.device_size();
                    self.scene_thread_proxy.set_drawable_size(view_box_size);
                    self.renderer.set_main_framebuffer_size(self.window_size.device_size());
                    self.dirty = true;
                }
                Event::User { message_type: event_id, message_data: expected_epoch } if
                        event_id == self.expire_message_event_id &&
                        expected_epoch as u32 == self.message_epoch => {
                    self.ui.message = String::new();
                    self.dirty = true;
                }
                _ => continue,
            }
        }

        ui_events
    }

    pub fn draw_scene(&mut self, render_scene_index: u32) {
        self.draw_environment(render_scene_index);
        self.render_vector_scene(render_scene_index);

        let frame = self.current_frame.as_mut().unwrap();
        let render_scene = &frame.render_msg.render_scenes[render_scene_index as usize];
        match frame.render_stats {
            None => {
                frame.render_stats = Some(RenderStats {
                    rendering_time: self.renderer.shift_timer_query(),
                    stats: render_scene.built_scene.stats(),
                })
            }
            Some(ref mut render_stats) => {
                render_stats.stats = render_stats.stats + render_scene.built_scene.stats()
            }
        }
    }

    pub fn finish_drawing_frame(&mut self) {
        let mut frame = self.current_frame.take().unwrap();

        let drawable_size = self.window_size.device_size();
        self.renderer.set_viewport(RectI32::new(Point2DI32::default(), drawable_size));

        if self.pending_screenshot_path.is_some() {
            self.take_screenshot();
        }

        if let Some(render_stats) = frame.render_stats.take() {
            self.renderer.debug_ui.add_sample(render_stats.stats,
                                              frame.render_msg.tile_time,
                                              render_stats.rendering_time);
            self.renderer.draw_debug_ui();
        }

        for ui_event in &frame.ui_events {
            self.dirty = true;
            self.renderer.debug_ui.ui.event_queue.push(*ui_event);
        }

        self.renderer.debug_ui.ui.mouse_position =
            Point2DI32::new(0, 0).to_f32().scale(self.window_size.backing_scale_factor);
        self.ui.show_text_effects = self.scene_is_monochrome;

        self.ui.update(&self.renderer.device,
                       &mut self.renderer.debug_ui);

        frame.ui_events = self.renderer.debug_ui.ui.event_queue.drain();

        for ui_event in frame.ui_events {
            match ui_event {
                _ => {}
            }
        }

        self.window.present();
        self.frame_counter += 1;
    }

    fn draw_environment(&self, viewport_index: u32) {
        let render_msg = &self.current_frame.as_ref().unwrap().render_msg;
        let render_transform = &render_msg.render_scenes[viewport_index as usize].transform;

        let perspective = match *render_transform {
            RenderTransform::Transform2D(..) => return,
            RenderTransform::Perspective(perspective) => perspective,
        };

        let ground_scale = self.scene_view_box.max_x() * 2.0;

        let mut base_transform = perspective.transform;
        base_transform = base_transform.post_mul(&Transform3DF32::from_translation(
            -0.5 * self.scene_view_box.max_x(),
            self.scene_view_box.max_y(),
            -0.5 * ground_scale));

        // Draw gridlines. Use the stencil buffer to avoid Z-fighting.
        let mut transform = base_transform;
        let gridline_scale = ground_scale / GRIDLINE_COUNT as f32;
        transform =
            transform.post_mul(&Transform3DF32::from_scale(gridline_scale, 1.0, gridline_scale));
        let device = &self.renderer.device;
        device.bind_vertex_array(&self.ground_line_vertex_array.vertex_array);
        device.use_program(&self.ground_program.program);
        device.set_uniform(&self.ground_program.transform_uniform, UniformData::Mat4([
            transform.c0,
            transform.c1,
            transform.c2,
            transform.c3,
        ]));
        device.set_uniform(&self.ground_program.color_uniform,
                           UniformData::Vec4(GROUND_LINE_COLOR.to_f32().0));
        device.draw_arrays(Primitive::Lines, (GRIDLINE_COUNT as u32 + 1) * 4, &RenderState {
            depth: Some(DepthState { func: DepthFunc::Always, write: true }),
            stencil: Some(StencilState {
                func: StencilFunc::Always,
                reference: 2,
                mask: 2,
                write: true,
            }),
            ..RenderState::default()
        });

        // Fill ground.
        let mut transform = base_transform;
        transform =
            transform.post_mul(&Transform3DF32::from_scale(ground_scale, 1.0, ground_scale));
        device.bind_vertex_array(&self.ground_solid_vertex_array.vertex_array);
        device.use_program(&self.ground_program.program);
        device.set_uniform(&self.ground_program.transform_uniform, UniformData::Mat4([
            transform.c0,
            transform.c1,
            transform.c2,
            transform.c3,
        ]));
        device.set_uniform(&self.ground_program.color_uniform,
                           UniformData::Vec4(GROUND_SOLID_COLOR.to_f32().0));
        device.draw_arrays(Primitive::TriangleFan, 4, &RenderState {
            depth: Some(DepthState { func: DepthFunc::Less, write: true }),
            stencil: Some(StencilState {
                func: StencilFunc::NotEqual,
                reference: 2,
                mask: 2,
                write: false,
            }),
            ..RenderState::default()
        });
    }

    fn render_vector_scene(&mut self, viewport_index: u32) {
        let render_msg = &self.current_frame.as_ref().unwrap().render_msg;
        let built_scene = &render_msg.render_scenes[viewport_index as usize].built_scene;

        let view_box_size = self.window_size.device_size();
        let viewport_origin_x = viewport_index as i32 * view_box_size.x();
        let viewport = RectI32::new(Point2DI32::new(viewport_origin_x, 0), view_box_size);
        self.renderer.set_viewport(viewport);

        if self.ui.gamma_correction_effect_enabled {
            self.renderer.enable_gamma_correction(self.background_color());
        } else {
            self.renderer.disable_gamma_correction();
        }

        if self.ui.subpixel_aa_effect_enabled {
            self.renderer.enable_subpixel_aa(&DEFRINGING_KERNEL_CORE_GRAPHICS);
        } else {
            self.renderer.disable_subpixel_aa();
        }

        self.renderer.disable_depth();

        self.renderer.render_scene(&built_scene);
    }

    fn take_screenshot(&mut self) {
        let screenshot_path = self.pending_screenshot_path.take().unwrap();
        let drawable_size = self.window_size.device_size();
        let pixels = self.renderer.device.read_pixels_from_default_framebuffer(drawable_size);
        image::save_buffer(screenshot_path,
                           &pixels,
                           drawable_size.x() as u32,
                           drawable_size.y() as u32,
                           ColorType::RGBA(8)).unwrap();
    }

    fn background_color(&self) -> ColorU {
        LIGHT_BG_COLOR
    }

}

struct SceneThreadProxy {
    sender: Sender<MainToSceneMsg>,
    receiver: Receiver<SceneToMainMsg>,
}

impl SceneThreadProxy {
    fn new(scene: Scene) -> SceneThreadProxy {
        let (main_to_scene_sender, main_to_scene_receiver) = mpsc::channel();
        let (scene_to_main_sender, scene_to_main_receiver) = mpsc::channel();
        SceneThread::new(scene, scene_to_main_sender, main_to_scene_receiver);
        SceneThreadProxy { sender: main_to_scene_sender, receiver: scene_to_main_receiver }
    }

    fn set_drawable_size(&self, drawable_size: Point2DI32) {
        self.sender.send(MainToSceneMsg::SetDrawableSize(drawable_size)).unwrap();
    }
}

struct SceneThread {
    scene: Scene,
    sender: Sender<SceneToMainMsg>,
    receiver: Receiver<MainToSceneMsg>,
}

impl SceneThread {
    fn new(scene: Scene,
           sender: Sender<SceneToMainMsg>,
           receiver: Receiver<MainToSceneMsg>) {
        thread::spawn(move || (SceneThread { scene, sender, receiver }).run());
    }

    fn run(mut self) {
        while let Ok(msg) = self.receiver.recv() {
            match msg {
                MainToSceneMsg::SetDrawableSize(size) => {
                    self.scene.view_box = RectF32::new(Point2DF32::default(), size.to_f32());
                }
                MainToSceneMsg::Build(build_options) => {
                    let start_time = Instant::now();
                    let render_scenes = build_options.render_transforms
                                                     .iter()
                                                     .map(|render_transform| {
                        let built_scene = build_scene(&self.scene,
                                                      &build_options,
                                                      (*render_transform).clone());
                        RenderScene { built_scene, transform: (*render_transform).clone() }
                    }).collect();
                    let tile_time = Instant::now() - start_time;
                    self.sender.send(SceneToMainMsg { render_scenes, tile_time }).unwrap();
                }
            }
        }
    }
}

enum MainToSceneMsg {
    SetDrawableSize(Point2DI32),
    Build(BuildOptions),
}

struct BuildOptions {
    render_transforms: Vec<RenderTransform>,
    stem_darkening_font_size: Option<f32>,
}

struct SceneToMainMsg {
    render_scenes: Vec<RenderScene>,
    tile_time: Duration,
}

pub struct RenderScene {
    built_scene: BuiltScene,
    transform: RenderTransform,
}

#[derive(Clone, Copy)]
struct RenderStats {
    rendering_time: Option<Duration>,
    stats: Stats,
}

fn load_scene(resource_loader: &dyn ResourceLoader) -> BuiltSVG {
    let data = resource_loader.slurp(DEFAULT_SVG_VIRTUAL_PATH).unwrap();

    BuiltSVG::from_tree(Tree::from_data(&data, &UsvgOptions::default()).unwrap())
}

fn build_scene(scene: &Scene,
               build_options: &BuildOptions,
               render_transform: RenderTransform)
               -> BuiltScene {
    let z_buffer = ZBuffer::new(scene.view_box);

    let render_options = RenderOptions {
        transform: render_transform,
        dilation: match build_options.stem_darkening_font_size {
            None => Point2DF32::default(),
            Some(font_size) => {
                let (x, y) = (STEM_DARKENING_FACTORS[0], STEM_DARKENING_FACTORS[1]);
                Point2DF32::new(x, y).scale(font_size)
            }
        },
        barrel_distortion: None,
    };

    let built_options = render_options.prepare(scene.bounds);
    let quad = built_options.quad();

    let built_objects = scene.build_objects_sequentially(built_options, &z_buffer);

    let mut built_scene = BuiltScene::new(scene.view_box, &quad, scene.objects.len() as u32);
    built_scene.shaders = scene.build_shaders();

    let mut scene_builder = SceneBuilder::new(built_objects, z_buffer, scene.view_box);
    built_scene.solid_tiles = scene_builder.build_solid_tiles();
    while let Some(batch) = scene_builder.build_batch() {
        built_scene.batches.push(batch);
    }

    built_scene
}

enum Camera {
    TwoD(Transform2DF32),
}

impl Camera {
    fn new_2d(view_box: RectF32, drawable_size: Point2DI32) -> Camera {
        let scale = i32::min(drawable_size.x(), drawable_size.y()) as f32 *
            scale_factor_for_view_box(view_box);
        let origin = drawable_size.to_f32().scale(0.5) - view_box.size().scale(scale * 0.5);
        Camera::TwoD(Transform2DF32::from_scale(&Point2DF32::splat(scale)).post_translate(origin))
    }
}

fn scale_factor_for_view_box(view_box: RectF32) -> f32 {
    1.0 / f32::min(view_box.size().x(), view_box.size().y())
}

fn get_svg_building_message(built_svg: &BuiltSVG) -> String {
    if built_svg.result_flags.is_empty() {
        return String::new();
    }
    format!("Warning: These features in the SVG are unsupported: {}.", built_svg.result_flags)
}

fn emit_message<W>(ui: &mut DemoUI,
                   message_epoch: &mut u32,
                   expire_message_event_id: u32,
                   message: String)
                   where W: Window {
    if message.is_empty() {
        return;
    }

    ui.message = message;
    let expected_epoch = *message_epoch + 1;
    *message_epoch = expected_epoch;
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(MESSAGE_TIMEOUT_SECS));
        W::push_user_event(expire_message_event_id, expected_epoch);
    });
}

struct Frame {
    render_msg: SceneToMainMsg,
    ui_events: Vec<UIEvent>,
    render_stats: Option<RenderStats>,
}

impl Frame {
    fn new(render_msg: SceneToMainMsg, ui_events: Vec<UIEvent>) -> Frame {
        Frame { render_msg, ui_events, render_stats: None }
    }
}
