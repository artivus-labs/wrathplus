//! The TAB cycler's scan mode, split out of `scan.rs`.

/// The reference's scan mode: the `TargetNearest*` shims (`0x489a80` enemy 1, `0x489aa0` friend 2,
/// `0x489ac0`/`0x489ae0` party and raid 3/4) all call one cycler, `0x493f60(reverse, mode)`, and
/// the mode reaches only the per-candidate filter `0x493e40`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanSide {
    /// Mode 1 (`0x493e73`): alive by the reads-dead triple `0x605f90`, and `CanAttack 0x606980`.
    Enemy,
    /// Mode 2 (`0x493eca`): `CanAssist 0x6066f0`, friendly or better (`0x60671e`), whose
    /// `IsPvP 0x605ff0` leg refuses a creature without `UNIT_FLAG_PVP`; then health > 0 with no
    /// dynflag leg, so a feigning ally counts.
    Friend,
    /// Mode 3 (`0x493eed`): not us, then `0x4e7f70`, a member of our party slots. No health, dead
    /// or reaction leg.
    Party,
    /// Mode 4 (`0x493f15`): not us, then `0x4918e0`, party (`0x4e7f70`) or raid roster
    /// (`0x4baee0`).
    Raid,
    /// Build 12340's `TargetNearestEnemyPlayer` (`0x525b10` → `0x524fc0`): mode 1's test on a
    /// player. Labelled: the player cycler `0x524fc0` is read as the TAB cycler with a player
    /// filter, not traced.
    EnemyPlayer,
    /// `TargetNearestFriendPlayer` (`0x525b90`): mode 2's test on a player, likewise.
    FriendPlayer,
    /// Build 12340's `TargetNearest` (`0x525a90`): mode 0 of `0x524fc0`, whose candidate filter
    /// `0x518e40` has no case for it and passes every unit.
    Any,
}

impl ScanSide {
    /// The side a `TargetNearest*` call's mode names.
    pub(crate) fn of(mode: wrathplus_ui::script::NearestMode) -> Self {
        use wrathplus_ui::script::NearestMode;
        match mode {
            NearestMode::Enemy => Self::Enemy,
            NearestMode::Friend => Self::Friend,
            NearestMode::EnemyPlayer => Self::EnemyPlayer,
            NearestMode::FriendPlayer => Self::FriendPlayer,
            NearestMode::PartyMember => Self::Party,
            NearestMode::RaidMember => Self::Raid,
            NearestMode::Any => Self::Any,
        }
    }
}
