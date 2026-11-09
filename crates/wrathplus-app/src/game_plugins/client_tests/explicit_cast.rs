//! Explicit unit casting through the ordered production script seam.
use super::*;

/// By-name casts accept units; indexed casts retain their boolean self modifier.
#[test]
fn on_self_casts_at_the_player_and_leaves_the_selection() {
    for (lua, target, selection) in [
        (r#"CastSpellByName("Heal", "player")"#, ME, OLD),
        (r#"CastSpell(1, "spell", "yes")"#, ME, OLD),
        (r#"CastSpell(1, "spell", 1)"#, ME, OLD),
        (
            r#"TargetUnit("party1") CastSpellByName("Heal", "player")"#,
            ME,
            NEW,
        ),
        (r#"CastSpellByName("Heal")"#, OLD, OLD),
        (r#"CastSpellByName("Heal", false)"#, OLD, OLD),
        (r#"CastSpellByName("Heal", nil)"#, OLD, OLD),
        (r#"CastSpell(1, "spell")"#, OLD, OLD),
        (r#"CastSpell(1, "spell", "0")"#, OLD, OLD),
    ] {
        let mut f = frame(true);
        run(&mut f, lua);
        assert_eq!(casts(&f), vec![(HEAL, Some(target))], "{lua}");
        assert_eq!(selected(&f), Some(selection), "{lua}");
    }
}

/// Power Word: Fortitude's word, an assist unit (implicit target 21), in the catalog and
/// the book.
const FORTITUDE: u32 = 1243;

fn with_fortitude(f: &mut Frame) {
    let world = f.app.world_mut();
    let display = wrathplus_formats::SpellDisplay {
        range_index: 4,
        implicit_target_a1: 21,
        ..Default::default()
    };
    world.insert_resource(crate::app_core::state::Spells {
        catalog: wrathplus_formats::SpellCatalog::from_displays([(FORTITUDE, display)].into()),
        ranges: wrathplus_formats::SpellRangeCatalog::from_rows(
            [(
                4,
                wrathplus_formats::SpellRange {
                    min: 0.0,
                    max: 30.0,
                    flags: 0,
                },
            )]
            .into(),
        ),
        ..crate::app_core::state::Spells::empty_for_tests()
    });
    world
        .non_send_resource_mut::<UiScript>()
        .set_spellbook(SpellBookState {
            tabs: vec![SpellTabView {
                name: "Priest".into(),
                texture: None,
                offset: 0,
                num_spells: 1,
            }],
            slots: vec![SpellSlotView {
                spell_id: FORTITUDE,
                name: "Fortitude".into(),
                ..Default::default()
            }],
        });
}

/// The bind runs the ordinary relation chain over the player: with a unit we cannot assist
/// selected and `autoSelfCast` off, the plain call binds nothing (the cursor comes up), while
/// `onSelf` binds us; with a friendly player selected the plain call binds that player.
#[test]
fn on_self_binds_the_player_where_the_selection_is_not_assistable() {
    for (selected_template, lua, target) in [
        (FOE, r#"CastSpellByName("Fortitude")"#, None),
        (FOE, r#"CastSpellByName("Fortitude", "player")"#, Some(ME)),
        (FOE, r#"CastSpell(1, "spell", 1)"#, Some(ME)),
        (FRIEND, r#"CastSpellByName("Fortitude")"#, Some(OLD)),
        (
            FRIEND,
            r#"CastSpellByName("Fortitude", "player")"#,
            Some(ME),
        ),
    ] {
        let mut f = frame(true);
        // `CanAssist` passes a friendly player-controlled unit (`UNIT_FLAG_PLAYER_CONTROLLED`).
        with_factions(&mut f, selected_template, &[(FLAGS, 0x8)]);
        with_fortitude(&mut f);
        // Off, as the doc says: build 12340 registers it "1".
        let world = f.app.world_mut();
        let events = {
            let mut cvars = world.resource_mut::<crate::cvars::Cvars>();
            cvars.set("autoSelfCast", "0");
            cvars.take_events()
        };
        for event in events {
            world.trigger(event);
        }
        run(&mut f, lua);
        let sent: Vec<_> = target.into_iter().map(|t| (FORTITUDE, Some(t))).collect();
        assert_eq!(casts(&f), sent, "{lua} at template {selected_template}");
        assert_eq!(selected(&f), Some(OLD), "{lua}");
    }
}

#[test]
fn explicit_focus_self_and_ordered_target_preserve_selection() {
    for (lua, target, selection) in [
        (r#"CastSpellByName('Heal', 'focus')"#, NEW, OLD),
        (r#"CastSpellByName('Heal', 'player')"#, ME, OLD),
        (
            r#"TargetUnit('party1') CastSpellByName('Heal', 'target')"#,
            NEW,
            NEW,
        ),
        (
            r#"CastSpellByName('Heal', 'focus') TargetUnit('party1')"#,
            NEW,
            NEW,
        ),
        (r#"CastSpell(1, 'spell', 'focus')"#, OLD, OLD),
        (r#"CastSpell(1, 'spell', 'target')"#, ME, OLD),
    ] {
        let mut f = frame(true);
        run(&mut f, lua);
        assert_eq!(casts(&f), vec![(HEAL, Some(target))], "{lua}");
        assert_eq!(selected(&f), Some(selection), "{lua}");
    }
}

#[test]
fn explicit_invalid_never_uses_the_unrelated_selection() {
    for token in ["unknown-unit", "party4", "none", "focusxxx"] {
        let mut f = frame(true);
        run(&mut f, &format!("CastSpellByName('Heal', '{token}')"));
        assert!(casts(&f).is_empty(), "{token}");
        assert_eq!(selected(&f), Some(OLD));
    }
}

#[test]
fn explicit_candidate_range_and_relation_gates_use_focus_not_selection() {
    for (distance, flags, sent) in [
        (6.0, 0x8, true),
        (80.0, 0x8, false),
        (6.0, 0x02000000, false),
    ] {
        let mut f = frame(true);
        with_factions(&mut f, FOE, &[(FLAGS, 0x8)]);
        with_fortitude(&mut f);
        let world = f.app.world_mut();
        let entity = world.resource::<GuidIndex>().0[&NEW];
        world.entity_mut(entity).insert((
            Transform::from_xyz(distance, 0.0, 0.0),
            ObjectStore(wrathplus_protocol::ObjectFields::from_fields(&[
                (Field::UnitHealth, 100),
                (Field::UnitFactionTemplate, FRIEND),
                (Field::UnitFlags, flags),
            ])),
        ));
        run(&mut f, "CastSpellByName('Fortitude', 'focus')");
        assert_eq!(!casts(&f).is_empty(), sent, "{distance} {flags}");
        assert_eq!(selected(&f), Some(OLD));
    }
}

#[test]
fn explicit_item_unit_reaches_the_real_use_item_packet() {
    use wrathplus_protocol::types::{ItemUseSpell, UseItemTarget};
    use wrathplus_ui::script::{ContainerSlot, ContainerState};
    const ITEM: u64 = 0x4000_0000_0000_0123;
    for lua in [
        "UseContainerItem(0, 1, 'focus')",
        "UseItemByName(2589, 'focus')",
        "UseInventoryItem(14, 'focus')",
    ] {
        let mut f = frame(true);
        let world = f.app.world_mut();
        crate::items::test_spawn_item(
            world,
            ITEM,
            wrathplus_protocol::ObjectFields::from_fields(&[(Field::ObjectEntry, 2589)]),
            false,
        );
        let me = world.resource::<GuidIndex>().0[&ME];
        let pack = Field::PlayerPackSlot1.index();
        let worn = Field::PlayerInvSlotHead.index() + 13 * 2;
        world
            .entity_mut(me)
            .insert(ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(
                &[
                    (Field::UnitHealth.index(), 100),
                    (pack, ITEM as u32),
                    (pack + 1, (ITEM >> 32) as u32),
                    (worn, ITEM as u32),
                    (worn + 1, (ITEM >> 32) as u32),
                ],
            )));
        let mut template = crate::items::test_template("Test item");
        template.bonding = 3;
        template.use_spell = Some(ItemUseSpell {
            spell_id: HEAL,
            cooldown_ms: -1,
            category: 0,
            category_cooldown_ms: -1,
        });
        world
            .resource_mut::<crate::items::Items>()
            .insert_template(2589, Some(template));
        let mut script = world.non_send_resource_mut::<UiScript>();
        let mut bag = ContainerState {
            num_slots: 16,
            ..Default::default()
        };
        bag.slots.insert(
            1,
            ContainerSlot {
                item_id: 2589,
                count: 1,
                ..Default::default()
            },
        );
        script.set_container(0, Some(bag));
        run(&mut f, lua);
        assert!(f.rx.is_empty(), "use waits for bind confirmation");
        run(&mut f, "TargetUnit('player') ConfirmBindOnUse()");
        // The ordinary item confirmation drain, after the ordered script calls.
        f.app
            .world_mut()
            .run_system_once(crate::ui_items::confirm_bind_on_use)
            .unwrap();
        let command =
            f.rx.try_iter()
                .find(|c| matches!(c, ClientCommand::UseItem { .. }))
                .expect(lua);
        assert!(
            matches!(
                &command,
                ClientCommand::UseItem {
                    item_guid: ITEM,
                    target: UseItemTarget::Unit(NEW),
                    ..
                }
            ),
            "{command:?}"
        );
        let (opcode, body) = crate::net::io::session_packet(command);
        assert_eq!(opcode, 0x00ab);
        assert!(
            body.ends_with(&[1, NEW as u8]),
            "packed target GUID: {body:?}"
        );
        assert_eq!(selected(&f), Some(ME));
    }
}

#[test]
fn explicit_nobody_still_allows_a_targetless_spell() {
    let mut f = frame(true);
    f.app
        .world_mut()
        .insert_resource(crate::app_core::state::Spells {
            catalog: wrathplus_formats::SpellCatalog::from_displays(
                [(HEAL, Default::default())].into(),
            ),
            ..crate::app_core::state::Spells::empty_for_tests()
        });
    run(&mut f, "CastSpellByName('Heal', 'party4')");
    assert_eq!(casts(&f), vec![(HEAL, None)]);
    assert_eq!(selected(&f), Some(OLD));
}

#[test]
fn binding_modifiers_focus_self_and_missing_focus_keep_native_precedence() {
    for (shift, alt, focus, target) in [
        (true, false, NEW, NEW),
        (true, true, NEW, ME),
        (false, true, NEW, ME),
        (true, false, 0, OLD),
        (false, false, NEW, OLD),
    ] {
        let mut f = frame(true);
        {
            let mut script = f.app.world_mut().non_send_resource_mut::<UiScript>();
            script.set_unit_guids(&wrathplus_ui::script::UnitGuids {
                player: ME,
                target: OLD,
                focus,
                held: [(ME, 0), (OLD, 0), (NEW, 0)].into(),
                ..Default::default()
            });
            script.register_modified_clicks(&wrathplus_ui::bindings_xml::parse_modified_clicks(
            r#"<Bindings><ModifiedClick action="SELFCAST" default="ALT"/><ModifiedClick action="FOCUSCAST" default="SHIFT"/></Bindings>"#).unwrap());
            script.set_unit(
                "focus",
                (focus != 0).then_some(wrathplus_ui::script::UnitState {
                    guid: focus,
                    exists: true,
                    ..Default::default()
                }),
            );
            script.set_modifiers(shift, false, alt);
            script.execute_binding("SPELL Heal", true).unwrap();
        }
        f.app
            .world_mut()
            .run_system_once(apply_script_calls)
            .unwrap();
        assert_eq!(
            casts(&f),
            vec![(HEAL, Some(target))],
            "{shift} {alt} {focus}"
        );
        assert_eq!(selected(&f), Some(OLD));
    }
}

#[test]
fn existing_macro_option_unit_executes_through_the_same_cast_seam() {
    let mut f = frame(true);
    run(
        &mut f,
        "local spell, unit = SecureCmdOptionParse('[@focus] Heal'); CastSpellByName(spell, unit)",
    );
    assert_eq!(casts(&f), vec![(HEAL, Some(NEW))]);
    assert_eq!(selected(&f), Some(OLD));
}

#[path = "explicit_cast_retry.rs"]
mod retry;
