//! The refusals 12340 runs before a teleport answer leaves: `AcceptBattlefieldPort`
//! (`0x54daf0`-`0x54dbd9`) and `LFGTeleport` (`0x556c2b`-`0x556cef`) test, in order, combat, death,
//! a fall, a lost client control and fatigue, each refusal a catalog error and no packet.

use super::Model;

/// The player's state the refusals read, gathered by [`PortGates::of`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PortGates {
    /// `UNIT_FIELD_FLAGS & 0x800`, which AzerothCore sets on an owner whose pet is in combat.
    pub pet_in_combat: bool,
    /// An auto-attack target (`[obj+0xa20]`), read by `0x71afb0` alone. [`Self::of`] leaves it
    /// false; `LFGTeleport`, the `FlagOrAttacking` caller, sets it from Attack (6603) being current.
    /// Inferred: ours turns on when the server starts the attack, 12340's when the swing is sent.
    pub attacking: bool,
    /// Vtable `+0x128` [inferred: the unit is dead]; read as health at 0 (`UnitIsDead`).
    pub dead: bool,
    /// Movement flag `0x1000`. Labelled: read as `IsFalling`, which also requires no root.
    pub falling: bool,
    /// `0x54a430` (vtable `+0x40`). Not modelled: false.
    pub locked_out: bool,
    /// The fatigue mirror timer (`0x513f20(0)`) running down: scale (`+0xc`) below 0.
    pub fatigued: bool,
}

/// Which combat test the caller runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CombatTest {
    /// `AcceptBattlefieldPort` reads the flag inline (`0x54dafc`).
    Flag,
    /// `LFGTeleport` calls `0x71afb0`: the flag, or an auto-attack target.
    FlagOrAttacking,
}

impl PortGates {
    pub(crate) fn of(model: &Model) -> Self {
        let player = model.unit("player");
        PortGates {
            pet_in_combat: player.is_some_and(|u| u.flags & 0x800 != 0),
            attacking: false,
            dead: player.is_some_and(|u| u.dead),
            falling: model.wrath_popup.falling(),
            locked_out: false,
            fatigued: model.wrath_state.fatigue_draining(),
        }
    }

    /// The first refusal's catalog key, in the reference's order; `None` lets the answer go.
    pub(crate) fn refusal(&self, combat: CombatTest) -> Option<&'static str> {
        let attacking = combat == CombatTest::FlagOrAttacking && self.attacking;
        [
            (self.pet_in_combat || attacking, "ERR_NOT_IN_COMBAT"),
            (self.dead, "ERR_PLAYER_DEAD"),
            (self.falling, "ERR_NOT_WHILE_FALLING"),
            (self.locked_out, "ERR_CLIENT_LOCKED_OUT"),
            (self.fatigued, "ERR_NOT_WHILE_FATIGUED"),
        ]
        .into_iter()
        .find_map(|(refused, key)| refused.then_some(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_come_in_the_references_order() {
        let all = PortGates {
            pet_in_combat: true,
            attacking: true,
            dead: true,
            falling: true,
            locked_out: true,
            fatigued: true,
        };
        assert_eq!(all.refusal(CombatTest::Flag), Some("ERR_NOT_IN_COMBAT"));
        let attacking = PortGates {
            attacking: true,
            fatigued: true,
            ..Default::default()
        };
        assert_eq!(
            attacking.refusal(CombatTest::FlagOrAttacking),
            Some("ERR_NOT_IN_COMBAT")
        );
        assert_eq!(
            attacking.refusal(CombatTest::Flag),
            Some("ERR_NOT_WHILE_FATIGUED")
        );
        let falling = PortGates {
            falling: true,
            locked_out: true,
            ..Default::default()
        };
        assert_eq!(
            falling.refusal(CombatTest::Flag),
            Some("ERR_NOT_WHILE_FALLING")
        );
        assert_eq!(PortGates::default().refusal(CombatTest::Flag), None);
    }
}
