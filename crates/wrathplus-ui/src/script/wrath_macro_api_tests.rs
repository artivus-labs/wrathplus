use super::RunningMacro;
use crate::script::{
    ItemTemplateView, MacroBinding, MacroState, MacroView, SpellBookState, SpellInfoView,
    SpellSlotView, UiScript,
};

fn macro_view(id: u32, name: &str, body: &str) -> MacroView {
    MacroView {
        id,
        name: name.into(),
        texture: None,
        body: body.into(),
    }
}

/// Two account macros, a spell catalog with Fireball and Frost Nova, Fireball in the book, and a
/// cached Hearthstone.
fn seeded() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_macros(MacroState {
        account: vec![
            macro_view(1, "Bolt", "/cast Fireball"),
            macro_view(2, "Plain", "/say hi"),
        ],
        character: Vec::new(),
    });
    s.set_spell_catalog([
        SpellInfoView {
            spell_id: 133,
            name: "Fireball".into(),
            rank: Some("Rank 1".into()),
            ..Default::default()
        },
        SpellInfoView {
            spell_id: 122,
            name: "Frost Nova".into(),
            ..Default::default()
        },
    ]);
    s.set_spellbook(SpellBookState {
        tabs: Vec::new(),
        slots: vec![SpellSlotView {
            spell_id: 133,
            name: "Fireball".into(),
            rank: Some("Rank 1".into()),
            ..Default::default()
        }],
    });
    s.set_item_template(
        6948,
        ItemTemplateView {
            name: "Hearthstone".into(),
            quality: 1,
            ..Default::default()
        },
    );
    s.set_macro_bindings(std::collections::HashMap::from([(
        1,
        MacroBinding {
            spell: 133,
            pet_book: false,
        },
    )]));
    s
}

fn answers(s: &UiScript, call: &str) -> String {
    s.eval::<String>(&format!(
        "local function pk(...) return {{n = select('#', ...), ...}} end \
         local t = pk({call}) for i = 1, t.n do t[i] = tostring(t[i]) end \
         return t.n .. ':' .. table.concat(t, ',', 1, t.n)"
    ))
    .unwrap()
}

#[test]
fn the_body_and_the_derived_spell_answer_by_index_or_name() {
    let s = seeded();
    assert_eq!(answers(&s, "GetMacroBody(1)"), "1:/cast Fireball");
    assert_eq!(answers(&s, "GetMacroBody('plain')"), "1:/say hi");
    assert_eq!(answers(&s, "GetMacroBody(9)"), "0:");
    assert_eq!(answers(&s, "GetMacroSpell('Bolt')"), "2:Fireball,Rank 1");
    assert_eq!(
        answers(&s, "GetMacroSpell(2)"),
        "2:nil,nil",
        "no spell: two nils"
    );
    assert_eq!(answers(&s, "GetMacroSpell(9)"), "0:", "no macro: nothing");
    assert_eq!(answers(&s, "GetMacroItem(1)"), "0:", "no item: nothing");
}

#[test]
fn the_setters_override_the_derived_pair_until_the_body_changes() {
    let s = seeded();
    // An unknown spell binds -1 and answers nil; a known one (a leading `!` dropped) answers 1.
    assert_eq!(answers(&s, "SetMacroSpell(2, 'Frost Nova')"), "1:nil");
    assert_eq!(answers(&s, "GetMacroSpell(2)"), "2:nil,nil");
    assert_eq!(answers(&s, "SetMacroSpell('Plain', '!Fireball')"), "1:1");
    assert_eq!(answers(&s, "GetMacroSpell(2)"), "2:Fireball,Rank 1");
    assert_eq!(
        answers(&s, "SetMacroSpell(9, 'Fireball')"),
        "0:",
        "no macro"
    );

    // An item by name or link, clearing the spell; a miss answers nil.
    assert_eq!(answers(&s, "SetMacroItem(1, 'hearthstone')"), "1:1");
    assert_eq!(answers(&s, "GetMacroSpell(1)"), "2:nil,nil");
    let link = answers(&s, "GetMacroItem(1)");
    assert!(
        link.starts_with("2:Hearthstone,") && link.contains("item:6948"),
        "{link}"
    );
    assert_eq!(
        answers(&s, "SetMacroItem(2, '|Hitem:6948:0|h[x]|h')"),
        "1:1"
    );
    assert!(answers(&s, "GetMacroItem(2)").starts_with("2:Hearthstone,"));
    assert_eq!(answers(&s, "SetMacroItem(2, 'Nothing')"), "1:nil");
    assert_eq!(answers(&s, "GetMacroItem(2)"), "0:");

    // Editing the body drops the explicit pair: the derived spell answers again.
    s.run("EditMacro(1, nil, nil, '/cast Fireball')").unwrap();
    assert_eq!(answers(&s, "GetMacroSpell(1)"), "2:Fireball,Rank 1");
    assert_eq!(answers(&s, "GetMacroItem(1)"), "0:");

    for chunk in [
        "SetMacroSpell(1)",
        "SetMacroSpell(1, {})",
        "SetMacroItem(1, nil)",
    ] {
        let e = s.run(chunk).unwrap_err().to_string();
        assert!(e.contains("Usage: SetMacro"), "{chunk}: {e}");
    }
}

#[test]
fn the_running_macro_unwinds_to_the_callers_run() {
    let mut s = seeded();
    assert_eq!(
        answers(&s, "GetRunningMacro(), GetRunningMacroButton()"),
        "2:nil,nil"
    );
    let outer = s.begin_running_macro(RunningMacro {
        id: Some(2),
        button: Some("LeftButton".into()),
    });
    assert_eq!(
        answers(&s, "GetRunningMacro(), GetRunningMacroButton()"),
        "2:2,LeftButton"
    );
    let inner = s.begin_running_macro(RunningMacro {
        id: None,
        button: None,
    });
    assert_eq!(
        answers(&s, "GetRunningMacro(), GetRunningMacroButton()"),
        "2:nil,nil",
        "a RunMacroText run has no record"
    );
    s.end_running_macro(inner);
    assert_eq!(answers(&s, "GetRunningMacro()"), "1:2");
    s.end_running_macro(outer);
    assert_eq!(answers(&s, "GetRunningMacro()"), "1:nil");
}

#[test]
fn cursor_has_macro_answers_the_macro_mode_only() {
    let s = seeded();
    assert_eq!(answers(&s, "CursorHasMacro()"), "1:nil");
    s.run("PickupMacro(1)").unwrap();
    assert_eq!(answers(&s, "CursorHasMacro()"), "1:1");
    s.run("ClearCursor()").unwrap();
    assert_eq!(answers(&s, "CursorHasMacro()"), "1:nil");
}

/// A carried copy answers its own name and link, a suffix included (`0x754a20` leg); the bag
/// copy is found after the equipped slots.
#[test]
fn a_carried_item_answers_its_own_name_and_link() {
    let mut s = seeded();
    let link = "|cffffffff|Hitem:6948:0:0:0:0:0:0:0:80|h[Hearthstone of the Bear]|h|r";
    s.set_container(
        1,
        Some(crate::script::ContainerState {
            name: None,
            num_slots: 4,
            slots: std::collections::HashMap::from([(
                3,
                crate::script::ContainerSlot {
                    item_id: 6948,
                    count: 1,
                    link: Some(link.into()),
                    ..Default::default()
                },
            )]),
        }),
    );
    s.run("SetMacroItem(1, 'Hearthstone')").unwrap();
    assert_eq!(
        answers(&s, "GetMacroItem(1)"),
        format!("2:Hearthstone of the Bear,{link}")
    );
}

/// A spell only the pet's book holds binds from it, and the push's book flag says so.
#[test]
fn set_macro_spell_falls_back_to_the_pets_book() {
    let mut s = seeded();
    s.set_pet_book(crate::script::PetBookState {
        token: Some("PET".into()),
        slots: vec![SpellSlotView {
            spell_id: 122,
            name: "Frost Nova".into(),
            ..Default::default()
        }],
    });
    assert_eq!(answers(&s, "SetMacroSpell(2, 'Frost Nova')"), "1:1");
    assert_eq!(answers(&s, "GetMacroSpell(2)"), "2:Frost Nova,");
    let explicit = s.macro_explicit_binds();
    assert_eq!(
        explicit.get(&2),
        Some(&super::ExplicitMacroBind {
            spell: 122,
            pet_book: true,
            item: 0,
            target: 0,
        })
    );
    let before = s.macro_explicit_generation();
    s.run("SetMacroSpell(1, 'Fireball')").unwrap();
    assert!(s.macro_explicit_generation() > before, "the bar re-derives");
    assert!(!s.macro_explicit_binds()[&1].pet_book);
}

/// A setter refreshes only the bar slots that show its macro, each firing `ACTIONBAR_SLOT_CHANGED`
/// in slot order within the call (`0x5aa920`), and only when its change test passes: the same
/// spell again, with no item and the same target, fires nothing; a new target does.
#[test]
fn a_setter_fires_slot_changed_for_its_macros_slots_on_a_change() {
    use crate::script::{macro_action_word, ActionSlot};
    let mut s = seeded();
    for (action, id) in [(30, 1), (2, 1), (5, 2)] {
        let (kind, word) = macro_action_word(id);
        s.set_action(
            action,
            Some(ActionSlot {
                kind,
                action: word,
                ..Default::default()
            }),
        );
    }
    s.run(
        "FIRED = {} local f = CreateFrame('Frame') f:RegisterEvent('ACTIONBAR_SLOT_CHANGED') \
         f:SetScript('OnEvent', function(_, _, slot) FIRED[#FIRED + 1] = slot end)",
    )
    .unwrap();
    let fired = |s: &UiScript| {
        s.eval::<String>("local t = table.concat(FIRED, ',') FIRED = {} return t")
            .unwrap()
    };
    // Macro 1 derives Fireball from its body: binding it again is no change.
    s.run("SetMacroSpell(1, 'Fireball')").unwrap();
    assert_eq!(fired(&s), "", "the derived pair already holds the spell");
    s.run("SetMacroSpell(1, 'Frost Nova')").unwrap();
    assert_eq!(fired(&s), "2,30", "macro 1's slots, in slot order");
    s.run("SetMacroSpell(1, 'Frost Nova', 'player')").unwrap();
    assert_eq!(
        fired(&s),
        "",
        "no player guid: the target reads 0 as before"
    );
    s.run("SetMacroItem(2, 'Hearthstone')").unwrap();
    assert_eq!(fired(&s), "5");
    s.run("SetMacroItem(2, 'Hearthstone')").unwrap();
    assert_eq!(fired(&s), "", "the same item again");
    s.run("SetMacroSpell(2, 'Nothing')").unwrap();
    assert_eq!(
        fired(&s),
        "5",
        "an item was bound, so the spell setter refreshes"
    );
}
