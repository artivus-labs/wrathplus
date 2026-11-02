//! L2: the dungeon finder's queue, role check, proposal, boot and status receipts
//! (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-LFG-L2-CONTRACT`), as build 12340's one LFG handler `0x55bdc0` treats them:
//! each packet updates its own store, then prints its system lines (message table `0xac8af0`)
//! and fires its events (`0x81b530`), in the handler's order. [`LfgQueue::apply`] is pure; the L1
//! feed ([`super::l1`]) drains the shared FIFO and runs the effects.
//!
//! Labelled: the sounds the handler plays (`0x4c8fc0`) are not; `SMSG_LFG_TELEPORT_DENIED`'s
//! per-reason lines (jump table `0x55d148`) print `ERR_CLIENT_LOCKED_OUT` only above reason 7;
//! the raid check behind `DAT_00beb608` in the update arms reads false.

use wrathplus_protocol::wrath_lfg::queue::{Packet as Queue, RoleMember};
use wrathplus_protocol::wrath_lfg::{self, boot::Boot, completion::Reward, proposal::Proposal};
use wrathplus_ui::script as q;
use wrathplus_ui::script::ScriptValue as V;

/// What one receipt does after its store, in order.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Effect {
    /// A chat system line (message kind 0): the `GlobalStrings` key and its `%s`/`%d` arguments.
    Line(&'static str, Vec<V>),
    /// A red `UIErrorsFrame` line (message kind 2).
    Error(&'static str),
    /// An event and its arguments.
    Event(&'static str, Vec<V>),
}

/// One side's queue state as the handler compares and stores it (`0x5570f0`, `0x5599f0`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Side {
    /// The update carried its extra block (byte 0).
    pub(super) joined: bool,
    /// Byte 1, the joined/left lines' test: the player's `joined`, the party's join byte.
    pub(super) active: bool,
    pub(super) queued: bool,
    /// The update's raw bytes after `queued`: the party's join byte and the reserved ones.
    pub(super) raw: Vec<u8>,
    pub(super) dungeons: Vec<u32>,
    pub(super) comment: String,
}

/// The receipts' stores.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LfgQueue {
    pub(super) player: Side,
    pub(super) party: Side,
    /// The last update type of each side, for the snapshot.
    pub(super) player_update: Option<u8>,
    pub(super) party_update: Option<u8>,
    /// `0x365`, and when the queue began on the `GetTime` clock.
    pub(super) queue: Option<(Queue, f64)>,
    pub(super) role_check: Option<Queue>,
    pub(super) proposal: Option<Proposal>,
    /// `DAT_00beaa18`: the last proposal shown, so a re-sent one updates without `SHOW`.
    pub(super) last_shown: u32,
    /// `0x36D`, and when it arrived on the `GetTime` clock.
    pub(super) boot: Option<(Boot, f64)>,
    pub(super) completion: Option<Reward>,
    pub(super) disabled: bool,
}

/// What a receipt reads besides itself.
pub(super) struct Inputs<'a> {
    pub(super) me: u64,
    /// In a party or raid (`0x4fb0b0`).
    pub(super) in_group: bool,
    /// The `GetTime` clock.
    pub(super) now: f64,
    /// A unit's name from the name cache (`0x67d770`); `None` while unknown.
    pub(super) name: &'a dyn Fn(u64) -> Option<String>,
    /// A dungeon entry's catalog row: its name, id and type (`0xad4060`); type 2 is a raid list.
    pub(super) dungeon: &'a dyn Fn(u32) -> Option<(String, u32, u32)>,
}

/// `ERR_LFG_*` by `SMSG_LFG_JOIN_RESULT` result (table `0xacdda8`).
fn join_result_key(result: u32) -> Option<&'static str> {
    Some(match result {
        1 | 18 => "ERR_LFG_ROLE_CHECK_FAILED",
        2 => "ERR_LFG_GROUP_FULL",
        4 => "ERR_LFG_NO_LFG_OBJECT",
        5 => "ERR_LFG_NO_SLOTS_PLAYER",
        6 => "ERR_LFG_NO_SLOTS_PARTY",
        7 => "ERR_LFG_MISMATCHED_SLOTS",
        8 => "ERR_LFG_PARTY_PLAYERS_FROM_DIFFERENT_REALMS",
        9 => "ERR_LFG_MEMBERS_NOT_PRESENT",
        10 => "ERR_LFG_GET_INFO_TIMEOUT",
        11 => "ERR_LFG_INVALID_SLOT",
        12 => "ERR_LFG_DESERTER_PLAYER",
        13 => "ERR_LFG_DESERTER_PARTY",
        14 => "ERR_LFG_RANDOM_COOLDOWN_PLAYER",
        15 => "ERR_LFG_RANDOM_COOLDOWN_PARTY",
        16 => "ERR_LFG_TOO_MANY_MEMBERS",
        _ => return None,
    })
}

/// The role bits `ROLE_CHOSEN` reports: tank, healer, damage (`>> 1`, `>> 2`, `>> 3`).
fn role_args(roles: u32) -> [V; 3] {
    [1, 2, 3].map(|b| V::Bool(roles >> b & 1 != 0))
}

impl LfgQueue {
    /// Whether `0x557260` reads the side as a raid list: joined, with a dungeon of type 2.
    fn is_list(side: &Side, inp: &Inputs) -> bool {
        side.joined
            && side
                .dungeons
                .iter()
                .any(|&e| (inp.dungeon)(e).is_some_and(|(_, _, kind)| kind == 2))
    }

    /// The proposal is dropped (`0x552850`).
    fn drop_proposal(&mut self) {
        self.proposal = None;
    }

    /// One receipt; `None` for a packet that is not L2's.
    pub(super) fn apply(
        &mut self,
        packet: &wrath_lfg::Packet,
        inp: &Inputs,
    ) -> Option<Vec<Effect>> {
        use wrath_lfg::Packet as P;
        let mut out = Vec::new();
        match packet {
            P::Queue(Queue::QueueStatus {
                tanks,
                healers,
                dps,
                queued_time,
                ..
            }) => {
                let mut status = match packet {
                    P::Queue(q) => q.clone(),
                    _ => unreachable!(),
                };
                // Three zero needs read one damager (`0x55c0fd`).
                if let Queue::QueueStatus { dps: d, .. } = &mut status {
                    if *tanks == 0 && *healers == 0 && *dps == 0 {
                        *d = 1;
                    }
                }
                self.queue = Some((status, inp.now - f64::from(*queued_time)));
                out.push(Effect::Event("LFG_QUEUE_STATUS_UPDATE", vec![]));
            }
            P::Queue(Queue::RoleChosen { guid, ready, roles }) => {
                // An unknown name is a bare `break`: nothing shows.
                let name = (inp.name)(*guid)?;
                if *ready == 0 {
                    out.push(Effect::Line(
                        "ERR_LFG_PLAYER_DECLINED_ROLE_CHECK",
                        vec![V::Str(name)],
                    ));
                } else {
                    let mut args = vec![V::Str(name)];
                    args.extend(role_args(*roles));
                    out.push(Effect::Event("LFG_ROLE_CHECK_ROLE_CHOSEN", args));
                }
            }
            P::Queue(Queue::RoleCheck {
                state,
                initializing,
                members,
                ..
            }) => {
                self.role_check = match packet {
                    P::Queue(q) => Some(q.clone()),
                    _ => unreachable!(),
                };
                if *initializing != 0 {
                    out.push(Effect::Line("ERR_LFG_ROLE_CHECK_INITIATED", vec![]));
                    for m in members.iter().filter(|m| m.ready != 0) {
                        if let Some(name) = (inp.name)(m.guid) {
                            let mut args = vec![V::Str(name)];
                            args.extend(role_args(m.roles));
                            out.push(Effect::Event("LFG_ROLE_CHECK_ROLE_CHOSEN", args));
                        }
                    }
                }
                out.push(Effect::Event("LFG_ROLE_CHECK_UPDATE", vec![]));
                // Our row, else the first.
                let own: Option<&RoleMember> = members
                    .iter()
                    .find(|m| m.guid == inp.me)
                    .or(members.first());
                if own.is_some_and(|m| m.ready == 0) && *state == 2 {
                    out.push(Effect::Event("LFG_ROLE_CHECK_SHOW", vec![]));
                } else {
                    if *state == 5 {
                        out.push(Effect::Line("ERR_LFG_ROLE_CHECK_ABORTED", vec![]));
                    }
                    out.push(Effect::Event("LFG_ROLE_CHECK_HIDE", vec![]));
                }
            }
            P::Queue(Queue::JoinResult { result, state, .. }) => {
                // Result 6's party locks are L1's.
                let key = match (*result, *state) {
                    (1, 3) => Some("ERR_LFG_ROLE_CHECK_FAILED_TIMEOUT"),
                    (1, 4) => Some("ERR_LFG_ROLE_CHECK_FAILED_NOT_VIABLE"),
                    (r, _) => join_result_key(r),
                };
                match (*result, key) {
                    (17, _) => out.push(Effect::Error("ERR_LFG_CANT_USE_DUNGEONS")),
                    (_, Some(key)) => out.push(Effect::Line(key, vec![])),
                    (_, None) => {}
                }
            }
            P::Proposal(p) => {
                self.proposal = Some(p.clone());
                // A decline: `answered` without `accepted`, ours or a party member's.
                let declined =
                    |m: &&wrath_lfg::proposal::Member| m.answered != 0 && m.accepted == 0;
                let party = p
                    .members
                    .iter()
                    .filter(declined)
                    .any(|m| m.is_self == 0 && m.same_group != 0);
                let own = p.members.iter().filter(declined).any(|m| m.is_self != 0);
                // `0x552900`.
                out.push(Effect::Event("LFG_PROPOSAL_UPDATE", vec![]));
                if self.last_shown != p.id {
                    self.last_shown = p.id;
                    if p.silent == 0 {
                        out.push(Effect::Event("LFG_PROPOSAL_SHOW", vec![]));
                    }
                }
                if party {
                    out.push(Effect::Line("ERR_LFG_PROPOSAL_DECLINED_PARTY", vec![]));
                    self.drop_proposal();
                    out.push(Effect::Event("LFG_PROPOSAL_FAILED", vec![]));
                } else if own {
                    out.push(Effect::Line("ERR_LFG_PROPOSAL_DECLINED_SELF", vec![]));
                    self.drop_proposal();
                    out.push(Effect::Event("LFG_PROPOSAL_FAILED", vec![]));
                }
            }
            P::Queue(Queue::UpdatePlayer {
                update_type, extra, ..
            }) => {
                let next = match extra {
                    Some(x) => Side {
                        joined: true,
                        active: true,
                        queued: x.queued != 0,
                        raw: x.reserved.to_vec(),
                        dungeons: x.dungeons.clone(),
                        comment: x.comment.clone(),
                    },
                    None => Side::default(),
                };
                self.player_update = Some(*update_type);
                // A proposal ends when we leave or enter the queue, solo.
                if self.proposal.is_some() && (!next.joined || next.queued) && !inp.in_group {
                    self.drop_proposal();
                    if next.joined {
                        out.push(Effect::Line("ERR_LFG_PROPOSAL_FAILED", vec![]));
                    }
                    let succeeded = *update_type == 10;
                    out.push(Effect::Event(
                        if succeeded {
                            "LFG_PROPOSAL_SUCCEEDED"
                        } else {
                            "LFG_PROPOSAL_FAILED"
                        },
                        vec![],
                    ));
                }
                if next == self.player {
                    return Some(out);
                }
                self.side_lines(&next, true, *update_type, inp, &mut out);
                self.player = next;
                if inp.in_group {
                    return Some(out);
                }
                if !self.player.queued {
                    self.queue = None;
                }
                out.push(Effect::Event("LFG_UPDATE", vec![]));
            }
            P::Queue(Queue::UpdateParty {
                update_type, extra, ..
            }) => {
                let next = match extra {
                    Some(x) => {
                        let mut raw = vec![x.join];
                        raw.extend(x.reserved);
                        Side {
                            joined: true,
                            active: x.join != 0,
                            queued: x.queued != 0,
                            raw,
                            dungeons: x.dungeons.clone(),
                            comment: x.comment.clone(),
                        }
                    }
                    None => Side::default(),
                };
                self.party_update = Some(*update_type);
                if self.proposal.is_some() && (!next.joined || next.queued) && inp.in_group {
                    self.drop_proposal();
                    if next.joined {
                        out.push(Effect::Line("ERR_LFG_PROPOSAL_FAILED", vec![]));
                    }
                    let succeeded = matches!(*update_type, 1 | 10 | 16);
                    out.push(Effect::Event(
                        if succeeded {
                            "LFG_PROPOSAL_SUCCEEDED"
                        } else {
                            "LFG_PROPOSAL_FAILED"
                        },
                        vec![],
                    ));
                }
                if next == self.party {
                    return Some(out);
                }
                self.side_lines(&next, false, *update_type, inp, &mut out);
                self.party = next;
                if !inp.in_group {
                    return Some(out);
                }
                if !self.party.queued {
                    self.queue = None;
                }
                out.push(Effect::Event("LFG_UPDATE", vec![]));
            }
            P::Boot(b) => {
                self.boot = Some((b.clone(), inp.now));
                if b.in_progress == 0 {
                    if let Some(name) = (inp.name)(b.victim) {
                        let key = if b.agrees < b.votes_needed {
                            "ERR_PARTY_LFG_BOOT_VOTE_FAILED"
                        } else {
                            "ERR_PARTY_LFG_BOOT_VOTE_SUCCEEDED"
                        };
                        out.push(Effect::Line(key, vec![V::Str(name)]));
                    }
                }
                out.push(Effect::Event("LFG_BOOT_PROPOSAL_UPDATE", vec![]));
            }
            P::Completion(r) => {
                self.completion = Some(r.clone());
                // The base and the per-stranger variable parts (`extra` is the stranger count).
                let var_xp = r.extra.saturating_mul(r.variable_xp);
                let var_money = r.extra.saturating_mul(r.variable_money);
                for xp in [r.xp, var_xp] {
                    if xp as i32 > 0 {
                        out.push(Effect::Line(
                            "ERR_QUEST_REWARD_EXP_I",
                            vec![V::Int(i64::from(xp))],
                        ));
                    }
                }
                for money in [r.money, var_money] {
                    if money as i32 > 0 {
                        out.push(Effect::Line(
                            "ERR_QUEST_REWARD_MONEY_S",
                            vec![V::Int(i64::from(money))],
                        ));
                    }
                }
                out.push(Effect::Event("LFG_COMPLETION_REWARD", vec![]));
            }
            P::Dungeon(wrath_lfg::dungeon::Packet::OfferContinue { entry }) => {
                let (name, id, kind) = (inp.dungeon)(*entry & 0xff_ffff)?;
                out.push(Effect::Event(
                    "LFG_OFFER_CONTINUE",
                    vec![V::Str(name), V::Int(i64::from(id)), V::Int(i64::from(kind))],
                ));
            }
            P::Dungeon(wrath_lfg::dungeon::Packet::TeleportDenied { reason }) => {
                if *reason > 7 {
                    out.push(Effect::Error("ERR_CLIENT_LOCKED_OUT"));
                }
            }
            P::Dungeon(wrath_lfg::dungeon::Packet::Disabled) => {
                *self = Self {
                    disabled: true,
                    ..Self::default()
                };
                out.push(Effect::Event("LFG_UPDATE", vec![]));
                out.push(Effect::Error("ERR_SYSTEM_DISABLED"));
            }
            _ => return None,
        }
        Some(out)
    }

    /// The joined, left and queue lines of an update (`0x55c2c0`-`0x55c338`, `0x55c5c0`).
    fn side_lines(
        &self,
        next: &Side,
        player: bool,
        update_type: u8,
        inp: &Inputs,
        out: &mut Vec<Effect>,
    ) {
        let prev = if player { &self.player } else { &self.party };
        let left = |out: &mut Vec<Effect>| {
            if matches!(update_type, 7 | 15) {
                let key = if Self::is_list(prev, inp) {
                    "ERR_LFG_LEFT_LIST"
                } else {
                    "ERR_LFG_LEFT_QUEUE"
                };
                out.push(Effect::Line(key, vec![]));
            }
        };
        // The party's join line also needs a dungeon (`0x55c5ea`).
        if next.active && !prev.active && (player || !next.dungeons.is_empty()) {
            let key = if Self::is_list(next, inp) {
                "ERR_LFG_JOINED_LIST"
            } else {
                "ERR_LFG_JOINED_QUEUE"
            };
            out.push(Effect::Line(key, vec![]));
        } else if (!next.active && prev.active) || (!next.queued && prev.queued) {
            left(out);
        }
    }
}

impl LfgQueue {
    /// Native UI's snapshot of these stores (`WrathLfgQueueSnapshot`), published before each
    /// receipt's effects. Labelled: a boot victim's realm reads `None`, as on one realm; the
    /// completion names its random dungeon, else the one done.
    pub(super) fn snapshot(
        &self,
        name: &dyn Fn(u64) -> Option<String>,
    ) -> q::WrathLfgQueueSnapshot {
        let update = |side: &Side, update_type: Option<u8>, party: bool| {
            update_type.map(|update_type| {
                // The player's raw bytes are its two flags; the party's lead with its join byte.
                let raw = if party {
                    side.raw.get(1..).unwrap_or(&[])
                } else {
                    &side.raw[..]
                };
                let byte = |i: usize| raw.get(i).copied().unwrap_or(0);
                q::WrathLfgUpdate {
                    update_type,
                    joined: side.joined,
                    queued: side.queued,
                    no_partial_clear: byte(0) != 0,
                    achievements: byte(1) != 0,
                    unknown: [byte(2), byte(3), byte(4)],
                    dungeons: side.dungeons.clone(),
                    comment: side.comment.clone(),
                }
            })
        };
        let queue = self.queue.as_ref().and_then(|(status, at)| match status {
            Queue::QueueStatus {
                dungeon,
                average_wait,
                wait,
                tank_wait,
                healer_wait,
                dps_wait,
                tanks,
                healers,
                dps,
                ..
            } => Some(q::WrathLfgQueueStatus {
                dungeon: *dungeon,
                avg_wait: *average_wait,
                tank_wait: *tank_wait,
                healer_wait: *healer_wait,
                dps_wait: *dps_wait,
                my_wait: *wait,
                tank_needs: *tanks,
                healer_needs: *healers,
                dps_needs: *dps,
                queued_at_ms: (at.max(0.0) * 1000.0) as u32,
            }),
            _ => None,
        });
        let role_check = self.role_check.as_ref().and_then(|r| match r {
            Queue::RoleCheck {
                state,
                initializing,
                dungeons,
                members,
            } => Some(q::WrathLfgRoleCheck {
                state: *state,
                is_beginning: *initializing != 0,
                dungeons: dungeons.clone(),
                members: members
                    .iter()
                    .map(|m| q::WrathLfgRoleCheckMember {
                        guid: m.guid,
                        ready: m.ready != 0,
                        roles: m.roles as u8,
                        level: m.level,
                        name: name(m.guid),
                    })
                    .collect(),
            }),
            _ => None,
        });
        q::WrathLfgQueueSnapshot {
            player: update(&self.player, self.player_update, false),
            party: update(&self.party, self.party_update, true),
            queue,
            role_check,
            proposal: self.proposal.as_ref().map(|p| q::WrathLfgProposal {
                dungeon: p.entry,
                state: p.state,
                id: p.id,
                completed_mask: p.encounter_mask,
                silent: p.silent != 0,
                rows: p
                    .members
                    .iter()
                    .map(|m| q::WrathLfgProposalRow {
                        roles: m.roles,
                        is_self: m.is_self != 0,
                        in_dungeon: m.in_dungeon != 0,
                        same_group: m.same_group != 0,
                        answered: m.answered != 0,
                        accepted: m.accepted != 0,
                    })
                    .collect(),
            }),
            boot: self.boot.as_ref().map(|(b, _)| q::WrathLfgBoot {
                in_progress: b.in_progress != 0,
                did_vote: b.did_vote != 0,
                my_vote: b.agree != 0,
                victim: b.victim,
                victim_name: name(b.victim),
                victim_realm: None,
                total_votes: b.votes,
                boot_votes: b.agrees,
                time_left: b.seconds_left as i32,
                reason: b.reason.clone(),
            }),
            completion: self.completion.as_ref().map(|c| q::WrathLfgCompletion {
                dungeon: if c.random_entry != 0 {
                    c.random_entry
                } else {
                    c.actual_entry
                },
                strangers: c.extra,
                money_base: c.money as i32,
                xp_base: c.xp as i32,
                money_var: c.variable_money as i32,
                xp_var: c.variable_xp as i32,
                items: c
                    .items
                    .iter()
                    .map(|i| q::WrathLfgCompletionItem {
                        item: i.item,
                        display: i.display,
                        count: i.count,
                    })
                    .collect(),
            }),
            disabled: self.disabled,
        }
    }
}

/// Run a receipt's effects: a line as `CHAT_MSG_SYSTEM` through the chat log, an error as
/// `UI_ERROR_MESSAGE`, an event as fired; each key's text from the VM's `GlobalStrings`.
pub(super) fn run(
    script: &mut wrathplus_ui::script::UiScript,
    mut chat: Option<&mut crate::app_core::chat::ChatLog>,
    effects: &[Effect],
) {
    use crate::app_core::errors::FillArg;
    use crate::app_core::errors::UiError;
    use crate::ui_action::ui_error_text_for_script;
    for effect in effects {
        match effect {
            Effect::Line(key, args) => {
                let get = |k: &str| script.lua().globals().get::<String>(k).unwrap_or_default();
                let args = args
                    .iter()
                    .map(|a| match a {
                        // The money line takes the coin string (`0x7e7d80`).
                        V::Int(n) if *key == "ERR_QUEST_REWARD_MONEY_S" => {
                            FillArg::S(crate::ui_quest::lines::coin_text(*n as u32, &get))
                        }
                        V::Int(n) => FillArg::D(*n),
                        V::Str(s) => FillArg::S(s.clone()),
                        _ => FillArg::S(String::new()),
                    })
                    .collect();
                let text = ui_error_text_for_script(&UiError::args(key, args), script);
                if let (Some(text), Some(chat)) = (text, chat.as_deref_mut()) {
                    chat.push_event(crate::app_core::chat::ChatEvent::text_only(
                        crate::app_core::chat::ChatEventKind::System,
                        text,
                    ));
                }
            }
            Effect::Error(key) => {
                if let Some(text) = ui_error_text_for_script(&UiError::key(key), script) {
                    script.fire_event("UI_ERROR_MESSAGE", vec![V::Str(text)]);
                }
            }
            Effect::Event(name, args) => script.fire_event(name, args.clone()),
        }
    }
}

#[cfg(test)]
#[path = "l2_tests.rs"]
mod tests;
