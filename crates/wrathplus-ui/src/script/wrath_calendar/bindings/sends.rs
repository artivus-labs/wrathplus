//! The calendar verbs that send on the open event, each behind build 12340's guards: nothing while
//! an action is pending, refusals through `0x5b9960`'s error line, and the pending bit on a send.

use mlua::Lua;

use super::super::Opened;
use super::{
    guild_event_right, many, pend_if, set, update_error, update_error_with, write, State,
    WrathCalendarDate, ANNOUNCEMENT, GUILD_EVENT, R,
};

/// What a guarded verb did.
enum Outcome {
    Quiet,
    Refused(&'static str),
    /// A refusal whose line takes a name or count.
    RefusedWith(&'static str, String),
    Sent,
}

fn finish(lua: &Lua, outcome: Outcome) -> mlua::Result<mlua::MultiValue> {
    match outcome {
        Outcome::Refused(key) => update_error(lua, key),
        Outcome::RefusedWith(key, arg) => update_error_with(lua, key, &arg),
        Outcome::Sent => pend_if(lua, true),
        Outcome::Quiet => {}
    }
    many(vec![])
}

/// `0x5b7d20`: the right to edit the open event, the guild right for an announcement.
pub(super) fn edit_right(st: &State, guild_right: bool) -> bool {
    match st.open.as_ref() {
        Some(o) if o.fields.flags & ANNOUNCEMENT != 0 => guild_right,
        Some(_) => st.can_edit(),
        None => false,
    }
}

/// `0x5b7c00`: a creatable day, today to `CalendarGetMaxCreateDate`'s.
fn creatable(st: &State, date: WrathCalendarDate) -> bool {
    let today = st.today();
    let day = |d: WrathCalendarDate| (d.year, d.month, d.day);
    let last = super::dates::max_create(today);
    (2004, 11, 24) <= day(date) && day(today) <= day(date) && day(date) <= day(last)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // CalendarAddEvent (`0x5bcc70` → `0x5bbd60`): nothing while pending; no event is an internal
    // error; a player adds once in 5 s (`0xc207fc`); the day must be creatable (`0x5b7c00`); then
    // `0x5f1b90` sends a draft with a title (`0x5f1ba9`), a guild for a guild event or
    // announcement (`0x5f1bca`) and, for another event, a size of 1..100 (`0x5f1c18`), still ahead
    // of the clock (`0x5f1c8b`).
    set(lua, "CalendarAddEvent", |lua, _| {
        let in_guild = guild_in(lua);
        let (outcome, size) = write(lua, |st, now| {
            use Outcome::{Quiet, Refused, Sent};
            let Some(open) = st.open.as_ref().filter(|_| !st.action_pending) else {
                let o = if st.action_pending {
                    Quiet
                } else {
                    Refused("CALENDAR_ERROR_INTERNAL")
                };
                return (o, false);
            };
            let f = open.fields.clone();
            if !st.view.gm && st.last_add.is_some_and(|t| now - t < 5.0) {
                return (Refused("CALENDAR_ERROR_EVENT_THROTTLED"), false);
            }
            if !creatable(st, f.date) || open.what != Opened::Draft {
                return (Quiet, false);
            }
            let guild = f.flags & (ANNOUNCEMENT | GUILD_EVENT) != 0;
            let outcome = if f.title.is_empty() {
                Refused("CALENDAR_ERROR_NEEDS_TITLE")
            } else if guild && !in_guild {
                Refused("ERR_GUILD_PLAYER_NOT_IN_GUILD")
            } else if !guild && f.max_invites == 0 {
                Quiet
            } else if !guild && f.max_invites > 100 {
                return (Quiet, true);
            } else if f.date.seconds() <= st.today().seconds() {
                Refused("CALENDAR_ERROR_EVENT_TIME_PASSED")
            } else {
                st.push(R::AddEvent(f));
                st.last_add = Some(now);
                Sent
            };
            (outcome, false)
        });
        if size {
            update_error_with(lua, "CALENDAR_ERROR_INVITES_EXCEEDED", "100");
        }
        finish(lua, outcome)
    })?;
    // CalendarUpdateEvent (`0x5bced0` → `0x5bc310`): the `0x5191c0(0x13)` gate passes for the
    // trusted UI; nothing while pending, without an event or for a draft; the edit right
    // (`0x5b7d20`) or `CALENDAR_ERROR_PERMISSIONS`; then `0x5f1ef0` sends only changed settings,
    // clearing the flag, and refuses an event no longer ahead of the clock.
    set(lua, "CalendarUpdateEvent", |lua, _| {
        let right = guild_event_right(lua);
        let outcome = write(lua, |st, _| {
            let Some(open) = st.open.as_ref().filter(|o| o.what != Opened::Draft) else {
                return Outcome::Quiet;
            };
            if st.action_pending {
                return Outcome::Quiet;
            }
            if !edit_right(st, right) {
                return Outcome::Refused("CALENDAR_ERROR_PERMISSIONS");
            }
            let (what, fields, changed) = (open.what.clone(), open.fields.clone(), open.changed);
            if !changed {
                return Outcome::Quiet;
            }
            if let Some(open) = st.open.as_mut() {
                open.changed = false;
            }
            if fields.date.seconds() <= st.today().seconds() {
                return Outcome::Refused("CALENDAR_ERROR_EVENT_PASSED");
            }
            let Opened::Event {
                event_id,
                invite_id,
            } = what
            else {
                return Outcome::Quiet;
            };
            let invite_id = st.my_invitee().map_or(invite_id, |m| m.invite_id);
            st.push(R::UpdateEvent {
                event_id,
                invite_id,
                fields,
            });
            Outcome::Sent
        });
        finish(lua, outcome)
    })?;
    // CalendarRemoveEvent (`0x5ba5a0` → `0x5b96a0`): nothing while pending, without an event or
    // for a draft; with the edit right `0x5f1e30` removes the event (its creator or moderator
    // only, else `CALENDAR_ERROR_PERMISSIONS`); without it `0x5f23b0` removes the player's own
    // invite; either sets the pending bit (`0x5b96d5`, `0x5b96f6`).
    set(lua, "CalendarRemoveEvent", |lua, _| {
        let right = guild_event_right(lua);
        let outcome = write(lua, |st, _| {
            let Some(open) = st.open.as_ref().filter(|o| o.what != Opened::Draft) else {
                return Outcome::Quiet;
            };
            let (
                Opened::Event {
                    event_id,
                    invite_id,
                },
                flags,
            ) = (open.what.clone(), open.fields.flags)
            else {
                return Outcome::Quiet;
            };
            if st.action_pending {
                return Outcome::Quiet;
            }
            let mine = st.my_invitee();
            if edit_right(st, right) {
                if !st.can_edit() {
                    return Outcome::Refused("CALENDAR_ERROR_PERMISSIONS");
                }
                let invite_id = mine.map_or(invite_id, |m| m.invite_id);
                st.push(R::RemoveEvent {
                    event_id,
                    invite_id,
                    flags,
                });
                return Outcome::Sent;
            }
            let Some(m) = mine else {
                return Outcome::Quiet;
            };
            st.push(R::RemoveInvite {
                event_id,
                invitee: m.guid,
                invite_id: m.invite_id,
                owner_invite_id: m.invite_id,
            });
            Outcome::Sent
        });
        finish(lua, outcome)
    })?;
    // CalendarEventInvite(name) (`0x5bcf00` → `0x5bc360`): any string; nothing while pending,
    // without an event or the creator/moderator right (`0x5f1b30`); a player invites once in 2 s
    // (`0xc207f8`) or gets CALENDAR_ERROR_INVITE_THROTTLED (`0x5bc3a6`); then `0x5f2070` refuses an
    // announcement (CALENDAR_ERROR_INVITES_DISABLED), a full list of 100 and a name already on
    // it, and sends; a draft's invite is a pre-invite.
    set(lua, "CalendarEventInvite", |lua, args| {
        let Some(name) = super::text_arg(lua, &args, 0) else {
            return many(vec![]);
        };
        let outcome = write(lua, |st, now| {
            let Some(open) = st
                .open
                .as_ref()
                .filter(|_| !st.action_pending && st.can_edit())
            else {
                return Outcome::Quiet;
            };
            let (what, flags) = (open.what.clone(), open.fields.flags);
            if !st.view.gm && st.last_invite.is_some_and(|t| now - t < 2.0) {
                return Outcome::Refused("CALENDAR_ERROR_INVITE_THROTTLED");
            }
            let invitees = st.event().map(|e| e.invitees.clone()).unwrap_or_default();
            if flags & ANNOUNCEMENT != 0 {
                return Outcome::Refused("CALENDAR_ERROR_INVITES_DISABLED");
            }
            if invitees.len() >= 100 {
                return Outcome::RefusedWith("CALENDAR_ERROR_INVITES_EXCEEDED", "100".into());
            }
            if invitees
                .iter()
                .any(|i| i.name.as_deref() == Some(name.as_str()))
            {
                return Outcome::RefusedWith("CALENDAR_ERROR_ALREADY_INVITED_TO_EVENT_S", name);
            }
            let (event_id, invite_id) = match what {
                Opened::Event {
                    event_id,
                    invite_id,
                } => (event_id, invite_id),
                Opened::Draft => (0, 0),
            };
            let invite_id = st.my_invitee().map_or(invite_id, |m| m.invite_id);
            st.push(R::Invite {
                event_id,
                invite_id,
                name,
                pre_invite: event_id == 0,
                guild_event: flags & GUILD_EVENT != 0,
            });
            st.last_invite = Some(now);
            Outcome::Sent
        });
        finish(lua, outcome)
    })?;
    // CalendarMassInviteGuild(minLevel, maxLevel, minRank) (`0x5bccb0` → `0x5bc160`): the rank
    // less one, unclamped (`0x5bcd64`); nothing while pending; outside a guild
    // ERR_GUILD_PLAYER_NOT_IN_GUILD; then only a player draft (`0x5bc1be`) sends CMSG 0x42b.
    set(lua, "CalendarMassInviteGuild", |lua, args| {
        let (Some(lo), Some(hi), Some(rank)) = (
            super::int_arg(lua, &args, 0),
            super::int_arg(lua, &args, 1),
            super::int_arg(lua, &args, 2),
        ) else {
            return super::usage("Usage: CalendarMassInviteGuild(minLevel, maxLevel, minRank)");
        };
        let in_guild = guild_in(lua);
        let outcome = write(lua, |st, _| {
            if st.action_pending {
                return Outcome::Quiet;
            }
            if !in_guild {
                return Outcome::Refused("ERR_GUILD_PLAYER_NOT_IN_GUILD");
            }
            if !player_draft(st) {
                return Outcome::Quiet;
            }
            st.push(R::MassInviteGuild {
                min_level: lo as u32,
                max_level: hi as u32,
                min_rank: (rank - 1) as u32,
            });
            Outcome::Sent
        });
        finish(lua, outcome)
    })?;
    // CalendarMassInviteArenaTeam(index) (`0x5bce60` → `0x5bc240`): raises the usage line of
    // CalendarNewArenaTeamEvent (`0xa18f08`); nothing while pending; only a player draft sends.
    set(lua, "CalendarMassInviteArenaTeam", |lua, args| {
        let Some(slot) = super::int_arg(lua, &args, 0) else {
            return super::usage("Usage: CalendarNewArenaTeamEvent(index)");
        };
        let outcome = write(lua, |st, _| {
            let Some(slot) = u8::try_from(slot)
                .ok()
                .filter(|_| !st.action_pending && player_draft(st))
            else {
                return Outcome::Quiet;
            };
            st.push(R::MassInviteArenaTeam { slot });
            Outcome::Sent
        });
        finish(lua, outcome)
    })?;
    Ok(())
}

/// A player event's draft, the mass invites' target: `+0x500` set, flags without `0x440`.
fn player_draft(st: &State) -> bool {
    st.open.as_ref().is_some_and(|o| {
        o.what == Opened::Draft && o.fields.flags & (ANNOUNCEMENT | GUILD_EVENT) == 0
    })
}

/// The player's guild (`[[player+0x1008]+0xc]`), `0x5bbf4c`'s and `0x5f1bfb`'s test.
fn guild_in(lua: &Lua) -> bool {
    let model = lua
        .app_data_ref::<crate::script::Model>()
        .expect("model app_data");
    model.guild.in_guild
}
