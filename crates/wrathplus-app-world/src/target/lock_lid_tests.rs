//! Lock 93's production click route after another player's opening cast.
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use wrathplus_formats::{LockCatalog, LockSlot, OpenLock, SpellCatalog, SpellDisplay};
use wrathplus_protocol::types::{Field, ObjectFields};

use crate::app_core::session::{ObjectStore, SelfGuid};
use crate::go_anim::{go_state, open_go_lid, GoAnim, GoLidOpen};
use crate::net::{GuidIndex, NetCommands};
use crate::target::click::{resolve_go_action, GoAction};
use crate::target::lock::GoLockInputs;

#[test]
fn underwater_lock93_remote_open_does_not_poison_the_local_click() {
    const GO: u64 = 17370386701549047277;
    const ME: u64 = 1057;
    let mut app = App::new();
    app.add_message::<GoLidOpen>()
        .init_resource::<GuidIndex>()
        .init_resource::<SelfGuid>()
        .init_resource::<crate::app_core::loot::LootState>()
        .init_resource::<crate::items::Items>()
        .init_resource::<crate::go_templates::GameObjectTemplates>()
        .add_systems(Update, (open_go_lid, crate::go_anim::close_go_lid).chain());
    let (tx, _rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx));
    // Installed Lock.dbc 93: slot zero empty, slot one Open Tinkering (12), Action 0.
    let mut slots = [LockSlot::default(); 8];
    slots[1] = LockSlot {
        key_type: 2,
        index: 12,
        skill: 0,
        action: 0,
    };
    app.insert_resource(crate::go_templates::Locks(LockCatalog::from_rows([(
        93, slots,
    )])));
    let opener = SpellDisplay {
        id: 6477,
        open_lock: Some(OpenLock {
            effect: 0,
            lock_type: 12,
        }),
        effect_base_points: [99, 0, 0],
        effect_base_dice: [1, 0, 0],
        ..Default::default()
    };
    app.insert_resource(crate::app_core::state::Spells {
        catalog: SpellCatalog::from_displays([(6477, opener)].into_iter().collect()),
        ..crate::app_core::state::Spells::empty_for_tests()
    });
    let mut data = [0; 24];
    data[0] = 93;
    app.world_mut()
        .resource_mut::<crate::go_templates::GameObjectTemplates>()
        .insert(175165, 3, "Silver Dawning's Lockbox".into(), &data);
    app.world_mut().resource_mut::<SelfGuid>().0 = Some(ME);
    // The reported create's nonzero interaction fields; no subsequent state update.
    let entity = app
        .world_mut()
        .spawn((
            ObjectStore(ObjectFields::from_pairs(&[
                (Field::GameobjectFlags.index(), 4),
                (Field::GameobjectDynamic.index(), 0xffff0009),
                (Field::GameobjectBytes1.index(), 1677722369),
            ])),
            GoAnim::default(),
        ))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(GO, entity);
    let click = |app: &mut App| {
        app.world_mut()
            .run_system_once(
                move |mut inputs: GoLockInputs,
                      net: Res<NetCommands>,
                      gos: Query<(&ObjectStore, &GoAnim)>| {
                    let (store, anim) = gos.get(entity).unwrap();
                    resolve_go_action(
                        GO,
                        &mut inputs,
                        &std::collections::BTreeSet::from([6477]),
                        Some((store, go_state(Some(anim), store))),
                        None,
                        &net,
                    )
                },
            )
            .unwrap()
    };
    assert!(matches!(click(&mut app), GoAction::OpenLock(6477)));
    // Four remote casters' actual spell-goes in 071253 at 21:18:52..54, all before the reported click.
    for caster in [1062, 1061, 1060, 1059] {
        app.world_mut().write_message(GoLidOpen {
            go_guid: GO,
            spell_id: 6477,
            caster,
        });
        app.update();
        assert!(
            matches!(click(&mut app), GoAction::OpenLock(6477)),
            "remote opener must not strand the local actionable state at ACTIVE"
        );
    }
    // The native local-caster path still opens the lid and refuses another Action-0 cast.
    app.world_mut().write_message(GoLidOpen {
        go_guid: GO,
        spell_id: 6477,
        caster: ME,
    });
    app.update();
    assert!(matches!(click(&mut app), GoAction::Refuse(_)));
    assert_eq!(app.world().get::<GoAnim>(entity).unwrap().state, Some(0));
    assert_eq!(
        app.world()
            .get::<ObjectStore>(entity)
            .unwrap()
            .0
            .gameobject_state(),
        Some(1)
    );
    app.world_mut()
        .resource_mut::<crate::app_core::loot::LootState>()
        .open(GO, 1, 1, vec![]);
    app.update();
    app.world_mut()
        .resource_mut::<crate::app_core::loot::LootState>()
        .clear();
    app.update();
    assert!(
        matches!(click(&mut app), GoAction::OpenLock(6477)),
        "local loot close restores READY"
    );
}

#[test]
fn underwater_lock93_installed_row_and_opener() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("installed chain");
    let locks = wrathplus_formats::load_lock_catalog(&mut chain).expect("Lock.dbc");
    let slots = locks.slots(93).expect("lock93");
    let mut expected = [LockSlot::default(); 8];
    expected[1] = LockSlot {
        key_type: 2,
        index: 12,
        skill: 0,
        action: 0,
    };
    assert_eq!(slots, &expected);
    let spells = wrathplus_formats::load_spell_catalog(&mut chain).expect("Spell.dbc");
    let opener = spells.get(6477).expect("Opening");
    assert_eq!(opener.open_lock_type(), Some(12));
    assert_eq!(opener.open_lock_skill(0), Some(100));
    assert_ne!(spells.get(3365).unwrap().open_lock_type(), Some(12));
}

#[test]
fn state_edges_use_real_layout_fields_and_normalize_wrath_bytes() {
    use wrathplus_protocol::types::{Field, ObjectType};
    let native_field = Field::GameobjectBytes1;
    let span = native_field.span().expect("native state dependency exists");
    let (field, offset) =
        wrathplus_protocol::types::Field::at(ObjectType::GameObject, span.index).unwrap();
    assert_eq!((field, offset), (native_field, 0));
    let packed = 0x87654300;
    let mut edge = crate::net::FieldChanged {
        entity: bevy::prelude::Entity::PLACEHOLDER,
        guid: 42,
        kind: ObjectType::GameObject,
        field,
        offset,
        old: packed | 1,
        new: packed,
    };
    assert_eq!(super::go_state_edge(&edge), Some(0));
    edge.old = packed;
    edge.new = packed | 1;
    assert_eq!(super::go_state_edge(&edge), Some(1));
    for mask in [0x100, 0x10000, 0x1000000] {
        edge.old = packed | 1;
        edge.new = edge.old ^ mask;
        assert_eq!(
            super::go_state_edge(&edge),
            None,
            "upper bytes aren't state"
        );
    }
    edge.kind = ObjectType::Unit;
    assert_eq!(super::go_state_edge(&edge), None);
    edge.kind = ObjectType::GameObject;
    edge.offset = 1;
    assert_eq!(super::go_state_edge(&edge), None);
    let span = Field::GameobjectFlags.span().unwrap();
    (edge.field, edge.offset) =
        wrathplus_protocol::types::Field::at(ObjectType::GameObject, span.index).unwrap();
    assert_eq!(
        super::go_state_edge(&edge),
        None,
        "unrelated fields don't re-close chests"
    );
}
