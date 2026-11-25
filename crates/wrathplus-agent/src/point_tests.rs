//! A pixel to a world point: the heightfield march, the hit against colliders, the bearing's
//! conventions, and `AgentPoint` in Lua.

use super::*;
use crate::look::tests::{view_at, SIZE};
use crate::look::{Frames, ShotPose};
use std::f32::consts::FRAC_PI_4;
use std::sync::{Arc, Mutex};

/// A stand-in world: a heightfield (Bevy space) and maybe one collider at a fixed distance.
struct Fake {
    height: fn(Vec3) -> Option<f32>,
    wall: Option<(f32, &'static str)>,
}

impl Ground for Fake {
    fn collide(&self, _: Ray3d, max: f32) -> Option<(f32, &'static str)> {
        self.wall.filter(|w| w.0 <= max)
    }
    fn height(&self, at: Vec3) -> Option<f32> {
        (self.height)(at)
    }
}

const FLAT: Fake = Fake {
    height: |_| Some(0.0),
    wall: None,
};
/// The ground rises northward, half a yard a yard: WoW `z = x / 2`, Bevy `y = -z / 2`.
const SLOPE: Fake = Fake {
    height: |p| Some(-0.5 * p.z),
    wall: None,
};

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    Vec3::from(a).distance(Vec3::from(b)) < 0.01
}

#[test]
fn the_march_finds_flat_and_sloped_ground_and_not_a_ceiling_over_the_eye() {
    let down = Ray3d::new(
        Vec3::new(0.0, 10.0, 0.0),
        Dir3::new(Vec3::new(0.0, -1.0, -1.0)).unwrap(),
    );
    let t = march(down, MAX_RANGE, |_| Some(0.0)).unwrap();
    assert!((t - 10.0 * 2f32.sqrt()).abs() < 1e-3, "{t}");
    // y = 10 - s and y = s / 2 (north is Bevy -z) meet at s = 20/3.
    let t = march(down, MAX_RANGE, |p| Some(-0.5 * p.z)).unwrap();
    assert!((t - 20.0 / 3.0 * 2f32.sqrt()).abs() < 1e-3, "{t}");
    // Under the terrain from the start (a cave): no crossing, no hit at the eye.
    assert_eq!(march(down, MAX_RANGE, |_| Some(20.0)), None);
    // Level, above flat ground: never meets it. Nothing loaded: nothing to meet.
    let level = Ray3d::new(Vec3::new(0.0, 10.0, 0.0), Dir3::NEG_Z);
    assert_eq!(march(level, MAX_RANGE, |_| Some(0.0)), None);
    assert_eq!(march(down, MAX_RANGE, |_| None), None);
}

/// The camera 10 yd up at the origin, looking north 45 degrees down; the player at the origin
/// facing north; a half-size image.
fn frame() -> Frame {
    Frame {
        id: 4,
        time: 0.0,
        width: 640,
        height: 360,
        pose: ShotPose {
            view: Some(view_at([0.0, 0.0, 10.0], 0.0, -FRAC_PI_4, SIZE)),
            body: Some(Body {
                pos: [0.0, 0.0, 0.0],
                facing: 0.0,
                map: 0,
            }),
        },
    }
}

#[test]
fn the_centre_meets_the_ground_ahead_flat_or_sloped() {
    let p = point(&frame(), 320.0, 180.0, Some(&FLAT), None).unwrap();
    let (at, kind) = p.hit.unwrap();
    assert!(near(at, [10.0, 0.0, 0.0]), "{at:?}");
    assert_eq!(kind, "terrain");
    assert!((p.distance.unwrap() - 10.0).abs() < 0.01);
    assert!((p.range.unwrap() - 10.0 * 2f32.sqrt()).abs() < 0.01);
    assert!(p.bearing.abs() < 1e-3 && p.turn.unwrap().abs() < 1e-3);
    let (at, _) = point(&frame(), 320.0, 180.0, Some(&SLOPE), None)
        .unwrap()
        .hit
        .unwrap();
    assert!(near(at, [20.0 / 3.0, 0.0, 10.0 / 3.0]), "{at:?}");
}

/// Right of centre is east of north: the point's y is negative (WoW +y is west), the bearing falls
/// below the facing and the turn is negative (clockwise); left of centre the reverse.
#[test]
fn right_of_centre_is_a_clockwise_turn_left_of_it_counter_clockwise() {
    let right = point(&frame(), 480.0, 180.0, Some(&FLAT), None).unwrap();
    let (at, _) = right.hit.unwrap();
    assert!(at[1] < -1.0 && at[0] > 5.0, "{at:?}");
    assert!(right.turn.unwrap() < 0.0 && right.bearing > std::f32::consts::PI);
    let left = point(&frame(), 160.0, 180.0, Some(&FLAT), None).unwrap();
    assert!(left.hit.unwrap().0[1] > 1.0 && left.turn.unwrap() > 0.0);
    // The player turned west: the same point north-east is now a long clockwise turn.
    let west = Body {
        pos: [0.0, 0.0, 0.0],
        facing: std::f32::consts::FRAC_PI_2,
        map: 0,
    };
    let turned = point(&frame(), 480.0, 180.0, Some(&FLAT), Some(west)).unwrap();
    assert!(turned.turn.unwrap() < -std::f32::consts::FRAC_PI_2);
}

#[test]
fn a_nearer_collider_wins_and_names_what_it_is() {
    let wall = Fake {
        height: |_| Some(0.0),
        wall: Some((5.0, "wmo")),
    };
    let p = point(&frame(), 320.0, 180.0, Some(&wall), None).unwrap();
    let (at, kind) = p.hit.unwrap();
    assert_eq!(kind, "wmo");
    // 5 yd down the 45-degree ray from the near plane.
    assert!(
        (at[0] - 5.0 / 2f32.sqrt()).abs() < 0.1 && at[2] > 6.0,
        "{at:?}"
    );
    let far_wall = Fake {
        height: |_| Some(0.0),
        wall: Some((100.0, "doodad")),
    };
    assert_eq!(
        point(&frame(), 320.0, 180.0, Some(&far_wall), None)
            .unwrap()
            .hit
            .unwrap()
            .1,
        "terrain"
    );
}

#[test]
fn the_sky_is_a_bearing_and_why() {
    // The top edge looks 22.5 degrees below level: still ground. A level camera's top: sky.
    let mut level = frame();
    level.pose.view = Some(view_at([0.0, 0.0, 10.0], 0.0, 0.0, SIZE));
    let sky = point(&level, 600.0, 10.0, Some(&FLAT), None).unwrap();
    assert_eq!(sky.hit, None);
    assert!(sky.why.as_deref().unwrap().contains("meets nothing"));
    assert!(sky.turn.unwrap() < 0.0 && sky.pitch > 0.0);
    let json = sky.json();
    assert!(json.get("x").is_none() && json["why"].is_string() && json["turn"].is_number());
    // No world lent (a frame script): the bearing alone, and it says why.
    let bare = point(&frame(), 320.0, 180.0, None, None).unwrap();
    assert!(bare.hit.is_none() && bare.why.unwrap().contains("control-port"));
}

#[test]
fn unusable_requests_say_why() {
    assert!(point(&frame(), 641.0, 10.0, Some(&FLAT), None)
        .unwrap_err()
        .contains("outside"));
    assert!(point(&frame(), 10.0, -1.0, Some(&FLAT), None).is_err());
    let mut out = frame();
    out.pose.view = None;
    assert!(point(&out, 10.0, 10.0, Some(&FLAT), None)
        .unwrap_err()
        .contains("no world camera"));
}

#[test]
fn agent_point_in_lua_traces_only_inside_a_lent_world() {
    let lua = Lua::new();
    let frames: SharedFrames = Arc::new(Mutex::new(Frames::default()));
    let (world, _link) = crate::world::link();
    install(&lua, &frames, &world).unwrap();
    let (none, why): (mlua::Value, String) = lua.load("return AgentPoint(1, 1)").eval().unwrap();
    assert!(none.is_nil() && why.contains("no screenshot yet"), "{why}");
    let id = frames.lock().unwrap().add(frame());
    let probe = format!(
        "local p, why, dir = AgentPoint(320, 180, {id}) \
         if p then return p.hit, p.x, p.distance, p.frame end \
         return why, dir.bearing, dir.turn, dir.frame"
    );
    let bare: (String, f32, f32, u64) = lua.load(&probe).eval().unwrap();
    assert!(bare.0.contains("control-port") && bare.3 == id, "{bare:?}");
    let traced: (String, f32, f32, u64) = with_ground(&lua, &frames, &world, &FLAT, || {
        lua.load(&probe).eval().unwrap()
    });
    assert_eq!(traced.0, "terrain");
    assert!(
        (traced.1 - 10.0).abs() < 0.01 && (traced.2 - 10.0).abs() < 0.1,
        "{traced:?}"
    );
    // After the request, frame scripts get the bearing-only form back.
    let after: (String, f32, f32, u64) = lua.load(&probe).eval().unwrap();
    assert!(after.0.contains("control-port"));
}
