//! `nav`'s unit tests: targets, the no-navmesh answers and the straight-approach fallback.

use super::*;

#[test]
fn targets_come_from_coordinates_or_a_nearby_guid() {
    let lua = Lua::new();
    let snapshot = world::Snapshot {
        me: None,
        objects: vec![world::Seen {
            guid: 0x10,
            pos: [1.0, 2.0, 3.0],
            ..Default::default()
        }],
        corpse: None,
    };
    let args = |code: &str| lua.load(code).eval::<MultiValue>().unwrap();
    assert_eq!(
        target(&args("return 4, 5, 6"), &snapshot),
        Ok([4.0, 5.0, 6.0])
    );
    assert_eq!(
        target(&args("return '0x0000000000000010'"), &snapshot),
        Ok([1.0, 2.0, 3.0])
    );
    assert!(target(&args("return '0x99'"), &snapshot).is_err());
    assert!(target(&args("return true"), &snapshot).is_err());
}

#[test]
fn without_mmaps_a_route_says_how_to_get_one() {
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        crossbeam_channel::unbounded().0,
    );
    handles.mesh.lock().unwrap().dir = None;
    let me = world::Me {
        pos: [0.0; 3],
        facing: 0.0,
        map: 0,
    };
    assert!(route_to(&handles, me, [1.0, 1.0, 0.0])
        .unwrap_err()
        .contains("WOW_NAV_MMAPS"));
}

#[test]
fn a_far_interaction_without_a_navmesh_falls_back_to_the_straight_approach() {
    let (world_handles, world_link) = world::link();
    let (handles, _) = link(
        Arc::new(crate::events::EventLog::new()),
        std::time::Instant::now(),
        world_handles.orders.clone(),
    );
    handles.mesh.lock().unwrap().dir = None;
    {
        let mut s = world_handles.snapshot.lock().unwrap();
        s.me = Some(world::Me {
            pos: [0.0; 3],
            facing: 0.0,
            map: 0,
        });
        s.objects = vec![world::Seen {
            guid: 0x10,
            distance: 24.0,
            pos: [24.0, 0.0, 0.0],
            ..Default::default()
        }];
    }
    let lua = Lua::new();
    world::install(&lua, &world_handles).unwrap();
    install(&lua, &handles, &world_handles).unwrap();
    let sent: bool = lua
        .load("return AgentInteract('0x0000000000000010')")
        .eval()
        .unwrap();
    assert!(sent);
    assert!(matches!(
        world_link.orders.try_recv(),
        Ok(world::Order::Interact(0x10))
    ));
    assert_eq!(handles.walk.lock().unwrap().then_interact, None);
}
