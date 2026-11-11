//! The per-unit collision height ([`CollisionHeight`]), 12340's `CMovement+0xcc` (`[unit+0x854]`):
//! stamped at create and recomputed on the reference's triggers by `0x725f50`.

use bevy::prelude::*;
use wrathplus_protocol::types::OwnerFallback;
use wrathplus_protocol::EntityKind;

use crate::app_core::session::ObjectStore;
use crate::net::{GuidIndex, NetEntity};

use super::Creatures;

pub(crate) use crate::app_core::state::CollisionHeight;

/// `CREATURE_TYPE_FLAG_USE_MODEL_COLLISION_SIZE`, the creature record's gate on the model law
/// (`0x725f82`, record `+0xc`).
const USE_MODEL_COLLISION: u32 = 0x20000;

/// AzerothCore's `DEFAULT_COLLISION_HEIGHT` (`ObjectDefines.h:49`), not the client's.
const AC_DEFAULT_COLLISION_HEIGHT: f32 = 2.03128;

/// What recomputes the height: `SCALE_X` (`0x72d070` → `0x72a560`), mount and dismount
/// (`0x740450`, `0x741a40` → `0x73d940`), the creature record's arrival (`0x72ce56`); not a display
/// change (`0x716860`). Whether `0x72a560` runs before the descriptor write is unread; this takes
/// the new `SCALE_X`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub(super) struct HeightTriggers {
    scale_x: f32,
    mount: u32,
    record: bool,
}

/// `UNIT_FIELD_NATIVEDISPLAYID` (`[unit+0xd0]+0xf8`), else the rendered display when absent or `0`.
pub(crate) fn native_display_id(store: Option<&ObjectStore>, rendered: Option<u32>) -> Option<u32> {
    store
        .and_then(|s| s.0.unit_native_displayid())
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id != 0)
        .or(rendered)
}

/// A player (`0x725f5f`), or a creature whose cached record carries [`USE_MODEL_COLLISION`].
fn takes_model_law(kind: EntityKind, type_flags: Option<u32>) -> bool {
    kind == EntityKind::Player || type_flags.is_some_and(|f| f & USE_MODEL_COLLISION != 0)
}

/// `collisionHeight × k × max(SCALE_X, 1)` (`0x6cf350`, `0x726030`); without a model column, the
/// ctor default, unscaled (`0x725fe0`). A 0 column (invisible triggers) keeps the default too.
fn collision_height_for(column: Option<f32>, k: f32, scale_x: f32) -> CollisionHeight {
    column
        .filter(|h| *h > 0.0)
        .map_or_else(CollisionHeight::default, |h| {
            CollisionHeight(h * k * scale_x.max(1.0))
        })
}

/// `0x716710`: a body a player drives, remote players included; the setter skips it (`0x6e95bf`).
fn client_controlled(
    kind: EntityKind,
    store: &ObjectStore,
    unit_of: impl Fn(u64) -> Option<(EntityKind, u32)>,
) -> bool {
    let flags = store.0.unit_flags();
    if flags & 0x2 == 0 && flags & 0x00c0_0004 != 0 {
        return false;
    }
    if flags & 0x0100_0000 != 0 {
        return store
            .0
            .unit_owner(OwnerFallback::CreatedBy)
            .and_then(unit_of)
            .is_some_and(|(k, f)| k == EntityKind::Player && f & 0x1 == 0);
    }
    kind == EntityKind::Player && store.0.unit_charmed_by().is_none() && flags & 0x1 == 0
}

/// AzerothCore's own unmounted height, which seats its swimmers (`Unit.cpp:17170`): `SCALE_X ×
/// collisionHeight × modelScale × creatureModelScale` of the native display, a 0 its default.
pub(crate) fn ac_collision_height(
    creatures: Option<&Creatures>,
    store: &ObjectStore,
    rendered: Option<u32>,
) -> f32 {
    let scale = store.0.object_scale_x().unwrap_or(1.0);
    let h = creatures
        .zip(native_display_id(Some(store), rendered))
        .and_then(|(c, d)| Some(c.collision_height(d)? * c.catalog.model_scale(d)?))
        .filter(|h| *h != 0.0)
        .unwrap_or(AC_DEFAULT_COLLISION_HEIGHT);
    scale * h
}

/// Stamp each unit's [`CollisionHeight`] at create and recompute it when a [`HeightTriggers`]
/// input moves. Only a player's create forces the write (`0x73febf`); a driven body keeps the
/// server's after it. Readers take the ctor default until the DBCs load, whose first frame is the
/// create.
#[allow(clippy::type_complexity)]
pub(super) fn stamp_collision_heights(
    mut commands: Commands,
    creatures: Option<Res<Creatures>>,
    families: Option<Res<crate::app_core::catalogs::PetFamilyTables>>,
    names: Option<Res<crate::names::NameCache>>,
    index: Option<Res<GuidIndex>>,
    others: Query<(&NetEntity, &ObjectStore)>,
    units: Query<(
        Entity,
        &NetEntity,
        Ref<ObjectStore>,
        Option<&HeightTriggers>,
        Option<&CollisionHeight>,
    )>,
) {
    let Some(creatures) = creatures.as_deref() else {
        return;
    };
    let names_changed = names.as_ref().is_some_and(|n| n.is_changed());
    for (entity, net, store, triggers, height) in &units {
        if !matches!(net.kind, EntityKind::Unit | EntityKind::Player)
            || (triggers.is_some() && !store.is_changed() && !names_changed)
        {
            continue;
        }
        let record = (net.kind == EntityKind::Unit)
            .then(|| store.0.object_entry())
            .flatten()
            .and_then(|entry| names.as_deref()?.creature_record(entry));
        let now = HeightTriggers {
            scale_x: store.0.object_scale_x().unwrap_or(net.scale),
            mount: store.0.unit_mount_display_id(),
            record: record.is_some(),
        };
        if triggers == Some(&now) {
            continue;
        }
        commands.entity(entity).insert(now);
        let first = triggers.is_none();
        let unit_of = |guid| {
            let e = *index.as_deref()?.0.get(&guid)?;
            others.get(e).ok().map(|(n, s)| (n.kind, s.0.unit_flags()))
        };
        if !(first && net.kind == EntityKind::Player)
            && client_controlled(net.kind, &store, unit_of)
        {
            if first && height.is_none() {
                commands.entity(entity).insert(CollisionHeight::default());
            }
            debug!("collision height: {entity} driven, kept {height:?} (first {first})");
            continue;
        }
        let native = native_display_id(Some(&store), net.display_id);
        let column = takes_model_law(net.kind, record.map(|r| r.type_flags))
            .then(|| native.and_then(|d| creatures.collision_height(d)))
            .flatten();
        let k = super::live_display::unit_model_scale(
            Some(creatures),
            families.as_deref(),
            names.as_deref(),
            net.kind,
            &store,
            native,
        );
        let h = collision_height_for(column, k, now.scale_x);
        if height != Some(&h) {
            debug!(
                "collision height: {entity} {:?} {:?} -> {:.3} (first {first}, native {native:?}, \
                 column {column:?}, k {k:.3}, SCALE_X {:.3})",
                net.kind,
                height.map(|c| c.0),
                h.0,
                now.scale_x
            );
            commands.entity(entity).insert(h);
        }
    }
}

#[cfg(test)]
mod native_display {
    use super::native_display_id;
    use crate::app_core::session::ObjectStore;
    use wrathplus_protocol::types::Field;
    use wrathplus_protocol::ObjectFields;

    /// `UNIT_FIELD_NATIVEDISPLAYID`, between DISPLAYID (67) and MOUNTDISPLAYID (69).
    const NATIVE: u16 = Field::UnitNativeDisplayId.index();

    fn store(pairs: &[(u16, u32)]) -> ObjectStore {
        ObjectStore(ObjectFields::from_pairs(pairs))
    }

    /// A druid in bear form: rendered display 2281, native still the night elf's 55.
    #[test]
    fn a_shapeshifted_unit_keeps_its_native_display() {
        let shifted = store(&[(NATIVE, 55)]);
        assert_eq!(native_display_id(Some(&shifted), Some(2281)), Some(55));
    }

    #[test]
    fn an_absent_or_zero_native_falls_back_to_the_rendered_display() {
        assert_eq!(native_display_id(None, Some(2281)), Some(2281));
        let empty = store(&[]);
        assert_eq!(native_display_id(Some(&empty), Some(2281)), Some(2281));
        let zeroed = store(&[(NATIVE, 0)]);
        assert_eq!(native_display_id(Some(&zeroed), Some(2281)), Some(2281));
        assert_eq!(native_display_id(Some(&empty), None), None);
    }

    #[test]
    fn an_unshifted_unit_is_unaffected() {
        let plain = store(&[(NATIVE, 4945)]);
        assert_eq!(native_display_id(Some(&plain), Some(4945)), Some(4945));
    }
}

#[cfg(test)]
mod law {
    use super::*;
    use wrathplus_protocol::types::Field;
    use wrathplus_protocol::ObjectFields;

    fn store(pairs: &[(Field, u32)]) -> ObjectStore {
        let pairs: Vec<_> = pairs.iter().map(|(f, v)| (f.index(), *v)).collect();
        ObjectStore(ObjectFields::from_pairs(&pairs))
    }

    #[test]
    fn only_players_and_flagged_creatures_take_the_model_law() {
        assert!(takes_model_law(EntityKind::Player, None));
        assert!(takes_model_law(EntityKind::Unit, Some(USE_MODEL_COLLISION)));
        assert!(
            !takes_model_law(EntityKind::Unit, Some(0x4)),
            "a boss without the bit"
        );
        assert!(
            !takes_model_law(EntityKind::Unit, None),
            "an uncached record"
        );
    }

    #[test]
    fn the_model_law_scales_by_k_and_by_scale_x_floored_at_one() {
        let h = |column, k, scale_x| collision_height_for(column, k, scale_x).0;
        assert!(
            (h(Some(2.111), 1.25, 1.0) - 2.639).abs() < 5e-4,
            "tauren female, display 60"
        );
        assert_eq!(h(Some(2.0), 1.0, 0.5), 2.0, "a shrink does not shrink it");
        assert_eq!(h(Some(2.0), 1.0, 1.5), 3.0);
        let default = CollisionHeight::default().0;
        assert_eq!(h(None, 1.25, 2.0), default, "the default is unscaled");
        assert_eq!(h(Some(0.0), 1.0, 1.0), default);
    }

    #[test]
    fn a_driven_body_is_a_player_or_a_unit_a_player_possesses() {
        let none = |_| None;
        let player = |flags| {
            client_controlled(
                EntityKind::Player,
                &store(&[(Field::UnitFlags, flags)]),
                none,
            )
        };
        assert!(player(0), "a remote player too");
        assert!(!player(0x1), "SERVER_CONTROLLED");
        assert!(!player(0x80_0000), "fleeing");
        assert!(!client_controlled(EntityKind::Unit, &store(&[]), none));
        let possessed = store(&[(Field::UnitFlags, 0x0100_0000), (Field::UnitCharmedby, 7)]);
        let by = |kind| move |g: u64| (g == 7).then_some((kind, 0));
        assert!(client_controlled(
            EntityKind::Unit,
            &possessed,
            by(EntityKind::Player)
        ));
        assert!(!client_controlled(
            EntityKind::Unit,
            &possessed,
            by(EntityKind::Unit)
        ));
    }

    #[test]
    fn a_driven_body_keeps_the_servers_height_and_an_npc_restamps() {
        let mut app = App::new();
        app.insert_resource(Creatures {
            catalog: Default::default(),
            models: Default::default(),
        });
        app.add_systems(Update, stamp_collision_heights);
        let mut spawn = |kind| {
            let net = NetEntity {
                kind,
                display_id: None,
                scale: 1.0,
            };
            app.world_mut().spawn((net, store(&[]))).id()
        };
        let (player, npc) = (spawn(EntityKind::Player), spawn(EntityKind::Unit));
        app.update();
        for e in [player, npc] {
            let grown = store(&[(Field::ObjectScaleX, 2.0f32.to_bits())]);
            app.world_mut()
                .entity_mut(e)
                .insert((CollisionHeight(5.0), grown));
        }
        app.update();
        let h = |e| app.world().get::<CollisionHeight>(e).copied();
        assert_eq!(
            h(player),
            Some(CollisionHeight(5.0)),
            "the server's after create"
        );
        assert_eq!(
            h(npc),
            Some(CollisionHeight::default()),
            "a SCALE_X change restamps"
        );
    }
}
