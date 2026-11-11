use super::*;

/// `IsSelectable` refuses the whole commit before the dedup (`0x4935ee`/`0x4935f3`): nothing is
/// sent and the held target survives. vmangos flags the pre-event Baron Rivendare this way
/// (`instance_stratholme.cpp:224`).
#[test]
fn not_selectable_refuses_the_commit_and_keeps_the_old_target() {
    use wrathplus_protocol::ObjectFields;
    const FLAGS: u16 = Field::UnitFlags.index();
    const CREATEDBY: u16 = Field::UnitCreatedby.index();
    const OBJECT_TYPE: u16 = Field::ObjectType.index();
    const NOT_SELECTABLE: u32 = 1 << 25;
    // `OBJECT_FIELD_TYPE` bit 3, a unit: the only kind whose `+0x58` slot is not the base stub.
    let unit = |pairs: &[(u16, u32)]| {
        let mut all = vec![(OBJECT_TYPE, 0x8u32)];
        all.extend_from_slice(pairs);
        ObjectStore(ObjectFields::from_pairs(&all))
    };

    let (mut world, rx) = commit_world();
    let sent = |rx: &crossbeam_channel::Receiver<ClientCommand>| rx.try_iter().count();

    let plain = unit(&[]);
    assert!(go_with(&mut world, 0xA, Some(plain), false, Some(1), true).changed);
    assert_eq!(sent(&rx), 1);

    // The Baron: flagged, created by nobody, refused; 0xA is still the target.
    let baron = unit(&[(FLAGS, NOT_SELECTABLE | 0x2)]);
    let out = go_with(&mut world, 0xB, Some(baron), true, Some(1), true);
    assert!(!out.changed && !out.swung, "the commit is a complete no-op");
    assert_eq!(sent(&rx), 0, "nothing goes out — not even the stop");
    assert_eq!(
        world.resource::<Selection>().guid,
        Some(0xA),
        "the target already in hand survives the refused click"
    );

    // The own-pet clause: the same flag on a unit I created selects only if it is my pet.
    let mine = |guid: u64| {
        unit(&[
            (FLAGS, NOT_SELECTABLE),
            (CREATEDBY, 1),
            (CREATEDBY + 1, 0),
            (Field::ObjectGuid.index(), guid as u32),
        ])
    };
    assert!(!go_with(&mut world, 0xF, Some(mine(0xF)), false, Some(1), true).changed);
    assert_eq!(sent(&rx), 0);
    assert!(go_with(&mut world, PET, Some(mine(PET)), false, Some(1), true).changed);
    assert_eq!(sent(&rx), 1);

    // Somebody else's flagged creation is not mine to click.
    let theirs = unit(&[(FLAGS, NOT_SELECTABLE), (CREATEDBY, 9), (CREATEDBY + 1, 0)]);
    assert!(!go_with(&mut world, 0xD, Some(theirs), false, Some(1), true).changed);
    assert_eq!(sent(&rx), 0);

    // A non-unit object answers the base stub: a GameObject is never the selection.
    let go_obj = ObjectStore(ObjectFields::from_pairs(&[(OBJECT_TYPE, 0x20)]));
    assert!(!go_with(&mut world, 0xE, Some(go_obj), false, Some(1), true).changed);
    assert_eq!(sent(&rx), 0);

    // An unresolved object (an out-of-range party member) skips the check.
    assert!(go_with(&mut world, 0xF, None, false, Some(1), true).changed);
    assert_eq!(sent(&rx), 1);
}
