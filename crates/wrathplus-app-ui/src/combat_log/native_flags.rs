//! A combat-log unit's `COMBATLOG_OBJECT_*` flags, build 12340's `0x74dcb0`
//! (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-COMBAT-LOG`, `/tmp/azb-app-mig/ghidra/combatlog-unitflags.log`).
use wrathplus_protocol::types::{ObjectType, OwnerFallback};

use crate::app_core::group::GroupState;
use crate::app_core::session::ObjectStore;

pub(crate) const AFFILIATION_MINE: u32 = 0x1;
pub(crate) const AFFILIATION_PARTY: u32 = 0x2;
pub(crate) const AFFILIATION_RAID: u32 = 0x4;
pub(crate) const AFFILIATION_OUTSIDER: u32 = 0x8;
pub(crate) const REACTION_FRIENDLY: u32 = 0x10;
pub(crate) const REACTION_NEUTRAL: u32 = 0x20;
pub(crate) const REACTION_HOSTILE: u32 = 0x40;
pub(crate) const CONTROL_PLAYER: u32 = 0x100;
pub(crate) const CONTROL_NPC: u32 = 0x200;
pub(crate) const TYPE_PLAYER: u32 = 0x400;
pub(crate) const TYPE_NPC: u32 = 0x800;
pub(crate) const TYPE_PET: u32 = 0x1000;
pub(crate) const TYPE_GUARDIAN: u32 = 0x2000;
pub(crate) const TYPE_OBJECT: u32 = 0x4000;
pub(crate) const TARGET: u32 = 0x1_0000;
pub(crate) const FOCUS: u32 = 0x2_0000;
pub(crate) const MAINTANK: u32 = 0x4_0000;
pub(crate) const MAINASSIST: u32 = 0x8_0000;
pub(crate) const RAIDTARGET1: u32 = 0x10_0000;
pub(crate) const NONE: u32 = 0x8000_0000;

/// The guid kinds `0x74dcb0` tests on the high dword.
fn is_player(guid: u64) -> bool {
    guid != 0 && (guid >> 32) as u32 & 0xf000_0000 == 0
}
fn kind(guid: u64) -> u32 {
    (guid >> 32) as u32 & 0xf0f0_0000
}
fn is_pet_guid(guid: u64) -> bool {
    kind(guid) == 0xf040_0000
}
/// A creature, vehicle or pet guid.
fn is_unit_guid(guid: u64) -> bool {
    matches!(kind(guid), 0xf030_0000 | 0xf040_0000 | 0xf050_0000)
}

/// What the flags read besides the guid.
pub(crate) struct FlagInputs<'a> {
    pub(crate) me: u64,
    pub(crate) target: u64,
    pub(crate) focus: u64,
    pub(crate) group: Option<&'a GroupState>,
    pub(crate) store: &'a dyn Fn(u64) -> Option<&'a ObjectStore>,
    /// The unit's reaction toward the local player on the internal 0-7 scale (`0x7251c0`).
    pub(crate) reaction: &'a dyn Fn(&ObjectStore) -> u8,
}

impl FlagInputs<'_> {
    fn in_group(&self) -> bool {
        self.group.is_some_and(|g| g.in_group)
    }
    fn in_raid(&self) -> bool {
        self.group.is_some_and(|g| g.in_group && g.group_type == 1)
    }
    /// The native roster row for `guid`: its subgroup and member flags.
    fn member(&self, guid: u64) -> Option<(u8, u8)> {
        let roster = self.group?.wrath_roster.as_ref()?;
        // The roster lists the others; our own row is its header.
        if guid == self.me {
            return Some((roster.subgroup, roster.flags));
        }
        roster
            .members
            .iter()
            .find(|m| m.guid == guid)
            .map(|m| (m.subgroup, m.flags))
    }
    /// A party member: in our own subgroup (`0x52c680`).
    fn in_party(&self, guid: u64) -> bool {
        let own = self
            .group
            .and_then(|g| g.wrath_roster.as_ref())
            .map(|r| r.subgroup);
        self.member(guid)
            .is_some_and(|(sub, _)| own.is_none_or(|own| own == sub))
    }
}

/// `0x74dcb0` for `guid`.
pub(crate) fn unit_flags(guid: u64, inp: &FlagInputs) -> u32 {
    if guid == 0 {
        return NONE;
    }
    let unit = (inp.store)(guid).filter(|s| {
        matches!(
            s.0.object_type(),
            Some(ObjectType::Unit | ObjectType::Player)
        )
    });
    // The controller: a unit's charmer, else creator; a game object's creator; one hop more when
    // that owner is a held non-player unit with an owner of its own.
    let mut controller = guid;
    if let Some(store) = (inp.store)(guid) {
        match store.0.object_type() {
            Some(ObjectType::Unit | ObjectType::Player) => {
                if let Some(owner) = store.0.unit_owner(OwnerFallback::CreatedBy) {
                    controller = owner;
                }
            }
            Some(ObjectType::GameObject) => {
                if let Some(owner) = store.0.gameobject_created_by().filter(|&g| g != 0) {
                    controller = owner;
                }
            }
            _ => {}
        }
    }
    if controller != guid && !is_player(controller) {
        if let Some(next) = (inp.store)(controller)
            .filter(|s| s.is_unit())
            .and_then(|s| s.0.unit_owner(OwnerFallback::CreatedBy))
        {
            controller = next;
        }
    }

    let mut flags = if is_player(guid) || is_unit_guid(guid) {
        if controller == guid {
            if is_player(guid) {
                TYPE_PLAYER
            } else if is_pet_guid(guid) {
                TYPE_PET
            } else {
                TYPE_NPC
            }
        } else {
            let owned = unit.is_some_and(|u| {
                u.0.unit_charmed_by().is_some_and(|g| g != 0)
                    || u.0.unit_summoned_by().is_some_and(|g| g != 0)
            });
            if owned {
                TYPE_PET
            } else {
                TYPE_GUARDIAN
            }
        }
    } else {
        TYPE_OBJECT
    };
    flags |= if is_player(controller) {
        CONTROL_PLAYER
    } else {
        CONTROL_NPC
    };

    if controller == inp.me {
        flags |= AFFILIATION_MINE | REACTION_FRIENDLY;
    } else {
        if let Some(unit) = unit {
            match (inp.reaction)(unit) {
                r if r > 3 => flags |= REACTION_FRIENDLY,
                r if r > 1 => {}
                _ => flags |= REACTION_HOSTILE,
            }
        }
        if inp.in_group() {
            if inp.in_party(controller) {
                flags |= AFFILIATION_PARTY | REACTION_FRIENDLY;
            } else if inp.in_raid() && inp.member(controller).is_some() {
                flags |= AFFILIATION_RAID | REACTION_FRIENDLY;
            }
        }
        if flags & 0xf == 0 {
            flags |= AFFILIATION_OUTSIDER;
        }
        if flags & 0xf0 == 0 {
            flags |= REACTION_NEUTRAL;
        }
    }
    if guid == inp.target {
        flags |= TARGET;
    }
    if guid == inp.focus {
        flags |= FOCUS;
    }
    // A player's group role: member flag 2 main tank, else 4 main assist (AC `GroupMemberFlags`).
    if flags & TYPE_PLAYER != 0
        && (flags & (AFFILIATION_PARTY | AFFILIATION_RAID) != 0 || guid == inp.me)
    {
        if !inp.in_group() {
            return flags;
        }
        match inp.member(guid).map(|(_, f)| f) {
            Some(f) if f & 2 != 0 => flags |= MAINTANK,
            Some(f) if f & 4 != 0 => flags |= MAINASSIST,
            _ => {}
        }
    }
    if inp.in_group() {
        let icon = inp.group.map_or(0, |g| g.raid_target_index(guid));
        if (1..=8).contains(&icon) {
            flags |= RAIDTARGET1 << (icon - 1);
        }
    }
    flags
}

#[cfg(test)]
#[path = "native_flags_tests.rs"]
mod tests;
