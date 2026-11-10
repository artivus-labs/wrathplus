//! The feeds that read the resolver inputs the aura feed pushes, by the type the schedule holds
//! them under, for the whole-client order tests. A build without `bevy/debug` names no system, but
//! each function system sits in its own `SystemTypeSet`, the set an `.after(feed)` resolves to.

use std::any::TypeId;

use bevy::ecs::schedule::{IntoSystemSet, SystemSet};

/// The type a function system runs under in a schedule (`System::type_id` of its instance).
pub fn system_type<M>(system: impl IntoSystemSet<M>) -> TypeId {
    system
        .into_system_set()
        .system_type()
        .expect("a function system's own set")
}

pub fn feed_unit_reach() -> TypeId {
    system_type(super::feed_unit_reach)
}

pub fn feed_chain_units() -> TypeId {
    system_type(super::held::feed_chain_units)
}

pub fn feed_targeting_to_vm() -> TypeId {
    system_type(crate::ui_cast::spell_targeting::feed_targeting_to_vm)
}
