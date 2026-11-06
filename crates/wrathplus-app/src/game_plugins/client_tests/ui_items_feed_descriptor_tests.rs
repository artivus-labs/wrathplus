//! Whole-client tests of `crate::ui_items::feed`, from `ui_items/feed/descriptor_tests.rs`.
//! They build the client with `headless_client`, so they live in the root layer.
//!
//! Instance data remains available while the item-template query is pending.

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::inventory::BANK_CONTAINER;
use crate::inventory::KEYRING_CONTAINER;
use crate::ui_items::feed::*;
use bevy::prelude::*;
use wrathplus_protocol::types::Field;
use wrathplus_ui::script::UiScript;

fn native_fields(values: &[(Field, u16, u32)]) -> wrathplus_protocol::ObjectFields {
    let pairs: Vec<_> = values
        .iter()
        .map(|&(field, offset, value)| (field.span().unwrap().index + offset, value))
        .collect();
    wrathplus_protocol::ObjectFields::from_pairs(&pairs)
}

#[test]
fn real_container_feed_includes_last_storage_slots_and_bag_lifecycle() {
    // Real feed + native descriptors, including a low-level Wrath player's entire keyring.
    let (bank_count, bag_count, key_count) = (28u8, 7u8, 32u8);
    let mut app = crate::game_plugins::schedule_tests::headless_client();
    let script = UiScript::new().unwrap();
    script.run("SEEN = {}; local f=CreateFrame('Frame'); f:RegisterEvent('BAG_OPEN'); f:RegisterEvent('BAG_CLOSED'); f:RegisterEvent('BAG_UPDATE'); f:RegisterEvent('PLAYERBANKSLOTS_CHANGED'); f:SetScript('OnEvent', function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) table.insert(SEEN,event..' '..tostring(arg1)) end)").unwrap();
    app.world_mut().insert_non_send_resource(script);
    let player = app
        .world_mut()
        .spawn((
            SelfPlayer,
            ObjectStore(native_fields(&[
                (Field::UnitLevel, 0, 1),
                (Field::PlayerBankSlot1, 2 * u16::from(bank_count - 1), 101),
                (Field::PlayerBankSlot1, 2 * u16::from(bank_count - 1) + 1, 0),
                (Field::PlayerBankbagSlot1, 2 * u16::from(bag_count - 1), 102),
                (
                    Field::PlayerBankbagSlot1,
                    2 * u16::from(bag_count - 1) + 1,
                    0,
                ),
                (Field::PlayerKeyringSlot1, 2 * u16::from(key_count - 1), 103),
                (
                    Field::PlayerKeyringSlot1,
                    2 * u16::from(key_count - 1) + 1,
                    0,
                ),
            ])),
        ))
        .id();
    for (guid, entry, count, container) in [
        (101, 117, 3, false),
        (102, 118, 1, true),
        (103, 119, 5, false),
        (104, 120, 7, false),
    ] {
        let mut values = vec![
            (Field::ObjectEntry, 0, entry),
            (Field::ItemStackCount, 0, count),
        ];
        if container {
            values.extend([
                (Field::ContainerNumSlots, 0, 1),
                (Field::ContainerSlot1, 0, 104),
                (Field::ContainerSlot1, 1, 0),
            ]);
        }
        let entity = app
            .world_mut()
            .spawn((
                crate::net::Guid(guid),
                ObjectStore(native_fields(&values)),
                crate::items::ItemObject { container },
                crate::items::Countdowns::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<crate::net::GuidIndex>()
            .0
            .insert(guid, entity);
    }
    let last_bag = 4 + i64::from(bag_count);
    let store = &app.world().get::<ObjectStore>(player).unwrap().0;
    assert_eq!(
        crate::ui_items::net::container_of(Some(1), Some(store), 102),
        Some(last_bag)
    );
    let mut schedule = Schedule::default();
    schedule.add_systems(feed_containers);
    app.world_mut()
        .resource_mut::<crate::ui_items::net::BagOpens>()
        .0
        .push(last_bag);
    schedule.run(app.world_mut());
    let script = app.world().non_send_resource::<UiScript>();
    for (bag, slot, count) in [
        (BANK_CONTAINER, u32::from(bank_count), 3),
        (KEYRING_CONTAINER, u32::from(key_count), 5),
        (last_bag, 1, 7),
    ] {
        assert_eq!(
            script
                .eval::<u32>(&format!(
                    "local _,count=GetContainerItemInfo({bag},{slot}); return count"
                ))
                .unwrap(),
            count
        );
    }
    let seen = script.eval::<Vec<String>>("return SEEN").unwrap();
    assert!(seen.contains(&format!("BAG_OPEN {last_bag}")), "{seen:?}");
    assert!(
        seen.contains(&"PLAYERBANKSLOTS_CHANGED 28".to_string()),
        "{seen:?}"
    );
    // Remove the final bag and bank item: the same persistent feed memo must close
    // container 11 and announce a transition in vault slot 28 on Wrath.
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("SEEN={}")
        .unwrap();
    app.world_mut().get_mut::<ObjectStore>(player).unwrap().0 = native_fields(&[
        (Field::UnitLevel, 0, 1),
        (Field::PlayerBankSlot1, 2 * u16::from(bank_count - 1), 0),
        (Field::PlayerBankSlot1, 2 * u16::from(bank_count - 1) + 1, 0),
        (Field::PlayerBankbagSlot1, 2 * u16::from(bag_count - 1), 0),
        (
            Field::PlayerBankbagSlot1,
            2 * u16::from(bag_count - 1) + 1,
            0,
        ),
    ]);
    schedule.run(app.world_mut());
    let script = app.world().non_send_resource::<UiScript>();
    let seen = script.eval::<Vec<String>>("return SEEN").unwrap();
    assert!(seen.contains(&format!("BAG_CLOSED {last_bag}")), "{seen:?}");
    assert!(
        seen.contains(&"PLAYERBANKSLOTS_CHANGED 28".to_string()),
        "{seen:?}"
    );
    assert!(
        !seen.contains(&format!("BAG_UPDATE {last_bag}")),
        "removed bag must not update: {seen:?}"
    );
}
