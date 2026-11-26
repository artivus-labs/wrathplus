//! The pet's packet handlers: `SMSG_PET_SPELLS` and `SMSG_PET_MODE` write [`PetBar`]; the
//! refusals, the tame failure and the pet's sounds go to their queues. The pet's refusals share
//! the player's red line but not its message tables: the reference maps them in their own handlers
//! (`0x4bdb70`, `0x6e8eb0`) to say "your pet" where the player's say "you".

use std::time::Instant;

use bevy::prelude::*;

use wrathplus_protocol::types::{PetMode, PetSpells};
use wrathplus_protocol::{SessionEvent, SessionEventKind};

use wrathplus_assets::coords::wow_to_bevy;

use super::PetBar;
use crate::app_core::errors::PetTameFailures;
use crate::app_core::errors::UiError;
use crate::app_core::errors::UiErrorKeys;
use crate::app_core::state::Spells;
use crate::net::{GuidIndex, NetHandlerApp, PetDismissSoundMessage, PetTalkMessage};

pub(super) fn register(app: &mut App) {
    use SessionEventKind as K;
    app.net_handler(K::PetSpells, on_spells)
        .net_handler(K::WrathPetSpells, on_native_spells)
        .net_handler(K::PetMode, on_mode)
        .net_handler(K::PetActionFeedback, on_action_feedback)
        .net_handler(K::PetCastFailed, on_cast_failed)
        .net_handler(K::PetTameFailure, on_tame_failure)
        .net_handler(K::PetNameInvalid, on_refusal_line)
        .net_handler(K::PetBroken, on_refusal_line)
        .net_handler(K::PetActionSound, on_action_sound)
        .net_handler(K::PetDismissSound, on_dismiss_sound)
        .net_handler(K::Disconnected, on_session_end);
}

/// A lost socket sends no zero-guid `SMSG_PET_SPELLS`, so the session end resets the bar itself.
fn on_session_end(In(_): In<SessionEvent>, mut bar: ResMut<PetBar>) {
    bar.clear();
}

fn on_spells(In(ev): In<SessionEvent>, catalog: Option<Res<Spells>>, mut bar: ResMut<PetBar>) {
    if let SessionEvent::PetSpells(spells) = ev {
        pet_spells(*spells, catalog.as_deref(), &mut bar);
    }
}

/// Retain native words and seed the native pet cooldown list without a Vanilla projection.
fn on_native_spells(
    In(ev): In<SessionEvent>,
    catalog: Option<Res<Spells>>,
    mut bar: ResMut<PetBar>,
    pet: super::PetUnit,
) {
    if let SessionEvent::WrathPetSpells { spells } = ev {
        native_pet_spells(*spells, &mut bar);
        let flags = pet.store(bar.guid()).map(|s| s.0.unit_flags());
        reseed_attack_latch(&mut bar, flags);
        let now = Instant::now();
        if let Some(native) = &bar.native {
            let duration = native.duration_ms;
            let cooldowns = native.cooldowns.clone();
            bar.expires = (duration != 0)
                .then(|| now + std::time::Duration::from_millis(u64::from(duration)));
            bar.cooldowns.clear_silent();
            for cd in cooldowns {
                let display = catalog.as_ref().and_then(|c| c.catalog.get(cd.spell));
                bar.cooldowns.seed_wrath_pet(&cd, display, now);
            }
            bar.bar_signals = bar.bar_signals.wrapping_add(1);
        }
    }
}

/// Each pet-spells packet reseeds the attack latch from the pet (`0x5d6a42`, `0x5d6a80`): its
/// in-combat flag `0x800` (`0x71afb0`). Labelled: that test's swing-target leg (`[pet+0xa20]`)
/// is not held here.
pub(super) fn reseed_attack_latch(bar: &mut PetBar, pet_flags: Option<u32>) {
    if bar.native.is_some() {
        bar.attacking = pet_flags.is_some_and(|f| f & super::unit::UNIT_FLAG_PET_IN_COMBAT != 0);
    }
}

fn native_pet_spells(spells: wrathplus_protocol::wrath_pet::PetSpells, bar: &mut PetBar) {
    if spells.guid == 0 {
        bar.clear();
        return;
    }
    // Switching profiles/owners must not leave a usable legacy bar or its cooldowns behind.
    // Same-native-GUID resends retain local latches; changing the GUID resets them.
    if bar.native.as_ref().map(|old| old.guid) != Some(spells.guid) {
        bar.clear();
    }
    bar.native = Some(spells);
}

fn on_mode(In(ev): In<SessionEvent>, mut bar: ResMut<PetBar>) {
    if let SessionEvent::PetMode(mode) = ev {
        pet_mode(mode, &mut bar);
    }
}

fn on_action_feedback(In(ev): In<SessionEvent>, mut errors: ResMut<UiErrorKeys>) {
    if let SessionEvent::PetActionFeedback { reason } = ev {
        pet_action_feedback(reason, &mut errors);
    }
}

fn on_cast_failed(In(ev): In<SessionEvent>) {
    if let SessionEvent::PetCastFailed { spell_id, reason } = ev {
        pet_cast_failed(spell_id, reason);
    }
}

fn on_tame_failure(In(ev): In<SessionEvent>, mut failures: ResMut<PetTameFailures>) {
    if let SessionEvent::PetTameFailure { reason } = ev {
        pet_tame_failure(reason, &mut failures);
    }
}

fn on_refusal_line(In(ev): In<SessionEvent>, mut errors: ResMut<UiErrorKeys>) {
    match ev {
        SessionEvent::PetNameInvalid { reason, .. } => pet_name_invalid(reason, &mut errors),
        SessionEvent::PetBroken => pet_broken(&mut errors),
        _ => {}
    }
}

fn on_action_sound(
    In(ev): In<SessionEvent>,
    index: Res<GuidIndex>,
    mut talks: MessageWriter<PetTalkMessage>,
) {
    if let SessionEvent::PetActionSound { pet_guid, talk } = ev {
        pet_action_sound(pet_guid, talk, &index, &mut talks);
    }
}

fn on_dismiss_sound(In(ev): In<SessionEvent>, mut sounds: MessageWriter<PetDismissSoundMessage>) {
    if let SessionEvent::PetDismissSound { model_id, position } = ev {
        pet_dismiss_sound(model_id, position, &mut sounds);
    }
}

/// `SMSG_PET_SPELLS`: replace the whole bar and reseed the pet's cooldowns from its tail. A zero
/// guid is the teardown (`Player::RemovePetActionBar`), cooldowns included.
fn pet_spells(spells: PetSpells, catalog: Option<&Spells>, bar: &mut PetBar) {
    if spells.pet_guid == 0 {
        if bar.spells.pet_guid != 0 {
            debug!("net: pet bar torn down");
        }
        bar.clear();
        return;
    }
    // A new pet guid clears the attack latch (`0x4bc8ce`); a re-send from the same pet keeps it.
    if bar.spells.pet_guid != spells.pet_guid {
        bar.attacking = false;
    }
    debug!(
        "net: pet bar for {:#x} — react {} command {} {}, {} known spell(s), {} cooldown(s)",
        spells.pet_guid,
        spells.react_state(),
        spells.command_state(),
        if spells.bar_disabled() {
            "DISABLED"
        } else {
            "usable"
        },
        spells.spells.len(),
        spells.cooldowns.len(),
    );
    debug!(
        "net: pet bar slots — {}",
        spells
            .bar
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let what = match e.kind() {
                    wrathplus_protocol::types::PET_ACT_COMMAND => format!("cmd {}", e.action()),
                    wrathplus_protocol::types::PET_ACT_REACTION => format!("react {}", e.action()),
                    _ if e.is_empty() => "empty".to_string(),
                    _ => match catalog.and_then(|c| c.catalog.get(e.action())) {
                        Some(d) => format!("{} ({})", d.name, e.action()),
                        None => format!("spell {} NOT IN CATALOG", e.action()),
                    },
                };
                format!("{}:{what}/{:#04x}", i + 1, e.kind())
            })
            .collect::<Vec<_>>()
            .join(" ")
    );
    // vmangos sends the pet's passives here too; the book skips `DO_NOT_DISPLAY` (`0x4b2f90`).
    debug!(
        "net: pet spellbook — {}",
        spells
            .spells
            .iter()
            .map(|e| {
                match catalog.and_then(|c| c.catalog.get(e.action())) {
                    Some(d) if d.in_pet_book() => format!("{} ({})", d.name, e.action()),
                    Some(d) => format!("{} ({}) DO_NOT_DISPLAY", d.name, e.action()),
                    None => format!("{} NOT IN CATALOG", e.action()),
                }
            })
            .collect::<Vec<_>>()
            .join(" · ")
    );
    let now = Instant::now();
    // `SetPet` (`0x4bc7e0`) re-arms the expiry on every packet, and a 0 duration clears it.
    bar.expires = (spells.duration_ms != 0)
        .then(|| now + std::time::Duration::from_millis(u64::from(spells.duration_ms)));
    bar.cooldowns.clear_silent();
    for cd in &spells.cooldowns {
        let display = catalog.and_then(|c| c.catalog.get(cd.spell_id));
        bar.cooldowns.seed_pet(cd, display, now);
    }
    bar.spells = spells;
    // `SetPet` (`0x4bc7e0`) ends in `SignalEvent(0x161)` on every packet (`0x4bc90c`), whatever
    // changed: the one repaint a packet that moves only the pet's cooldowns gets.
    bar.bar_signals = bar.bar_signals.wrapping_add(1);
}

/// `SMSG_PET_MODE`: the state word alone, applied only to the bar of the pet it names.
fn pet_mode(mode: PetMode, bar: &mut PetBar) {
    if bar.spells.pet_guid == 0 || bar.spells.pet_guid != mode.pet_guid {
        return;
    }
    debug!("net: pet mode — state {:#010x}", mode.state);
    // The whole dword, as the client stores both packets' state (`0x4bc930`).
    bar.spells.state = mode.state;
}

/// `SMSG_PET_ACTION_FEEDBACK`: a refused order's reason byte, onto the red line.
fn pet_action_feedback(reason: u8, errors: &mut UiErrorKeys) {
    debug!("net: pet action feedback {reason}");
    if let Some(key) = pet_feedback_key(reason) {
        errors.0.push(UiError::key(key));
    }
}

/// The reference's map (`0x4bdb70`, jump table `0x4bdbe8`): vmangos `FEEDBACK_PET_DEAD`,
/// `FEEDBACK_NOTHING_TO_ATT`, `FEEDBACK_CANT_ATT_TARGET` and `FEEDBACK_NO_PATH_TO` (1-4) raise
/// errorIds `0x150`, `0xa0`, `0xa1` and `0x151` through `0x496720`; other bytes show nothing.
/// `ERR_PET_SPELL_NOPATH` has no 1.12 string, so a no-path order is silent in the reference too.
fn pet_feedback_key(reason: u8) -> Option<&'static str> {
    Some(match reason {
        1 => "ERR_PET_SPELL_DEAD",
        2 => "ERR_NO_ATTACK_TARGET",
        3 => "ERR_INVALID_ATTACK_TARGET",
        4 => "ERR_PET_SPELL_NOPATH",
        _ => return None,
    })
}

/// `SMSG_PET_TAME_FAILURE`: the reason byte, queued. vmangos also sends it for a summon while a
/// pet or a charmed unit is out (`Spell.cpp:5467`).
fn pet_tame_failure(reason: u8, failures: &mut PetTameFailures) {
    debug!(
        "net: pet tame failure {reason} ({})",
        wrathplus_protocol::types::pet_tame_failure_key(reason)
    );
    failures.0.push(reason);
}

/// `SMSG_PET_NAME_INVALID`: build 12340's handler (`0x6e2e90`, case `0x178`) reads the reason and
/// raises its name-check message ([`name_check_key`]). No local name was applied, so nothing rolls
/// back. Unresolved: with declined forms it also fires event `0x252` with the name and its five
/// forms, which no handler here receives (only Russian clients send them).
fn pet_name_invalid(reason: u32, errors: &mut UiErrorKeys) {
    debug!("net: pet name refused ({reason})");
    errors.0.push(UiError::key(name_check_key(reason)));
}

/// Build 12340's name-check code to its message (`0x5218c0`), AzerothCore's
/// `PetNameInvalidReason`: rows `0x256`-`0x262`, and the generic `ERR_NAME_INVALID` for any other
/// code.
pub(crate) fn name_check_key(code: u32) -> &'static str {
    match code {
        2 => "ERR_NAME_NO_NAME",
        3 => "ERR_NAME_TOO_SHORT",
        4 => "ERR_NAME_TOO_LONG",
        6 => "ERR_NAME_MIXED_LANGUAGES",
        7 => "ERR_NAME_PROFANE",
        8 => "ERR_NAME_RESERVED",
        0xb => "ERR_NAME_THREE_CONSECUTIVE",
        0xc => "ERR_NAME_INVALID_SPACE",
        0xd => "ERR_NAME_CONSECUTIVE_SPACES",
        0xe => "ERR_NAME_RUSSIAN_CONSECUTIVE_SILENT_CHARACTERS",
        0xf => "ERR_NAME_RUSSIAN_SILENT_CHARACTER_AT_BEGINNING_OR_END",
        0x10 => "ERR_NAME_DECLENSION_DOESNT_MATCH_BASE_NAME",
        _ => "ERR_NAME_INVALID",
    }
}

/// `SMSG_PET_BROKEN`, an empty body: loyalty hit zero; raise `ERR_PET_BROKEN` (`0x4bdc00`). The
/// zero-guid `SMSG_PET_SPELLS` of the unsummon right after it clears the bar (`Pet.cpp:822`).
fn pet_broken(errors: &mut UiErrorKeys) {
    debug!("net: pet ran away");
    errors.0.push(UiError::key("ERR_PET_BROKEN"));
}

/// `SMSG_PET_ACTION_SOUND`: dropped when the pet is not streamed (`0x604101`); the sound layer
/// checks the selector ([`crate::sound::creature`], `0x604106`/`0x60411c`).
fn pet_action_sound(
    pet_guid: u64,
    talk: u32,
    index: &GuidIndex,
    talks: &mut MessageWriter<PetTalkMessage>,
) {
    debug!("net: pet talk {talk} from {pet_guid:#x}");
    if let Some(&unit) = index.0.get(&pet_guid) {
        talks.write(PetTalkMessage { unit, talk });
    }
}

/// `SMSG_PET_DISMISS_SOUND`: sounded a yard above the wire point (`0x6041d0 fadd [0x7ff9d8]`),
/// in WoW space, so before the change to Bevy's.
fn pet_dismiss_sound(
    model_id: u32,
    position: [f32; 3],
    sounds: &mut MessageWriter<PetDismissSoundMessage>,
) {
    debug!("net: pet dismissed — model {model_id} at {position:?}");
    sounds.write(PetDismissSoundMessage {
        model_id,
        pos: wow_to_bevy([position[0], position[1], position[2] + 1.0]),
    });
}

/// The shared event's pet refusal, which a build 12340 session never raises: its
/// `SMSG_PET_CAST_FAILED` is `spell::native_failure`'s. Logged only.
fn pet_cast_failed(spell_id: u32, reason: Option<u8>) {
    debug!("net: pet cast failed — spell {spell_id} reason {reason:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::{PetActionEntry, PET_ACT_COMMAND, PET_COMMAND_ATTACK};

    /// `0x5d6a80`: each native packet reseeds the latch from the pet's flag `0x800`.
    #[test]
    fn a_native_packet_reseeds_the_attack_latch_from_the_pet() {
        let mut bar = PetBar {
            native: Some(Default::default()),
            attacking: true,
            ..Default::default()
        };
        reseed_attack_latch(&mut bar, Some(0x1000));
        assert!(!bar.attacking, "a pet out of combat lowers it");
        reseed_attack_latch(&mut bar, Some(0x0800));
        assert!(bar.attacking, "a fighting pet raises it");
        reseed_attack_latch(&mut bar, None);
        assert!(!bar.attacking, "an unresolved pet lowers it");
        let mut legacy = PetBar {
            attacking: true,
            ..Default::default()
        };
        reseed_attack_latch(&mut legacy, None);
        assert!(legacy.attacking, "the 1.12 bar is left alone");
    }

    #[test]
    fn native_capture_is_lossless_replacement_and_session_scoped() {
        use wrathplus_protocol::wrath_pet::{Action, Cooldown, PetSpells as Native};
        let mut bar = PetBar::default();
        pet_spells(a_bar(42), None, &mut bar);
        bar.attacking = true;
        let packet = Native {
            guid: 43,
            family: 0xfedc,
            duration_ms: 123,
            react: 2,
            command: 1,
            flags: 0xfffe,
            actions: [Action(0xc1abcdef); 10],
            spells: vec![Action(0x81012345)],
            cooldowns: vec![Cooldown {
                spell: 0xabcdef,
                category: 5,
                cooldown_ms: 1,
                category_cooldown_ms: 0x80000000,
            }],
        };
        native_pet_spells(packet.clone(), &mut bar);
        assert_eq!(bar.native, Some(packet.clone()));
        assert_eq!(bar.spells, PetSpells::default());
        assert!(!bar.attacking);
        bar.attacking = true;
        native_pet_spells(packet.clone(), &mut bar);
        assert!(bar.attacking);
        let mut other = packet.clone();
        other.guid = 44;
        native_pet_spells(other.clone(), &mut bar);
        assert_eq!(bar.native, Some(other));
        assert!(!bar.attacking);
        native_pet_spells(Native::default(), &mut bar);
        assert!(bar.native.is_none());
        native_pet_spells(packet, &mut bar);
        // The actual disconnect handler uses the same complete reset.
        bar.clear();
        assert!(bar.native.is_none());
    }

    fn a_bar(guid: u64) -> PetSpells {
        let mut s = PetSpells {
            pet_guid: guid,
            state: 0x0101, // react Defensive, command Follow
            ..Default::default()
        };
        s.bar[0] = PetActionEntry::from(PET_COMMAND_ATTACK | (u32::from(PET_ACT_COMMAND) << 24));
        s
    }

    #[test]
    fn a_zero_guid_tears_the_whole_bar_down() {
        let mut bar = PetBar::default();
        pet_spells(a_bar(0x2A), None, &mut bar);
        assert_eq!(bar.spells.pet_guid, 0x2A);

        pet_spells(PetSpells::default(), None, &mut bar);
        assert_eq!(bar.spells, PetSpells::default());
        assert!(!bar.has_bar());
    }

    /// `SetPet` (`0x4bc7e0`): each packet re-arms the expiry from its own duration, 0 clears it,
    /// and the teardown clears it too.
    #[test]
    fn each_pet_packet_rearms_or_clears_the_expiry() {
        let mut bar = PetBar::default();
        let mut charm = a_bar(0x2A);
        charm.duration_ms = 60_000;
        let before = Instant::now();
        pet_spells(charm.clone(), None, &mut bar);
        let left = bar.expires.expect("armed").duration_since(before);
        assert!(
            left >= std::time::Duration::from_secs(60) && left < std::time::Duration::from_secs(61)
        );

        pet_spells(a_bar(0x2A), None, &mut bar);
        assert_eq!(bar.expires, None, "a 0 duration clears it");
        pet_spells(charm, None, &mut bar);
        assert!(bar.expires.is_some());
        pet_spells(PetSpells::default(), None, &mut bar);
        assert_eq!(bar.expires, None, "the teardown clears it");
    }

    #[test]
    fn pet_mode_only_writes_its_own_bar() {
        let mut bar = PetBar::default();
        pet_spells(a_bar(0x2A), None, &mut bar);

        pet_mode(
            PetMode {
                pet_guid: 0x99,
                state: 0x0202,
            },
            &mut bar,
        );
        assert_eq!(bar.spells.react_state(), 1, "a stranger's mode is ignored");

        pet_mode(
            PetMode {
                pet_guid: 0x2A,
                state: 0x0202,
            },
            &mut bar,
        );
        assert_eq!(bar.spells.react_state(), 2);
        assert_eq!(bar.spells.command_state(), 2);
        assert_eq!(bar.spells.bar[0].action(), PET_COMMAND_ATTACK);
    }

    /// The client's pet-guid writer clears the latch only on a change (`0x4bc8ce`).
    #[test]
    fn the_attack_latch_survives_a_resend_and_dies_on_a_new_pet() {
        let mut bar = PetBar::default();
        pet_spells(a_bar(0x2A), None, &mut bar);
        bar.attacking = true;

        pet_spells(a_bar(0x2A), None, &mut bar);
        assert!(bar.attacking, "the same pet re-sending keeps its attack");

        pet_spells(a_bar(0x99), None, &mut bar);
        assert!(!bar.attacking, "a different pet is not attacking");
    }

    #[test]
    fn the_feedback_map_is_the_jump_table_at_0x4bdbe8() {
        let mut errors = UiErrorKeys::default();
        for reason in [0u8, 1, 2, 3, 4, 5, 200] {
            pet_action_feedback(reason, &mut errors);
        }
        assert_eq!(
            errors.0.iter().map(|e| e.key).collect::<Vec<_>>(),
            [
                "ERR_PET_SPELL_DEAD",
                "ERR_NO_ATTACK_TARGET",
                "ERR_INVALID_ATTACK_TARGET",
                "ERR_PET_SPELL_NOPATH",
            ]
        );
    }

    #[test]
    fn every_feedback_arm_is_a_catalog_row() {
        for reason in 1..=4u8 {
            let key = pet_feedback_key(reason).expect("an arm");
            let row = wrathplus_ui::messages::by_key(key)
                .unwrap_or_else(|| panic!("{key} is not a message-catalog row"));
            assert_eq!(
                row.kind,
                wrathplus_ui::messages::MsgKind::Error,
                "{key} is the red line"
            );
        }
    }

    /// Every reason a refused rename can carry lands on a build-12340 row, the red line, never the
    /// 1.12 `ERR_INVALID_PETNAME` the 12340 table lacks.
    #[test]
    fn every_pet_name_refusal_is_a_12340_red_line() {
        use wrathplus_ui::messages::{by_key, MsgKind};
        let mut errors = UiErrorKeys::default();
        for reason in 0..=0x12 {
            pet_name_invalid(reason, &mut errors);
        }
        assert_eq!(errors.0[2].key, "ERR_NAME_NO_NAME");
        assert_eq!(errors.0[0xd].key, "ERR_NAME_CONSECUTIVE_SPACES");
        assert_eq!(
            errors.0[1].key, "ERR_NAME_INVALID",
            "an unmapped code is the generic row"
        );
        for e in &errors.0 {
            let row = by_key(e.key).unwrap_or_else(|| panic!("{} is not a catalog row", e.key));
            assert!(
                row.id >= 0x256 && row.id <= 0x262,
                "{} is a 12340 name row",
                e.key
            );
            assert_eq!(row.kind, MsgKind::Error, "{} is the red line", e.key);
        }
    }
}
