use super::*;
use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};
#[test]
fn adapter_fifo_is_lossless_repeated_ordered_and_disconnect_scoped() {
    let (mut app, _rx) = test_app();
    let mut adapter = WrathEventAdapter::new();
    let mut body = 99u64.to_le_bytes().to_vec();
    body.extend([1, 255]);
    for word in [7u32, 8, 9] {
        body.extend(word.to_le_bytes());
    }
    body.extend([255, 0, 254]);
    let list = wrathplus_protocol::wrath_stable::decode(0x26f, &body).unwrap();
    let mut expected = Vec::new();
    for (opcode, bytes) in [
        (0x26f, body.clone()),
        (0x273, vec![255]),
        (0x26f, body.clone()),
        (0x273, vec![0]),
    ] {
        let WrathDispatch::Handled { session_events, .. } =
            adapter.dispatch(opcode, &bytes, 0).unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            &session_events[..],
            [SessionEvent::WrathStable { .. }]
        ));
        crate::net::handlers::dispatch(app.world_mut(), session_events);
    }
    expected.extend([
        list.clone(),
        Packet::Result { result: 255 },
        list,
        Packet::Result { result: 0 },
    ]);
    assert_eq!(
        app.world()
            .resource::<NativeStablePackets>()
            .0
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        expected
    );
    for end in 0..body.len() {
        assert!(matches!(
            adapter.dispatch(0x26f, &body[..end], 0).unwrap(),
            WrathDispatch::Unhandled { .. }
        ));
    }
    body.push(0);
    assert!(matches!(
        adapter.dispatch(0x26f, &body, 0).unwrap(),
        WrathDispatch::Unhandled { .. }
    ));
    for bad in [vec![], vec![0, 0]] {
        assert!(matches!(
            adapter.dispatch(0x273, &bad, 0).unwrap(),
            WrathDispatch::Unhandled { .. }
        ));
    }
    crate::net::handlers::dispatch(
        app.world_mut(),
        vec![SessionEvent::Disconnected {
            reason: "test".into(),
            end: wrathplus_protocol::SessionEnd::Lost,
        }],
    );
    assert!(app.world().resource::<NativeStablePackets>().0.is_empty());
}

fn test_app() -> (App, crossbeam_channel::Receiver<crate::net::ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx))
        .init_resource::<StableOpen>()
        .init_resource::<StableErrors>()
        .init_resource::<NameCache>()
        .init_resource::<crate::ui_gossip::GossipState>()
        .init_resource::<crate::app_core::state::QuestGiver>()
        .init_resource::<crate::app_core::state::PetBar>()
        .init_resource::<crate::net::GuidIndex>()
        .init_resource::<crate::app_core::session::SelfGuid>()
        .init_resource::<crate::net::wrath_notices::WrathProgress>()
        .add_plugins(crate::ui_pet_talent_tree::UiPetTalentTreePlugin)
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::sound::MessageSounds>()
        .insert_resource(boar_tables())
        .insert_resource(crate::ui_talent::Talents {
            catalog: wrathplus_formats::TalentCatalog::from_rows(
                vec![wrathplus_formats::TalentTabInfo {
                    id: 410,
                    name: "Ferocity".into(),
                    race_mask: 0,
                    class_mask: 0,
                    pet_talent_mask: 1,
                    background: String::new(),
                }],
                Vec::new(),
            ),
        })
        .insert_non_send_resource(wrathplus_ui::script::UiScript::new().unwrap())
        .add_systems(
            Update,
            (super::super::feed_stable, super::super::drain_stable).chain(),
        );
    register(&mut app);
    // Every row's entry (`list_body`) is a boar whose creature query has answered.
    app.world_mut().resource_mut::<NameCache>().insert_creature(
        1190,
        Some(crate::names::CreatureRecord {
            name: "Boar".into(),
            subname: None,
            creature_type: 1,
            pet_family: 5,
            rank: 0,
            type_flags: 0,
            civilian: false,
            racial_leader: false,
            display_id: 0,
            modifiers: None,
        }),
    );
    (app, rx)
}

/// `CreatureFamily.dbc` row 5, Boar, on pet talent type 0 (Ferocity's mask bit).
fn boar_tables() -> crate::app_core::catalogs::PetFamilyTables {
    let boar = wrathplus_formats::CreatureFamily {
        name: "Boar".into(),
        icon: "Interface\\Icons\\Ability_Hunter_Pet_Boar".into(),
        pet_food_mask: 0,
        min_scale: 1.0,
        min_scale_level: 1,
        max_scale: 1.0,
        max_scale_level: 2,
        pet_talent_type: 0,
    };
    crate::app_core::catalogs::PetFamilyTables {
        families: [(5, boar)].into_iter().collect(),
        foods: Default::default(),
    }
}

const ULBREK: u64 = 17379391129526469420;

fn list_body(purchased: u8, pets: &[(u32, u8, &str)]) -> Vec<u8> {
    let mut body = ULBREK.to_le_bytes().to_vec();
    body.extend([pets.len() as u8, purchased]);
    for &(number, flags, name) in pets {
        for word in [number, 1190, 13] {
            body.extend(word.to_le_bytes());
        }
        body.extend(name.as_bytes());
        body.extend([0, flags]);
    }
    body
}

fn dispatch(app: &mut App, opcode: u16, body: &[u8]) {
    let WrathDispatch::Handled { session_events, .. } =
        WrathEventAdapter::new().dispatch(opcode, body, 0).unwrap()
    else {
        panic!("valid native stable packet refused")
    };
    crate::net::handlers::dispatch(app.world_mut(), session_events);
    app.update();
}

fn lua(app: &App, code: &str) {
    app.world()
        .non_send_resource::<wrathplus_ui::script::UiScript>()
        .run(code)
        .unwrap();
}

#[test]
fn ulbrek_packet_opens_current_porkchop_closes_gossip_and_drains_real_stable_verb() {
    let (mut app, rx) = test_app();
    app.world_mut()
        .resource_mut::<crate::ui_gossip::GossipState>()
        .npc = Some(ULBREK);
    lua(&app, "shows=0; closed=0; local f=CreateFrame('Frame'); f:RegisterEvent('PET_STABLE_SHOW'); f:RegisterEvent('PET_STABLE_CLOSED'); f:SetScript('OnEvent', function(_,e) if e=='PET_STABLE_SHOW' then assert(IsAtStableMaster()); shows=shows+1 else closed=closed+1 end end)");
    let body = list_body(0, &[(8492, 1, "Porkchop")]);
    dispatch(&mut app, 0x26f, &body);
    assert_eq!(
        app.world()
            .resource::<NameCache>()
            .peek(0xF140_0000_0000_0001 | (8492u64 << 24)),
        Some("Porkchop")
    );
    let open = app.world().resource::<StableOpen>();
    assert_eq!(
        (open.npc, open.num_stable_slots, open.pets[0].slot),
        (Some(ULBREK), 0, 0)
    );
    assert!(app
        .world()
        .resource::<crate::ui_gossip::GossipState>()
        .npc
        .is_none());
    lua(&app, "local _,name,level,family,talent=GetStablePetInfo(0); assert(name=='Porkchop' and level==13 and family=='Boar' and talent=='Ferocity'); assert(IsAtStableMaster()); assert(GetNumStableSlots()==0); assert(shows==1)");
    // A repeated list owes SHOW even when every row is identical.
    dispatch(&mut app, 0x26f, &body);
    lua(&app, "assert(shows==2); StablePet()");
    app.update();
    assert!(rx
        .try_iter()
        .any(|c| matches!(c, crate::net::ClientCommand::StablePet { npc: ULBREK })));
    crate::net::handlers::dispatch(
        app.world_mut(),
        vec![SessionEvent::Disconnected {
            reason: "test".into(),
            end: wrathplus_protocol::SessionEnd::Lost,
        }],
    );
    app.update();
    lua(
        &app,
        "assert(not IsAtStableMaster()); assert(GetStablePetInfo(0)==nil); assert(closed==1)",
    );
    assert!(app.world().resource::<NativeStablePackets>().0.is_empty());
}

#[test]
fn inactive_flags_are_dense_display_slots_and_success_refreshes_without_local_pet_moves() {
    let (mut app, rx) = test_app();
    // No slot-0 row; flags 2 means inactive, not physical slot 2.
    dispatch(
        &mut app,
        0x26f,
        &list_body(2, &[(8492, 2, "Porkchop"), (9000, 2, "Boar")]),
    );
    lua(&app, "assert(GetStablePetInfo(0)==nil); local _,a=GetStablePetInfo(1); local _,b=GetStablePetInfo(2); assert(a=='Porkchop' and b=='Boar'); ClickStablePet(1); UnstablePet(1)");
    app.update();
    assert!(rx.try_iter().any(|c| matches!(
        c,
        crate::net::ClientCommand::UnstablePet {
            npc: ULBREK,
            pet_number: 8492
        }
    )));
    for code in [8, 9, 10] {
        dispatch(&mut app, 0x273, &[code]);
        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(
                    c,
                    crate::net::ClientCommand::ListStabledPets { npc: ULBREK }
                ))
                .count(),
            1
        );
        assert_eq!(app.world().resource::<StableOpen>().pets[0].slot, 1);
    }
    for code in [0, 6, 255] {
        dispatch(&mut app, 0x273, &[code]);
        assert!(rx.try_iter().next().is_none());
    }
    // Server refresh is authoritative: pet movement and purchased count come from this list.
    dispatch(&mut app, 0x26f, &list_body(1, &[(8492, 1, "Porkchop")]));
    lua(&app, "assert(GetNumStableSlots()==1); local _,n=GetStablePetInfo(0); assert(n=='Porkchop'); assert(GetStablePetInfo(1)==nil)");
}

#[test]
fn unsupported_rows_remain_lossless_without_replacing_the_visible_session() {
    let (mut app, _) = test_app();
    dispatch(&mut app, 0x26f, &list_body(0, &[(8492, 1, "Porkchop")]));
    for pets in [
        vec![(1, 3, "bad")],
        vec![(1, 1, "a"), (2, 1, "b")],
        vec![(1, 2, "unbought")],
    ] {
        dispatch(&mut app, 0x26f, &list_body(0, &pets));
        assert_eq!(
            app.world().resource::<StableOpen>().pets[0].pet_number,
            8492
        );
    }
    assert_eq!(app.world().resource::<NativeStablePackets>().0.len(), 4);
}

#[test]
fn refusals_surface_without_refresh_and_closed_success_never_queries() {
    let (mut app, rx) = test_app();
    // Inspect keys before the feed resolves them against localized GlobalStrings.
    for (code, key) in [
        (1, "ERR_NOT_ENOUGH_MONEY"),
        (12, "PETTAME_CANTCONTROLEXOTIC"),
    ] {
        let WrathDispatch::Handled { session_events, .. } = WrathEventAdapter::new()
            .dispatch(0x273, &[code], 0)
            .unwrap()
        else {
            panic!()
        };
        crate::net::handlers::dispatch(app.world_mut(), session_events);
        assert_eq!(app.world().resource::<StableErrors>().0, vec![key]);
        app.world_mut().resource_mut::<StableErrors>().0.clear();
        assert!(rx.try_iter().next().is_none());
    }
    for code in [8, 9, 10] {
        dispatch(&mut app, 0x273, &[code]);
        assert!(rx.try_iter().next().is_none());
        assert_eq!(app.world().resource::<StableOpen>().num_stable_slots, 0);
    }
}
