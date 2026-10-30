use super::*;

use wrathplus_assets::WmoPortalRef;

/// Any visible referrer draws a prop; an index past the visible set fails open.
#[test]
fn a_prop_is_drawn_while_any_of_its_rooms_is_visible() {
    let inst = WmoPortalInstance {
        handle: Handle::default(),
        world_from_local: Affine3A::IDENTITY,
        name_set: 0,
        visible: vec![false, true, false],
        interior_fog: vec![false; 3],
        liquid_visited: vec![false; 3],
        flooded: vec![None; 3],
    };
    let key = |groups: &[u16]| WmoGroupVis {
        instance: Entity::PLACEHOLDER,
        groups: Arc::from(groups),
    };
    assert!(key(&[1]).drawn_by(&inst), "its own room is visible");
    assert!(!key(&[0]).drawn_by(&inst), "its own room is culled");
    assert!(
        key(&[0, 2, 1]).drawn_by(&inst),
        "one visible referrer out of three draws the fall"
    );
    assert!(!key(&[0, 2]).drawn_by(&inst), "no referrer is visible");
    assert!(
        !key(&[]).drawn_by(&inst),
        "no referrers at all: nothing ORs"
    );
    assert!(key(&[9]).drawn_by(&inst), "index past the set fails open");
}

/// The interior-fog key ORs like `drawn_by`, but a lookup miss reads false.
#[test]
fn the_interior_fog_key_ors_over_rooms_and_fails_closed() {
    let inst = WmoPortalInstance {
        handle: Handle::default(),
        world_from_local: Affine3A::IDENTITY,
        name_set: 0,
        visible: vec![true; 3],
        interior_fog: vec![false, true, false],
        liquid_visited: vec![false; 3],
        flooded: vec![None; 3],
    };
    let key = |groups: &[u16]| WmoGroupVis {
        instance: Entity::PLACEHOLDER,
        groups: Arc::from(groups),
    };
    assert!(key(&[1]).interior_fogged_by(&inst), "its own room is on it");
    assert!(
        !key(&[0]).interior_fogged_by(&inst),
        "its own room is off it"
    );
    assert!(
        key(&[0, 2, 1]).interior_fogged_by(&inst),
        "one referrer on the lane is enough"
    );
    assert!(
        !key(&[0, 2]).interior_fogged_by(&inst),
        "no referrer is on it"
    );
    assert!(
        !key(&[]).interior_fogged_by(&inst),
        "no referrers: nothing ORs"
    );
    assert!(
        !key(&[9]).interior_fogged_by(&inst),
        "index past the set fails CLOSED — the opposite of drawn_by"
    );
}

/// Only `0x8` opens a window (`0x6b44f8`). The flags are Stormwind's own, from a live cull
/// trace: of its 306 groups only `g46` (`0x08809`) carries `0x8`; its streets and rooms are
/// `0x40` without `0x8`.
#[test]
fn only_an_exterior_group_opens_a_window_onto_the_open_world() {
    let g = |flags: u32| WmoGroupNav {
        flags,
        bbox_min: [0.0; 3],
        bbox_max: [0.0; 3],
        ref_start: 0,
        ref_count: 0,
        area_table_id: 0,
        fog_indices: [0; 4],
        group_liquid: 0xf,
    };
    // Stormwind rooms and streets…
    let nav = [g(0x0b841), g(0x0a841), g(0x0ba41), g(0x08809), g(0x02a05)];
    for to in [0, 1, 2] {
        assert!(
            !opens_a_window(&nav, to),
            "g{to} is EXTERIOR_LIT (0x40) without EXTERIOR (0x8) — a lit indoor room, not a \
             doorway onto Elwynn"
        );
    }
    assert!(!opens_a_window(&nav, 4), "0x02a05 carries neither bit");
    // …and g46, the city's one exterior shell.
    assert!(
        opens_a_window(&nav, 3),
        "0x08809 carries 0x8 — this one IS the open world"
    );
    // A destination past the nav (a corrupt MOPR ref) fails closed: no doorway.
    assert!(
        !opens_a_window(&nav, 99),
        "an out-of-range group is no window"
    );
}

#[test]
fn eye_on_portal_only_within_the_band_and_polygon() {
    // A vertical doorway in the x=0 plane, spanning y ∈ [-2,2], z ∈ [0,4].
    let verts = vec![
        [0.0, -2.0, 0.0],
        [0.0, 2.0, 0.0],
        [0.0, 2.0, 4.0],
        [0.0, -2.0, 4.0],
    ];
    let info = WmoPortalInfo {
        start_vertex: 0,
        count: 4,
        plane: [1.0, 0.0, 0.0, 0.0],
    };
    assert!(eye_on_portal(&verts, &info, [0.0, 0.0, 2.0]));
    assert!(eye_on_portal(&verts, &info, [0.005, 1.5, 3.5]));
    // In the plane but above the door.
    assert!(!eye_on_portal(&verts, &info, [0.0, 0.0, 5.0]));
    // Within the door's span but off the plane.
    assert!(!eye_on_portal(&verts, &info, [0.5, 0.0, 2.0]));
}

#[test]
fn point_in_poly_2d_hits_inside_and_misses_outside() {
    let sq = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    assert!(point_in_poly_2d(sq.iter().copied(), (0.0, 0.0)));
    assert!(point_in_poly_2d(sq.iter().copied(), (-0.9, 0.9)));
    assert!(!point_in_poly_2d(sq.iter().copied(), (1.5, 0.0)));
    assert!(!point_in_poly_2d(sq.iter().copied(), (0.0, -1.5)));
}

#[test]
fn intersect_rect_collapses_to_none() {
    assert_eq!(
        intersect_rect([-1.0, -1.0, 0.0, 0.0], [-0.5, -0.5, 1.0, 1.0]),
        Some([-0.5, -0.5, 0.0, 0.0])
    );
    // Disjoint rects: the window collapses.
    assert_eq!(
        intersect_rect([-1.0, -1.0, -0.5, -0.5], [0.5, 0.5, 1.0, 1.0]),
        None
    );
    // A sliver thinner than the client's 0.001 zero-area eps also collapses.
    assert_eq!(
        intersect_rect([-1.0, -1.0, 1.0, 1.0], [0.0, -1.0, 0.0005, 1.0]),
        None
    );
}

#[test]
fn clip_side_planes_drops_behind_and_off_screen_keeps_straddling() {
    // Fully behind the eye (w < 0, inside the mirror cone): outside every side half-space.
    let behind = [
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.1, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.1, 0.0, -1.0),
    ];
    assert!(clip_side_planes(&behind).is_none());
    let front = [
        Vec4::new(0.0, 0.0, 0.0, 1.0),
        Vec4::new(0.5, 0.0, 0.0, 1.0),
        Vec4::new(0.0, 0.5, 0.0, 1.0),
    ];
    assert_eq!(clip_side_planes(&front).map(|p| p.len()), Some(3));
    // Off-screen laterally, x > w for every vertex.
    let off = [
        Vec4::new(2.0, 0.0, 0.0, 1.0),
        Vec4::new(3.0, 0.0, 0.0, 1.0),
        Vec4::new(2.0, 1.0, 0.0, 1.0),
    ];
    assert!(clip_side_planes(&off).is_none());
    // Straddling the eye plane, one vertex behind: survives, with boundary points near w = 0.
    let straddle = [
        Vec4::new(0.0, -0.5, 0.0, 1.0),
        Vec4::new(0.0, 0.5, 0.0, 1.0),
        Vec4::new(0.0, 0.0, 0.0, -0.5),
    ];
    assert!(clip_side_planes(&straddle).is_some());
}

#[test]
fn portal_rect_explodes_open_on_the_doorway_approach() {
    // The camera 0.05 yd before the doorway plane, outside the 0.01 on-plane band, looking
    // through it: with no near-plane clip the rect blows out to the whole screen.
    let model = portal_model();
    let info = &model.portal_infos[0];
    // WoW (−0.05, 0, 0) → Bevy (0, 0, 0.05); looking along WoW +x <=> Bevy −z.
    let view = Mat4::look_at_rh(
        bevy::math::Vec3::new(0.0, 0.0, 0.05),
        bevy::math::Vec3::new(0.0, 0.0, -10.0),
        bevy::math::Vec3::Y,
    );
    let clip = Mat4::perspective_rh(0.9, 16.0 / 9.0, 0.1, 1000.0) * view;
    let rect = portal_screen_rect(&model, info, &clip, &Affine3A::IDENTITY)
        .expect("a doorway ahead must keep a rect");
    let inter = intersect_rect(FULL_SCREEN, rect).expect("must overlap the screen");
    assert!(inter[0] <= -0.99 && inter[1] <= -0.99 && inter[2] >= 0.99 && inter[3] >= 0.99);
}

#[test]
fn portal_rect_is_none_when_fully_behind_the_camera() {
    let model = portal_model();
    let info = &model.portal_infos[0];
    // Camera 5 yd past the doorway (WoW +x is Bevy −z), looking on along −z.
    let view = Mat4::look_at_rh(
        bevy::math::Vec3::new(0.0, 0.0, -5.0),
        bevy::math::Vec3::new(0.0, 0.0, -10.0),
        bevy::math::Vec3::Y,
    );
    let clip = Mat4::perspective_rh(0.9, 16.0 / 9.0, 0.1, 1000.0) * view;
    assert!(portal_screen_rect(&model, info, &clip, &Affine3A::IDENTITY).is_none());
}

/// Pass 2 is a per-window test (`0x6b3d13`/`0x6b3d22`): of two portal-disconnected shells, the
/// one through the doorway draws and the one 30 yd aside does not.
#[test]
fn pass_two_admits_only_the_exterior_shells_a_doorway_shows() {
    let model = doorway_model();
    // Eye 3 yd back from the doorway, WoW (-3, 0, 1) = Bevy (0, 1, 3), looking along WoW +x
    // (Bevy −z), straight through the opening.
    let eye_local = [-3.0, 0.0, 1.0];
    let view = Mat4::look_at_rh(
        bevy::math::Vec3::new(0.0, 1.0, 3.0),
        bevy::math::Vec3::new(0.0, 1.0, -10.0),
        bevy::math::Vec3::Y,
    );
    let clip = Mat4::perspective_rh(0.9, 16.0 / 9.0, 0.1, 1000.0) * view;
    let pvs = compute_pvs_traced(&model, eye_local, None, &clip, &Affine3A::IDENTITY, &mut ());

    assert_eq!(
        pvs.windows.len(),
        1,
        "the one portal onto an exterior group is the one deferred window"
    );
    assert!(pvs.visible[0], "the camera's own room");
    assert!(pvs.visible[1], "the exterior group the portal opens onto");
    assert!(
        pvs.visible[2],
        "a portal-disconnected shell straight through the doorway is admitted by its window \
         — this is the half the flood alone can never reach"
    );
    assert!(
        !pvs.visible[3],
        "a portal-disconnected shell 30 yd off the doorway's axis is culled: no window shows \
         it, and drawing it would be pure over-draw"
    );
    assert!(
        pvs.visible[4],
        "an admitted shell is FLOODED FROM, not merely marked — g4 is an interior behind \
         g2 with no edge to the seed's half of the graph, so the walk through g2's own back \
         door is its only route into the PVS (`0x6b3d39 call 0x6b41c0`). A marking Pass 2 \
         leaves it culled, which from a Darnassus shop is most of the city"
    );

    // The control: standing in the doorway takes the full-screen branch, and the side shell
    // comes back, so the cull above is the window's.
    let in_door = compute_pvs_traced(
        &model,
        [0.0, 0.0, 1.0],
        None,
        &clip,
        &Affine3A::IDENTITY,
        &mut (),
    );
    assert!(
        in_door.visible[3],
        "with the doorway full-screen, every shell in front of the camera draws"
    );
}

/// Pass 3 draws a `0x10000` group from a sealed room (the `jbe` fall-through at `0x6b3d6f`):
/// with an empty worklist every `0x8` group is dark, yet the callback group draws, as
/// Razorfen's enclosing shells (`0x8` and `0x10000`) and Stormwind's canals do.
#[test]
fn pass_three_draws_a_callback_group_from_a_sealed_room() {
    // The doorway model, sealed: no portals, no windows. g1..g3 keep `0x8`; g3 gains `0x10000`.
    let mut model = doorway_model();
    model.portal_refs.clear();
    model.group_nav[0].ref_count = 0;
    model.group_nav[1].ref_count = 0;
    model.group_nav[3].flags |= CALLBACK_PASS;
    // Aim at g3, 30 yd off-axis, so the full frustum holds it: WoW (2..6, 30..34) is Bevy
    // x ∈ [-34,-30], z ∈ [-6,-2].
    let view = Mat4::look_at_rh(
        bevy::math::Vec3::new(0.0, 1.0, 3.0),
        bevy::math::Vec3::new(-32.0, 1.0, -4.0),
        bevy::math::Vec3::Y,
    );
    let clip = Mat4::perspective_rh(0.9, 16.0 / 9.0, 0.1, 1000.0) * view;
    let pvs = compute_pvs_traced(
        &model,
        [-3.0, 0.0, 1.0],
        None,
        &clip,
        &Affine3A::IDENTITY,
        &mut (),
    );

    assert!(
        pvs.windows.is_empty(),
        "a sealed room opens no doorway onto the outdoors"
    );
    assert!(pvs.visible[0], "the camera's own room");
    assert!(
        !pvs.visible[1] && !pvs.visible[2],
        "plain 0x8 shells stay dark with an empty worklist — Pass 2 never runs"
    );
    assert!(
        pvs.visible[3],
        "the 0x10000 group draws anyway: Pass 3 is the jbe fall-through, full frustum, ungated"
    );

    // The control: Pass 3 is still a frustum test, so facing away darkens g3.
    let away = Mat4::perspective_rh(0.9, 16.0 / 9.0, 0.1, 1000.0)
        * Mat4::look_at_rh(
            bevy::math::Vec3::new(0.0, 1.0, 3.0),
            bevy::math::Vec3::new(32.0, 1.0, 10.0),
            bevy::math::Vec3::Y,
        );
    let behind = compute_pvs_traced(
        &model,
        [-3.0, 0.0, 1.0],
        None,
        &away,
        &Affine3A::IDENTITY,
        &mut (),
    );
    assert!(
        !behind.visible[3],
        "a callback group behind the camera is still frustum-culled"
    );

    // The outside leg draws it too (`0x6b3dd0` forks on the same bit): an eye with no floor
    // under it seeds only `0x8` groups, and g3, with no portal refs, is Pass 3's alone.
    let outside = compute_pvs_traced(
        &model,
        [-3.0, 0.0, 500.0],
        None,
        &clip,
        &Affine3A::IDENTITY,
        &mut (),
    );
    assert!(
        outside.visible[3],
        "the callback arm belongs to BOTH legs — gating it on the inside leg left 20 of the 24 \
         real groups undrawable from anywhere"
    );
}

/// An interior room `g0` (floor at z=0, the seed) whose doorway `p0` (WoW x=0, y ∈ [-0.5,0.5],
/// z ∈ [0,2]) opens onto exterior `g1`, two portal-disconnected exterior shells, `g2` ahead
/// through the opening and `g3` 30 yd aside, and `g4`, reached only through `g2`.
pub(super) fn doorway_model() -> WmoModel {
    let nav =
        |flags: u32, min: [f32; 3], max: [f32; 3], ref_start: u16, ref_count: u16| WmoGroupNav {
            flags,
            bbox_min: min,
            bbox_max: max,
            ref_start,
            ref_count,
            area_table_id: 0,
            fog_indices: [0; 4],
            group_liquid: wrathplus_formats::NO_GROUP_LIQUID,
        };
    let floor = |z: f32| {
        vec![
            [[-10.0, -10.0, z], [0.0, -10.0, z], [0.0, 10.0, z]],
            [[-10.0, -10.0, z], [0.0, 10.0, z], [-10.0, 10.0, z]],
        ]
    };
    WmoModel {
        portal_vertices: vec![
            [0.0, -0.5, 0.0],
            [0.0, 0.5, 0.0],
            [0.0, 0.5, 2.0],
            [0.0, -0.5, 2.0],
            // p1, the back door of g2 and g4's only way in.
            [6.0, -0.5, 0.0],
            [6.0, 0.5, 0.0],
            [6.0, 0.5, 2.0],
            [6.0, -0.5, 2.0],
        ],
        portal_infos: vec![
            WmoPortalInfo {
                start_vertex: 0,
                count: 4,
                plane: [1.0, 0.0, 0.0, 0.0],
            },
            WmoPortalInfo {
                start_vertex: 4,
                count: 4,
                plane: [1.0, 0.0, 0.0, -6.0],
            },
        ],
        // g0 ↔ g1, `side = -1` putting the eye at x < 0 on the entering half-space; then
        // g2 ↔ g4 through p1, reachable only by Pass 2 admitting g2 and walking on.
        portal_refs: vec![
            WmoPortalRef {
                portal: 0,
                group: 1,
                side: -1,
            },
            WmoPortalRef {
                portal: 0,
                group: 0,
                side: 1,
            },
            WmoPortalRef {
                portal: 1,
                group: 4,
                side: -1,
            },
            WmoPortalRef {
                portal: 1,
                group: 2,
                side: 1,
            },
        ],
        group_nav: vec![
            nav(0x2000, [-10.0, -10.0, 0.0], [0.0, 10.0, 3.0], 0, 1),
            nav(EXTERIOR, [1.0, -0.5, 0.0], [5.0, 0.5, 2.0], 1, 1),
            nav(EXTERIOR, [2.0, -0.5, 0.0], [6.0, 0.5, 2.0], 2, 1),
            nav(EXTERIOR, [2.0, 30.0, 0.0], [6.0, 34.0, 2.0], 0, 0),
            nav(0x2000, [7.0, -0.5, 0.0], [11.0, 0.5, 2.0], 3, 1),
        ],
        group_collision_tris: vec![floor(0.0), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        ..portal_model()
    }
}

/// A bare model with one vertical doorway portal in the WoW x=0 plane, y and z in [-1,1].
fn portal_model() -> WmoModel {
    WmoModel {
        wmo_id: 0,
        submeshes: Vec::new(),
        submesh_group: Vec::new(),
        portal_vertices: vec![
            [0.0, -1.0, -1.0],
            [0.0, 1.0, -1.0],
            [0.0, 1.0, 1.0],
            [0.0, -1.0, 1.0],
        ],
        portal_infos: vec![WmoPortalInfo {
            start_vertex: 0,
            count: 4,
            plane: [1.0, 0.0, 0.0, 0.0],
        }],
        portal_refs: Vec::new(),
        group_nav: Vec::new(),
        fogs: Vec::new(),
        skybox: None,
        group_collision_tris: Vec::new(),
        group_camera_only_tris: Vec::new(),
        group_collision_bounds: Vec::new(),
        group_collision_grids: Vec::new(),
        collision_bounds: None,
        collision: None,
        collision_camera: None,
        doodads: Vec::new(),
        doodad_sets: Vec::new(),
        lights: Vec::new(),
        group_bounds: Vec::new(),
        group_footprints: Vec::new(),
        material_ground_type: Vec::new(),
        material_diff_color: Vec::new(),
        group_footprint_bounds: Vec::new(),
        group_footprint_grids: Vec::new(),
        group_light_refs: Vec::new(),
        group_liquids: Vec::new(),
        wrath_liquid_states: None,
        doodad_base: Default::default(),
        doodad_owner: Default::default(),
        doodad_groups: Default::default(),
    }
}
