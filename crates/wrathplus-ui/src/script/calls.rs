//! The script calls whose effects touch the selection, the player's cast or the targeting cursor,
//! held in one queue in the order the script made them. In the reference each such call is done
//! when it returns: the macro runner `0x4f14e0` fires every line in one pass, `TargetUnit`
//! (`0x4899d0`), `TargetByName` (`0x489d60`) and `ClearTarget` (`0x489ff0`) commit through
//! `SetSelection 0x493540` before returning, and `CastSpellByName` (`0x4b4ab0`) casts at the
//! selection as it stands then. So `/target Bob` then `/cast Flash Heal` heals Bob, and the app
//! applies this queue front to back at one point in the frame.

use super::{ActionUse, SelectionRequest, UiScript};

/// One queued call, in the reference's terms.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptCall {
    /// `TargetUnit`, `AssistUnit`, `AssistByName` and `TargetLastEnemy`.
    Select(SelectionRequest),
    /// `TargetByName(name, exactMatch)` (`0x489d60`), the stock `/target`.
    TargetByName { name: String, exact: bool },
    /// `TargetNearestEnemy`, `TargetNearestFriend`, `TargetNearestPartyMember` and
    /// `TargetNearestRaidMember` (`0x489a80`/`0x489aa0`/`0x489ac0`/`0x489ae0`): one TAB cycle,
    /// `0x493f60(reverse, mode)`.
    TargetNearest { mode: NearestMode, reverse: bool },
    /// `TargetLastTarget()` (`0x489b00`): re-select the last-target pair, or deselect when it is
    /// empty.
    TargetLastTarget,
    /// `ClearTarget()` (`0x489ff0` → `0x493540(0,0)`), a no-op when nothing is selected by then.
    ClearTarget,
    /// `AttackTarget()` (`0x489b50` → `0x6131a0(0,0)`): the attack validator's pick from the
    /// selection as it stands then, which may move it to the nearest enemy, then the toggle.
    AttackTarget,
    /// `SpellTargetUnit(unit)` (`0x6e6d90`), made while the cursor was up.
    SpellTargetUnit(String),
    /// `SpellStopTargeting()` (`0x6e6e30`), made while the cursor was up.
    SpellStopTargeting,
    /// `SpellStopCasting()` (`0x6e6e80`), made with something to stop.
    SpellStopCasting,
    /// `CastSpell(id, "spell", onSelf)` and `CastSpellByName(name, onSelf)`, through `0x4b3300`'s
    /// player leg; `onSelf` swaps the target guid for the active player's (`0x4b4afa`, `0x4b4345`).
    CastSpell { spell_id: u32, on_self: bool },
    /// `CastSpell(id, "pet", onSelf)`, `0x4b3300`'s `CMSG_PET_ACTION` fork (`0x4b34ce`), with the
    /// same guid swap.
    CastPetSpell { spell_id: u32, on_self: bool },
    /// `CastShapeshiftForm(index)` (`0x4b4810`), as the form's spell.
    CastShapeshiftForm(u32),
    /// `UseAction(slot, …)` (`0x4e7140` → `0x4e5ee0`); a macro slot runs its lines inside it.
    UseAction(ActionUse),
    /// `UseContainerItem(bag, slot)` outside repair mode.
    UseContainerItem { bag: i64, slot: u32 },
    /// `UseInventoryItem(id)` outside repair mode.
    UseInventoryItem(u32),
    /// `CastPetAction(slot)`, a 1-based pet bar slot.
    PetAction(u32),
    /// `PetAttack` and the other one-shot orders, as their bar slot's packed word.
    PetOrder(u32),
    /// Build 12340 `StartAttack([unit])` (`0x523090`): the named unit, else the target; no toggle.
    StartAttack { unit: Option<String> },
    /// Build 12340 `StopAttack()` (`0x51d0b0`).
    StopAttack,
    /// Build 12340 `TargetLastFriend()` (`0x525e50`).
    TargetLastFriend,
    /// Build 12340 `CancelShapeshiftForm()` (`0x53de60` → `0x726ce0`, the cancel-aura send).
    CancelShapeshiftForm,
    /// Build 12340 `Dismount()` (`0x51d170` → `0x7412e0`, CMSG `0x375`).
    Dismount,
    /// Build 12340 `DismissCompanion("CRITTER")` (`0x53e6b0` → `0x802f80`): CMSG `0x48D`
    /// (`CMSG_DISMISS_CRITTER`) with the player's `UNIT_FIELD_CRITTER` guid.
    DismissCritter { critter: u64 },
    /// Build 12340 `CastSpellByID(id, unit)` (`0x53e060`) at a unit token other than the
    /// selection or the player.
    CastSpellAt { spell_id: u32, unit: String },
    /// Build 12340 `RunMacro(index or name[, button])` (`0x566e80`): the macro's lines, as a macro
    /// slot, with `button` for `GetRunningMacroButton` (`0x564db0` keeps it in `[0xbeaf48]`).
    RunMacro { index: u32, button: Option<String> },
    /// Build 12340 `RunMacroText(text[, button])` (`0x566400`): the text's lines, as a macro body.
    RunMacroText {
        text: String,
        button: Option<String>,
    },
    /// Build 12340 `DestroyTotem(slot)` (`0x51d520` → `0x5197d0`): `CMSG_TOTEM_DESTROYED`, 0-based.
    DestroyTotem { slot: u8 },
    /// Build 12340 `SetActiveTalentGroup(group)` (`0x5c5e70`): cast the 1-based group's
    /// activation spell (63645/63644, table `0xad05e8`).
    ActivateTalentGroup { group: u8 },
    /// Build 12340 `InteractUnit(unit)` (`0x527f00` → `0x5277b0`): the object's own interact, as a
    /// right-click on it runs it.
    InteractUnit { guid: u64 },
    /// Build 12340 `TargetTotem(slot)` (`0x5262e0`): the slot's totem guid straight into
    /// `SetSelection` (`0x524bf0`).
    TargetGuid(u64),
}

/// The mode the four `TargetNearest*` shims hand the TAB cycler `0x493f60`, the only byte that
/// differs between them; it reaches only the per-candidate filter `0x493e40`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NearestMode {
    /// Mode 1, `TargetNearestEnemy`.
    Enemy,
    /// Mode 2, `TargetNearestFriend`.
    Friend,
    /// Mode 3, `TargetNearestPartyMember`: a party member other than us.
    PartyMember,
    /// Mode 4, `TargetNearestRaidMember`: a party or raid member other than us.
    RaidMember,
    /// Build 12340 `TargetNearestEnemyPlayer` (`0x525b10`, mode 2 of `0x524fc0`).
    EnemyPlayer,
    /// Build 12340 `TargetNearestFriendPlayer` (`0x525b90`, mode 4 of `0x524fc0`).
    FriendPlayer,
    /// Build 12340 `TargetNearest` (`0x525a90`, mode 0 of `0x524fc0`): any unit, as the candidate
    /// filter `0x518e40` tests nothing for mode 0.
    Any,
}

impl UiScript {
    /// Take every queued call, in call order.
    pub fn take_script_calls(&mut self) -> Vec<ScriptCall> {
        std::mem::take(&mut self.model_mut().script_calls)
    }

    /// Put calls back at the front of the queue, ahead of any queued since.
    pub fn requeue_script_calls(&mut self, calls: Vec<ScriptCall>) {
        let mut model = self.model_mut();
        let later = std::mem::replace(&mut model.script_calls, calls);
        model.script_calls.extend(later);
    }

    /// Take the queued calls `pick` answers for, in call order, leaving the rest queued.
    pub(crate) fn take_calls_where<T>(
        &mut self,
        mut pick: impl FnMut(&ScriptCall) -> Option<T>,
    ) -> Vec<T> {
        let mut model = self.model_mut();
        let mut taken = Vec::new();
        model.script_calls.retain(|call| match pick(call) {
            Some(t) => {
                taken.push(t);
                false
            }
            None => true,
        });
        taken
    }
}
