use super::*;

#[test]
fn living_target_commit_preserves_ranged_repeat_without_melee_reswing() {
    let (mut world, rx) = commit_world();
    world.resource_mut::<crate::spell::AutoRepeatActive>().0 = Some(75);
    assert!(go(&mut world, 0xA, false, Some(1), true).changed);
    let switched = go(&mut world, 0xB, false, Some(1), true);
    assert!(switched.changed && !switched.swung);
    assert_eq!(
        world.resource::<crate::spell::AutoRepeatActive>().0,
        Some(75)
    );
    assert_eq!(world.resource::<Selection>().guid, Some(0xB));
    let commands: Vec<_> = rx.try_iter().collect();
    assert!(matches!(
        commands.as_slice(),
        [
            ClientCommand::SetSelection { guid: 0xA },
            ClientCommand::SetSelection { guid: 0xB },
        ]
    ));
}
