//! Descriptor writes the reference notifies on even when the value did not move.
//!
//! A field watcher is registered through `0x4d5a80`, whose node (`0x4d5850`) keeps the handler
//! at `+0x10`, the field offset at `+0x1c`, the length at `+0x24` and, from the last argument,
//! an always-fire flag at `+0x2e`. A values block (`0x4d5550`) runs `0x4d5150` on every field
//! whose bit is in the update mask: a watcher without `+0x2e` is called only when its bytes
//! differ from the snapshot (the [`FieldChanged`] edge), and one with it is called regardless.
//! A create block for an object already initialised (`0x4d63b0`, `+0xbc & 0x40000`) is applied
//! as `0x4d5550(.., 1, ..)`, which runs the pass for every field, masked or not.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use wrathplus_protocol::types::{Field, ObjectType};
use wrathplus_protocol::ObjectFields;

use super::FieldChanged;

/// The always-fire watchers this client models, by unit-block field: the player's form-byte
/// callback `0x6da130`, registered at `0x6e5393` over class 3's byte `0x1d3`, length 1. The
/// unit class starts at descriptor byte `0x18` (`0x4d3bf0`), so that is byte 3 of
/// `UNIT_FIELD_BYTES_2`, a word that also carries the sheath state and the PvP and pet flags.
const ALWAYS_FIRE: [Field; 1] = [Field::UnitBytes2];

/// A descriptor update wrote an always-fire field ([`ALWAYS_FIRE`]) on a streamed unit or
/// player, changed or not.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldWritten {
    pub entity: Entity,
    pub guid: u64,
    pub field: Field,
}

/// The descriptor-update edges: moved fields and always-fire writes.
#[derive(SystemParam)]
pub struct FieldWriters<'w> {
    pub changed: MessageWriter<'w, FieldChanged>,
    pub written: MessageWriter<'w, FieldWritten>,
}

/// The always-fire fields `delta` writes on `store`, before it is merged: those in its mask, or
/// all of them for a re-create. A store with no create block has no class to place them in.
pub fn always_fire_writes(
    store: &ObjectFields,
    delta: &ObjectFields,
    entity: Entity,
    guid: u64,
    mut emit: impl FnMut(FieldWritten),
) {
    let unit = matches!(
        store.created_as(),
        Some(ObjectType::Unit | ObjectType::Player)
    );
    if !unit {
        return;
    }
    let recreate = delta.created_as().is_some();
    for field in ALWAYS_FIRE {
        if recreate || delta.get(field).is_some() {
            emit(FieldWritten {
                entity,
                guid,
                field,
            });
        }
    }
}

/// Apply a descriptor update the way the drain does. The first one must be
/// [`ObjectFields::into_created`], like a real create block.
#[cfg(any(test, feature = "test-support"))]
pub fn apply_fields_for_test(world: &mut World, entity: Entity, delta: ObjectFields) {
    let guid = world.get::<super::Guid>(entity).map_or(0, |g| g.0);
    let (mut edges, mut writes) = (Vec::new(), Vec::new());
    match world.get_mut::<super::ObjectStore>(entity) {
        Some(mut store) => {
            always_fire_writes(&store.0, &delta, entity, guid, |w| writes.push(w));
            super::merge_store_fields(&mut store.0, delta, entity, guid, |e| edges.push(e));
        }
        None => {
            assert!(
                delta.created_as().is_some(),
                "a fixture's first descriptor is a create block: chain `.into_created(..)`"
            );
            world.entity_mut(entity).insert(super::ObjectStore(delta));
        }
    }
    world
        .resource_mut::<Messages<FieldChanged>>()
        .write_batch(edges);
    if let Some(mut messages) = world.get_resource_mut::<Messages<FieldWritten>>() {
        messages.write_batch(writes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::merge_store_fields;

    fn player(bytes_2: u32) -> ObjectFields {
        ObjectFields::from_pairs(&[(Field::UnitBytes2.index(), bytes_2)])
            .into_created(ObjectType::Player)
    }

    fn writes(store: &ObjectFields, delta: &ObjectFields) -> Vec<Field> {
        let mut out = Vec::new();
        always_fire_writes(store, delta, Entity::PLACEHOLDER, 7, |w| out.push(w.field));
        out
    }

    #[test]
    fn a_masked_bytes_2_word_fires_even_unchanged() {
        let store = player(5 << 24);
        let same = ObjectFields::from_pairs(&[(Field::UnitBytes2.index(), 5 << 24)]);
        assert_eq!(writes(&store, &same), [Field::UnitBytes2]);
        let mut merged = store.clone();
        let mut moved = 0;
        merge_store_fields(&mut merged, same, Entity::PLACEHOLDER, 7, |_| moved += 1);
        assert_eq!(moved, 0, "no FieldChanged edge: the value did not move");
    }

    #[test]
    fn a_delta_without_bytes_2_does_not_fire() {
        let store = player(5 << 24);
        let other = ObjectFields::from_pairs(&[(Field::UnitBytes1.index(), 1)]);
        assert!(writes(&store, &other).is_empty());
    }

    #[test]
    fn a_re_create_fires_every_always_fire_field() {
        let store = player(5 << 24);
        let recreate = ObjectFields::from_pairs(&[(Field::UnitHealth.index(), 1)])
            .into_created(ObjectType::Player);
        assert_eq!(writes(&store, &recreate), [Field::UnitBytes2]);
    }

    #[test]
    fn only_a_created_unit_or_player_store_fires() {
        let bytes = ObjectFields::from_pairs(&[(Field::UnitBytes2.index(), 0)]);
        let bare = ObjectFields::from_pairs(&[(Field::UnitBytes2.index(), 0)]);
        assert!(
            writes(&bare, &bytes).is_empty(),
            "no create block, no class"
        );
        let go = ObjectFields::from_pairs(&[]).into_created(ObjectType::GameObject);
        assert!(
            writes(&go, &bytes).is_empty(),
            "the index is another class's field"
        );
    }
}
