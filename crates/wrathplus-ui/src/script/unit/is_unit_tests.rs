//! `UnitIsUnit`'s token-or-name resolution.

use crate::script::UiScript;

/// 12340's `UnitIsUnit` (`0x60c3d0`): equal strings answer 1 before any lookup, a shared guid
/// through two tokens or a group member's name answers 1, and anything unresolved answers nil.
#[test]
fn unit_is_unit_takes_tokens_and_member_names_and_never_raises() {
    let mut s = UiScript::new().unwrap();
    s.set_unit_guids(&crate::script::UnitGuids {
        target: 42,
        ..Default::default()
    });
    s.set_party(crate::script::PartyState {
        members: vec![crate::script::PartyMemberInfo {
            name: "Alice".into(),
            guid: 42,
        }],
        ..Default::default()
    });
    let is = |s: &UiScript, a: &str, b: &str| {
        s.eval::<Option<i64>>(&format!("return UnitIsUnit({a:?}, {b:?})"))
            .unwrap()
    };
    assert_eq!(
        is(&s, "focus", "FOCUS"),
        Some(1),
        "equal strings, no lookup"
    );
    assert_eq!(
        is(&s, "target", "Alice"),
        Some(1),
        "a member's name resolves"
    );
    assert_eq!(is(&s, "alice", "target"), Some(1), "case-insensitively");
    assert_eq!(is(&s, "target", "Bob"), None, "an unknown name is nil");
    assert_eq!(is(&s, "target", "focus"), None, "an empty token is nil");
    // "player" with no pushed snapshot: the guid table alone answers.
    s.set_unit_guids(&crate::script::UnitGuids {
        player: 42,
        target: 42,
        ..Default::default()
    });
    assert_eq!(
        is(&s, "player", "target"),
        Some(1),
        "the guid table resolves"
    );
    assert!(
        s.run("UnitIsUnit(\"target\")").is_err(),
        "the usage gate stays"
    );
}
