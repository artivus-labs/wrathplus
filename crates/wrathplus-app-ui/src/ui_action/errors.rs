//! The UI error line's queues and resolvers, which `super::feed_actions` drains into
//! [`show_messages`]; cast failures resolve in [`super::cast_fail`].

use bevy::prelude::*;
use wrathplus_ui::messages::MessageRecord;
use wrathplus_ui::script::{ScriptValue, UiScript};

use crate::app_core::chat::ChatEvent;
use crate::app_core::chat::ChatEventKind;
use crate::app_core::chat::ChatLog;
use crate::app_core::session::ObjectStore;
use crate::sound::MessageSounds;
use crate::target::attack_gate::attack_actor_blocked;

#[cfg(test)]
pub(crate) use crate::app_core::errors::CastFail;
pub(crate) use crate::app_core::errors::{
    CastErrors, FillArg, MountErrors, PetTameFailures, UiError, UiErrorKeys, UiErrorTexts,
};

/// Where a message shows: the `kind` field (`+0x04`) of the reference's message record, read from
/// the catalog ([`wrathplus_ui::messages`]).
pub(crate) use wrathplus_ui::messages::MsgKind;

/// Resolve a [`UiError`] to its text, the template filled in push order. Deviation: an absent or
/// empty result shows nothing, where the reference shows an empty line (no test of the string
/// between `0x4967d7` and `0x496842`), because a blank line is worse than none; only a chain
/// missing a stock key reaches it.
pub(crate) fn ui_error_text(e: &UiError, get: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let args: Vec<wrathplus_ui::strings::Arg<'_>> = e
        .args
        .iter()
        .map(|a| match a {
            FillArg::S(s) => wrathplus_ui::strings::Arg::S(s),
            FillArg::D(d) => wrathplus_ui::strings::Arg::D(*d),
        })
        .collect();
    let text = wrathplus_ui::strings::fill(&get(e.key)?, &args);
    (!text.is_empty()).then_some(text)
}

/// Resolve one VM global, a missing localization made diagnosable. The marker is host-authored
/// rather than a pretend stock template.
pub(crate) fn global_string(script: &UiScript, key: &str) -> Option<String> {
    Some(
        script
            .lua()
            .globals()
            .get::<String>(key)
            .unwrap_or_else(|_| format!("[unavailable localization: {key}]")),
    )
}

/// [`ui_error_text`] with the missing-localization policy: a miss keeps the declared key and appends arguments because there is no template left to place them.
pub(crate) fn ui_error_text_for_script(e: &UiError, script: &UiScript) -> Option<String> {
    match script.lua().globals().get::<String>(e.key).ok() {
        Some(template) => ui_error_text(e, &|_| Some(template.clone())),
        None => {
            let mut text = format!("[unavailable localization: {}]", e.key);
            for arg in &e.args {
                use std::fmt::Write as _;
                match arg {
                    FillArg::S(value) => write!(text, " {value}"),
                    FillArg::D(value) => write!(text, " {value}"),
                }
                .expect("writing to a String cannot fail");
            }
            Some(text)
        }
    }
}

/// One resolved line: its text, its surface, and its catalog record, whose sound fields
/// (`+0x08`/`+0x0c`) `DisplayError` reads with the kind (`+0x04`).
pub(crate) struct Shown {
    record: Option<&'static MessageRecord>,
    key: Option<&'static str>,
    kind: MsgKind,
    text: String,
}

impl Shown {
    #[cfg(test)]
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> MsgKind {
        self.kind
    }

    /// A catalog message by GlobalStrings key, the reference's `DisplayError(id)`: its record gives
    /// the surface and the sound. A key with no row cannot occur in the reference, where a message
    /// is an index, so it warns and falls back to a silent [`MsgKind::Error`]; the warning is the
    /// only check on keys the source walk for `ERR_` literals cannot see.
    pub(crate) fn keyed(key: &'static str, text: String) -> Self {
        let record = wrathplus_ui::messages::by_key(key);
        if record.is_none() {
            warn!("message {key:?} is not a catalog row — surface and sound are a guess");
        }
        Self {
            record,
            key: Some(key),
            kind: record.map_or(MsgKind::Error, |r| r.kind),
            text,
        }
    }

    /// A line with no catalog row, the wire's own text ([`UiErrorTexts`]), which reaches the sink
    /// `0x4945b0` without a record and so makes no sound.
    pub(crate) fn unkeyed(kind: MsgKind, text: String) -> Self {
        Self {
            record: None,
            key: None,
            kind,
            text,
        }
    }
}

/// Resolve a message key against the VM's `GlobalStrings.lua` into a [`Shown`]. An absent key is
/// diagnostic; an explicitly empty string stays silent.
pub(crate) fn keyed_line(script: &UiScript, key: &'static str) -> Option<Shown> {
    let text = global_string(script, key)?;
    (!text.is_empty()).then(|| Shown::keyed(key, text))
}

/// [`keyed_line`] for a `%s` template, each argument filling the next `%s`.
pub(crate) fn keyed_line_s(script: &UiScript, key: &'static str, args: &[&str]) -> Option<Shown> {
    let Some(template) = script.lua().globals().get::<String>(key).ok() else {
        let mut text = format!("[unavailable localization: {key}]");
        for arg in args {
            text.push(' ');
            text.push_str(arg);
        }
        return Some(Shown::keyed(key, text));
    };
    let args: Vec<wrathplus_ui::strings::Arg<'_>> = args
        .iter()
        .copied()
        .map(wrathplus_ui::strings::Arg::S)
        .collect();
    let text = wrathplus_ui::strings::fill(&template, &args);
    (!text.is_empty()).then(|| Shown::keyed(key, text))
}

/// The engine's own messages, one frame late: its by-key refusals (`ERR_PASSIVE_ABILITY`, the
/// macro compiler's `ERR_UNKNOWN_MACRO_OPTION_S` with its `%s`), which `wrathplus_ui` cannot put
/// on [`UiErrorKeys`], then the system lines its bindings print with no catalog row
/// (`0x509dd0(text, 0)`), raised as chat lines so the stock handler gets every argument.
/// Labelled: 12340 prints each at its call, so within one frame the order across the two queues
/// is lost.
pub(crate) fn engine_lines(script: &mut UiScript) -> Vec<Shown> {
    let keys = script.take_ui_errors().into_iter().map(UiError::key);
    let errors: Vec<UiError> = keys
        .chain(
            script
                .take_ui_errors_s()
                .into_iter()
                .map(|(k, s)| UiError::s(k, s)),
        )
        .collect();
    let keyed = errors
        .into_iter()
        .filter_map(|e| ui_error_text_for_script(&e, script).map(|t| Shown::keyed(e.key, t)));
    let mut lines: Vec<Shown> = keyed.collect();
    let system = script.take_system_lines().into_iter();
    lines.extend(system.map(|t| Shown::unkeyed(MsgKind::Chat, t)));
    lines
}

/// The one sink for a resolved line: show it on its [`MsgKind`]'s surface and queue its catalog
/// row's sound, which `crate::sound::message` plays after the input pass, the same frame. `who`
/// tags the debug line. [`MsgKind::Chat`] lines go out as `CHAT_MSG_SYSTEM`; the three rows that
/// ask for `CHAT_MSG_SKILL` ([`wrathplus_ui::messages::MessageRecord::chat_type`]) are not raised
/// yet.
pub(crate) fn show_messages(
    script: &mut UiScript,
    sink: &mut MessageSink,
    who: &str,
    lines: impl IntoIterator<Item = Shown>,
) {
    for Shown {
        record,
        key,
        kind,
        text,
    } in lines
    {
        debug!("{who}: message ({kind:?}) {text:?}");
        // No visible native overlay is claimed. The lines are nevertheless preserved in the
        // durable host log, while the normal addon event continues below.
        match kind {
            MsgKind::Error => {
                warn!(key = ?key, kind = ?kind, text = %text, source = who, "Wrath UI message")
            }
            MsgKind::Info | MsgKind::Chat => {
                info!(key = ?key, kind = ?kind, text = %text, source = who, "Wrath UI message")
            }
        }
        // The reference sounds a message beside its text, so the sound shares the display's guards.
        if let Some(record) = record {
            sink.sounds.push(record);
        }
        match kind {
            MsgKind::Chat => {
                sink.chat
                    .push_event(ChatEvent::text_only(ChatEventKind::System, text));
            }
            MsgKind::Info => {
                script.fire_event("UI_INFO_MESSAGE", vec![ScriptValue::Str(text)]);
            }
            MsgKind::Error => {
                script.fire_event("UI_ERROR_MESSAGE", vec![ScriptValue::Str(text)]);
            }
        }
    }
}

/// Where a shown message lands besides the VM: the chat log and the sound queue, one parameter
/// because the feeds that raise messages are at Bevy's parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct MessageSink<'w> {
    pub(crate) chat: ResMut<'w, ChatLog>,
    pub(crate) sounds: ResMut<'w, MessageSounds>,
}

/// Phase A of the attack-start validator `0x612df0`, the actor's own eligibility: the first
/// failing gate raises its `ERR_ATTACK_*` line and returns `true`, and no packet is sent. The
/// actor is the player for Attack (`0x6131aa`) and the pet for the pet bar (`0x4bd40d`).
pub(crate) fn attack_actor_refusal(
    actor: Option<&ObjectStore>,
    self_guid: Option<u64>,
    errors: &mut UiErrorKeys,
) -> bool {
    let Some(key) = attack_actor_blocked(actor, self_guid) else {
        return false;
    };
    debug!("attack refused locally by the actor's own state — {key}");
    errors.0.push(UiError::key(key));
    true
}

#[cfg(test)]
pub(crate) use crate::spell::reagents::reagent_totem_refusal;
pub(crate) use crate::spell::reagents::{first_missing_totem, first_short_reagent};

/// A (dis)mount result code to its GlobalStrings key, by vmangos `UnitMountResult` and
/// `UnitDismountResult` (`UnitDefines.h:842-863`); every key ships in 1.12, `ERR_MOUNT_OTHER`'s
/// "UNKNOWN MOUNT ERROR" included. The success codes (10, 3) show nothing.
pub(super) fn mount_result_key(mount: bool, code: u32) -> Option<&'static str> {
    if mount {
        match code {
            0 => Some("ERR_MOUNT_INVALIDMOUNTEE"),
            1 => Some("ERR_MOUNT_TOOFARAWAY"),
            2 => Some("ERR_MOUNT_ALREADYMOUNTED"),
            3 => Some("ERR_MOUNT_NOTMOUNTABLE"),
            4 => Some("ERR_MOUNT_NOTYOURPET"),
            5 => Some("ERR_MOUNT_OTHER"),
            6 => Some("ERR_MOUNT_LOOTING"),
            7 => Some("ERR_MOUNT_RACECANTMOUNT"),
            8 => Some("ERR_MOUNT_SHAPESHIFTED"),
            9 => Some("ERR_MOUNT_FORCEDDISMOUNT"),
            _ => None, // 10 is OK; an unknown code shows nothing
        }
    } else {
        match code {
            0 => Some("ERR_DISMOUNT_NOPET"),
            1 => Some("ERR_DISMOUNT_NOTMOUNTED"),
            2 => Some("ERR_DISMOUNT_NOTYOURPET"),
            _ => None, // 3 is OK
        }
    }
}
#[cfg(test)]
mod shown_tests {
    use super::{MessageSink, MessageSounds, MsgKind, Shown};
    use crate::app_core::chat::ChatLog;
    use bevy::prelude::*;
    use wrathplus_ui::script::UiScript;

    /// Through [`super::show_messages`]: a keyed line queues its catalog row; an unkeyed line and
    /// an unknown key queue nothing.
    #[test]
    fn a_keyed_line_queues_its_row_and_an_unkeyed_one_queues_nothing() {
        let mut world = World::new();
        world.init_resource::<ChatLog>();
        world.init_resource::<MessageSounds>();
        let mut state = bevy::ecs::system::SystemState::<MessageSink>::new(&mut world);
        let mut script = UiScript::new().expect("VM");

        {
            let mut sink = state.get_mut(&mut world);
            super::show_messages(
                &mut script,
                &mut sink,
                "test",
                [
                    // A voiced refusal, a named cue, a chat row and a silent row.
                    Shown::keyed("ERR_OUT_OF_MANA", "Not enough mana".into()),
                    Shown::keyed("ERR_NEWTAXIPATH", "New flight path discovered!".into()),
                    Shown::keyed("ERR_ALREADY_IN_GROUP_S", "Bob is already in a group".into()),
                    Shown::keyed("ERR_CANT_STACK", "That item cannot stack".into()),
                    Shown::unkeyed(MsgKind::Error, "the server said so".into()),
                    Shown::keyed("ERR_NOT_A_REAL_MESSAGE", "invented".into()),
                ],
            );
        }
        state.apply(&mut world);

        let queued = world.resource::<MessageSounds>().queued().to_vec();
        let keys: Vec<&str> = queued.iter().map(|r| r.key).collect();
        assert_eq!(
            keys,
            [
                "ERR_OUT_OF_MANA",
                "ERR_NEWTAXIPATH",
                "ERR_ALREADY_IN_GROUP_S",
                "ERR_CANT_STACK",
            ],
            "the unkeyed line and the unknown key have no row to sound"
        );
        // What each asks the drain for.
        assert_eq!(queued[0].type_tag, 0x0f, "the voice line");
        assert_eq!(queued[1].sound, Some("TaxiNodeDiscovered"), "the named cue");
        assert_eq!(queued[2].kind, MsgKind::Chat, "a chat row sounds too");
        assert_eq!(
            (queued[3].type_tag, queued[3].sound),
            (0x44, None),
            "queued and silent — the drain's own no-op"
        );
    }
}

#[cfg(test)]
mod mount_error_tests {
    use super::mount_result_key;

    #[test]
    fn success_codes_are_silent_and_failures_map() {
        assert_eq!(mount_result_key(true, 10), None); // MOUNTRESULT_OK
        assert_eq!(mount_result_key(false, 3), None); // DISMOUNTRESULT_OK
        assert_eq!(mount_result_key(true, 2), Some("ERR_MOUNT_ALREADYMOUNTED"));
        assert_eq!(mount_result_key(true, 8), Some("ERR_MOUNT_SHAPESHIFTED"));
        assert_eq!(mount_result_key(false, 1), Some("ERR_DISMOUNT_NOTMOUNTED"));
        // Off-table codes show nothing.
        assert_eq!(mount_result_key(true, 11), None);
        assert_eq!(mount_result_key(false, 4), None);
    }

    #[test]
    fn every_mount_key_resolves_in_the_real_global_strings() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let s = wrathplus_ui::script::UiScript::new().expect("VM");
        s.run(&String::from_utf8_lossy(&src)).expect("runs clean");
        let g = |key: &str| s.lua().globals().get::<String>(key).ok();

        for (mount, codes) in [(true, 0..=9), (false, 0..=2)] {
            for code in codes {
                let key = mount_result_key(mount, code).expect("failure code maps");
                let text = g(key).unwrap_or_default();
                assert!(!text.is_empty(), "{key} missing from GlobalStrings");
            }
        }
        assert_eq!(
            g("ERR_MOUNT_ALREADYMOUNTED").unwrap(),
            "You're already mounted."
        );
        assert_eq!(g("ERR_DISMOUNT_NOTMOUNTED").unwrap(), "You're not mounted!");
    }
}

#[cfg(test)]
mod ui_error_tests {
    use super::{
        keyed_line, keyed_line_s, ui_error_text, ui_error_text_for_script, FillArg, UiError,
    };
    use wrathplus_ui::script::UiScript;

    fn filled(key: &'static str, s: Option<&str>, d: Option<u32>) -> UiError {
        let mut args = Vec::new();
        args.extend(s.map(|s| FillArg::S(s.to_string())));
        args.extend(d.map(|d| FillArg::D(i64::from(d))));
        UiError::args(key, args)
    }

    /// `%s` then `%d`; an absent or empty key shows nothing.
    #[test]
    fn fills_substitute_and_absent_keys_are_silent() {
        let get = |key: &str| match key {
            "REQ_S" => Some("Requires %s".to_string()),
            "REQ_SI" => Some("Requires %s %d".to_string()),
            "PLAIN" => Some("Can't attack while mounted.".to_string()),
            "EMPTY" => Some(String::new()),
            _ => None,
        };
        let t = |e: &UiError| ui_error_text(e, &get);
        assert_eq!(
            t(&filled("REQ_S", Some("Herbalism"), None)).as_deref(),
            Some("Requires Herbalism")
        );
        assert_eq!(
            t(&filled("REQ_SI", Some("Mining"), Some(100))).as_deref(),
            Some("Requires Mining 100")
        );
        assert_eq!(
            t(&UiError::key("PLAIN")).as_deref(),
            Some("Can't attack while mounted.")
        );
        assert_eq!(t(&UiError::key("EMPTY")), None);
        assert_eq!(t(&UiError::key("ABSENT")), None);
    }

    /// Wrath has no authored root to render a missing key yet, so the declared key and available
    /// arguments survive as host diagnostics.
    #[test]
    fn missing_key_is_preserved_only_for_wrath() {
        let wrath = UiScript::new().expect("Wrath VM");
        let error = UiError::args(
            "TEST_MISSING_SD",
            vec![FillArg::S("Mining".into()), FillArg::D(155)],
        );

        assert_eq!(
            ui_error_text_for_script(&error, &wrath).as_deref(),
            Some("[unavailable localization: TEST_MISSING_SD] Mining 155")
        );
        assert_eq!(
            keyed_line(&wrath, "TEST_MISSING")
                .map(|line| line.text().to_string())
                .as_deref(),
            Some("[unavailable localization: TEST_MISSING]")
        );
        assert_eq!(
            keyed_line_s(&wrath, "TEST_MISSING_S", &["Mining"])
                .map(|line| line.text().to_string())
                .as_deref(),
            Some("[unavailable localization: TEST_MISSING_S] Mining")
        );
    }

    #[test]
    fn an_explicitly_empty_key_remains_silent() {
        let wrath = UiScript::new().expect("Wrath VM");
        wrath.run("TEST_EMPTY = ''").expect("set Wrath key");

        assert!(keyed_line(&wrath, "TEST_EMPTY").is_none());
    }

    /// The lock-refusal keys (the use sender `0x5f33e0`) against the shipped file.
    #[test]
    fn every_lock_refusal_key_resolves_in_the_real_global_strings() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let s = wrathplus_ui::script::UiScript::new().expect("VM");
        s.run(&String::from_utf8_lossy(&src)).expect("runs clean");
        let g = |key: &str| s.lua().globals().get::<String>(key).ok();

        assert_eq!(g("ERR_USE_LOCKED_WITH_SPELL_S").unwrap(), "Requires %s");
        assert_eq!(
            g("ERR_USE_LOCKED_WITH_SPELL_KNOWN_SI").unwrap(),
            "Requires %s %d"
        );
        assert_eq!(g("ERR_USE_LOCKED_WITH_ITEM_S").unwrap(), "Requires %s");
        assert_eq!(g("ERR_USE_LOCKED").unwrap(), "Item is locked.");
        assert_eq!(g("ERR_DOOR_LOCKED").unwrap(), "The door is locked.");
        assert_eq!(
            g("ERR_BUTTON_LOCKED").unwrap(),
            "That has already been used."
        );
        assert_eq!(g("ERR_USE_CANT_OPEN").unwrap(), "You can't open that.");
        // The engine's own refusal (`place_action`, errorId `0x9e`).
        assert_eq!(
            g("ERR_PASSIVE_ABILITY").unwrap(),
            "You can't put a passive ability in the action bar."
        );
        // The drain's `0x78` template.
        assert_eq!(g("SPELL_FAILED_TOTEMS").unwrap(), "Requires %s");

        // Through the formatter: the herb and vein lines.
        let herb = filled("ERR_USE_LOCKED_WITH_SPELL_S", Some("Herbalism"), None);
        assert_eq!(
            ui_error_text(&herb, &g).as_deref(),
            Some("Requires Herbalism")
        );
        let vein = filled(
            "ERR_USE_LOCKED_WITH_SPELL_KNOWN_SI",
            Some("Mining"),
            Some(155),
        );
        assert_eq!(
            ui_error_text(&vein, &g).as_deref(),
            Some("Requires Mining 155")
        );
    }
}

#[cfg(test)]
mod attack_actor_tests {
    use super::*;
    use wrathplus_protocol::ObjectFields;

    const HEALTH: u16 = wrathplus_protocol::types::Field::UnitHealth.index();
    const MAXHEALTH: u16 = wrathplus_protocol::types::Field::UnitMaxHealth.index();
    const FLAGS: u16 = wrathplus_protocol::types::Field::UnitFlags.index();
    const CHARMEDBY: u16 = wrathplus_protocol::types::Field::UnitCharmedby.index();
    const MOUNT: u16 = wrathplus_protocol::types::Field::UnitMountDisplayId.index();
    const PLAYER_FLAGS: u16 = wrathplus_protocol::types::Field::PlayerFlags.index();

    fn actor(pairs: &[(u16, u32)]) -> ObjectStore {
        // A live, unowned, unmounted, unimpaired unit unless a case says otherwise.
        let mut all = vec![(HEALTH, 100), (MAXHEALTH, 100)];
        all.extend_from_slice(pairs);
        ObjectStore(ObjectFields::from_pairs(&all))
    }

    fn refusal(store: Option<&ObjectStore>, self_guid: Option<u64>) -> Option<&'static str> {
        let mut errors = UiErrorKeys::default();
        let refused = attack_actor_refusal(store, self_guid, &mut errors);
        assert_eq!(
            refused,
            !errors.0.is_empty(),
            "a refusal and a message are the same event — the ref raises one per veto"
        );
        errors.0.first().map(|e| e.key)
    }

    /// Every gate of `0x612df0`'s Phase A, each on its own, against an otherwise-healthy actor.
    #[test]
    fn each_actor_gate_raises_its_own_error() {
        assert_eq!(
            refusal(Some(&actor(&[])), Some(7)),
            None,
            "a fit actor swings"
        );
        assert_eq!(
            refusal(Some(&actor(&[(HEALTH, 0)])), Some(7)),
            Some("ERR_ATTACK_DEAD")
        );
        // A released ghost has health 1; `0x605f30`'s ghost-flag leg refuses it.
        assert_eq!(
            refusal(Some(&actor(&[(HEALTH, 1), (PLAYER_FLAGS, 0x10)])), Some(7)),
            Some("ERR_ATTACK_DEAD")
        );
        assert_eq!(
            refusal(Some(&actor(&[(MOUNT, 1147)])), Some(7)),
            Some("ERR_ATTACK_MOUNTED")
        );
        for (bit, key) in crate::target::attack_gate::ATTACK_FLAG_REFUSALS {
            assert_eq!(
                refusal(Some(&actor(&[(FLAGS, bit)])), Some(7)),
                Some(key),
                "unit flag {bit:#x}"
            );
        }
        // An unrelated flag bit is no refusal.
        assert_eq!(refusal(Some(&actor(&[(FLAGS, 0x1000)])), Some(7)), None);
    }

    /// `0x612e33` compares the charmer with the active player's guid.
    #[test]
    fn charmed_by_us_still_swings_and_charmed_away_does_not() {
        let mine = actor(&[(CHARMEDBY, 7), (CHARMEDBY + 1, 0)]);
        assert_eq!(refusal(Some(&mine), Some(7)), None);

        let theirs = actor(&[(CHARMEDBY, 9), (CHARMEDBY + 1, 0)]);
        assert_eq!(refusal(Some(&theirs), Some(7)), Some("ERR_ATTACK_CHARMED"));
        // An unknown own guid never matches another's charm.
        assert_eq!(refusal(Some(&theirs), None), Some("ERR_ATTACK_CHARMED"));
    }

    /// The reference's order: dead before the flags, the flags before mounted.
    #[test]
    fn the_first_failing_gate_names_the_message() {
        let both = actor(&[(HEALTH, 0), (MOUNT, 1147), (FLAGS, 0x0004_0000)]);
        assert_eq!(refusal(Some(&both), Some(7)), Some("ERR_ATTACK_DEAD"));
        let cc = actor(&[(MOUNT, 1147), (FLAGS, 0x0004_0000)]);
        assert_eq!(refusal(Some(&cc), Some(7)), Some("ERR_ATTACK_STUNNED"));
    }

    /// A descriptor with no health field yet is not a corpse.
    #[test]
    fn an_unresolved_actor_is_never_a_refusal() {
        assert_eq!(refusal(None, Some(7)), None);
        assert_eq!(
            refusal(Some(&ObjectStore(ObjectFields::default())), Some(7)),
            None
        );
    }

    #[test]
    fn every_attack_error_key_resolves_in_the_real_global_strings() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let s = wrathplus_ui::script::UiScript::new().expect("VM");
        s.run(&String::from_utf8_lossy(&src)).expect("runs clean");
        let g = |key: &str| s.lua().globals().get::<String>(key).ok();

        for key in [
            "ERR_NO_ATTACK_TARGET",               // 0xa0
            "ERR_INVALID_ATTACK_TARGET",          // 0xa1
            "ERR_ATTACK_STUNNED",                 // 0xa2
            "ERR_ATTACK_PACIFIED",                // 0xa3
            "ERR_ATTACK_MOUNTED",                 // 0xa4
            "ERR_ATTACK_FLEEING",                 // 0xa5
            "ERR_ATTACK_CONFUSED",                // 0xa6
            "ERR_ATTACK_CHARMED",                 // 0xa7
            "ERR_ATTACK_DEAD",                    // 0xa8
            "ERR_ATTACK_PREVENTED_BY_MECHANIC_S", // 0xa9
        ] {
            assert!(!g(key).unwrap_or_default().is_empty(), "{key} missing");
        }
        assert_eq!(g("ERR_ATTACK_DEAD").unwrap(), "Can't attack while dead.");
    }
}

#[cfg(test)]
mod totem_reagent_tests {
    use super::*;
    use crate::spell::cast_fail_keys::reason::{REAGENTS, TOTEMS};
    use wrathplus_formats::SpellDisplay;
    use wrathplus_protocol::ObjectFields;

    fn store() -> ObjectStore {
        // Empty bags: every count reads 0.
        ObjectStore(ObjectFields::default())
    }

    /// The object index the count walks read, with nothing streamed.
    fn objects() -> crate::items::TestObjects {
        crate::items::TestObjects::new()
    }

    fn spell(totems: [u32; 2], reagents: [(u32, u32); 8]) -> SpellDisplay {
        SpellDisplay {
            totems,
            reagents,
            ..Default::default()
        }
    }

    /// Against empty bags: a totem spell refuses TOTEMS, a reagent spell REAGENTS, totems first; a
    /// spell with neither passes, and no spell or no store skips the check.
    #[test]
    fn missing_materials_refuse_with_the_refs_reasons() {
        let mut objs = objects();
        let st = store();
        let mining = spell([2901, 0], [(0, 0); 8]);
        let mut errors = CastErrors::default();
        assert!(reagent_totem_refusal(
            2575,
            Some(&mining),
            Some(&st),
            &objs.get(),
            &mut errors
        ));
        assert_eq!(errors.0.as_slice(), &[CastFail::local(2575, TOTEMS)]);

        let mut reagents = [(0, 0); 8];
        reagents[0] = (17056, 1); // Slow Fall's Light Feather
        let slow_fall = spell([0, 0], reagents);
        let mut errors = CastErrors::default();
        assert!(reagent_totem_refusal(
            130,
            Some(&slow_fall),
            Some(&st),
            &objs.get(),
            &mut errors
        ));
        assert_eq!(errors.0.as_slice(), &[CastFail::local(130, REAGENTS)]);

        let both = spell([2901, 0], reagents);
        let mut errors = CastErrors::default();
        assert!(reagent_totem_refusal(
            1,
            Some(&both),
            Some(&st),
            &objs.get(),
            &mut errors
        ));
        assert_eq!(errors.0.as_slice(), &[CastFail::local(1, TOTEMS)]);

        let plain = spell([0, 0], [(0, 0); 8]);
        let mut errors = CastErrors::default();
        assert!(!reagent_totem_refusal(
            133,
            Some(&plain),
            Some(&st),
            &objs.get(),
            &mut errors
        ));
        assert!(!reagent_totem_refusal(
            2575,
            None,
            Some(&st),
            &objs.get(),
            &mut errors
        ));
        assert!(!reagent_totem_refusal(
            2575,
            Some(&mining),
            None,
            &objs.get(),
            &mut errors
        ));
        assert!(errors.0.is_empty());
    }

    /// Against empty bags, the first nonzero totem and reagent.
    #[test]
    fn first_failing_slot_is_named() {
        let mut objs = objects();
        let st = store();
        let mut reagents = [(0, 0); 8];
        reagents[1] = (17056, 1);
        let d = spell([0, 7005], reagents);
        assert_eq!(first_missing_totem(&d, &st, &objs.get()), Some(7005));
        assert_eq!(first_short_reagent(&d, &st, &objs.get()), Some(17056));
        let none = spell([0, 0], [(0, 0); 8]);
        assert_eq!(first_missing_totem(&none, &st, &objs.get()), None);
        assert_eq!(first_short_reagent(&none, &st, &objs.get()), None);
    }
}
