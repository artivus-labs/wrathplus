use super::*;
use bevy::ecs::system::RunSystemOnce;
use wrathplus_protocol::types::Field;
use wrathplus_protocol::types::ObjectFields;

const BANK_BAG_LIVE_FIRST: u32 = 68;

/// Wrath Item.h: permanent/temporary slots 0/1 and random-property slots 7..11.
#[test]
fn wrath_equipped_random_enchants_reach_inventory_tooltip() {
    use std::collections::HashMap;
    let catalog = || {
        crate::items::Enchants(wrathplus_formats::EnchantCatalog::from_rows(
            Default::default(),
            HashMap::from([
                (2307, "Fixture healing effect".into()),
                (82, "Fixture final property".into()),
            ]),
            Default::default(),
        ))
    };
    for (ids, expected) in [
        (vec![(7, 2307), (11, 82)], vec![(7, false), (11, false)]),
        (
            vec![(0, 2307), (1, (-82i32) as u32)],
            vec![(0, false), (1, true)],
        ),
        (vec![], vec![]),
        (vec![(7, 999999), (11, 0)], vec![]),
    ] {
        let mut app = world_with_bank_bag(None);
        let inv = Field::PlayerInvSlotHead.index() + 2 * u16::from(SLOT_OFF_HAND);
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ObjectStore, With<SelfPlayer>>();
        q.single_mut(app.world_mut()).unwrap().0 =
            ObjectFields::from_pairs(&[(inv, BAG as u32), (inv + 1, (BAG >> 32) as u32)]);
        let mut pairs = vec![
            (F_OBJECT_ENTRY, 15969),
            (Field::ItemFlags.index(), 1),
            (Field::ItemRandomPropertiesId.index(), 2027),
        ];
        for &(slot, id) in &ids {
            let first = Field::ItemEnchantment.index() + 3 * slot;
            pairs.extend([(first, id), (first + 2, 3)]);
        }
        let entity = crate::items::test_spawn_item(
            app.world_mut(),
            BAG,
            ObjectFields::from_pairs(&pairs),
            false,
        );
        // Keep the real seven-cell deadline component while projecting all twelve slots.
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::items::Countdowns::default());
        let mut template = crate::items::test_template("Beaded Orb");
        template.inventory_type = 23;
        app.world_mut()
            .resource_mut::<Items>()
            .insert_template(15969, Some(template));
        app.world_mut().insert_resource(catalog());
        app.world_mut().run_system_once(feed_char).unwrap();
        app.world_mut()
            .resource_scope(|world, mut state: Mut<CharFeedState>| {
                let memo = state.vm.get(world.non_send_resource::<UiScript>());
                let slot = memo.last_inv.as_ref().unwrap()[17].as_ref().unwrap();
                assert_eq!(
                    slot.enchants
                        .iter()
                        .map(|e| (e.slot, e.negative))
                        .collect::<Vec<_>>(),
                    expected
                );
                assert!(slot.already_bound);
                assert!(slot
                    .enchants
                    .iter()
                    .all(|e| e.remaining_ms.is_none() && e.charges == 3));
            });
        let mut vm = app.world_mut().non_send_resource_mut::<UiScript>();
        vm.set_item_template(
            15969,
            wrathplus_ui::script::ItemTemplateView {
                name: "Beaded Orb".into(),
                inventory_type: 23,
                ..Default::default()
            },
        );
        vm.run(
            r#"
            local tt = CreateFrame('GameTooltip', 'EquippedOrbTip')
            EQUIPPED_ORB_HAS_ITEM = tt:SetInventoryItem('player', 17)
            EQUIPPED_ORB_LINES = {}
            for i = 1, tt:NumLines() do
                EQUIPPED_ORB_LINES[getglobal('EquippedOrbTipTextLeft' .. i):GetText()] = true
            end
        "#,
        )
        .unwrap();
        assert!(vm.eval::<bool>("return EQUIPPED_ORB_HAS_ITEM").unwrap());
        let found = vm
            .eval::<HashMap<String, bool>>("return EQUIPPED_ORB_LINES")
            .unwrap();
        for line in ["Fixture healing effect", "Fixture final property"] {
            assert_eq!(found.contains_key(line), !expected.is_empty(), "{found:?}");
        }
    }
}

/// The reference's split (`0x5ea460`); a value-0 line skips the permanent add (`0x5ea4b4`).
#[test]
fn a_skill_pair_folds_the_permanent_bonus_into_the_base() {
    /// `PLAYER_SKILL_INFO_1_1`: id|step, value|max, temp|perm per slot.
    const F_SKILL: u16 = Field::PlayerSkillInfo11.index();
    let pair = |lo: u16, hi: u16| u32::from(lo) | (u32::from(hi) << 16);
    // Slot 0: Defense (95) at 300, +10 temporary, +5 permanent.
    // Slot 1: Swords (43) at 0, with a +7 permanent that must not surface.
    let store = ObjectStore(ObjectFields::from_pairs(&[
        (F_SKILL, pair(95, 0)),
        (F_SKILL + 1, pair(300, 300)),
        (F_SKILL + 2, pair(10, 5)),
        (F_SKILL + 3, pair(43, 0)),
        (F_SKILL + 4, pair(0, 300)),
        (F_SKILL + 5, pair(0, 7)),
    ]));
    assert_eq!(
        skill_pair(&store, 95),
        (305, 10),
        "the permanent bonus belongs to the base and the temporary one is the modifier"
    );
    assert_eq!(
        skill_pair(&store, 43),
        (0, 0),
        "a line at value 0 skips the perm add — a bare talent bonus is not a skill"
    );
    assert_eq!(
        skill_pair(&store, 162),
        (0, 0),
        "a line the player does not have at all"
    );
    // A malus deeper than the skill is representable, which is why the pair is signed.
    let cursed = ObjectStore(ObjectFields::from_pairs(&[
        (F_SKILL, pair(95, 0)),
        (F_SKILL + 1, pair(5, 300)),
        (F_SKILL + 2, pair(0, u16::MAX - 9)), // perm = -10
    ]));
    assert_eq!(skill_pair(&cursed, 95), (-5, 0));
}

/// `PLAYER_FIELD_BANK_BAG_SLOT_1`, bank bag slot 0's guid pair.
const F_BANK_BAG_1: u16 = Field::PlayerBankbagSlot1.index();
/// `OBJECT_FIELD_ENTRY` on the item object.
const F_OBJECT_ENTRY: u16 = Field::ObjectEntry.index();
/// `ITEM_FIELD_STACK_COUNT`, one of the six item fields the reference's watcher covers.
const F_ITEM_STACK_COUNT: u16 = Field::ItemStackCount.index();
/// Any container entry ("Traveler's Backpack"); the feed only needs it to resolve.
const BAG_ENTRY: u32 = 4500;
const BAG: u64 = 0x4000_0000_0000_0abc;

/// Re-stream a live item object's fields as a values update does, moving its change tick.
fn restream(app: &mut App, guid: u64, fields: ObjectFields) {
    let entity = app
        .world()
        .resource::<crate::net::GuidIndex>()
        .0
        .get(&guid)
        .copied()
        .expect("the item was streamed");
    *app.world_mut().get_mut::<ObjectStore>(entity).unwrap() = ObjectStore(fields);
}

/// A world with `bag`, or nothing, in the first bank bag slot, and a frame recording events.
fn world_with_bank_bag(bag: Option<u64>) -> App {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.init_resource::<CharFeedState>()
        .init_resource::<PaperDollBooth>()
        .init_resource::<PendingItemOps>()
        .init_resource::<Items>()
        .init_resource::<crate::net::GuidIndex>()
        .init_resource::<crate::names::NameCache>()
        .insert_resource(NetCommands(tx));
    let fields = match bag {
        Some(g) => ObjectFields::from_pairs(&[
            (F_BANK_BAG_1, g as u32),
            (F_BANK_BAG_1 + 1, (g >> 32) as u32),
        ]),
        None => ObjectFields::from_pairs(&[(F_BANK_BAG_1, 0), (F_BANK_BAG_1 + 1, 0)]),
    };
    app.world_mut().spawn((SelfPlayer, ObjectStore(fields)));
    if let Some(g) = bag {
        // The bag is an item entity; its template lives on `Items`.
        crate::items::test_spawn_item(
            app.world_mut(),
            g,
            ObjectFields::from_pairs(&[(F_OBJECT_ENTRY, BAG_ENTRY)]),
            true,
        );
        app.world_mut().resource_mut::<Items>().insert_template(
            BAG_ENTRY,
            Some(crate::items::test_template("Traveler's Backpack")),
        );
    }
    let script = UiScript::new().unwrap();
    script
        .run(
            r#"
            SEEN = {}
            local f = CreateFrame("Frame")
            f:RegisterEvent("PLAYERBANKSLOTS_CHANGED")
            f:RegisterEvent("UNIT_INVENTORY_CHANGED")
            f:SetScript("OnEvent", function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9)
                table.insert(SEEN, event .. " " .. tostring(arg1))
            end)
        "#,
        )
        .unwrap();
    app.insert_non_send_resource(script);
    app
}

/// Lock identity is the live inventory ID, not the Vanilla bank item's ID on Wrath.
#[test]
fn bank_bag_lock_identity_follows_descriptor_geometry() {
    use wrathplus_protocol::types::Field;
    for (live, slot) in [(68, 0), (74, 6)] {
        let mut app = world_with_bank_bag(Some(BAG));
        let first = Field::PlayerBankbagSlot1.span().unwrap().index + 2 * slot as u16;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ObjectStore, With<SelfPlayer>>();
        q.single_mut(app.world_mut()).unwrap().0 =
            ObjectFields::from_pairs(&[(first, BAG as u32), (first + 1, (BAG >> 32) as u32)]);
        app.world_mut()
            .resource_mut::<PendingItemOps>()
            .add([(EQUIPMENT_BAG, live, BAG, 1)]);
        app.world_mut().run_system_once(feed_char).unwrap();
        app.world_mut()
            .resource_scope(|world, mut state: Mut<CharFeedState>| {
                let memo = state.vm.get(world.non_send_resource::<UiScript>());
                let bags = memo.last_bank_bags.as_ref().unwrap();
                assert!(bags[slot].as_ref().unwrap().locked);
            });
        assert!(app
            .world()
            .non_send_resource::<UiScript>()
            .eval::<bool>(&format!(
                "return IsInventoryItemLocked({live}) and true or false"
            ))
            .unwrap());
        app.world_mut()
            .resource_mut::<PendingItemOps>()
            .clear_by_failure(BAG);
        app.world_mut().run_system_once(feed_char).unwrap();
        assert!(!app
            .world()
            .non_send_resource::<UiScript>()
            .eval::<bool>(&format!(
                "return IsInventoryItemLocked({live}) and true or false"
            ))
            .unwrap());
    }
}

fn seen(app: &mut App) -> Vec<String> {
    let script = app.world_mut().non_send_resource_mut::<UiScript>();
    let out = script.eval::<Vec<String>>("return SEEN").unwrap();
    script.run("SEEN = {}").unwrap();
    out
}

#[test]
fn a_bank_bag_slot_filling_fires_playerbankslots_changed() {
    let mut app = world_with_bank_bag(Some(BAG));
    app.world_mut().run_system_once(feed_char).unwrap();
    let events = seen(&mut app);
    assert!(
        events.contains(&"PLAYERBANKSLOTS_CHANGED 29".to_string()),
        "the repaint event, for bank slot 29, the first bag slot after the 28 item slots \
         (`0x6dedd0`'s `%d`). Got {events:?}"
    );
    assert!(
        events.contains(&"UNIT_INVENTORY_CHANGED player".to_string()),
        "and the doll's, unchanged, got {events:?}"
    );
}

/// A bag's own field moving is the item-object path (`0x5e84c0`), which fires the slot too.
#[test]
fn a_bank_bags_own_field_moving_fires_the_item_object_producer() {
    let mut app = world_with_bank_bag(Some(BAG));
    app.world_mut().run_system_once(feed_char).unwrap();
    assert!(seen(&mut app).contains(&"PLAYERBANKSLOTS_CHANGED 29".to_string()));

    // The same bag guid, restacked: only the item object moved.
    restream(
        &mut app,
        BAG,
        ObjectFields::from_pairs(&[(F_OBJECT_ENTRY, BAG_ENTRY), (F_ITEM_STACK_COUNT, 3)]),
    );
    app.world_mut().run_system_once(feed_char).unwrap();
    let events = seen(&mut app);
    assert!(
        events.contains(&"PLAYERBANKSLOTS_CHANGED 29".to_string()),
        "the item path fires the same slot, got {events:?}"
    );
}

/// Identical bags push identical views; the reference watches the guids and fires twice.
#[test]
fn two_identical_bags_swapped_between_bank_bag_slots_still_announce() {
    let mut app = world_with_bank_bag(Some(BAG));
    // A second, byte-identical bag in bank bag slot 2.
    const BAG2: u64 = 0x4000_0000_0000_0abd;
    // Rewritten wholesale: `ObjectFields` has no setter.
    let set_slots = |app: &mut App, a: u64, b: u64| {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ObjectStore, With<SelfPlayer>>();
        let mut store = q.single_mut(app.world_mut()).unwrap();
        *store = ObjectStore(ObjectFields::from_pairs(&[
            (F_BANK_BAG_1, a as u32),
            (F_BANK_BAG_1 + 1, (a >> 32) as u32),
            (F_BANK_BAG_1 + 2, b as u32),
            (F_BANK_BAG_1 + 3, (b >> 32) as u32),
        ]));
    };
    set_slots(&mut app, BAG, BAG2);
    crate::items::test_spawn_item(
        app.world_mut(),
        BAG2,
        ObjectFields::from_pairs(&[(F_OBJECT_ENTRY, BAG_ENTRY)]),
        true,
    );
    app.world_mut().run_system_once(feed_char).unwrap();
    let _ = seen(&mut app);

    // Exchange them. Every pushed view is equal before and after.
    set_slots(&mut app, BAG2, BAG);
    app.world_mut().run_system_once(feed_char).unwrap();
    let events = seen(&mut app);
    assert!(
        events.contains(&"PLAYERBANKSLOTS_CHANGED 29".to_string())
            && events.contains(&"PLAYERBANKSLOTS_CHANGED 30".to_string()),
        "one event per slot whose guid moved, got {events:?}"
    );
}

/// A lock flip is `ITEM_LOCK_CHANGED`'s, not a slot change.
#[test]
fn a_lock_flip_alone_does_not_fire_the_repaint_event() {
    let mut app = world_with_bank_bag(Some(BAG));
    app.world_mut().run_system_once(feed_char).unwrap();
    let _ = seen(&mut app);

    // The same bag, now locked by an outbound move.
    app.world_mut().resource_mut::<PendingItemOps>().add([(
        EQUIPMENT_BAG,
        BANK_BAG_LIVE_FIRST,
        BAG,
        1,
    )]);
    app.world_mut().run_system_once(feed_char).unwrap();
    let events = seen(&mut app);
    assert!(
        events.contains(&"UNIT_INVENTORY_CHANGED player".to_string()),
        "the snapshot did move — `locked` is part of it, got {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| e.starts_with("PLAYERBANKSLOTS_CHANGED")),
        "…but the ITEM did not change, so the repaint event stays quiet, got {events:?}"
    );
}

/// The frame a lock clears, `!pending.is_empty()` already reads false;
/// [`PendingItemOps::epoch`] is what still runs the feed.
#[test]
fn the_frame_a_lock_clears_still_repushes_the_slot() {
    let mut app = world_with_bank_bag(Some(BAG));
    app.world_mut().resource_mut::<PendingItemOps>().add([(
        EQUIPMENT_BAG,
        BANK_BAG_LIVE_FIRST,
        BAG,
        1,
    )]);
    app.world_mut().run_system_once(feed_char).unwrap();
    let _ = seen(&mut app);
    assert!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<bool>(r#"return IsInventoryItemLocked(68) and true or false"#)
            .unwrap(),
        "in flight, the slot reads locked"
    );

    // A refusal: the lock clears without any object moving.
    let cleared = app
        .world_mut()
        .resource_mut::<PendingItemOps>()
        .clear_by_failure(BAG);
    assert_eq!(cleared, vec![(EQUIPMENT_BAG, BANK_BAG_LIVE_FIRST)]);
    app.world_mut().run_system_once(feed_char).unwrap();
    assert!(
        !app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<bool>(r#"return IsInventoryItemLocked(68) and true or false"#)
            .unwrap(),
        "the unlock has to reach the VM in the same frame it happened"
    );
}
