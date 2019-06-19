// pathfinder/renderer/src/concurrent/scene_proxy.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A version of `Scene` that proxies all method calls out to a separate
//! thread.
//!
//! This is useful for:
//!
//!   * Avoiding GPU driver stalls on synchronous APIs such as OpenGL.
//!
//!   * Avoiding UI latency by building scenes off the main thread.
//!
//! You don't need to use this API to use Pathfinder; it's only a convenience.

use crate::concurrent::executor::Executor;
use crate::gpu::renderer::Renderer;
use crate::gpu_data::RenderCommand;
use crate::options::{RenderCommandListener, RenderOptions};
use crate::scene::Scene;
use pathfinder_geometry::basic::rect::RectF;
use pathfinder_gpu::Device;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

const MAX_MESSAGES_IN_FLIGHT: usize = 1024;

pub struct SceneProxy {
    sender: Sender<MainToWorkerMsg>,
}

impl SceneProxy {
    pub fn new<E>(executor: E) -> SceneProxy where E: Executor + Send + 'static {
        SceneProxy::from_scene(Scene::new(), executor)
    }

    pub fn from_scene<E>(scene: Scene, executor: E) -> SceneProxy
                         where E: Executor + Send + 'static {
        let (main_to_worker_sender, main_to_worker_receiver) = mpsc::channel();
        thread::spawn(move || scene_thread(scene, executor, main_to_worker_receiver));
        SceneProxy { sender: main_to_worker_sender }
    }

    #[inline]
    pub fn replace_scene(&self, new_scene: Scene) {
        self.sender.send(MainToWorkerMsg::ReplaceScene(new_scene)).unwrap();
    }

    #[inline]
    pub fn set_view_box(&self, new_view_box: RectF) {
        self.sender.send(MainToWorkerMsg::SetViewBox(new_view_box)).unwrap();
    }

    #[inline]
    pub fn build_with_listener(&self,
                               options: RenderOptions,
                               listener: Box<dyn RenderCommandListener>) {
        self.sender.send(MainToWorkerMsg::Build(options, listener)).unwrap();
    }

    #[inline]
    pub fn build_with_stream(&self, options: RenderOptions) -> RenderCommandStream {
        let (sender, receiver) = mpsc::sync_channel(MAX_MESSAGES_IN_FLIGHT);
        let listener = Box::new(move |command| sender.send(command).unwrap());
        self.build_with_listener(options, listener);
        RenderCommandStream::new(receiver)
    }

    /// A convenience method to build a scene and send the resulting commands
    /// to the given renderer.
    ///
    /// Exactly equivalent to:
    ///
    ///     for command in scene_proxy.build_with_stream(options) {
    ///         renderer.render_command(&command)
    ///     }
    #[inline]
    pub fn build_and_render<D>(&self, renderer: &mut Renderer<D>, options: RenderOptions)
                               where D: Device {
        renderer.begin_scene();
        for command in self.build_with_stream(options) {
            renderer.render_command(&command)
        }
        renderer.end_scene();
    }

    pub fn as_svg(&self) -> Vec<u8> {
        let (sender, receiver) = mpsc::channel();
        self.sender.send(MainToWorkerMsg::GetSVG(sender)).unwrap();
        receiver.recv().unwrap()
    }
}

fn scene_thread<E>(mut scene: Scene,
                   executor: E,
                   main_to_worker_receiver: Receiver<MainToWorkerMsg>)
                   where E: Executor {
    while let Ok(msg) = main_to_worker_receiver.recv() {
        match msg {
            MainToWorkerMsg::ReplaceScene(new_scene) => scene = new_scene,
            MainToWorkerMsg::SetViewBox(new_view_box) => scene.set_view_box(new_view_box),
            MainToWorkerMsg::Build(options, listener) => scene.build(options, listener, &executor),
            MainToWorkerMsg::GetSVG(sender) => {
                let mut bytes = vec![];
                scene.write_svg(&mut bytes).unwrap();
                sender.send(bytes).unwrap();
            }
        }
    }
}

enum MainToWorkerMsg {
    ReplaceScene(Scene),
    SetViewBox(RectF),
    Build(RenderOptions, Box<dyn RenderCommandListener>),
    GetSVG(Sender<Vec<u8>>),
}

pub struct RenderCommandStream {
    receiver: Receiver<RenderCommand>,
    done: bool,
}

impl RenderCommandStream {
    fn new(receiver: Receiver<RenderCommand>) -> RenderCommandStream {
        RenderCommandStream { receiver, done: false }
    }
}

impl Iterator for RenderCommandStream {
    type Item = RenderCommand;

    #[inline]
    fn next(&mut self) -> Option<RenderCommand> {
        if self.done {
            None
        } else {
            let command = self.receiver.recv().unwrap();
            if let RenderCommand::Finish { .. } = command {
                self.done = true;
            }
            Some(command)
        }
    }
}
