use super::*;
use wrathplus_formats::ElevatorKeyframe;

/// The post-WORLDPORT_ACK create from the Stormwind–Auberdine crossing: the old
/// 234510 ms client period put this server anchor back on map 0 and hid the deck.
#[test]
fn auberdine_recreated_deck_stays_visible_at_the_server_progress() {
    use crate::app_core::session::ObjectStore;
    use wrathplus_protocol::{types::Field, ObjectFields};
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let nodes = load_taxi_path_nodes(&mut chain).expect("TaxiPathNode.dbc");
    let mut templates = GameObjectTemplates::default();
    let mut tuple = [0; 24];
    tuple[0] = 967;
    tuple[1] = 30;
    tuple[2] = 1;
    templates.insert(176310, 15, "The Bravery".into(), &tuple);
    let mut app = App::new();
    app.insert_resource(CurrentMap(1))
        .insert_resource(templates)
        .insert_resource(Timetables {
            nodes: Some(nodes),
            ..default()
        })
        .add_systems(Update, (arm_transports, tick_transports).chain());
    let boat = app
        .world_mut()
        .spawn((
            Guid(0x1FC0_0000_0000_0002),
            ObjectStore(ObjectFields::from_field_slots(&[
                (Field::ObjectEntry, 0, 176310),
                (Field::GameobjectLevel, 0, 234433),
            ])),
            TransportAnchor {
                progress_ms: 1_451_256_056,
                at: Instant::now(),
            },
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    app.update();
    let transport = app.world().get::<Transport>(boat).unwrap();
    assert_eq!(transport.period_ms(), 234433, "captured server period");
    assert_eq!(
        *app.world().get::<Visibility>(boat).unwrap(),
        Visibility::Inherited,
        "the recreated server anchor must not hide the destination deck"
    );
    let position = app.world().get::<Transform>(boat).unwrap().translation;
    let wow = wrathplus_assets::coords::bevy_to_wow(position);
    assert!(wow[0] > 6000.0 && wow[0] < 7000.0, "Auberdine leg: {wow:?}");
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(boat).unwrap(),
        Visibility::Inherited
    );
}

/// The Ratchet–Booty Bay ferry's real timetable (taxi path 241): the builder can truncate a
/// short synthetic cycle so the far continent's legs are never reached.
fn ferry_241(chain: &mut wrathplus_formats::Chain) -> Transport {
    let cat = wrathplus_formats::load_taxi_path_nodes(chain).expect("TaxiPathNode.dbc");
    let nodes = cat.path(241).expect("taxi path 241 (Ratchet–Booty Bay)");
    let tt = TransportTimetable::build(nodes, 30.0, 1.0).expect("path 241 builds");
    Transport {
        drive: Drive::Taxi(Arc::new(tt)),
        was_moving: false,
    }
}

/// Path 241's Kalimdor legs are the first frames of the cycle, so crossing to Kalimdor is the
/// cycle wrap, not an interior map change.
#[test]
fn the_seam_reanchor_lands_on_the_new_map_and_leaves_an_agreeing_clock_alone() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let transport = ferry_241(&mut chain);
    let period = transport.period_ms();

    // Where each continent's legs fall in the built timetable.
    let on = |map: u32| {
        (0..period)
            .step_by(500)
            .find(|&ms| transport.sample(ms, 0).map == map)
            .unwrap_or_else(|| panic!("path 241 has an instant on map {map}"))
    };
    let (on_azeroth, on_kalimdor) = (on(0), on(1));

    // On Azeroth's leg, told Kalimdor: the stored anchor moves onto Kalimdor.
    let mut anchor = TransportAnchor {
        progress_ms: on_azeroth,
        at: Instant::now(),
    };
    let (sample, moved_to) = transport
        .reseek_to_map(&mut anchor, 1)
        .expect("path 241 visits Kalimdor");
    assert_eq!(sample.map, 1, "the re-anchored sample must be on Kalimdor");
    assert!(
        moved_to.is_some(),
        "a clock on Azeroth's leg told it is on Kalimdor must report that it moved"
    );
    assert_eq!(
        transport.sample(anchor.progress_ms, 0).map,
        1,
        "the stored anchor must itself sit on Kalimdor, not just the returned sample"
    );

    // Already on Kalimdor: the clock is left alone.
    let mut anchor = TransportAnchor {
        progress_ms: on_kalimdor,
        at: Instant::now(),
    };
    let (sample, moved_to) = transport
        .reseek_to_map(&mut anchor, 1)
        .expect("path 241 visits Kalimdor");
    assert_eq!(sample.map, 1);
    assert_eq!(
        anchor.progress_ms, on_kalimdor,
        "an agreeing clock must not be touched"
    );
    assert_eq!(
        moved_to, None,
        "a no-op seek must report that it did not move the clock"
    );

    // A map the path never visits has no answer.
    let mut anchor = TransportAnchor {
        progress_ms: on_azeroth,
        at: Instant::now(),
    };
    assert!(transport.reseek_to_map(&mut anchor, 4242).is_none());
    assert_eq!(
        anchor.progress_ms, on_azeroth,
        "a failed seek must not move the clock"
    );
}

#[test]
fn a_lift_has_no_seam_to_reanchor() {
    let lift = Transport {
        drive: Drive::Lift(parked_lift()),
        was_moving: false,
    };
    let mut anchor = TransportAnchor {
        progress_ms: 0,
        at: Instant::now(),
    };
    assert!(lift.reseek_to_map(&mut anchor, 1).is_none());
}

/// A two-frame pause path at `base_pos`.
fn parked_lift() -> Lift {
    Lift {
        frames: Arc::new(vec![
            ElevatorKeyframe {
                time_ms: 0,
                pos: [0.0; 3],
            },
            ElevatorKeyframe {
                time_ms: 1000,
                pos: [0.0; 3],
            },
        ]),
        period_ms: 1000,
        base_pos: [10.0, 20.0, 30.0],
        quat: [0.0, 0.0, 0.0, 1.0],
        yaw: 0.5,
    }
}

/// A pause path samples the same pose forever, so after the arming write nothing changes.
#[test]
fn a_docked_lift_stops_dirtying_its_transform_and_visibility() {
    #[derive(Resource, Default)]
    struct Dirty {
        transforms: usize,
        visibilities: usize,
    }
    fn spy(
        t: Query<Entity, (Changed<Transform>, With<Transport>)>,
        v: Query<Entity, (Changed<Visibility>, With<Transport>)>,
        mut out: ResMut<Dirty>,
    ) {
        out.transforms = t.iter().count();
        out.visibilities = v.iter().count();
    }
    let mut app = App::new();
    app.insert_resource(CurrentMap(0))
        .init_resource::<Dirty>()
        .add_systems(Update, (tick_transports, spy).chain());
    app.world_mut().spawn((
        Guid(0xF110_0000_0000_0001),
        Transport {
            drive: Drive::Lift(parked_lift()),
            was_moving: false,
        },
        TransportAnchor {
            progress_ms: 0,
            at: Instant::now(),
        },
        Transform::default(),
        Visibility::default(),
    ));
    app.update(); // arming frame: the first pose write lands (and spawn reads as Changed)
    for _ in 0..3 {
        app.update();
    }
    let dirty = app.world().resource::<Dirty>();
    assert_eq!(
        (dirty.transforms, dirty.visibilities),
        (0, 0),
        "a docked lift must stop dirtying its transform and visibility"
    );
}

/// At login lifts arm before `CurrentMap` is written, so the off-map hide must never judge a
/// lift against a map.
#[test]
fn a_lift_is_visible_on_whatever_map_the_viewer_is_on() {
    let mut app = App::new();
    // Alterac Valley, a map with no type-11 GameObject: the value must not matter.
    app.insert_resource(CurrentMap(30))
        .add_systems(Update, tick_transports);
    let lift = app
        .world_mut()
        .spawn((
            Guid(0xF110_0000_0000_0002),
            Transport {
                drive: Drive::Lift(parked_lift()),
                was_moving: false,
            },
            TransportAnchor {
                progress_ms: 0,
                at: Instant::now(),
            },
            Transform::default(),
            // Anchored transports spawn hidden until their first ticked pose.
            Visibility::Hidden,
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(lift),
        Some(&Visibility::Inherited),
        "the tick must unhide a lift on the viewer's map, not judge it against a stamped one"
    );
    // The pose is the base plus its zero offset, in Bevy axes.
    assert_eq!(
        app.world().get::<Transform>(lift).map(|t| t.translation),
        Some(wrathplus_assets::coords::wow_to_bevy([10.0, 20.0, 30.0])),
    );
}

/// AC transports.sql spawn 3 is The Moonspray, not template 3. Exercise the real
/// arming/tick systems with the installed taxi DBC and the descriptor identity.
#[test]
fn auberdine_boat_arms_from_descriptor_entry_and_unhides_on_map_one() {
    use crate::app_core::session::ObjectStore;
    use wrathplus_protocol::{types::Field, ObjectFields};
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let nodes = load_taxi_path_nodes(&mut chain).expect("TaxiPathNode.dbc");
    let path = nodes.path(293).expect("Auberdine–Rut'theran path 293");
    assert!(path.iter().all(|node| node.map_id == 1));
    let mut templates = GameObjectTemplates::default();
    let mut tuple = [0; 24];
    tuple[0] = 293;
    tuple[1] = 30;
    tuple[2] = 1;
    templates.insert(176244, 15, "The Moonspray".into(), &tuple);
    let mut app = App::new();
    app.insert_resource(CurrentMap(1))
        .insert_resource(templates)
        .insert_resource(Timetables {
            nodes: Some(nodes),
            ..default()
        })
        .add_systems(Update, (arm_transports, tick_transports).chain());
    let boat = app
        .world_mut()
        .spawn((
            Guid(0x1FC0_0000_0000_0003),
            ObjectStore(ObjectFields::from_field_slots(&[(
                Field::ObjectEntry,
                0,
                176244,
            )])),
            TransportAnchor {
                progress_ms: 0,
                at: Instant::now(),
            },
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    app.update();
    assert!(
        app.world().get::<Transport>(boat).is_some(),
        "descriptor template must arm the boat"
    );
    assert_eq!(
        *app.world().get::<Visibility>(boat).unwrap(),
        Visibility::Inherited
    );
    let transport = app.world().get::<Transport>(boat).unwrap();
    assert!(transport.touches_map(1));
    assert!(!transport.touches_map(0));
    let pos = app.world().get::<Transform>(boat).unwrap().translation;
    assert!(
        pos.is_finite() && pos.length() > 1000.0,
        "real DBC pose, not origin"
    );
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(boat).unwrap(),
        Visibility::Inherited
    );
}

/// AC transports.sql/gameobject_template.sql identities, independently of spawn counters.
/// Installed DBCs exercise the production arm and tick on every continent each route visits.
#[test]
fn horde_airships_arm_by_descriptor_and_tick_each_map() {
    use crate::app_core::session::ObjectStore;
    use wrathplus_protocol::{types::Field, ObjectFields};
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let catalog = wrathplus_formats::load_gameobject_catalog(&mut chain).expect("GO displays");
    type Route<'a> = (u64, u32, u32, u32, i32, &'a [u32]);
    let routes: &[Route<'_>] = &[
        (1, 176495, 301, 3031, 30, &[0]),
        (5, 175080, 285, 3031, 30, &[0, 1]),
        (6, 164871, 302, 3031, 30, &[0, 1]),
        (11, 181689, 737, 7546, 30, &[0, 571]),
        (12, 186238, 712, 7546, 30, &[1, 571]),
        (20, 190549, 1221, 3031, 30, &[1]),
        (18, 192241, 1095, 8253, 2, &[571]),
    ];
    for &(counter, entry, path, display, speed, maps) in routes {
        let model = catalog.model_path(display).expect("airship display model");
        assert!(model.to_ascii_lowercase().ends_with(".wmo"), "{model}");
        let nodes = load_taxi_path_nodes(&mut chain).expect("TaxiPathNode.dbc");
        let raw = nodes
            .path(path)
            .unwrap_or_else(|| panic!("entry {entry} path {path}"));
        let mut actual_maps: Vec<_> = raw.iter().map(|n| n.map_id).collect();
        actual_maps.sort_unstable();
        actual_maps.dedup();
        assert_eq!(actual_maps, maps, "entry {entry} installed node maps");
        let mut templates = GameObjectTemplates::default();
        let mut tuple = [0; 24];
        tuple[0] = path as i32;
        tuple[1] = speed;
        tuple[2] = 1;
        templates.insert(entry, 15, format!("airship {entry}"), &tuple);
        let mut app = App::new();
        app.insert_resource(CurrentMap(maps[0]))
            .insert_resource(templates)
            .insert_resource(Timetables {
                nodes: Some(nodes),
                ..default()
            })
            .add_systems(Update, (arm_transports, tick_transports).chain());
        let ship = app
            .world_mut()
            .spawn((
                Guid(0x1FC0_0000_0000_0000 | counter),
                ObjectStore(ObjectFields::from_field_slots(&[(
                    Field::ObjectEntry,
                    0,
                    entry,
                )])),
                TransportAnchor {
                    progress_ms: 0,
                    at: Instant::now(),
                },
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        app.update();
        assert!(
            app.world().get::<Transport>(ship).is_some(),
            "entry {entry} arms"
        );
        for &map in maps {
            let transport = app.world().get::<Transport>(ship).unwrap();
            assert!(transport.touches_map(map), "entry {entry} visits {map}");
            let cycle = (0..transport.period_ms())
                .step_by(500)
                .find(|&ms| transport.sample(ms, map).map == map)
                .expect("built timetable reaches every installed map");
            // A real seam seek must change a disagreeing clock, but not an agreeing one.
            let mut anchor = TransportAnchor {
                progress_ms: cycle,
                at: Instant::now(),
            };
            let (sample, moved) = transport.reseek_to_map(&mut anchor, map).unwrap();
            assert_eq!(sample.map, map);
            assert_eq!(moved, None);
            assert_eq!(anchor.progress_ms, cycle);
            if maps.len() > 1 {
                let other = *maps.iter().find(|&&m| m != map).unwrap();
                let (sample, moved) = transport.reseek_to_map(&mut anchor, other).unwrap();
                assert_eq!(sample.map, other);
                assert!(moved.is_some());
            }
            let before = anchor.progress_ms;
            assert!(transport.reseek_to_map(&mut anchor, 4242).is_none());
            assert_eq!(anchor.progress_ms, before);
            app.world_mut().insert_resource(CurrentMap(map));
            *app.world_mut().get_mut::<TransportAnchor>(ship).unwrap() = TransportAnchor {
                progress_ms: cycle,
                at: Instant::now(),
            };
            app.update();
            assert_eq!(
                app.world().get::<Visibility>(ship),
                Some(&Visibility::Inherited)
            );
            let pose = *app.world().get::<Transform>(ship).unwrap();
            assert!(pose.translation.is_finite() && pose.translation.length() > 100.0);
            assert!(pose.rotation.is_finite());
            // Off-map ticking hides without writing an unrelated continent's pose.
            app.world_mut().insert_resource(CurrentMap(4242));
            app.update();
            assert_eq!(
                app.world().get::<Visibility>(ship),
                Some(&Visibility::Hidden)
            );
            assert_eq!(*app.world().get::<Transform>(ship).unwrap(), pose);
            app.world_mut().insert_resource(CurrentMap(map));
            app.update();
            assert_eq!(
                app.world().get::<Visibility>(ship),
                Some(&Visibility::Inherited)
            );
        }
    }
}
