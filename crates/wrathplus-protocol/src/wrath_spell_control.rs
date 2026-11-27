//! Build-12340 spell control, pet and visual feedback, framed strictly from what AzerothCore
//! writes and reads: `Player::_StartCooldown`/`AddSpellMod` (Player.cpp), `Pet::RemoveSpellCooldown`
//! (Pet.cpp), `Spell::SendChannelStart`/`SendChannelUpdate`/`SendResurrectRequest` (Spell.cpp),
//! `Unit::SendTameFailure`, `SendPlaySpellVisual`, `SendPlaySpellImpact`, `SendPetActionFeedback`,
//! `SendPetActionSound`, `SendPetDismissSound`, `SendComboPoints` (Unit.cpp),
//! `WorldSession::SendPetNameInvalid` (PetHandler.cpp), `TotemCreated` (TotemPackets.cpp), and the
//! matching `WorldSession::Handle*` readers for the requests.
//!
//! Replies that mean what their 1.12 counterparts mean become the shared events their owners
//! read, the channel events now for every caster. A spell impact and a created totem have no shared
//! event and are capture-only ([`Packet`]). Registry-only opcodes in this family stay undecoded:
//! [`REGISTRY_ONLY`].

use std::io::{self, Cursor};

use crate::events::SessionEvent;
use crate::wire::{read_cstring, read_packed_guid};
use crate::wrath_session::{f32_le, u32_le, u8_le};

pub const SMSG_COOLDOWN_EVENT: u16 = 0x0135;
pub const MSG_CHANNEL_START: u16 = 0x0139;
pub const MSG_CHANNEL_UPDATE: u16 = 0x013A;
pub const SMSG_RESURRECT_REQUEST: u16 = 0x015B;
pub const SMSG_PET_TAME_FAILURE: u16 = 0x0173;
pub const SMSG_PET_NAME_INVALID: u16 = 0x0178;
pub const SMSG_CLEAR_COOLDOWN: u16 = 0x01DE;
pub const SMSG_PLAY_SPELL_VISUAL: u16 = 0x01F3;
pub const SMSG_PLAY_SPELL_IMPACT: u16 = 0x01F7;
pub const SMSG_SET_FLAT_SPELL_MODIFIER: u16 = 0x0266;
pub const SMSG_SET_PCT_SPELL_MODIFIER: u16 = 0x0267;
pub const SMSG_PET_ACTION_FEEDBACK: u16 = 0x02C6;
pub const SMSG_PET_ACTION_SOUND: u16 = 0x0324;
pub const SMSG_PET_DISMISS_SOUND: u16 = 0x0325;
pub const SMSG_UPDATE_COMBO_POINTS: u16 = 0x039D;
pub const SMSG_TOTEM_CREATED: u16 = 0x0413;
/// Spell and pet remainders (AzerothCore Pet.cpp, Player.cpp, Unit.cpp, SpellEffects.cpp,
/// SpellAuraEffects.cpp, SpellHandler.cpp).
pub const SMSG_BREAK_TARGET: u16 = 0x0152;
pub const SMSG_FEIGN_DEATH_RESISTED: u16 = 0x02b4;
pub const SMSG_CLEAR_TARGET: u16 = 0x03bf;
pub const SMSG_SEND_UNLEARN_SPELLS: u16 = 0x041e;
pub const SMSG_CONVERT_RUNE: u16 = 0x0486;
pub const SMSG_RESYNC_RUNES: u16 = 0x0487;
pub const SMSG_ADD_RUNE_POWER: u16 = 0x0488;
pub const SMSG_MODIFY_COOLDOWN: u16 = 0x0491;
pub const SMSG_PET_UPDATE_COMBO_POINTS: u16 = 0x0492;
pub const SMSG_PET_LEARNED_SPELL: u16 = 0x0499;
pub const SMSG_PET_UNLEARNED_SPELL: u16 = 0x049a;
pub const SMSG_SET_PROJECTILE_POSITION: u16 = 0x04bf;

/// `SMSG_RESUME_CAST_BAR`, `SMSG_PET_MODE`, `SMSG_RESURRECT_FAILED`, `SMSG_PET_BROKEN`,
/// `SMSG_PET_UNLEARN_CONFIRM`, `SMSG_SPELL_UPDATE_CHAIN_TARGETS`, the four obsolete extra-aura-info
/// opcodes, `SMSG_FORCE_DISPLAY_UPDATE`, `SMSG_PLAY_DANCE` and `SMSG_PET_RENAMEABLE`: in
/// AzerothCore's opcode table only, with no send site, so there is no layout to frame.
pub const REGISTRY_ONLY: [u16; 13] = [
    0x014D, 0x017A, 0x0252, 0x02AF, 0x02F1, 0x0330, 0x03A3, 0x03A4, 0x03A5, 0x03A6, 0x0403, 0x044C,
    0x0475,
];

pub const CMSG_CANCEL_CHANNELLING: u32 = 0x013B;
pub const CMSG_RESURRECT_RESPONSE: u32 = 0x015C;
pub const CMSG_PET_SET_ACTION: u32 = 0x0174;
pub const CMSG_PET_ACTION: u32 = 0x0175;
pub const CMSG_PET_ABANDON: u32 = 0x0176;
pub const CMSG_PET_RENAME: u32 = 0x0177;
pub const CMSG_PET_CANCEL_AURA: u32 = 0x026B;
pub const CMSG_CANCEL_AUTO_REPEAT_SPELL: u32 = 0x026D;
pub const CMSG_REQUEST_PET_INFO: u32 = 0x0279;
pub const CMSG_CANCEL_GROWTH_AURA: u32 = 0x029B;
pub const CMSG_PET_STOP_ATTACK: u32 = 0x02EA;
pub const CMSG_PET_SPELL_AUTOCAST: u32 = 0x02F3;
pub const CMSG_CANCEL_MOUNT_AURA: u32 = 0x0375;
pub const CMSG_SPELLCLICK: u32 = 0x03F8;
pub const CMSG_TOTEM_DESTROYED: u32 = 0x0414;
pub const CMSG_PET_LEARN_TALENT: u32 = 0x047A;
pub const CMSG_STABLE_REVIVE_PET: u32 = 0x0274;
pub const CMSG_FAR_SIGHT: u32 = 0x027A;
pub const CMSG_DISMISS_CRITTER: u32 = 0x048D;
pub const CMSG_LEARN_PREVIEW_TALENTS_PET: u32 = 0x04C2;

/// Capture-only receipts: no shared event or presentation owns them yet.
#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    /// `SMSG_PLAY_SPELL_IMPACT`: a `SpellVisualKit` id played as an impact on `target`.
    SpellImpact {
        target: u64,
        kit_id: u32,
    },
    /// `SMSG_TOTEM_CREATED`: totem `slot` holds `totem` for `duration_ms`, from `spell_id`.
    TotemCreated {
        slot: u8,
        totem: u64,
        duration_ms: u32,
        spell_id: u32,
    },
    /// `SMSG_PET_LEARNED_SPELL` / `SMSG_PET_UNLEARNED_SPELL`: `u32 spell`; the pet book's whole list is
    /// `SMSG_PET_SPELLS`.
    PetLearnedSpell {
        spell_id: u32,
    },
    PetUnlearnedSpell {
        spell_id: u32,
    },
    /// `SMSG_PET_UPDATE_COMBO_POINTS` (`Unit::SendComboPoints` to the charmer or owner of a pet or
    /// vehicle that builds them): `packed unit, packed target, u8 points`.
    PetComboPoints {
        unit: u64,
        target: u64,
        points: u8,
    },
    /// `SMSG_MODIFY_COOLDOWN` (`Player::ModifySpellCooldown`): `u32 spell, u64 player, i32 delta ms`.
    ModifyCooldown {
        spell_id: u32,
        unit: u64,
        delta_ms: i32,
    },
    /// `SMSG_FEIGN_DEATH_RESISTED`: `u64 unit, u8` (AzerothCore writes 0).
    FeignDeathResisted {
        unit: u64,
        flag: u8,
    },
    /// `SMSG_BREAK_TARGET` (`Unit::SendClearTarget`-like, packed guid): stop targeting this unit.
    BreakTarget {
        unit: u64,
    },
    /// `SMSG_CLEAR_TARGET`: `u64 caster`.
    ClearTarget {
        caster: u64,
    },
    /// `SMSG_SET_PROJECTILE_POSITION`: `u64 caster, u8 cast count, f32 x, y, z`.
    ProjectilePosition {
        caster: u64,
        cast_count: u8,
        position: [f32; 3],
    },
    /// `SMSG_SEND_UNLEARN_SPELLS` (`Player::SendUnlearnSpells`): `u32 count`, then spell ids.
    UnlearnSpells {
        spell_ids: Vec<u32>,
    },
}

/// Death-knight runes (AzerothCore `Player::ConvertRune`, `ResyncRunes`, `AddRunePower`), kept as
/// retained native state by the app: there are six runes, `type` 0 blood, 1 unholy, 2 frost, 3 death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunePacket {
    /// `u8 index, u8 type`.
    Convert { index: u8, rune_type: u8 },
    /// `u32 count`, then `(u8 type, u8 passed)` per rune; `passed` 255 is ready (AzerothCore writes
    /// `255 - cooldown * 51`).
    Resync { runes: Vec<(u8, u8)> },
    /// `u32 mask`: these runes are ready now.
    AddPower { mask: u32 },
    /// Our own `SMSG_SPELL_GO`'s rune list (`CAST_FLAG_RUNE_LIST`, AzerothCore `Spell::SendSpellGo`):
    /// the runes ready before and after the cast, and for each rune that went down (set in
    /// `before`, clear in `after`, ascending) how far its new cooldown has run, 255 ready.
    Spend {
        before: u8,
        after: u8,
        passed: Vec<u8>,
    },
}

/// Requests, each with the body AC's handler reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    CancelChannelling {
        spell_id: u32,
    },
    /// AC treats status 0 as a decline, anything else as an accept.
    ResurrectResponse {
        caster: u64,
        accept: bool,
    },
    /// One `(position, word)` pair removes or sets; two move (AC counts them by body size).
    PetSetAction {
        pet: u64,
        entries: Vec<(u32, u32)>,
    },
    PetAction {
        pet: u64,
        word: u32,
        target: u64,
    },
    PetAbandon {
        pet: u64,
    },
    /// No declined names: the `isdeclined` byte is 0.
    PetRename {
        pet: u64,
        name: String,
    },
    PetCancelAura {
        pet: u64,
        spell_id: u32,
    },
    CancelAutoRepeatSpell,
    RequestPetInfo,
    CancelGrowthAura,
    PetStopAttack {
        pet: u64,
    },
    PetSpellAutocast {
        pet: u64,
        spell_id: u32,
        enabled: bool,
    },
    CancelMountAura,
    SpellClick {
        unit: u64,
    },
    TotemDestroyed {
        slot: u8,
    },
    PetLearnTalent {
        pet: u64,
        talent_id: u32,
        rank: u32,
    },
    /// `HandleFarSightOpcode`: `u8 apply`.
    FarSight {
        engage: bool,
    },
    /// `HandleDismissCritter`: the critter's raw guid.
    DismissCritter {
        critter: u64,
    },
    /// `HandleStableRevivePet`: `u64 stable master, u32 pet number`.
    StableRevivePet {
        npc: u64,
        pet_number: u32,
    },
    /// `HandleLearnPreviewTalentsPet`: `u64 pet, u32 count`, then `(u32 talent, u32 rank)` pairs.
    LearnPreviewTalentsPet {
        pet: u64,
        talents: Vec<(u32, u32)>,
    },
}

pub const fn handles(opcode: u16) -> bool {
    matches!(
        opcode,
        SMSG_COOLDOWN_EVENT
            | MSG_CHANNEL_START
            | MSG_CHANNEL_UPDATE
            | SMSG_RESURRECT_REQUEST
            | SMSG_PET_TAME_FAILURE
            | SMSG_PET_NAME_INVALID
            | SMSG_CLEAR_COOLDOWN
            | SMSG_PLAY_SPELL_VISUAL
            | SMSG_PLAY_SPELL_IMPACT
            | SMSG_SET_FLAT_SPELL_MODIFIER
            | SMSG_SET_PCT_SPELL_MODIFIER
            | SMSG_PET_ACTION_FEEDBACK
            | SMSG_PET_ACTION_SOUND
            | SMSG_PET_DISMISS_SOUND
            | SMSG_UPDATE_COMBO_POINTS
            | SMSG_TOTEM_CREATED
            | SMSG_BREAK_TARGET
            | SMSG_FEIGN_DEATH_RESISTED
            | SMSG_CLEAR_TARGET
            | SMSG_SEND_UNLEARN_SPELLS
            | SMSG_CONVERT_RUNE
            | SMSG_RESYNC_RUNES
            | SMSG_ADD_RUNE_POWER
            | SMSG_MODIFY_COOLDOWN
            | SMSG_PET_UPDATE_COMBO_POINTS
            | SMSG_PET_LEARNED_SPELL
            | SMSG_PET_UNLEARNED_SPELL
            | SMSG_SET_PROJECTILE_POSITION
    )
}

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

fn u64_le(r: &mut Cursor<&[u8]>) -> io::Result<u64> {
    Ok(u64::from(u32_le(r)?) | u64::from(u32_le(r)?) << 32)
}

/// Decode one of the [`handles`] opcodes.
pub fn decode(opcode: u16, body: &[u8]) -> io::Result<Vec<SessionEvent>> {
    let mut r = Cursor::new(body);
    let event = match opcode {
        SMSG_COOLDOWN_EVENT => SessionEvent::CooldownEvent {
            spell_id: u32_le(&mut r)?,
            caster: u64_le(&mut r)?,
        },
        SMSG_CLEAR_COOLDOWN => SessionEvent::ClearCooldown {
            spell_id: u32_le(&mut r)?,
            caster: u64_le(&mut r)?,
        },
        // Build 12340 names the caster and sends to the whole set: every caster's channel reaches
        // the shared event, and its owner decides whose bar it drives.
        MSG_CHANNEL_START => SessionEvent::ChannelStart {
            caster: Some(read_packed_guid(&mut r)?),
            spell_id: u32_le(&mut r)?,
            duration_ms: u32_le(&mut r)?,
        },
        MSG_CHANNEL_UPDATE => SessionEvent::ChannelUpdate {
            caster: Some(read_packed_guid(&mut r)?),
            remaining_ms: u32_le(&mut r)?,
        },
        SMSG_RESURRECT_REQUEST => resurrect_request(&mut r)?,
        SMSG_PET_TAME_FAILURE => SessionEvent::PetTameFailure {
            reason: u8_le(&mut r)?,
        },
        SMSG_PET_NAME_INVALID => pet_name_invalid(&mut r)?,
        SMSG_PLAY_SPELL_VISUAL => SessionEvent::PlaySpellVisual {
            unit: u64_le(&mut r)?,
            kit_id: u32_le(&mut r)?,
        },
        SMSG_PLAY_SPELL_IMPACT => SessionEvent::WrathSpellControl {
            packet: Packet::SpellImpact {
                target: u64_le(&mut r)?,
                kit_id: u32_le(&mut r)?,
            },
        },
        // `u8 eff` (a `SpellFamilyFlags` bit, 0..95 in build 12340), `u8 op`, `i32` absolute total.
        SMSG_SET_FLAT_SPELL_MODIFIER | SMSG_SET_PCT_SPELL_MODIFIER => SessionEvent::SpellModifier {
            flat: opcode == SMSG_SET_FLAT_SPELL_MODIFIER,
            mask_bit: u8_le(&mut r)?,
            op: u8_le(&mut r)?,
            value: u32_le(&mut r)? as i32,
        },
        SMSG_PET_ACTION_FEEDBACK => SessionEvent::PetActionFeedback {
            reason: u8_le(&mut r)?,
        },
        SMSG_PET_ACTION_SOUND => SessionEvent::PetActionSound {
            pet_guid: u64_le(&mut r)?,
            talk: u32_le(&mut r)?,
        },
        SMSG_PET_DISMISS_SOUND => SessionEvent::PetDismissSound {
            model_id: u32_le(&mut r)?,
            position: [f32_le(&mut r)?, f32_le(&mut r)?, f32_le(&mut r)?],
        },
        // An empty packed guid (mask 0) is "no combo target".
        SMSG_UPDATE_COMBO_POINTS => SessionEvent::ComboPoints {
            target: read_packed_guid(&mut r)?,
            points: u8_le(&mut r)?,
        },
        SMSG_TOTEM_CREATED => SessionEvent::WrathSpellControl {
            packet: Packet::TotemCreated {
                slot: u8_le(&mut r)?,
                totem: u64_le(&mut r)?,
                duration_ms: u32_le(&mut r)?,
                spell_id: u32_le(&mut r)?,
            },
        },
        SMSG_PET_LEARNED_SPELL => SessionEvent::WrathSpellControl {
            packet: Packet::PetLearnedSpell {
                spell_id: u32_le(&mut r)?,
            },
        },
        SMSG_PET_UNLEARNED_SPELL => SessionEvent::WrathSpellControl {
            packet: Packet::PetUnlearnedSpell {
                spell_id: u32_le(&mut r)?,
            },
        },
        SMSG_PET_UPDATE_COMBO_POINTS => SessionEvent::WrathSpellControl {
            packet: Packet::PetComboPoints {
                unit: read_packed_guid(&mut r)?,
                target: read_packed_guid(&mut r)?,
                points: u8_le(&mut r)?,
            },
        },
        SMSG_MODIFY_COOLDOWN => SessionEvent::WrathSpellControl {
            packet: Packet::ModifyCooldown {
                spell_id: u32_le(&mut r)?,
                unit: u64_le(&mut r)?,
                delta_ms: u32_le(&mut r)? as i32,
            },
        },
        SMSG_FEIGN_DEATH_RESISTED => SessionEvent::WrathSpellControl {
            packet: Packet::FeignDeathResisted {
                unit: u64_le(&mut r)?,
                flag: u8_le(&mut r)?,
            },
        },
        SMSG_BREAK_TARGET => SessionEvent::WrathSpellControl {
            packet: Packet::BreakTarget {
                unit: read_packed_guid(&mut r)?,
            },
        },
        SMSG_CLEAR_TARGET => SessionEvent::WrathSpellControl {
            packet: Packet::ClearTarget {
                caster: u64_le(&mut r)?,
            },
        },
        SMSG_SET_PROJECTILE_POSITION => SessionEvent::WrathSpellControl {
            packet: Packet::ProjectilePosition {
                caster: u64_le(&mut r)?,
                cast_count: u8_le(&mut r)?,
                position: [f32_le(&mut r)?, f32_le(&mut r)?, f32_le(&mut r)?],
            },
        },
        SMSG_SEND_UNLEARN_SPELLS => {
            let count = u32_le(&mut r)? as usize;
            if count.saturating_mul(4) != body.len() - r.position() as usize {
                return Err(invalid("unlearn list count disagrees with the body"));
            }
            let spell_ids = (0..count)
                .map(|_| u32_le(&mut r))
                .collect::<io::Result<_>>()?;
            SessionEvent::WrathSpellControl {
                packet: Packet::UnlearnSpells { spell_ids },
            }
        }
        SMSG_CONVERT_RUNE => SessionEvent::WrathRunes {
            packet: RunePacket::Convert {
                index: u8_le(&mut r)?,
                rune_type: u8_le(&mut r)?,
            },
        },
        SMSG_RESYNC_RUNES => {
            let count = u32_le(&mut r)? as usize;
            if count.saturating_mul(2) != body.len() - r.position() as usize {
                return Err(invalid("rune resync count disagrees with the body"));
            }
            let runes = (0..count)
                .map(|_| Ok((u8_le(&mut r)?, u8_le(&mut r)?)))
                .collect::<io::Result<_>>()?;
            SessionEvent::WrathRunes {
                packet: RunePacket::Resync { runes },
            }
        }
        SMSG_ADD_RUNE_POWER => SessionEvent::WrathRunes {
            packet: RunePacket::AddPower {
                mask: u32_le(&mut r)?,
            },
        },
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a Wrath spell-control opcode",
            ))
        }
    };
    if r.position() as usize != body.len() {
        return Err(invalid("Wrath spell-control trailing bytes"));
    }
    Ok(vec![event])
}

/// `u64 caster`, `u32 nameLength`, then build 12340's read (`0x6dbd00`): exactly that many name
/// bytes (`0x47b6b0`), a byte into `[0xc9eac0]` (`ResurrectHasSickness`) and one into
/// `[0xc9eac4]` (`ResurrectHasTimer`); what follows is never read. AzerothCore sends the name with
/// its terminator, a 0, then its caster-is-not-a-player byte and an optional `u32` delay override
/// (`Spell::SendResurrectRequest`), so there the first flag is always 0. A player caster's name is
/// empty: the client looks it up.
fn resurrect_request(r: &mut Cursor<&[u8]>) -> io::Result<SessionEvent> {
    let caster = u64_le(r)?;
    let length = u32_le(r)? as usize;
    let body = *r.get_ref();
    let start = r.position() as usize;
    let raw = body
        .get(start..start + length)
        .ok_or_else(|| invalid("resurrect name past the body"))?;
    let name = String::from_utf8_lossy(raw.split(|&b| b == 0).next().unwrap_or_default()).into();
    r.set_position((start + length) as u64);
    let sickness = u8_le(r)? != 0;
    let has_timer = u8_le(r)? != 0;
    // AzerothCore's delay word, unread by the client, is the only tail its framing allows.
    if body.len() - r.position() as usize == 4 {
        u32_le(r)?;
    }
    Ok(SessionEvent::ResurrectRequest {
        caster,
        name,
        sickness,
        has_timer,
    })
}

/// `u32 PetNameInvalidReason`, the refused name, then a declined-names flag and, when set, the
/// five declined forms.
fn pet_name_invalid(r: &mut Cursor<&[u8]>) -> io::Result<SessionEvent> {
    let reason = u32_le(r)?;
    let name = read_cstring(r)?;
    let declined = match u8_le(r)? {
        0 => None,
        _ => {
            let mut forms: [String; 5] = Default::default();
            for form in &mut forms {
                *form = read_cstring(r)?;
            }
            Some(Box::new(forms))
        }
    };
    Ok(SessionEvent::PetNameInvalid {
        reason,
        name,
        declined,
    })
}

fn cstring(body: &mut Vec<u8>, s: &str) -> io::Result<()> {
    if s.as_bytes().contains(&0) {
        return Err(invalid("string contains NUL"));
    }
    body.extend(s.as_bytes());
    body.push(0);
    Ok(())
}

/// The opcode and body AC's handler reads for `command`.
pub fn encode(command: &Command) -> io::Result<(u32, Vec<u8>)> {
    let mut b = Vec::new();
    let opcode = match command {
        Command::CancelChannelling { spell_id } => {
            b.extend(spell_id.to_le_bytes());
            CMSG_CANCEL_CHANNELLING
        }
        Command::ResurrectResponse { caster, accept } => {
            b.extend(caster.to_le_bytes());
            b.push(u8::from(*accept));
            CMSG_RESURRECT_RESPONSE
        }
        Command::PetSetAction { pet, entries } => {
            if !(1..=2).contains(&entries.len()) {
                return Err(invalid("pet set-action takes one or two entries"));
            }
            b.extend(pet.to_le_bytes());
            for (position, word) in entries {
                b.extend(position.to_le_bytes());
                b.extend(word.to_le_bytes());
            }
            CMSG_PET_SET_ACTION
        }
        Command::PetAction { pet, word, target } => {
            b.extend(pet.to_le_bytes());
            b.extend(word.to_le_bytes());
            b.extend(target.to_le_bytes());
            CMSG_PET_ACTION
        }
        Command::PetAbandon { pet } => {
            b.extend(pet.to_le_bytes());
            CMSG_PET_ABANDON
        }
        Command::PetRename { pet, name } => {
            b.extend(pet.to_le_bytes());
            cstring(&mut b, name)?;
            b.push(0);
            CMSG_PET_RENAME
        }
        Command::PetCancelAura { pet, spell_id } => {
            b.extend(pet.to_le_bytes());
            b.extend(spell_id.to_le_bytes());
            CMSG_PET_CANCEL_AURA
        }
        Command::CancelAutoRepeatSpell => CMSG_CANCEL_AUTO_REPEAT_SPELL,
        Command::RequestPetInfo => CMSG_REQUEST_PET_INFO,
        Command::CancelGrowthAura => CMSG_CANCEL_GROWTH_AURA,
        Command::PetStopAttack { pet } => {
            b.extend(pet.to_le_bytes());
            CMSG_PET_STOP_ATTACK
        }
        Command::PetSpellAutocast {
            pet,
            spell_id,
            enabled,
        } => {
            b.extend(pet.to_le_bytes());
            b.extend(spell_id.to_le_bytes());
            b.push(u8::from(*enabled));
            CMSG_PET_SPELL_AUTOCAST
        }
        Command::CancelMountAura => CMSG_CANCEL_MOUNT_AURA,
        Command::SpellClick { unit } => {
            b.extend(unit.to_le_bytes());
            CMSG_SPELLCLICK
        }
        Command::TotemDestroyed { slot } => {
            b.push(*slot);
            CMSG_TOTEM_DESTROYED
        }
        Command::PetLearnTalent {
            pet,
            talent_id,
            rank,
        } => {
            b.extend(pet.to_le_bytes());
            b.extend(talent_id.to_le_bytes());
            b.extend(rank.to_le_bytes());
            CMSG_PET_LEARN_TALENT
        }
        Command::FarSight { engage } => {
            b.push(u8::from(*engage));
            CMSG_FAR_SIGHT
        }
        Command::DismissCritter { critter } => {
            b.extend(critter.to_le_bytes());
            CMSG_DISMISS_CRITTER
        }
        Command::StableRevivePet { npc, pet_number } => {
            b.extend(npc.to_le_bytes());
            b.extend(pet_number.to_le_bytes());
            CMSG_STABLE_REVIVE_PET
        }
        Command::LearnPreviewTalentsPet { pet, talents } => {
            b.extend(pet.to_le_bytes());
            b.extend((talents.len() as u32).to_le_bytes());
            for (talent, rank) in talents {
                b.extend(talent.to_le_bytes());
                b.extend(rank.to_le_bytes());
            }
            CMSG_LEARN_PREVIEW_TALENTS_PET
        }
    };
    Ok((opcode, b))
}

#[cfg(test)]
#[path = "wrath_spell_control_tests.rs"]
mod tests;
