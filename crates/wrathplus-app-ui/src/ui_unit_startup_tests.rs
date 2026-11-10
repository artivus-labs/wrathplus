//! First-pass callbacks must see the resolver inputs seated by the production aura feed.

use super::*;
use crate::app_core::group::GroupState;
use crate::app_core::session::SelfGuid;
use crate::app_core::state::PetBar;
use crate::net::GuidIndex;
use crate::portrait::PetDollBooth;
use wrathplus_protocol::types::{member_status, GroupMemberEntry, ObjectFields};

const ME: u64 = 1;
const MEMBER: u64 = 2;
const PET: u64 = 0xF140_0000_0000_0003;

/// Registers the actual plugins, including the unit feed's chain and the independent aura feed
/// inside UnitFeed. No test setter seats UnitGuids, and no replacement feed supplies them.
fn startup_app() -> App {
    let mut app = App::new();
    app.init_resource::<Selection>()
        .init_resource::<NameCache>()
        .init_resource::<Reputations>()
        .init_resource::<GroupState>()
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::guild::GuildState>()
        .init_resource::<PetDollBooth>()
        .insert_resource(SelfGuid(Some(ME)))
        .add_message::<crate::creature_anim::SwingImpact>();
    let (tx, _rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx));
    crate::test_guids::with_aura_feed(&mut app, || {});
    app.add_plugins((UiUnitPlugin, crate::ui_pet_doll::UiPetDollPlugin));
    let player = app
        .world_mut()
        .spawn((
            SelfPlayer,
            Guid(ME),
            ObjectStore(ObjectFields::default().into_created(ObjectType::Player)),
        ))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(ME, player);
    app
}

fn new_vm(app: &mut App) {
    let script = UiScript::new().unwrap();
    assert!(
        script.held_unit_guids().is_empty(),
        "the regression starts with an empty resolver, including on VM replacement"
    );
    app.insert_non_send_resource(script);
}

/// Check explicit edges, not an executor's accidental serialization of non-send systems.
/// Building this graph also rejects a cycle through the unit-feed chain or its shared set.
#[test]
fn startup_stat_and_health_feeds_are_declared_after_aura_events_without_a_cycle() {
    use super::aura_readers::system_type;
    use bevy::ecs::schedule::NodeId;

    let mut app = startup_app();
    app.world_mut().schedule_scope(Update, |world, schedule| {
        schedule.initialize(world).expect("Update has no cycle");
        let graph = schedule.graph();
        let aura = graph
            .system_sets
            .iter()
            .find(|(_, set, _)| format!("{set:?}") == "AuraEvents")
            .map(|(key, _, _)| NodeId::Set(key))
            .expect("the production aura feed's set");
        let systems: Vec<_> = schedule.systems().unwrap().collect();
        // By the type each system runs under: a build without `bevy/debug` names none.
        for (feed, ty) in [
            (
                "feed_pet_doll",
                system_type(crate::ui_pet_doll::feed_pet_doll),
            ),
            ("feed_units", system_type(feed_units)),
        ] {
            let matching: Vec<_> = systems
                .iter()
                .filter(|(_, s)| bevy::ecs::system::System::type_id(&***s) == ty)
                .collect();
            assert_eq!(matching.len(), 1, "one production {feed}");
            let system = NodeId::System(matching[0].0);
            assert!(
                graph
                    .dependency()
                    .graph()
                    .all_edges()
                    .any(|edge| edge == (aura, system)),
                "{feed} must explicitly run after AuraEvents"
            );
        }
    });
}

#[test]
fn first_pet_stat_callback_resolves_pet_on_login_and_a_replacement_vm() {
    let mut app = startup_app();
    app.world_mut().resource_mut::<PetBar>().spells.pet_guid = PET;
    let pet = app
        .world_mut()
        .spawn((
            Guid(PET),
            ObjectStore(
                ObjectFields::from_pairs(&[
                    (Field::UnitStat0.index() + 2, 68),
                    (Field::UnitResistances.index(), 1810),
                    (Field::UnitMindamage.index(), 30.5f32.to_bits()),
                    (Field::UnitMaxdamage.index(), 44.5f32.to_bits()),
                ])
                .into_created(ObjectType::Unit),
            ),
        ))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(PET, pet);

    for _ in 0..2 {
        new_vm(&mut app);
        app.world()
            .non_send_resource::<UiScript>()
            .run(
                r#"
                HITS = 0
                local f = CreateFrame("Frame")
                f:RegisterEvent("UNIT_STATS")
                f:SetScript("OnEvent", function(self, event, unit)
                    if unit == "pet" then
                        HITS = HITS + 1
                        STAMINA = UnitStat(unit, 3)
                        ARMOR = UnitArmor(unit)
                        LOW, HIGH = UnitDamage(unit)
                    end
                end)
                "#,
            )
            .unwrap();
        app.update();
        let script = app.world().non_send_resource::<UiScript>();
        assert_eq!(
            script
                .eval::<(i64, i64, i64, f64, f64)>("return HITS, STAMINA, ARMOR, LOW, HIGH")
                .unwrap(),
            (1, 68, 1810, 30.5, 44.5),
            "the first synchronous stat callback must not paint the absent-pet shape"
        );
        assert_eq!(app.world().resource::<PetDollBooth>().unit, Some(pet));
        app.update();
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<i64>("return HITS")
                .unwrap(),
            1,
            "a later frame cannot repair the first paint: unchanged stats fire no event"
        );
    }
}

#[test]
fn first_health_callback_sees_group_feign_exemption_on_login_and_a_replacement_vm() {
    let mut app = startup_app();
    let target = app
        .world_mut()
        .spawn((
            Guid(MEMBER),
            ObjectStore(
                ObjectFields::from_pairs(&[
                    (Field::UnitHealth.index(), 1200),
                    (Field::UnitMaxHealth.index(), 1500),
                    (Field::UnitDynamicFlags.index(), 0x20),
                ])
                .into_created(ObjectType::Player),
            ),
        ))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(MEMBER, target);
    {
        let mut selection = app.world_mut().resource_mut::<Selection>();
        selection.target = Some(target);
        selection.guid = Some(MEMBER);
    }
    app.world_mut().resource_mut::<GroupState>().apply_list(
        0,
        0,
        vec![GroupMemberEntry {
            name: "Feigning member".into(),
            guid: MEMBER,
            status: member_status::ONLINE,
            flags: 0,
        }],
        ME,
        None,
        Some(ME),
    );

    for _ in 0..2 {
        new_vm(&mut app);
        app.world()
            .non_send_resource::<UiScript>()
            .run(
                r#"
                HITS = 0
                local f = CreateFrame("Frame")
                f:RegisterEvent("UNIT_HEALTH")
                f:SetScript("OnEvent", function(self, event, unit)
                    if unit == "target" then
                        HITS = HITS + 1
                        HEALTH = UnitHealth(unit)
                        MAX_HEALTH = UnitHealthMax(unit)
                        DEAD = UnitIsDead(unit) and true or false
                    end
                end)
                "#,
            )
            .unwrap();
        app.update();
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<(i64, i64, i64, bool)>("return HITS, HEALTH, MAX_HEALTH, DEAD")
                .unwrap(),
            (1, 1200, 1500, false),
            "a grouped feigning target must not paint zero health on the first callback"
        );
        app.update();
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<i64>("return HITS")
                .unwrap(),
            1,
            "an unchanged snapshot emits no corrective health event"
        );
    }
}
