//! Instance data remains available while the item-template query is pending.
use super::*;
use bevy::ecs::system::SystemState;
use wrathplus_protocol::types::Field;

fn pending_slot(current: Option<u32>, max: Option<u32>) -> ContainerSlot {
    let mut pairs = vec![(Field::ObjectEntry, 117), (Field::ItemStackCount, 3)];
    if let Some(value) = current {
        pairs.push((Field::ItemDurability, value));
    }
    if let Some(value) = max {
        pairs.push((Field::ItemMaxdurability, value));
    }
    let native: Vec<_> = pairs
        .into_iter()
        .map(|(field, value)| (field.span().unwrap().index, value))
        .collect();
    let mut world = World::new();
    let entity = world
        .spawn(ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(
            &native,
        )))
        .id();
    world.insert_resource(crate::net::GuidIndex(HashMap::from([(42, entity)])));
    let mut state = SystemState::<Objects>::new(&mut world);
    let objects = state.get(&world);
    let (tx, _rx) = crossbeam_channel::unbounded();
    let commands = crate::net::NetCommands(tx);
    resolve_slot(
        42,
        &objects,
        &Items::default(),
        None,
        crate::items::RollCatalogs::NONE,
        &commands,
        &crate::app_core::cooldowns::Cooldowns::default(),
        None,
        &crate::names::NameCache::default(),
        &mut crate::ui_petition::PetitionState::default(),
        std::time::Instant::now(),
        0.0,
        false,
    )
    .unwrap()
}

#[test]
fn pending_template_keeps_native_item_id_count_and_durability() {
    for current in [0, 17] {
        let slot = pending_slot(Some(current), Some(80));
        assert_eq!((slot.item_id, slot.count), (117, 3));
        assert_eq!(slot.durability, Some((current, 80)));
        assert!(slot.texture.is_none());
        assert!(slot.link.is_none());
        assert!(slot.quality.is_none());
    }
}

#[test]
fn pending_template_does_not_invent_missing_or_indestructible_durability() {
    for (current, max) in [(Some(0), Some(0)), (None, Some(80)), (Some(17), None)] {
        assert_eq!(pending_slot(current, max).durability, None);
    }
}

// Wrath Item.h: random properties occupy slots 7..11, after sockets and their bonus.
fn rolled_slot(
    enchants: &crate::items::Enchants,
    props: Option<&crate::items::RandomProperties>,
    ids: &[(u16, u32)],
) -> ContainerSlot {
    let mut pairs = vec![
        (Field::ObjectEntry.span().unwrap().index, 15969),
        (Field::ItemStackCount.span().unwrap().index, 1),
        (Field::ItemFlags.span().unwrap().index, 1),
        (Field::ItemRandomPropertiesId.span().unwrap().index, 2027),
    ];
    let first = Field::ItemEnchantment.span().unwrap().index;
    pairs.extend(ids.iter().map(|&(slot, id)| (first + 3 * slot, id)));
    let mut world = World::new();
    let entity = world
        .spawn(ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(
            &pairs,
        )))
        .id();
    world.insert_resource(crate::net::GuidIndex(HashMap::from([(42, entity)])));
    let mut state = SystemState::<Objects>::new(&mut world);
    let objects = state.get(&world);
    let (tx, _rx) = crossbeam_channel::unbounded();
    let commands = crate::net::NetCommands(tx);
    let mut items = Items::default();
    let mut template = crate::items::test_template("Beaded Orb");
    template.quality = 2;
    template.inventory_type = 23;
    template.required_level = 5;
    template.random_property = 6273;
    template.stats.clear();
    template.spells.clear();
    items.insert_template(15969, Some(template));
    resolve_slot(
        42,
        &objects,
        &items,
        None,
        crate::items::RollCatalogs {
            enchants: Some(enchants),
            props,
        },
        &commands,
        &crate::app_core::cooldowns::Cooldowns::default(),
        None,
        &crate::names::NameCache::default(),
        &mut crate::ui_petition::PetitionState::default(),
        std::time::Instant::now(),
        0.0,
        false,
    )
    .unwrap()
}

#[test]
fn wrath_inventory_random_enchants_reach_bag_tooltip() {
    let cat = crate::items::Enchants(wrathplus_formats::EnchantCatalog::from_rows(
        Default::default(),
        HashMap::from([
            (2307, "Fixture healing effect".into()),
            (82, "Fixture final property".into()),
        ]),
        Default::default(),
    ));
    let slot = rolled_slot(&cat, None, &[(7, 2307), (11, 82)]);
    assert_eq!(
        slot.enchants.iter().map(|e| e.slot).collect::<Vec<_>>(),
        vec![7, 11]
    );
    assert!(slot.enchants.iter().all(|e| e.remaining_ms.is_none()));
    let control = rolled_slot(&cat, None, &[(0, 2307), (1, (-82i32) as u32)]);
    assert_eq!(
        control
            .enchants
            .iter()
            .map(|e| (e.slot, e.negative))
            .collect::<Vec<_>>(),
        vec![(0, false), (1, true)]
    );
    assert!(rolled_slot(&cat, None, &[]).enchants.is_empty());
    assert!(rolled_slot(&cat, None, &[(7, 999999)]).enchants.is_empty());
    assert_bag_lines(slot, &["Fixture healing effect", "Fixture final property"]);
}

fn assert_bag_lines(slot: ContainerSlot, expected: &[&str]) {
    let mut vm = UiScript::new().unwrap();
    vm.set_item_template(
        15969,
        wrathplus_ui::script::ItemTemplateView {
            name: "Beaded Orb".into(),
            quality: 2,
            inventory_type: 23,
            random_property: 6273,
            ..Default::default()
        },
    );
    vm.set_container(
        0,
        Some(ContainerState {
            num_slots: 1,
            slots: HashMap::from([(1, slot)]),
            ..Default::default()
        }),
    );
    vm.run(
        r#"
        local tt = CreateFrame('GameTooltip', 'OrbTip')
        tt:SetBagItem(0, 1)
        local found = {}
        for i = 1, tt:NumLines() do found[getglobal('OrbTipTextLeft' .. i):GetText()] = true end
        ORB_LINES = found
    "#,
    )
    .unwrap();
    let found = vm
        .eval::<HashMap<String, bool>>("return ORB_LINES")
        .unwrap();
    for line in expected {
        assert!(found.contains_key(*line), "missing {line:?} from {found:?}");
    }
}

#[test]
fn wrath_inventory_logged_orb_resolves_installed_property_and_enchant() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("read-only chain");
    let cat = crate::items::Enchants(wrathplus_formats::load_enchant_catalog(&mut chain).unwrap());
    let props = crate::items::RandomProperties(
        wrathplus_formats::load_random_property_catalog(&mut chain).unwrap(),
    );
    // 093136/client.log:4479: GUID 4611686018430031000, property 2027, field43=2307.
    let row = props.0.get(2027).expect("logged roll");
    assert_eq!(row.enchants[0], 2307);
    let name = cat.0.name(2307).expect("logged enchant");
    eprintln!(
        "logged orb property2027 suffix={:?} enchant2307={name:?}",
        row.suffix
    );
    let slot = rolled_slot(&cat, Some(&props), &[(7, 2307)]);
    assert!(slot
        .link
        .as_ref()
        .unwrap()
        .contains(&format!("Beaded Orb {}", row.suffix)));
    assert_eq!(slot.enchants.len(), 1);
    assert_eq!(slot.enchants[0].name, name);
    assert_bag_lines(slot, &[name]);
}
