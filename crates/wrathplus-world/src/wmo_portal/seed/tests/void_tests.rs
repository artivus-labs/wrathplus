use super::*;

#[test]
fn camera_void_fallback_names_the_detail_floored_room() {
    // The Deadmines pocket: an interior group floored only with DETAIL faces, which are in the
    // camera-only set and not the walking set.
    let pocket = nav(0, [-10.0, -10.0, 0.0], [10.0, 10.0, 12.0], 0, 0);
    let walk: [Vec<[[f32; 3]; 3]>; 1] = [Vec::new()];
    let detail = [quad(0.0, 10.0)];
    // Legs A and B miss, no terrain below: Leg C names the room, a single root.
    let s = down_ray_pick(
        &walk,
        &[],
        &detail,
        &[pocket],
        &[],
        &[],
        &[],
        [0.0, 0.0, 2.0],
        None,
    );
    assert_eq!(s.in_group, Some(0));
    assert_eq!(s.across, None);
    // Terrain above the eye is off the segment: Leg C still fires.
    assert_eq!(
        down_ray_pick(
            &walk,
            &[],
            &detail,
            &[pocket],
            &[],
            &[],
            &[],
            [0.0, 0.0, 2.0],
            Some(8.0)
        )
        .in_group,
        Some(0)
    );
}

#[test]
fn camera_void_fallback_defers_to_terrain_and_to_the_walking_leg() {
    let pocket = nav(0, [-10.0, -10.0, 0.0], [10.0, 10.0, 12.0], 0, 0);
    let detail = [quad(0.0, 10.0)];
    // Any terrain at or below the eye keeps the client's outside verdict.
    assert_eq!(
        down_ray_pick(
            &[Vec::new()],
            &[],
            &detail,
            &[pocket],
            &[],
            &[],
            &[],
            [0.0, 0.0, 2.0],
            Some(1.9)
        )
        .in_group,
        None
    );
    // A walking-leg answer, even a lower floor of another group, pre-empts Leg C.
    let hall = nav(0, [-30.0, -30.0, -5.0], [30.0, 30.0, 12.0], 0, 0);
    let walk = [Vec::new(), quad(-5.0, 30.0)];
    let det2: [Vec<[[f32; 3]; 3]>; 2] = [quad(0.0, 10.0), Vec::new()];
    assert_eq!(
        down_ray_pick(
            &walk,
            &[],
            &det2,
            &[pocket, hall],
            &[],
            &[],
            &[],
            [0.0, 0.0, 2.0],
            None
        )
        .in_group,
        Some(1)
    );
    // An EXTERIOR group's detail surface reads as outside, as in Leg A.
    let ext = nav(EXTERIOR, [-10.0, -10.0, 0.0], [10.0, 10.0, 12.0], 0, 0);
    assert_eq!(
        down_ray_pick(
            &[Vec::new()],
            &[],
            &detail,
            &[ext],
            &[],
            &[],
            &[],
            [0.0, 0.0, 2.0],
            None
        )
        .in_group,
        None
    );
}
