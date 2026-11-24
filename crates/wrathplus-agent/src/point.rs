//! `AgentPoint(u, v [, frame])` and `GET /point?u=&v=[&frame=]`: what is at a pixel of a
//! screenshot. The ray is that frame's camera's through the pixel (`look::View::ray`, the mouse
//! pick's unprojection), met against the world as it is now:
//!
//! - the mouse pick's occluders (`collision::PickOccluder`: terrain, WMO faces, static doodad
//!   hulls), the reference's `CWorld::Intersect` leg, through the physics query; `hit` names the
//!   collider's source, "terrain", "wmo", "doodad" or "other";
//! - the MCNK heightfield (`WorldPoint::terrain_height_under`), marched, for terrain farther out
//!   than the colliders reach; `hit` "terrain".
//!
//! The nearer answer wins. Units and liquid surfaces are not met: the ray passes through a
//! creature and stops on the lake bed. Nothing loaded on the ray (sky, beyond the streamed tiles)
//! is `nil, why` with the ray's bearing as a third value, so the agent can still turn that way.
//!
//! The world is traced only inside a control-port request (`/lua`, `/point`), where the game
//! loop lends it; called from a frame script, `AgentPoint` answers the bearing alone.

use crate::look::{Body, Frame, SharedFrames};
use crate::world::{Handles as WorldHandles, Me};
use avian3d::prelude::SpatialQuery;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use mlua::{IntoLua, Lua, MultiValue};
use serde_json::{json, Value};
use std::f32::consts::{PI, TAU};
use wrathplus_assets::coords::bevy_to_wow;
use wrathplus_world::collision::{ColliderSource, PickOccluder, WorldCollision};
use wrathplus_world::world_point::WorldPoint;

/// How far along the ray the world is looked for, yards: past the streamed terrain's reach.
pub(crate) const MAX_RANGE: f32 = 1200.0;

/// The world a ray meets.
pub(crate) trait Ground {
    /// The nearest collider on the ray within `max`: its distance along the ray and its kind.
    fn collide(&self, ray: Ray3d, max: f32) -> Option<(f32, &'static str)>;
    /// The terrain height (Bevy y) under a Bevy point, `None` off the loaded tiles.
    fn height(&self, at: Vec3) -> Option<f32>;
}

/// The live world, as the game loop lends it to a request.
#[derive(SystemParam)]
pub(crate) struct LiveGround<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    occluders: Query<'w, 's, (), With<PickOccluder>>,
    sources: Query<'w, 's, &'static ColliderSource>,
    point: WorldPoint<'w, 's>,
}

impl Ground for LiveGround<'_, '_> {
    fn collide(&self, ray: Ray3d, max: f32) -> Option<(f32, &'static str)> {
        let hit = self.spatial.cast_ray_predicate(
            ray.origin,
            ray.direction,
            max,
            true,
            &WorldCollision::body_filter(),
            &|e| self.occluders.contains(e),
        )?;
        let kind = match self.sources.get(hit.entity).copied().unwrap_or_default() {
            ColliderSource::Terrain => "terrain",
            ColliderSource::Wmo => "wmo",
            ColliderSource::M2 => "doodad",
            ColliderSource::Runtime => "other",
        };
        Some((hit.distance, kind))
    }

    fn height(&self, at: Vec3) -> Option<f32> {
        self.point.terrain_height_under(at)
    }
}

/// The step along the ray at distance `t`: fine near the camera, coarser far off.
fn step(t: f32) -> f32 {
    (t * 0.01).clamp(0.25, 2.0)
}

/// Where `ray` first goes from above the heightfield to on or under it, within `max`: its
/// distance along the ray. A start under the terrain (a camera in a cave) waits for the ray to
/// come out above it first; a gap in the loaded tiles breaks the crossing.
pub(crate) fn march(ray: Ray3d, max: f32, height: impl Fn(Vec3) -> Option<f32>) -> Option<f32> {
    let gap = |t: f32| {
        let p = ray.get_point(t);
        height(p).map(|h| p.y - h)
    };
    let mut above: Option<f32> = None;
    let mut t = 0.0;
    while t <= max {
        match gap(t) {
            Some(g) if g <= 0.0 => {
                if let Some(mut lo) = above {
                    let mut hi = t;
                    for _ in 0..24 {
                        let mid = 0.5 * (lo + hi);
                        if gap(mid).is_some_and(|g| g <= 0.0) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    return Some(hi);
                }
            }
            Some(_) => above = Some(t),
            None => above = None,
        }
        t += step(t);
    }
    None
}

/// What a pixel shows: the world point the ray meets, or the bearing alone.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pointed {
    pub frame: u64,
    /// The point in world coordinates and what was met.
    pub hit: Option<([f32; 3], &'static str)>,
    /// From the camera to the point along the ray, yards.
    pub range: Option<f32>,
    /// From the player to the point, yards.
    pub distance: Option<f32>,
    /// Which way the point (or the ray, on a miss) lies from the player, as
    /// `AgentPosition().facing`: 0 north, counter-clockwise, radians.
    pub bearing: f32,
    /// How far to turn from the player's facing to face it: -pi..pi, positive to the left.
    pub turn: Option<f32>,
    /// The ray's pitch, positive up.
    pub pitch: f32,
    /// Why nothing was met.
    pub why: Option<String>,
}

/// An angle into -pi..pi.
fn wrap(angle: f32) -> f32 {
    let a = (angle + PI).rem_euclid(TAU) - PI;
    if a <= -PI {
        a + TAU
    } else {
        a
    }
}

/// Cast through `(u, v)` of `frame` and meet `ground` (none: the bearing alone, `why` saying so).
/// `me` is where the player is now (the frame's own player without it). An unusable request (no
/// camera, a pixel off the image) is `Err`.
pub(crate) fn point(
    frame: &Frame,
    u: f32,
    v: f32,
    ground: Option<&dyn Ground>,
    me: Option<Body>,
) -> Result<Pointed, String> {
    let (w, h) = (frame.width as f32, frame.height as f32);
    if !(0.0..=w).contains(&u) || !(0.0..=h).contains(&v) {
        return Err(format!(
            "pixel ({u}, {v}) is outside frame {}'s {}x{} image",
            frame.id, frame.width, frame.height
        ));
    }
    let view = frame
        .pose
        .view
        .as_ref()
        .ok_or_else(|| format!("frame {} has no world camera (not in the world)", frame.id))?;
    let ray = view
        .ray(u, v, frame.width, frame.height)
        .ok_or_else(|| format!("frame {}'s camera had no viewport", frame.id))?;
    let (ray_yaw, pitch) = crate::look::yaw_pitch(*ray.direction);
    let me = me.or(frame.pose.body);
    let mut out = Pointed {
        frame: frame.id,
        hit: None,
        range: None,
        distance: None,
        bearing: ray_yaw,
        turn: me.map(|m| wrap(ray_yaw - m.facing)),
        pitch,
        why: None,
    };
    let Some(ground) = ground else {
        out.why = Some(
            "AgentPoint traces the world only in a control-port call (/lua, /point); here, the bearing alone"
                .into(),
        );
        return Ok(out);
    };
    let collider = ground.collide(ray, MAX_RANGE);
    let terrain = march(ray, MAX_RANGE, |p| ground.height(p)).map(|t| (t, "terrain"));
    let Some((t, kind)) = [collider, terrain]
        .into_iter()
        .flatten()
        .min_by(|a, b| a.0.total_cmp(&b.0))
    else {
        out.why = Some(format!(
            "the ray meets nothing loaded within {MAX_RANGE} yd (sky, or past the streamed world)"
        ));
        return Ok(out);
    };
    let at = bevy_to_wow(ray.get_point(t));
    let eye = bevy_to_wow(view.transform.translation());
    out.hit = Some((at, kind));
    out.range = Some(Vec3::from(at).distance(Vec3::from(eye)));
    if let Some(m) = me {
        let (dx, dy) = (at[0] - m.pos[0], at[1] - m.pos[1]);
        out.distance = Some(Vec3::from(at).distance(Vec3::from(m.pos)));
        if dx.hypot(dy) > 1e-3 {
            out.bearing = dy.atan2(dx).rem_euclid(TAU);
            out.turn = Some(wrap(out.bearing - m.facing));
        }
    }
    Ok(out)
}

fn round(value: f32, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (f64::from(value) * scale).round() / scale
}

impl Pointed {
    /// The answer's fields: `{frame, x, y, z, hit, range, distance, bearing, turn, pitch}` on a
    /// hit, `{frame, bearing, turn, pitch, why}` on a miss.
    pub(crate) fn json(&self) -> Value {
        let mut v = json!({"frame": self.frame, "bearing": round(self.bearing, 4),
            "turn": self.turn.map(|t| round(t, 4)), "pitch": round(self.pitch, 4)});
        if let Some((at, kind)) = self.hit {
            v["x"] = json!(round(at[0], 2));
            v["y"] = json!(round(at[1], 2));
            v["z"] = json!(round(at[2], 2));
            v["hit"] = json!(kind);
            v["range"] = json!(self.range.map(|r| round(r, 1)));
            v["distance"] = json!(self.distance.map(|d| round(d, 1)));
        }
        if let Some(why) = &self.why {
            v["why"] = json!(why);
        }
        v
    }
}

/// The player now, from the world snapshot.
fn me_now(world: &WorldHandles) -> Option<Body> {
    let me: Me = world.snapshot.lock().ok()?.me?;
    Some(Body {
        pos: me.pos,
        facing: me.facing,
        map: me.map,
    })
}

/// `GET /point`'s answer, and `AgentPoint`'s before it becomes Lua: `Err` is the reason.
pub(crate) fn answer(
    frames: &SharedFrames,
    world: &WorldHandles,
    (u, v, frame): (f32, f32, Option<u64>),
    ground: Option<&dyn Ground>,
) -> Result<Pointed, String> {
    let frames = frames.lock().map_err(|_| "frames unavailable".to_owned())?;
    point(frames.get(frame)?, u, v, ground, me_now(world))
}

/// A flat JSON object as a Lua table.
fn table(lua: &Lua, value: &Value) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    if let Value::Object(map) = value {
        for (k, v) in map {
            match v {
                Value::Number(n) => t.set(k.as_str(), n.as_f64())?,
                Value::String(s) => t.set(k.as_str(), s.as_str())?,
                Value::Bool(b) => t.set(k.as_str(), *b)?,
                _ => {}
            }
        }
    }
    Ok(t)
}

/// `AgentPoint`'s Lua results: the hit table; or `nil, why, {frame, bearing, turn, pitch}`.
fn to_lua(lua: &Lua, result: Result<Pointed, String>) -> mlua::Result<MultiValue> {
    match result {
        Ok(p) if p.hit.is_some() => Ok(MultiValue::from_vec(vec![
            table(lua, &p.json())?.into_lua(lua)?
        ])),
        Ok(p) => {
            let mut fields = p.json();
            let why = fields
                .as_object_mut()
                .and_then(|m| m.remove("why"))
                .and_then(|w| w.as_str().map(str::to_owned));
            Ok(MultiValue::from_vec(vec![
                mlua::Value::Nil,
                why.into_lua(lua)?,
                table(lua, &fields)?.into_lua(lua)?,
            ]))
        }
        Err(why) => Ok(MultiValue::from_vec(vec![
            mlua::Value::Nil,
            why.into_lua(lua)?,
        ])),
    }
}

type Args = (f32, f32, Option<u64>);

/// Install `AgentPoint` into this VM, the bearing-only form a frame script gets.
pub(crate) fn install(lua: &Lua, frames: &SharedFrames, world: &WorldHandles) -> mlua::Result<()> {
    let (frames, world) = (frames.clone(), world.clone());
    lua.globals().set(
        "AgentPoint",
        lua.create_function(move |lua, args: Args| {
            to_lua(lua, answer(&frames, &world, args, None))
        })?,
    )
}

/// Run `body` with `AgentPoint` tracing `ground`, then put the bearing-only form back.
pub(crate) fn with_ground<R>(
    lua: &Lua,
    frames: &SharedFrames,
    world: &WorldHandles,
    ground: &dyn Ground,
    body: impl FnOnce() -> R,
) -> R {
    let globals = lua.globals();
    let saved: mlua::Value = globals.get("AgentPoint").unwrap_or(mlua::Value::Nil);
    let mut body = Some(body);
    let mut out = None;
    let traced = lua.scope(|scope| {
        let f = scope.create_function(|lua, args: Args| {
            to_lua(lua, answer(frames, world, args, Some(ground)))
        })?;
        globals.set("AgentPoint", f)?;
        out = body.take().map(|b| b());
        Ok(())
    });
    let _ = globals.set("AgentPoint", saved);
    match (out, body) {
        (Some(out), _) => out,
        // The scope never ran the body (it could not make the function): run it untraced.
        (None, Some(body)) => {
            if let Err(e) = traced {
                error!("agent port: AgentPoint: {e}");
            }
            body()
        }
        (None, None) => unreachable!("the body runs once"),
    }
}

#[cfg(test)]
#[path = "point_tests.rs"]
mod tests;
