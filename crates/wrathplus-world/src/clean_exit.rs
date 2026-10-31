//! A clean exit under pipelined rendering: on the frame that asks to exit, the render world comes
//! home and stays there, so teardown has no frame in flight to wait for.
//!
//! Bevy 0.18's `RenderAppChannels::drop` waits for the render world with a bare `recv_blocking`
//! and does not run the main thread's tasks while it waits. The render schedule's main-thread
//! systems need them: on macOS `create_surfaces` (a `NonSendMarker` system) runs whenever a
//! surface needs configuring, which a closing window does. So the last frame in flight waited on
//! the main thread while the main thread waited on it in `World::clear_all`, until the stall
//! watchdog's 8 s `_exit` (session 174100, sample `stall-72048-teardown-1447554`).
//!
//! [`extract_or_reclaim`] replaces Bevy's `renderer_extract` with the same steps, plus one: when
//! the main world has an `AppExit` this frame, the render world received back is dropped here,
//! on the main thread, instead of being sent off to render a frame no one will see. The channel
//! then has nothing in flight and its drop returns at once. The watchdog stays as a backstop.

use bevy::app::AppExit;
use bevy::ecs::message::Messages;
use bevy::ecs::schedule::MainThreadExecutor;
use bevy::prelude::*;
use bevy::render::pipelined_rendering::{RenderAppChannels, RenderExtractApp};
use bevy::tasks::ComputeTaskPool;

pub struct CleanExitPlugin;

impl Plugin for CleanExitPlugin {
    fn build(&self, _app: &mut App) {}

    /// After every plugin's build, so the pipelined-rendering sub-app exists; absent when headless.
    fn finish(&self, app: &mut App) {
        if let Some(sub_app) = app.get_sub_app_mut(RenderExtractApp) {
            sub_app.set_extract(extract_or_reclaim);
        }
    }
}

/// Whether the main world asks to exit this frame: what the runner reads right after it.
fn exiting(app_world: &World) -> bool {
    app_world
        .get_resource::<Messages<AppExit>>()
        .is_some_and(|m| !m.is_empty())
}

/// Bevy's `renderer_extract`, plus the reclaim on an exiting frame. Waits for the render world
/// while running the main thread's tasks, as Bevy's does.
pub fn extract_or_reclaim(app_world: &mut World, _world: &mut World) {
    // winit can run another update after requesting exit. A reclaimed pipeline has no next
    // render world to receive; removing the channels makes that terminal state persistent.
    if !app_world.contains_resource::<RenderAppChannels>() {
        return;
    }
    let exit = exiting(app_world);
    let mut retired = exit;
    app_world.resource_scope(|world, executor: Mut<MainThreadExecutor>| {
        world.resource_scope(|world, mut channels: Mut<RenderAppChannels>| {
            let received = ComputeTaskPool::get()
                .scope_with_executor(true, Some(&*executor.0), |s| {
                    s.spawn(async { channels.recv().await });
                })
                .pop()
                .unwrap();
            match received {
                Some(mut render_app) if !exit => {
                    render_app.extract(world);
                    channels.send_blocking(render_app);
                }
                // Exiting: the render world stays home and drops here, on the main thread.
                Some(render_app) => drop(render_app),
                // The render thread panicked.
                None => {
                    retired = true;
                    world.write_message(AppExit::error());
                }
            }
        });
    });
    if retired {
        // Drop the sender too: the render thread can now leave its receive loop.
        app_world.remove_resource::<RenderAppChannels>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::schedule::ScheduleLabel;
    use bevy::ecs::system::NonSendMarker;
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct Frames(u32);

    fn main_thread_system(_main: NonSendMarker, mut frames: ResMut<Frames>) {
        frames.0 += 1;
    }

    /// The deadlock's shape without a GPU: a render sub-app whose schedule runs a main-thread
    /// system on the multi-threaded executor, on a render thread like Bevy's. After an exiting
    /// frame, dropping the main world must not wait.
    #[test]
    fn an_exiting_frame_leaves_nothing_in_flight_and_teardown_returns() {
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);
            let (to_render, render_rx) = async_channel::bounded::<bevy::app::SubApp>(1);
            let (render_tx, to_app) = async_channel::bounded::<bevy::app::SubApp>(1);

            let mut main = World::new();
            main.init_resource::<Messages<AppExit>>();
            let executor = MainThreadExecutor::new();
            let mut render_app = bevy::app::SubApp::new();
            render_app.world_mut().insert_resource(executor.clone());
            render_app.world_mut().init_resource::<Frames>();
            render_app.add_systems(bevy::app::Main, main_thread_system);
            render_app.update_schedule = Some(bevy::app::Main.intern());
            main.insert_resource(executor);
            render_tx.send_blocking(render_app).unwrap();
            main.insert_resource(RenderAppChannels::new(to_render, to_app));

            // The render thread, as `PipelinedRenderingPlugin::cleanup` runs it.
            let render_thread = std::thread::spawn(move || loop {
                let Ok(mut app) = render_rx.recv_blocking() else {
                    break;
                };
                app.update();
                if render_tx.send_blocking(app).is_err() {
                    break;
                }
            });

            let mut scratch = World::new();
            for _ in 0..3 {
                extract_or_reclaim(&mut main, &mut scratch);
            }
            main.write_message(AppExit::Success);
            extract_or_reclaim(&mut main, &mut scratch);
            assert!(!main.contains_resource::<RenderAppChannels>());
            main.resource_mut::<Messages<AppExit>>().clear();
            // macOS may schedule another update after the runner requests exit. The render
            // world was already reclaimed: there is no next frame to receive.
            extract_or_reclaim(&mut main, &mut scratch);
            drop(main);
            render_thread.join().expect("render thread stopped");
            done.send(()).unwrap();
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "teardown waited on a render frame in flight"
        );
    }
}
