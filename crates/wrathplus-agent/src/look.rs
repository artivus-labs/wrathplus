//! `GET /screenshot` and the frames an agent can point into: each capture carries the camera
//! and the player of the frame it shows, read where that frame's render reads them, and each
//! image sent is kept as a numbered [`Frame`] so `AgentPoint` (`point`) can cast a ray through
//! one of its pixels.
//!
//! The pose is stamped onto the `Screenshot` entity in `PostUpdate` after transform propagation
//! and the camera update, the frame it was spawned in: the renderer extracts that entity and the
//! camera together right after, so the image and the pose are the same frame's.

use crate::server::Format;
use bevy::camera::CameraUpdateSystems;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::transform::TransformSystems;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use wrathplus_app_core::app_core::state::ClientState;
use wrathplus_assets::coords::bevy_to_wow;
use wrathplus_world::view::WorldCamera;

/// How many sent images stay addressable by frame number.
pub(crate) const KEPT: usize = 16;

/// What a capture's answer carries: the encoded image and its frame (`meta`).
pub(crate) type Shot = Result<(Vec<u8>, Value), String>;

/// The world camera as one frame rendered it: Bevy's camera with its computed projection and
/// target, so a pixel unprojects through `Camera::viewport_to_world`, the mouse pick's own path.
#[derive(Clone, Debug)]
pub(crate) struct View {
    pub camera: Camera,
    pub transform: GlobalTransform,
    /// The vertical field of view, radians (`view::CAM_FOVY` unless a spyglass narrowed it).
    pub fov_v: f32,
}

/// The player in a frame: world coordinates, facing as `AgentPosition` reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Body {
    pub pos: [f32; 3],
    pub facing: f32,
    pub map: u32,
}

/// The pose stamped onto a `Screenshot` entity in the frame it renders.
#[derive(Component, Clone, Debug, Default)]
pub(crate) struct ShotPose {
    pub view: Option<View>,
    pub body: Option<Body>,
}

/// One image as sent: its size after the asker's scale, and the pose of the frame it shows.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub id: u64,
    /// Seconds since the port opened, as `/status` and the events count.
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub pose: ShotPose,
}

/// The last [`KEPT`] frames sent, newest last.
#[derive(Default)]
pub(crate) struct Frames {
    next: u64,
    kept: VecDeque<Frame>,
}

pub(crate) type SharedFrames = Arc<Mutex<Frames>>;

impl Frames {
    /// Keep a frame, numbering it; the oldest beyond [`KEPT`] is dropped.
    pub(crate) fn add(&mut self, mut frame: Frame) -> u64 {
        self.next += 1;
        frame.id = self.next;
        self.kept.push_back(frame);
        while self.kept.len() > KEPT {
            self.kept.pop_front();
        }
        self.next
    }

    /// Frame `id`, or the newest without one; why not when it is gone or never was.
    pub(crate) fn get(&self, id: Option<u64>) -> Result<&Frame, String> {
        match id {
            None => self
                .kept
                .back()
                .ok_or_else(|| "no screenshot yet: take one, then point into it".to_owned()),
            Some(id) => self.kept.iter().find(|f| f.id == id).ok_or_else(|| {
                format!("no frame {id}: only the last {KEPT} screenshots are kept; take a new one")
            }),
        }
    }
}

impl View {
    /// The camera ray through `(u, v)` of a `width` × `height` image of this frame: pixels from
    /// the top-left corner, x right and y down, fractions allowed. The world camera's viewport is
    /// the whole window, so the image's fractions are the viewport's.
    pub(crate) fn ray(&self, u: f32, v: f32, width: u32, height: u32) -> Option<Ray3d> {
        let rect = self.camera.logical_viewport_rect()?;
        let at = rect.min + Vec2::new(u / width as f32, v / height as f32) * rect.size();
        self.camera.viewport_to_world(&self.transform, at).ok()
    }

    /// Width over height of the viewport.
    fn aspect(&self) -> f32 {
        self.camera
            .logical_viewport_size()
            .map_or(1.0, |s| s.x / s.y.max(1.0))
    }

    /// The horizontal field of view, radians.
    pub(crate) fn fov_h(&self) -> f32 {
        2.0 * ((self.fov_v / 2.0).tan() * self.aspect()).atan()
    }

    /// Where the camera looks: yaw as `AgentPosition().facing` (0 north, counter-clockwise) and
    /// pitch (positive up), radians.
    pub(crate) fn yaw_pitch(&self) -> (f32, f32) {
        yaw_pitch(*self.transform.forward())
    }
}

/// A Bevy direction as WoW yaw (0 = north, +x; counter-clockwise, toward +y west) and pitch.
pub(crate) fn yaw_pitch(dir: Vec3) -> (f32, f32) {
    let [x, y, z] = bevy_to_wow(dir.normalize_or_zero());
    (
        y.atan2(x).rem_euclid(std::f32::consts::TAU),
        z.clamp(-1.0, 1.0).asin(),
    )
}

fn round(value: f32, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (f64::from(value) * scale).round() / scale
}

impl Frame {
    /// The frame as the screenshot's `X-Agent-Frame` header and the MCP tool report it.
    pub(crate) fn meta(&self) -> Value {
        let camera = self.pose.view.as_ref().map(|view| {
            let [x, y, z] = bevy_to_wow(view.transform.translation());
            let (yaw, pitch) = view.yaw_pitch();
            json!({"x": round(x, 2), "y": round(y, 2), "z": round(z, 2),
                "yaw": round(yaw, 4), "pitch": round(pitch, 4),
                "fov_v": round(view.fov_v, 4), "fov_h": round(view.fov_h(), 4)})
        });
        let player = self.pose.body.map(|b| {
            json!({"x": round(b.pos[0], 2), "y": round(b.pos[1], 2), "z": round(b.pos[2], 2),
                "facing": round(b.facing, 4), "map": b.map})
        });
        json!({"frame": self.id, "time": (self.time * 1000.0).round() / 1000.0,
            "width": self.width, "height": self.height, "camera": camera, "player": player})
    }
}

/// Stamp every capture spawned this frame with the frame's camera and player. Runs after the
/// transforms propagate and the projections update, so nothing moves the camera before the render
/// reads it.
#[allow(clippy::type_complexity)]
pub(crate) fn stamp(
    mut commands: Commands,
    shots: Query<Entity, (With<Screenshot>, Without<ShotPose>)>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<WorldCamera>>,
    player: Option<Res<wrathplus_app_core::app_core::player::Player>>,
    map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
    state: Option<Res<State<ClientState>>>,
) {
    if shots.is_empty() {
        return;
    }
    let view = cameras.iter().find(|(camera, ..)| camera.is_active).map(
        |(camera, transform, projection)| View {
            camera: camera.clone(),
            transform: *transform,
            fov_v: match projection {
                Projection::Perspective(p) => p.fov,
                _ => wrathplus_world::view::CAM_FOVY,
            },
        },
    );
    let in_world = state.is_some_and(|s| *s.get() == ClientState::InWorld);
    let body = player.filter(|_| in_world).map(|p| Body {
        pos: bevy_to_wow(p.pos),
        facing: p.facing().rem_euclid(std::f32::consts::TAU),
        map: map.map_or(0, |m| m.0),
    });
    for shot in &shots {
        commands.entity(shot).insert(ShotPose {
            view: view.clone(),
            body,
        });
    }
}

/// Where [`stamp`] runs: `PostUpdate`, after the camera's final pose and projection.
pub(crate) fn add_stamp(app: &mut App) {
    app.add_systems(
        PostUpdate,
        stamp
            .after(TransformSystems::Propagate)
            .after(CameraUpdateSystems),
    );
}

/// One asker of the capture in flight: its scale, encoding and reply.
pub(crate) struct Asker {
    pub scale: f32,
    pub format: Format,
    pub reply: crossbeam_channel::Sender<Shot>,
}

/// The capture in flight and its askers.
#[derive(Default)]
pub(crate) struct Capture {
    waiting: Arc<Mutex<Vec<Asker>>>,
    at: Option<f64>,
}

impl Capture {
    /// Ask for the next frame. One capture at a time serves everyone who asks while it is in
    /// flight: a second capture of the same target is dropped by the renderer ("Duplicate render
    /// target for screenshot"), and its asker would wait out the port's timeout. A capture that
    /// never lands (2 s is many frames) must not hold the rest: ask again.
    pub(crate) fn ask(
        &mut self,
        commands: &mut Commands,
        asker: Asker,
        now: f64,
        headless: Option<&wrathplus_world::headless::HeadlessFrame>,
        frames: &SharedFrames,
    ) {
        let mut waiting = self.waiting.lock().unwrap_or_else(|p| p.into_inner());
        let in_flight = !waiting.is_empty() && self.at.is_some_and(|at| now - at < 2.0);
        waiting.push(asker);
        if in_flight {
            return;
        }
        drop(waiting);
        self.at = Some(now);
        // A headless client draws into an image, not a window.
        let shot = match headless {
            Some(frame) => Screenshot::image(frame.0.clone()),
            None => Screenshot::primary_window(),
        };
        let waiting = self.waiting.clone();
        let frames = frames.clone();
        commands.spawn(shot).observe(
            move |captured: On<ScreenshotCaptured>, poses: Query<&ShotPose>| {
                let pose = poses.get(captured.entity).cloned().unwrap_or_default();
                let image = captured.image.clone();
                let askers =
                    std::mem::take(&mut *waiting.lock().unwrap_or_else(|p| p.into_inner()));
                let mut frames = frames.lock().unwrap_or_else(|p| p.into_inner());
                let sends: Vec<(Asker, Value)> = askers
                    .into_iter()
                    .map(|asker| {
                        let (width, height) = scaled(image.width(), image.height(), asker.scale);
                        let mut frame = Frame {
                            id: 0,
                            time: now,
                            width,
                            height,
                            pose: pose.clone(),
                        };
                        frame.id = frames.add(frame.clone());
                        (asker, frame.meta())
                    })
                    .collect();
                drop(frames);
                std::thread::spawn(move || {
                    for (asker, meta) in sends {
                        let encoded = encode(image.clone(), asker.scale, asker.format);
                        let _ = asker.reply.send(encoded.map(|bytes| (bytes, meta)));
                    }
                });
            },
        );
    }
}

/// An image's size after `scale` (0 < scale <= 1), as [`encode`] resizes it.
pub(crate) fn scaled(width: u32, height: u32, scale: f32) -> (u32, u32) {
    if scale >= 1.0 {
        return (width, height);
    }
    let by = |n: u32| ((n as f32) * scale).round().max(1.0) as u32;
    (by(width), by(height))
}

/// A captured frame as PNG or JPEG bytes, downscaled by `scale` (0 < scale <= 1).
fn encode(image: Image, scale: f32, format: Format) -> Result<Vec<u8>, String> {
    let mut frame = image
        .try_into_dynamic()
        .map_err(|e| format!("frame conversion: {e}"))?;
    let (width, height) = scaled(frame.width(), frame.height(), scale);
    if (width, height) != (frame.width(), frame.height()) {
        frame = frame.resize_exact(width, height, image::imageops::FilterType::Triangle);
    }
    let rgb = frame.to_rgb8();
    let mut bytes = Vec::new();
    match format {
        Format::Png => rgb
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .map_err(|e| format!("png: {e}"))?,
        Format::Jpeg(quality) => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
                .encode_image(&rgb)
                .map_err(|e| format!("jpeg: {e}"))?
        }
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "look_tests.rs"]
pub(crate) mod tests;
