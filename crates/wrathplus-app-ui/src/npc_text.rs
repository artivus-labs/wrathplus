//! The `$`-token expansion of server-authored NPC text (gossip, questgiver panels, the quest log),
//! which the client does, not the server; every feed that pushes NPC text into the VM runs it, and
//! the chat feed for its monster, boss, BG system and achievement lines ([`chat_macros`]), as 12340
//! routes them through one expander (`0x579590`, `QuestTextParser.cpp`, token handler `0x578f20`).
//!
//! - The accepted set is `B C E G N R T W` in either case; anything else re-emits the `$` and
//!   leaves the letter as text. `0x578f20` also takes `A` (chat's achievement link, built in
//!   `ui_chat::native_message`), `D` (a duration) and `K`; `D` and `K` are not built.
//! - A decimal prefix is consumed ahead of every token, but only `W` and `E` read it.
//! - Case is the output switch: `$r` is `$R` through `_strlwr`, and likewise `$c` and `$t`.
//!
//! Spell descriptions use a separate expander in the reference, as here
//! ([`wrathplus_formats::substitute`]).

use bevy::prelude::*;

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::names::NameCache;
use crate::net::{Guid, GuidIndex, NetCommands};

/// The unit a `$`-macro expands against: the self player, or the speaker at the reference's four
/// chat sites.
pub(crate) struct Subject {
    pub name: String,
    /// `UNIT_FIELD_BYTES_0` bytes 0 and 1, into `ChrRaces` and `ChrClasses`, which the reference
    /// reads at the client's locale column; we read [`crate::ui_unit`]'s rows, the enUS column's.
    pub race: u8,
    pub class: u8,
    /// `UNIT_FIELD_BYTES_0` byte 2; `$G` takes the first arm on 0, the second on anything else.
    pub gender: u8,
}

/// Everything a `$`-token can read; `states` is the reference's global world-state table
/// (`[0xb71ec8]`).
pub(crate) struct MacroContext<'a> {
    /// `None` is the reference's no-subject case.
    pub subject: Option<&'a Subject>,
    pub states: &'a crate::app_core::state::WorldStates,
}

/// Expands the macros in `text`. With no subject every person-token re-emits its `$`, so a name
/// not yet known shows `$N` until the feeds re-substitute.
pub(crate) fn substitute(text: &str, ctx: &MacroContext) -> String {
    substitute_checked(text, ctx).0
}

/// `QUEST_FLAGS_OBJ_TEXT`: the quest log shows the objectives text as its completion text.
const OBJECTIVES_AS_COMPLETION: u32 = 0x0004_0000;

/// `GetQuestLogCompletionText`'s text (`0x5e06d0`), expanded: the end text (template `+0x20a4`),
/// or the objectives text (`+0x2b4`) when the end text is empty or the template's flags carry
/// [`OBJECTIVES_AS_COMPLETION`]. WatchFrame shows it under a zero-objective quest, which stock
/// counts as complete (`WatchFrame.lua:808`, `:834`).
pub(crate) fn completion_text(
    template: &wrathplus_protocol::types::QuestTemplate,
    ctx: &MacroContext,
) -> String {
    let own = template.flags & OBJECTIVES_AS_COMPLETION == 0 && !template.end_text.is_empty();
    let text = if own {
        &template.end_text
    } else {
        &template.objectives_text
    };
    substitute(text, ctx)
}

/// The objective list's head (`0x5e1460`): an exploration or event quest's one `"event"` line, the
/// template's area description verbatim (`+0x1a24`, unexpanded), always the first leaderboard
/// row and counted by `GetNumQuestLeaderBoards` (`0x5e1300`). It is finished once the log slot's
/// state word carries COMPLETE (`& 1`), as the server sets on the trigger. Without a description
/// the list starts empty. `cur` and `req` stay 0: no count to announce.
pub(crate) fn event_objective(
    template: &wrathplus_protocol::types::QuestTemplate,
    slot_state: u8,
) -> Vec<wrathplus_ui::script::QuestLogObjectiveView> {
    let mut lines = Vec::with_capacity(template.objectives.len() + 1);
    let area = template
        .wrath
        .as_ref()
        .map_or("", |w| w.area_description.as_str());
    if !area.is_empty() {
        lines.push(wrathplus_ui::script::QuestLogObjectiveView {
            text: area.to_string(),
            kind: "event".into(),
            finished: slot_state & wrathplus_protocol::types::quest_slot_state::COMPLETE != 0,
            cur: 0,
            req: 0,
            // Wrath's event/exploration POI uses objective 16 (AC quest_poi), not a row index.
            poi_bit: Some(16),
        });
    }
    lines
}

/// [`substitute`] plus `0x579590`'s return flag, `false` when a token failed (no subject, or a
/// token outside the set). Panels ignore it; chat then shows the wire text (`0x509dd0`).
pub(crate) fn substitute_checked(text: &str, ctx: &MacroContext) -> (String, bool) {
    let subject = ctx.subject;
    let mut clean = true;
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // An unaccepted `$5X` loses its digits too.
        let mut j = i + 1;
        while chars.get(j).is_some_and(char::is_ascii_digit) {
            j += 1;
        }
        // On a fail the `$` comes back and the letter is copied as text.
        let mut fail = false;
        match chars.get(j).copied() {
            // Exactly one `\n`, no CR.
            Some('B' | 'b') => {
                out.push('\n');
                i = j + 1;
            }
            // The world-state table (`SMSG_INIT_WORLD_STATES`, `SMSG_UPDATE_WORLD_STATE`); `E`
            // reads the negated key; both render `%d`, and a miss prints "0".
            Some(tok @ ('W' | 'w' | 'E' | 'e')) => {
                // `SStrToInt` over the prefix. No digits reads an uninitialized buffer in the
                // reference; we take key 0, as for an id too wide for a dword.
                let n: u32 = chars[i + 1..j]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                let key = if matches!(tok, 'E' | 'e') {
                    n.wrapping_neg()
                } else {
                    n
                };
                out.push_str(&ctx.states.get(key).to_string());
                i = j + 1;
            }
            Some('N' | 'n') => match subject {
                Some(s) => {
                    out.push_str(&s.name);
                    i = j + 1;
                }
                None => fail = true,
            },
            Some(tok @ ('R' | 'r' | 'C' | 'c')) => match subject {
                Some(s) => {
                    // An id outside the table faults the reference; we emit nothing.
                    let name = if matches!(tok, 'R' | 'r') {
                        crate::app_core::catalogs::race_names(s.race)
                            .map_or("", |(display, _)| display)
                    } else {
                        crate::app_core::catalogs::class_names(s.class)
                            .map_or("", |(display, _)| display)
                    };
                    if tok.is_ascii_lowercase() {
                        out.push_str(&name.to_ascii_lowercase());
                    } else {
                        out.push_str(name);
                    }
                    i = j + 1;
                }
                None => fail = true,
            },
            // `$T` is the PvP rank title, falling back to this gender branch, never lower-cased,
            // when the `PVP_RANK_<rank>_<team>` GlobalString misses. We ship no rank titles, so it
            // always misses.
            Some('G' | 'g' | 'T' | 't') => match subject {
                Some(s) => {
                    let mut k = j + 1;
                    while chars.get(k) == Some(&' ') {
                        k += 1;
                    }
                    match parse_branch(&chars, k) {
                        Some((first, second, end)) => {
                            out.extend(if s.gender == 0 { first } else { second });
                            i = end;
                        }
                        // Malformed: the marker and its spaces go, the argument stays as text.
                        None => i = k,
                    }
                }
                None => fail = true,
            },
            // Not in `B C E G N R T W`, or the string ended on the `$`.
            _ => fail = true,
        }
        if fail {
            out.push('$');
            i = j;
            clean = false;
        }
    }
    (out, clean)
}

/// Parses a `first:second;` branch body: the two arms and the index past the `;`.
fn parse_branch(chars: &[char], start: usize) -> Option<(&[char], &[char], usize)> {
    let colon = (start..chars.len()).find(|&j| chars[j] == ':' || chars[j] == ';')?;
    if chars[colon] != ':' {
        return None;
    }
    let semi = (colon + 1..chars.len()).find(|&j| chars[j] == ';')?;
    Some((
        trim_spaces(&chars[start..colon]),
        trim_spaces(&chars[colon + 1..semi]),
        semi + 1,
    ))
}

/// Trims leading and trailing `' '` only, as the reference's branch copy does.
fn trim_spaces(arm: &[char]) -> &[char] {
    let start = arm.iter().position(|&c| c != ' ').unwrap_or(arm.len());
    let end = arm.iter().rposition(|&c| c != ' ').map_or(start, |p| p + 1);
    &arm[start..end]
}

/// The self player as a macro [`Subject`], once it is streamed and its name is known.
pub(crate) fn player_identity(
    self_q: &Query<(&ObjectStore, &Guid), With<SelfPlayer>>,
    names: &NameCache,
    commands: &NetCommands,
) -> Option<Subject> {
    let (store, guid) = self_q.iter().next()?;
    Some(Subject {
        name: names.resolve(guid.0, commands)?.to_string(),
        race: store.0.unit_race().unwrap_or(0),
        class: store.0.unit_class().unwrap_or(0),
        gender: store.0.unit_gender().unwrap_or(0),
    })
}

/// The chat feed's [`Subject`] for any guid: the object manager's descriptors, else the name
/// cache (`0x579590`). Guid 0 has no subject.
pub(crate) fn subject_for_guid(
    guid: u64,
    index: &GuidIndex,
    stores: &Query<&ObjectStore>,
    names: &NameCache,
    commands: &NetCommands,
) -> Option<Subject> {
    if guid == 0 {
        return None;
    }
    let store = index.0.get(&guid).and_then(|e| stores.get(*e).ok());
    let name = names.resolve_unit(guid, store, commands)?.to_string();
    if let Some(store) = store {
        return Some(Subject {
            name,
            race: store.0.unit_race().unwrap_or(0),
            class: store.0.unit_class().unwrap_or(0),
            gender: store.0.unit_gender().unwrap_or(0),
        });
    }
    // A creature lands on zeros, so `$R` and `$C` emit nothing; `0x578f20`'s non-player arm
    // (`0x72aa70`/`0x72aab0`, untraced) is not built.
    let (race, class, gender) = names.player_traits(guid).unwrap_or((0, 0, 0));
    Some(Subject {
        name,
        race,
        class,
        gender,
    })
}

/// The chat lines' expansion (`0x579590`), the quest panels' expander against `guid`'s subject;
/// `None` when a token failed.
pub(crate) fn chat_macros(
    text: &str,
    guid: u64,
    (index, stores): (&GuidIndex, &Query<&ObjectStore>),
    names: &NameCache,
    commands: &NetCommands,
    states: &crate::app_core::state::WorldStates,
) -> Option<String> {
    let subject = subject_for_guid(guid, index, stores, names, commands);
    let ctx = MacroContext {
        subject: subject.as_ref(),
        states,
    };
    let (text, clean) = substitute_checked(text, &ctx);
    clean.then_some(text)
}

#[cfg(test)]
mod tests {
    use super::{substitute, substitute_checked, MacroContext, Subject};
    use crate::app_core::state::WorldStates;

    /// Race 4, class 5: both real rows.
    fn subject(gender: u8) -> Subject {
        Subject {
            name: "Thrall".into(),
            race: 4,
            class: 5,
            gender,
        }
    }

    fn expand(text: &str, subject: Option<&Subject>) -> String {
        substitute(
            text,
            &MacroContext {
                subject,
                states: &WorldStates::default(),
            },
        )
    }

    #[test]
    fn the_return_flag_reports_whether_any_token_failed() {
        let s = subject(0);
        let states = WorldStates::default();
        let ctx = |subject| MacroContext {
            subject,
            states: &states,
        };

        assert_eq!(
            substitute_checked("plain text", &ctx(None)),
            ("plain text".to_string(), true)
        );
        assert_eq!(
            substitute_checked("hi $N", &ctx(Some(&s))),
            ("hi Thrall".to_string(), true)
        );
        assert_eq!(
            substitute_checked("hi $N", &ctx(None)),
            ("hi $N".to_string(), false)
        );
        // Outside the set: fails even with a subject.
        assert_eq!(
            substitute_checked("hi $X", &ctx(Some(&s))),
            ("hi $X".to_string(), false)
        );
        // One failure anywhere poisons the flag, though the rest still expands.
        assert_eq!(
            substitute_checked("$N and $X", &ctx(Some(&s))),
            ("Thrall and $X".to_string(), false)
        );
    }

    #[test]
    fn name_and_newline() {
        let s = subject(0);
        assert_eq!(expand("Greetings $N", Some(&s)), "Greetings Thrall");
        assert_eq!(expand("Greetings $n", Some(&s)), "Greetings Thrall");
        assert_eq!(expand("Hail,$Bfriend", Some(&s)), "Hail,\nfriend");
    }

    #[test]
    fn race_and_class_follow_the_token_case() {
        let s = subject(0);
        assert_eq!(expand("A $C of $R", Some(&s)), "A Priest of Night Elf");
        assert_eq!(expand("a $c of $r", Some(&s)), "a priest of night elf");
        let unknown = Subject {
            class: 12,
            ..subject(0)
        };
        assert_eq!(expand("[$C]", Some(&unknown)), "[]");
    }

    #[test]
    fn gender_branches_on_zero() {
        assert_eq!(
            expand("Well met, $Glad:lass;.", Some(&subject(0))),
            "Well met, lad."
        );
        assert_eq!(
            expand("Well met, $Glad:lass;.", Some(&subject(1))),
            "Well met, lass."
        );
        assert_eq!(expand("$Glad:lass;", Some(&subject(2))), "lass");
        assert_eq!(
            expand("Well met, $G lad : lass ;.", Some(&subject(1))),
            "Well met, lass."
        );
        assert_eq!(expand("$tLad:Lass;", Some(&subject(0))), "Lad");
    }

    #[test]
    fn malformed_branch_drops_the_marker_not_the_text() {
        let s = subject(0);
        assert_eq!(expand("Broken $Gbranch", Some(&s)), "Broken branch");
        assert_eq!(expand("$G male female;", Some(&s)), "male female;");
        assert_eq!(expand("$G male:female", Some(&s)), "male:female");
    }

    #[test]
    fn world_state_tokens_read_an_empty_table() {
        let s = subject(0);
        assert_eq!(expand("$2077w gathered", Some(&s)), "0 gathered");
        assert_eq!(expand("$2077e gathered", Some(&s)), "0 gathered");
        // A bare `$w` is key 0.
        assert_eq!(expand("$w", Some(&s)), "0");
    }

    #[test]
    fn world_state_tokens_render_received_values() {
        let s = subject(0);
        let mut states = WorldStates::default();
        states.write(&[
            (2077, 12),
            (2077u32.wrapping_neg(), 3),
            (2264, -5i32 as u32),
        ]);
        let filled = |text: &str| {
            substitute(
                text,
                &MacroContext {
                    subject: Some(&s),
                    states: &states,
                },
            )
        };
        assert_eq!(filled("$2077w gathered"), "12 gathered");
        assert_eq!(filled("$2077e gathered"), "3 gathered");
        assert_eq!(filled("$2264w"), "-5", "rendered %d, not %u");
        assert_eq!(filled("$9999w"), "0", "an id the zone never sent");
        // `W` and `w` are one token.
        assert_eq!(filled("$2077W"), "12");
    }

    #[test]
    fn unaccepted_tokens_keep_the_dollar() {
        let s = subject(0);
        assert_eq!(expand("A $X here", Some(&s)), "A $X here");
        assert_eq!(expand("A $5X here", Some(&s)), "A $X here");
        assert_eq!(expand("Cost: 5$", Some(&s)), "Cost: 5$");
    }

    #[test]
    fn no_subject_leaves_the_text_literal() {
        assert_eq!(expand("Greetings $N", None), "Greetings $N");
        assert_eq!(expand("$Glad:lass; $C", None), "$Glad:lass; $C");
        assert_eq!(expand("Hail,$Bfriend $w", None), "Hail,\nfriend 0");
    }
}
