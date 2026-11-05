use crate::bindings_xml::parse;
use crate::script::keybind::{parse_bindings_wtf, parse_bindings_wtf_layers};
use crate::script::UiScript;

/// Two commands with default keys, loaded live, and a counter of `UPDATE_BINDINGS`.
fn script() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_default_bindings(parse_bindings_wtf("bind W MOVEFORWARD\nbind SPACE JUMP\n"));
    s.load_binding_set(1);
    s.register_bindings(
        &parse(
            r#"<Bindings>
                <Binding name="MOVEFORWARD">x()</Binding>
                <Binding name="JUMP">x()</Binding>
            </Bindings>"#,
        )
        .unwrap(),
    );
    s.run(
        "UPDATES = 0 local f = CreateFrame('Frame') f:RegisterEvent('UPDATE_BINDINGS') \
         f:SetScript('OnEvent', function() UPDATES = UPDATES + 1 end)",
    )
    .unwrap();
    s
}

/// `UPDATE_BINDINGS` fired since the last read.
fn updates(s: &UiScript) -> i64 {
    s.eval("local n = UPDATES UPDATES = 0 return n").unwrap()
}

fn text(s: &UiScript, chunk: &str) -> String {
    s.eval(&format!("return table.concat({{ {chunk} }}, ',')"))
        .unwrap()
}

/// `SetBinding`'s mode stores a key apart from mode 1; `GetBinding` reads mode 1 unless told,
/// `GetBindingKey` and `GetBindingAction` read the resolved view, which takes mode 2 only while
/// combat switches it on. A flip fires `UPDATE_BINDINGS` once; dispatch follows the view.
#[test]
fn a_higher_mode_resolves_only_while_switched_on() {
    let mut s = script();
    assert_eq!(text(&s, "SetBinding('F', 'JUMP', 2)"), "1");
    assert_eq!(text(&s, "GetBinding(2)"), "JUMP,SPACE");
    assert_eq!(text(&s, "GetBinding(2, 2)"), "JUMP,F");
    assert_eq!(text(&s, "GetBindingKey('JUMP')"), "SPACE");
    assert_eq!(text(&s, "GetBindingKey('JUMP', 2)"), "F");
    assert_eq!(text(&s, "GetBindingAction('F')"), "");
    assert_eq!(text(&s, "GetBindingAction('F', nil, 2)"), "JUMP");
    updates(&s);
    s.set_combat_binding_mode(true);
    s.set_combat_binding_mode(true);
    assert_eq!(updates(&s), 1, "one flip, one event");
    // SPACE (mode 1) and F (mode 2) are each their mode's first JUMP key: both order 0.
    assert_eq!(text(&s, "GetBindingKey('JUMP')"), "SPACE");
    assert_eq!(text(&s, "GetBindingAction('F')"), "JUMP");
    assert!(s
        .effective_binding_keys()
        .contains(&("F".into(), "JUMP".into())));
    s.set_combat_binding_mode(false);
    assert_eq!(updates(&s), 1);
    assert!(!s.effective_binding_keys().iter().any(|(k, _)| k == "F"));
    // A mode argument outside 1-4 is the default.
    assert_eq!(text(&s, "GetBinding(2, 5)"), "JUMP,SPACE");
    assert_eq!(text(&s, "GetBindingKey('JUMP', 0)"), "SPACE");
}

/// An unbind in mode 2 stores `NONE`, which shadows the mode-1 command while the mode is on: the
/// key reads as no command and dispatch leaves it out.
#[test]
fn an_unbind_in_a_higher_mode_shadows_the_base_key() {
    let mut s = script();
    s.run("SetBinding('W', nil, 2)").unwrap();
    assert_eq!(text(&s, "GetBindingAction('W')"), "MOVEFORWARD");
    s.set_combat_binding_mode(true);
    assert_eq!(text(&s, "GetBindingAction('W')"), "");
    assert_eq!(text(&s, "GetBindingKey('MOVEFORWARD')"), "");
    assert_eq!(text(&s, "GetBindingKey('NONE', 2)"), "W");
    assert!(!s.effective_binding_keys().iter().any(|(k, _)| k == "W"));
}

/// Under the resolved view a key's order is its place in the mode it resolves in, so two modes'
/// first keys of one command share order 0 and only the first in record order answers.
#[test]
fn resolved_keys_walk_orders_until_one_is_missing() {
    let mut s = script();
    s.run("SetBinding('F', 'MOVEFORWARD', 2)").unwrap();
    s.set_combat_binding_mode(true);
    assert_eq!(text(&s, "GetBindingKey('MOVEFORWARD')"), "W");
    s.run("SetBinding('G', 'MOVEFORWARD', 2)").unwrap();
    assert_eq!(text(&s, "GetBindingKey('MOVEFORWARD')"), "W,G");
}

/// `GetBindingAction` answers a form or a declared command only; with `checkOverride` an
/// override of the key answers first, and has no higher modes.
#[test]
fn get_binding_action_checks_the_command_and_the_overrides() {
    let s = script();
    s.run(
        "SetBinding('T', 'NOTDECLARED') SetBinding('Y', 'spell Fireball') \
         O = CreateFrame('Frame') SetOverrideBinding(O, false, 'W', 'JUMP')",
    )
    .unwrap();
    assert_eq!(text(&s, "GetBindingAction('T')"), "");
    assert_eq!(text(&s, "GetBindingAction('Y')"), "spell Fireball");
    assert_eq!(text(&s, "GetBindingAction('W')"), "MOVEFORWARD");
    assert_eq!(text(&s, "GetBindingAction('W', true)"), "JUMP");
    assert_eq!(text(&s, "GetBindingAction('W', true, 1)"), "JUMP");
    assert_eq!(text(&s, "GetBindingAction('W', true, 2)"), "");
    assert!(s.run("GetBindingAction()").is_err());
    assert!(s.run("GetBindingKey({})").is_err());
    assert!(s.run("SetBinding()").is_err());
}

/// The stores fire `UPDATE_BINDINGS` in the call as 12340 does: `SetBinding` and the forms on a
/// stored key, `LoadBindings`; not a refused key, `SaveBindings`, an override or the mouselook
/// table.
#[test]
fn update_bindings_fires_where_the_reference_fires_it() {
    let s = script();
    updates(&s);
    for (chunk, n) in [
        ("SetBinding('F', 'JUMP')", 1),
        ("SetBinding('F', 'JUMP')", 1),
        ("SetBinding('NOTAKEY', 'JUMP')", 0),
        ("SetBindingSpell('G', 'Fireball', 3)", 1),
        ("SetBindingClick('H', 'Nobody')", 1),
        ("SaveBindings(1)", 0),
        ("LoadBindings(1)", 1),
        (
            "SetOverrideBinding(CreateFrame('Frame'), true, 'J', 'JUMP')",
            0,
        ),
        ("SetMouselookOverrideBinding('K', 'JUMP')", 0),
    ] {
        s.run(chunk).unwrap();
        assert_eq!(updates(&s), n, "{chunk}");
    }
    assert_eq!(text(&s, "GetBindingAction('G', nil, 3)"), "SPELL Fireball");
    let e = s.run("SaveBindings(3)").unwrap_err().to_string();
    assert!(e.contains("Usage: SaveBindings(1||2)"), "{e}");
    let e = s.run("LoadBindings(3)").unwrap_err().to_string();
    assert!(e.contains("Usage: LoadBindings(0||1||2)"), "{e}");
}

/// A set carries its higher modes: saved with the live set, restored by a load, and the account
/// save drops the character set's.
#[test]
fn sets_carry_their_higher_modes() {
    let mut s = script();
    s.run("SetBinding('F', 'JUMP', 2) SaveBindings(2) SetBinding('F', 'MOVEFORWARD', 2)")
        .unwrap();
    assert_eq!(
        s.binding_layer_lines(),
        [(1, "F".to_string(), "MOVEFORWARD".to_string())]
    );
    s.run("LoadBindings(2)").unwrap();
    assert_eq!(text(&s, "GetBindingAction('F', nil, 2)"), "JUMP");
    s.run("LoadBindings(0)").unwrap();
    assert!(s.binding_layer_lines().is_empty(), "the defaults have none");
    s.seed_binding_set(1, Some(Vec::new()));
    s.seed_binding_layers(
        1,
        vec![
            (3, "G".into(), "JUMP".into()),
            (1, "bad key".into(), "JUMP".into()),
        ],
    );
    s.load_binding_set(1);
    assert_eq!(
        s.binding_layer_lines(),
        [(3, "G".to_string(), "JUMP".to_string())]
    );
}

/// The `.wtf` reader's `BINDINGMODE n` sends the lines after it to mode index `n` below 4, a
/// negative or larger one ignored, no digits reading 0; leading blanks are skipped.
#[test]
fn a_bindings_wtf_reads_its_modes() {
    let text =
        "bind A JUMP\nBINDINGMODE 2\n  bind B JUMP\nbindingmode 7\n\tbind C SPELL Holy Light\n\
                BINDINGMODE -1\nbind D JUMP\nBINDINGMODE x\nbind E JUMP\n";
    assert_eq!(
        parse_bindings_wtf(text),
        [
            ("A".to_string(), "JUMP".to_string()),
            ("E".to_string(), "JUMP".to_string())
        ]
    );
    assert_eq!(
        parse_bindings_wtf_layers(text),
        [
            (2, "B".to_string(), "JUMP".to_string()),
            (2, "C".to_string(), "SPELL Holy Light".to_string()),
            (2, "D".to_string(), "JUMP".to_string()),
        ]
    );
}
