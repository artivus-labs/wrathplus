//! Build 12340's slash-command verbs App applies (`/tmp/azb-app-mig/ghidra/chatverbs.log`).
//!
//! - The arena-team sends ([`WrathChatVerbRequest`]): `ArenaTeamInviteByName` (`0x515cc0`),
//!   `ArenaTeamLeave` (`0x515dd0`) and the uninvite and leader bindings send their team slot's id,
//!   0 for a slot out of 1-3, which still goes out. A name must be non-empty and under 49
//!   characters; a longer one is the binding's "Name too long" error, which this drain logs
//!   (labelled). Labelled: the team id is read off the descriptor's `ARENA_TEAM_INFO` slot, where
//!   native reads its own arena-team table (`0xc0f840`).
//! - `SetCurrentTitle` (CMSG `0x374`) and `SetTaxiBenchmarkMode` (CMSG `0x389`) send as they are.
//! - `CancelItemTempEnchantment` sends `CMSG_CANCEL_TEMP_ENCHANTMENT` (`0x379`) for equipment slot
//!   15 or 16; `Sound_GameSystem_RestartSoundSystem` is a labelled no-op.
//! - The cast-order verbs, applied in call order by [`crate::script_calls`]: [`cancel_form`],
//!   [`dismount`] and [`activate_talent_group`].

use bevy::prelude::*;

use wrathplus_protocol::wrath_arena_team::Command as ArenaTeam;
use wrathplus_ui::script::{UiScript, WrathChatVerbRequest};

use crate::app_core::session::ObjectStore;
use crate::net::{ClientCommand, NetCommands};

/// `0x76ee30`'s character count must stay under `0x31`.
const MAX_NAME_CHARS: usize = 48;

/// One arena verb's command, or `None` for a name the binding refuses.
pub(crate) fn arena_command(
    request: &WrathChatVerbRequest,
    team_id: impl Fn(u8) -> u32,
) -> Option<ArenaTeam> {
    let named = |name: &str| {
        let chars = name.chars().count();
        (chars > 0 && chars <= MAX_NAME_CHARS).then(|| name.as_bytes().to_vec())
    };
    Some(match request {
        WrathChatVerbRequest::ArenaTeamInvite { team, name } => ArenaTeam::Invite {
            team_id: team_id(*team),
            name: named(name)?,
        },
        WrathChatVerbRequest::ArenaTeamLeave { team } => ArenaTeam::Leave {
            team_id: team_id(*team),
        },
        WrathChatVerbRequest::ArenaTeamUninvite { team, name } => ArenaTeam::Remove {
            team_id: team_id(*team),
            name: named(name)?,
        },
        WrathChatVerbRequest::ArenaTeamSetLeader { team, name } => ArenaTeam::Leader {
            team_id: team_id(*team),
            name: named(name)?,
        },
        _ => return None,
    })
}

/// `CancelShapeshiftForm()` (`0x53de60` → `0x726ce0`): with a form up, each helpful aura whose
/// spell applies `MOD_SHAPESHIFT` (36) into the current form is cancelled (`CMSG_CANCEL_AURA`).
pub(crate) fn cancel_form(
    cast: &mut crate::spell::ScriptCast,
    auras: &crate::net::wrath_auras::PacketAuras,
    me: u64,
) {
    let Some(store) = cast.targeting.self_store.iter().next() else {
        return;
    };
    let form = u32::from(store.0.unit_shapeshift_form());
    let Some(spells) = cast.ladder.spells.as_ref() else {
        return;
    };
    if form == 0 {
        return;
    }
    for held in auras.by_guid.get(&me).into_iter().flatten() {
        let spell_id = held.aura.spell_id;
        let in_form = spells
            .catalog
            .get(spell_id)
            .is_some_and(|d| d.shapeshift_form == Some(form));
        if held.aura.is_helpful() && in_form {
            debug!("CancelShapeshiftForm: cancel aura {spell_id}");
            let _ = cast
                .ladder
                .commands
                .0
                .send(ClientCommand::CancelAura { spell_id });
        }
    }
}

/// `UNIT_FLAG_TAXI_FLIGHT`: `0x7412e0` refuses on a flight path.
const UNIT_FLAG_TAXI_FLIGHT: u32 = 0x10_0000;

/// `Dismount()` (`0x51d170` → `0x7412e0`): off a flight path, `CMSG_CANCEL_MOUNT_AURA` (`0x375`).
/// Labelled: the local flying-mount fall handling after the send is not modelled.
pub(crate) fn dismount(cast: &mut crate::spell::ScriptCast) {
    let on_taxi = cast
        .targeting
        .self_store
        .iter()
        .next()
        .is_some_and(|s| s.0.unit_flags() & UNIT_FLAG_TAXI_FLIGHT != 0);
    if on_taxi {
        return;
    }
    let _ = cast
        .ladder
        .commands
        .0
        .send(ClientCommand::NativeSpellControl(
            wrathplus_protocol::wrath_spell_control::Command::CancelMountAura,
        ));
}

/// The activation spells `SetActiveTalentGroup` casts by 1-based group (table `0xad05e8`).
const TALENT_GROUP_SPELLS: [u32; 2] = [63645, 63644];

/// `SetActiveTalentGroup(group)` (`0x5c5e70`): another group the player has, of the first two,
/// casts its activation spell on the player; the active group or one past the count does nothing.
pub(crate) fn activate_talent_group(
    cast: &mut crate::spell::ScriptCast,
    progress: &crate::net::wrath_notices::WrathProgress,
    group: u8,
) {
    let Some((_, active, groups)) = progress.talents.get(&false) else {
        return;
    };
    let Some(index) = group.checked_sub(1) else {
        return;
    };
    if index != *active && usize::from(index) < groups.len() && usize::from(index) < 2 {
        crate::ui_spellbook::cast_spell(cast, TALENT_GROUP_SPELLS[usize::from(index)], true);
    }
}

/// Drain the frame's sends. Folded into [`crate::ui_talent`]'s learn drain, which reads the same
/// VM, self store and commands, so it adds no schedule edge of its own.
pub(crate) fn drain(script: &mut UiScript, store: Option<&ObjectStore>, commands: &NetCommands) {
    let requests = script.take_wrath_chat_verb_requests();
    if requests.is_empty() {
        return;
    }
    // The 1-based slot's id; 0 out of range, as native sends it.
    let team_id = |team: u8| {
        team.checked_sub(1)
            .filter(|&slot| slot < 3)
            .and_then(|slot| store.and_then(|s| s.0.player_arena_team_id(slot)))
            .unwrap_or(0)
    };
    for request in requests {
        let command = match request {
            WrathChatVerbRequest::SetTitle { title } => ClientCommand::NativeSessionMeta(
                wrathplus_protocol::wrath_session_meta::Command::SetTitle { title },
            ),
            WrathChatVerbRequest::TaxiBenchmarkMode { on } => ClientCommand::NativeMovementMisc(
                wrathplus_protocol::wrath_movement_misc::Command::TaxiBenchmark { on },
            ),
            // `CancelItemTempEnchantment` (`0x8021d0`) hands `0x6d7120` the equipment slot, the
            // argument plus 14 (15 main hand, 16 off hand). Labelled: `0x6d7120`'s body is not
            // decompiled; it is taken as the `CMSG_CANCEL_TEMP_ENCHANTMENT` send of that slot.
            WrathChatVerbRequest::CancelTempEnchantment { slot } => ClientCommand::NativeInventory(
                wrathplus_protocol::wrath_inventory::Command::CancelTempEnchantment {
                    slot: u32::from(slot) + 14,
                },
            ),
            // `Sound_GameSystem_RestartSoundSystem` (`0x985d30`) re-opens the sound device. Labelled
            // no-op: WrathPlus's audio output follows the system default device and has no restart.
            WrathChatVerbRequest::RestartSoundSystem => {
                debug!("chat verb RestartSoundSystem: no audio restart (labelled no-op)");
                continue;
            }
            ref arena => match arena_command(arena, team_id) {
                Some(command) => ClientCommand::ArenaTeamCommand(command),
                None => {
                    debug!("chat verb {request:?}: not sent (a refused name)");
                    continue;
                }
            },
        };
        let _ = commands.0.send(command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_verbs_send_their_slots_team_id_and_refuse_bad_names() {
        let id = |team: u8| u32::from(team) * 100;
        let name = |s: &str| s.to_string();
        assert_eq!(
            arena_command(
                &WrathChatVerbRequest::ArenaTeamInvite {
                    team: 2,
                    name: name("Bob")
                },
                id
            ),
            Some(ArenaTeam::Invite {
                team_id: 200,
                name: b"Bob".to_vec()
            })
        );
        assert_eq!(
            arena_command(&WrathChatVerbRequest::ArenaTeamLeave { team: 0 }, |_| 0),
            Some(ArenaTeam::Leave { team_id: 0 })
        );
        assert_eq!(
            arena_command(
                &WrathChatVerbRequest::ArenaTeamSetLeader {
                    team: 1,
                    name: name("Ann")
                },
                id
            ),
            Some(ArenaTeam::Leader {
                team_id: 100,
                name: b"Ann".to_vec()
            })
        );
        for bad in ["", &"x".repeat(49)] {
            assert_eq!(
                arena_command(
                    &WrathChatVerbRequest::ArenaTeamUninvite {
                        team: 1,
                        name: name(bad)
                    },
                    id
                ),
                None
            );
        }
        assert_eq!(
            arena_command(
                &WrathChatVerbRequest::ArenaTeamUninvite {
                    team: 1,
                    name: "é".repeat(48)
                },
                id
            ),
            Some(ArenaTeam::Remove {
                team_id: 100,
                name: "é".repeat(48).into_bytes()
            })
        );
    }
}
