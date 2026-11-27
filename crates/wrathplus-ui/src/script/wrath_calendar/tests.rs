use std::sync::Arc;

use super::date::{add_months, days_in_month, weekday};
use super::*;
use crate::script::UiScript;

const ME: u64 = 7;

fn vm(view: WrathCalendarView) -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_wrath_calendar(Arc::new(view));
    s.set_game_date(today());
    s
}

fn today() -> WrathCalendarDate {
    WrathCalendarDate::new(2010, 3, 15, 20, 30)
}

fn event_row(day: u8, hour: u8, rank: u8) -> WrathCalendarEntry {
    WrathCalendarEntry {
        kind: WrathCalendarKind::Player,
        date: WrathCalendarDate::new(2010, 3, day, hour, 0),
        title: "Naxx".into(),
        event_id: 100,
        invite_id: 200,
        flags: 1,
        creator: if rank == 2 { ME } else { 9 },
        invited_by: Some("Arthas".into()),
        event_type: 0,
        dungeon: -1,
        invite: Some((0, rank)),
        bits: 1 | [0, 2, 6][usize::from(rank)],
        ..Default::default()
    }
}

fn base() -> WrathCalendarView {
    WrathCalendarView {
        me: ME,
        ..Default::default()
    }
}

#[test]
fn dates_pack_and_count_like_the_wire() {
    let d = WrathCalendarDate::new(2010, 3, 15, 20, 30);
    assert_eq!(WrathCalendarDate::from_packed(d.packed()), Some(d));
    assert_eq!(WrathCalendarDate::from_packed(0), None);
    // 24 November 2004 was a Wednesday; February 2008 had 29 days.
    assert_eq!(weekday(2004, 11, 24), 4);
    assert_eq!(days_in_month(2008, 2), 29);
    assert_eq!(add_months(2010, 1, -1), (2009, 12));
    let t = d.seconds();
    assert_eq!(WrathCalendarDate::from_seconds(t), d);
    // `0x5b7fd0`: 0x1 is tested first, and flags naming nothing are "".
    let token = |flags| WrathCalendarKind::of_flags(flags).token();
    assert_eq!(
        [0x401, 0x400, 0x41, 0x40, 0x100, 0x10, 0].map(token),
        [
            "PLAYER",
            "GUILD_EVENT",
            "PLAYER",
            "GUILD_ANNOUNCEMENT",
            "PLAYER",
            "",
            ""
        ]
    );
}

/// `CalendarGetMonth` and the two setters move the shown month off today.
#[test]
fn the_shown_month_starts_at_today_and_moves() {
    let s = vm(base());
    let month = |s: &UiScript, expr: &str| -> (i64, i64, i64, i64) {
        s.eval(&format!("return {expr}")).unwrap()
    };
    assert_eq!(month(&s, "CalendarGetMonth()"), (3, 2010, 31, 2));
    assert_eq!(month(&s, "CalendarGetMonth(-1)"), (2, 2010, 28, 2));
    s.run("CalendarSetMonth(-3)").unwrap();
    assert_eq!(month(&s, "CalendarGetMonth()"), (12, 2009, 31, 3));
    s.run("CalendarSetAbsMonth(2, 2012)").unwrap();
    assert_eq!(month(&s, "CalendarGetMonth()"), (2, 2012, 29, 4));
    assert_eq!(
        s.eval::<(i64, i64, i64, i64)>("return CalendarGetDate()")
            .unwrap(),
        (2, 3, 15, 2010)
    );
    assert_eq!(
        s.eval::<(i64, i64, i64)>("local _, m, d, y = CalendarGetMinDate() return m, d, y")
            .unwrap(),
        (11, 24, 2004)
    );
    // `0x5b7a80`: the last day of this month next year; `0x5b82a0`: 31 December 2030, a Tuesday;
    // `0x5b8350`: today less 14 days.
    let date = |s: &UiScript, verb: &str| -> (i64, i64, i64, i64) {
        s.eval(&format!("return {verb}()")).unwrap()
    };
    assert_eq!(date(&s, "CalendarGetMaxCreateDate"), (5, 3, 31, 2011));
    assert_eq!(date(&s, "CalendarGetMaxDate"), (3, 12, 31, 2030));
    assert_eq!(date(&s, "CalendarGetMinHistoryDate"), (2, 3, 1, 2010));
    // Arguments truncate; `0x5b8f70` takes `|(month - 1) % 12|` and holds the year to today ±5;
    // `0x5b8ec0` holds an offset to 2030 (keeping the month) and to November 2004.
    assert_eq!(month(&s, "CalendarGetMonth(0.6)"), (2, 2012, 29, 4));
    assert_eq!(month(&s, "CalendarGetAbsMonth(13, 2010)"), (1, 2010, 31, 6));
    assert_eq!(month(&s, "CalendarGetAbsMonth(1, 2040)"), (1, 2015, 31, 5));
    s.run("CalendarSetAbsMonth(12, 2015)").unwrap();
    assert_eq!(month(&s, "CalendarGetMonth(200)"), (8, 2030, 31, 5));
    assert_eq!(month(&s, "CalendarGetMonth(-200)"), (11, 2004, 30, 2));
    assert!(s.run("CalendarSetMonth()").is_err());
}

/// A day lists App's rows, then the raid resets that fall on it.
#[test]
fn a_day_lists_events_holidays_and_projected_resets() {
    let mut view = base();
    view.entries = vec![
        WrathCalendarEntry {
            kind: WrathCalendarKind::Holiday,
            date: WrathCalendarDate::new(2010, 3, 15, 0, 0),
            title: "Noblegarden".into(),
            texture: "Calendar_Noblegarden".into(),
            sequence: (1, 3),
            holiday_description: "Eggs.".into(),
            ..Default::default()
        },
        event_row(15, 19, 0),
        WrathCalendarEntry {
            kind: WrathCalendarKind::Holiday,
            date: WrathCalendarDate::new(2010, 3, 18, 0, 0),
            title: "Fishing".into(),
            sequence: (0, 1),
            ..Default::default()
        },
        // The player's own guild event, its invite answered: creator and sign-up bits, no pending.
        WrathCalendarEntry {
            kind: WrathCalendarKind::GuildEvent,
            date: WrathCalendarDate::new(2010, 3, 19, 20, 0),
            flags: 0x400,
            invite: Some((0, 0)),
            bits: 4 | 8,
            ..Default::default()
        },
        event_row(20, 19, 1),
    ];
    // Weekly from Tuesday 2 March 2010 05:00: Tuesday the 16th.
    view.raid_resets = vec![WrathCalendarRaidReset {
        map: 533,
        name: "Naxxramas".into(),
        first: WrathCalendarDate::new(2010, 3, 2, 5, 0).seconds(),
        period: 7 * 86_400,
        difficulty_key: String::new(),
    }];
    let mut s = vm(view);
    assert_eq!(
        s.eval::<i64>("return CalendarGetNumDayEvents(0, 15)")
            .unwrap(),
        2
    );
    let (title, hour, kind, seq, status, by): (String, i64, String, String, i64, String) = s
        .eval(
            "local t, h, m, k, q, e, x, mod, st, by = CalendarGetDayEvent(0, 15, 2)
             return t, h, k, q, st, by",
        )
        .unwrap();
    assert_eq!(
        (
            title.as_str(),
            hour,
            kind.as_str(),
            seq.as_str(),
            status,
            by.as_str()
        ),
        ("Naxx", 19, "PLAYER", "", 1, "Arthas")
    );
    assert_eq!(
        s.eval::<(String, String, String)>("return CalendarGetHolidayInfo(0, 15, 1)")
            .unwrap(),
        (
            "Noblegarden".into(),
            "Eggs.".into(),
            "Calendar_Noblegarden".into()
        )
    );
    assert_eq!(
        s.eval::<i64>("return CalendarGetFirstPendingInvite(0, 15)")
            .unwrap(),
        2
    );
    assert_eq!(
        s.eval::<i64>("return CalendarGetNumDayEvents(0, 16)")
            .unwrap(),
        1
    );
    assert_eq!(
        s.eval::<(String, String, i64)>(
            "local n, k, id, h = CalendarGetRaidInfo(0, 16, 1) return n, k, h"
        )
        .unwrap(),
        ("Naxxramas".into(), "RAID_RESET".into(), 5)
    );
    assert_eq!(
        s.eval::<i64>("return CalendarGetNumDayEvents(0, 17)")
            .unwrap(),
        0
    );
    // An empty slot answers the fifteen defaults.
    assert_eq!(
        s.eval::<i64>("return select('#', CalendarGetDayEvent(0, 17, 1))")
            .unwrap(),
        15
    );
    // `0x5b8040` names a holiday's day only when it lasts more than one; local rows carry type
    // `+0xa0` -1 and status `+0xa8` 0 (`0x5b784d`), so eventType 0 and inviteStatus 1.
    let row = |s: &UiScript, d: i64| -> (String, i64, i64, i64, i64) {
        s.eval(&format!(
            "local _, _, _, _, q, e, _, _, st, _, _, _, n, of = CalendarGetDayEvent(0, {d}, 1)
             return q, e, st, n, of"
        ))
        .unwrap()
    };
    assert_eq!(row(&s, 15), ("ONGOING".into(), 0, 1, 2, 3));
    assert_eq!(row(&s, 16), ("".into(), 0, 1, 1, 0));
    assert_eq!(row(&s, 18), ("".into(), 0, 1, 1, 1));
    // `0x5b8080`, `0x5b80b0` and `0x5b92b0` read the bits `0x5c2cd9` set, not the invite.
    assert_eq!(
        s.eval::<(String, i64, i64)>(
            "local _, _, _, _, _, _, _, mod, _, _, _, kind = CalendarGetDayEvent(0, 19, 1)
             return mod, kind, CalendarGetFirstPendingInvite(0, 19)"
        )
        .unwrap(),
        ("CREATOR".into(), 2, 0)
    );
    assert_eq!(
        s.eval::<(i64, i64, String)>("return CalendarGetDayEventSequenceInfo(0, 15, 1)")
            .unwrap(),
        (1, 3, "ONGOING".into())
    );
    // `0x5b91a0`: an offset past -1..1 or a day before 24 November 2004 lists nothing.
    assert_eq!(
        s.eval::<i64>("return CalendarGetNumDayEvents(2, 4)")
            .unwrap(),
        0
    );
    // The context verbs read their own arguments (`0x5baeb0` and siblings), without any
    // selection, and raise their usage with neither.
    assert_eq!(
        s.eval::<(bool, bool, bool, bool)>(
            "return CalendarContextEventCanEdit(0, 20, 1), CalendarContextEventCanEdit(0, 19, 1),
                    CalendarContextInviteIsPending(0, 15, 2), CalendarContextEventCanComplain(0, 15, 2)"
        )
        .unwrap(),
        (true, false, true, true)
    );
    assert_eq!(
        s.eval::<(String, i64, i64, String)>(
            "return CalendarContextInviteModeratorStatus(0, 19, 1), CalendarContextInviteStatus(0, 18, 1),
                    CalendarContextInviteType(0, 19, 1), CalendarContextEventGetCalendarType(0, 19, 1)"
        )
        .unwrap(),
        ("CREATOR".into(), 1, 2, "GUILD_EVENT".into())
    );
    assert!(s.run("CalendarContextEventCanEdit()").is_err());
    // `0x5b97d0` keeps the last context event for a row that does not exist.
    s.run("CalendarContextSelectEvent(0, 20, 1) CalendarContextSelectEvent(0, 17, 1)")
        .unwrap();
    assert_eq!(
        s.eval::<(i64, i64, i64)>("return CalendarContextGetEventIndex()")
            .unwrap(),
        (0, 20, 1)
    );
    // Removing needs the moderator or creator bits (`0x5bd31e`), and sets the pending bit.
    s.run("CalendarContextEventRemove(0, 15, 2) CalendarContextEventRemove()")
        .unwrap();
    assert!(matches!(
        s.take_wrath_calendar_requests().as_slice(),
        [WrathCalendarRequest::RemoveEvent { event_id: 100, .. }]
    ));
    assert!(s.eval::<bool>("return CalendarIsActionPending()").unwrap());
    s.run("CalendarSetAbsMonth(11, 2005) CalendarSetMonth(-12)")
        .unwrap();
    assert_eq!(
        s.eval::<(i64, i64)>(
            "return CalendarGetNumDayEvents(0, 30), CalendarGetNumDayEvents(0, 23)"
        )
        .unwrap(),
        (1, 0)
    );
}

/// Opening a player event asks for it; its fields follow the server until edited, and the update
/// sends the edits.
#[test]
fn an_opened_event_edits_and_updates() {
    let mut view = base();
    let holiday = WrathCalendarEntry {
        kind: WrathCalendarKind::Holiday,
        date: WrathCalendarDate::new(2010, 3, 16, 0, 0),
        ..Default::default()
    };
    view.entries = vec![event_row(15, 19, 2), holiday];
    let mut s = vm(view.clone());
    // `0x5b9310`: a player row asks and sets the pending bit; the selection waits for the reply.
    let index = |s: &UiScript| {
        s.eval::<(i64, i64, i64)>("return CalendarGetEventIndex()")
            .unwrap()
    };
    s.run("CalendarOpenEvent(0, 15, 1)").unwrap();
    assert_eq!(
        s.take_wrath_calendar_requests(),
        [WrathCalendarRequest::GetEvent { event_id: 100 }]
    );
    assert!(s.eval::<bool>("return CalendarIsActionPending()").unwrap());
    assert_eq!(index(&s), (0, 0, 0));
    s.set_wrath_calendar_action_pending(false);
    view.event = Some(WrathCalendarEvent {
        event_id: 100,
        creator: ME,
        creator_name: Some("Me".into()),
        title: "Naxx".into(),
        flags: 1,
        date: WrathCalendarDate::new(2010, 3, 15, 19, 0),
        invitees: vec![
            WrathCalendarInvitee {
                guid: ME,
                name: Some("Me".into()),
                level: 80,
                rank: 2,
                invite_id: 200,
                status: 3,
                ..Default::default()
            },
            WrathCalendarInvitee {
                guid: 30,
                invite_id: 201,
                status: 4,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    s.set_wrath_calendar(Arc::new(view.clone()));
    assert_eq!(index(&s), (0, 15, 1));
    // `0x5f26f0`: statuses 1..8, less SIGNEDUP, NOT_SIGNEDUP and the invitee's own (OUT).
    assert_eq!(
        s.eval::<String>(
            "local t, o = {CalendarEventGetStatusOptions(2)}, {}
             for k = 1, #t, 2 do o[#o + 1] = t[k] end
             return table.concat(o, ' ')"
        )
        .unwrap(),
        "2 3 4 6 9"
    );
    // `0x5f2750` sends only a status the filter offers; `0x5f2860` only a change of rank;
    // `0x5f2238` never removes the creator.
    s.run(
        "CalendarEventSetStatus(2, 5) CalendarEventSetStatus(2, 2)
         CalendarEventSetModerator(2) CalendarEventClearModerator(2) CalendarEventRemoveInvite(1)",
    )
    .unwrap();
    assert!(matches!(
        s.take_wrath_calendar_requests().as_slice(),
        [
            WrathCalendarRequest::SetStatus { status: 1, .. },
            WrathCalendarRequest::SetModerator { rank: 1, .. }
        ]
    ));
    assert!(s.run("CalendarEventSetModerator()").is_err());
    // `0x5ba2c4`: an uncached name is ""; `0x5b8ae0` rounds half to even and keeps a selection
    // past the list's end out.
    assert_eq!(
        s.eval::<String>("return (CalendarEventGetInvite(2))")
            .unwrap(),
        ""
    );
    s.run("CalendarEventSelectInvite(2.5) CalendarEventSelectInvite(5)")
        .unwrap();
    assert_eq!(
        s.eval::<i64>("return CalendarEventGetSelectedInvite()")
            .unwrap(),
        2
    );
    assert!(s.run("CalendarEventSelectInvite()").is_err());
    // The selection is the invite's id (`+0x20`), found again after a re-sort (`0x5b8e00`).
    s.run("CalendarEventSortInvites('level', false)").unwrap();
    assert_eq!(
        s.eval::<i64>("return CalendarEventGetSelectedInvite()")
            .unwrap(),
        1
    );
    assert_eq!(
        s.eval::<(String, String, i64)>(
            "local t, d, c = CalendarGetEventInfo() return t, c, select(25, CalendarGetEventInfo()) and 1 or 0"
        )
        .unwrap(),
        ("Naxx".into(), "Me".into(), 1)
    );
    assert!(s.eval::<bool>("return CalendarEventCanEdit()").unwrap());
    s.run("CalendarEventSetTitle('Naxx 25') CalendarEventSetTime(21, 15)")
        .unwrap();
    assert!(s
        .eval::<bool>("return CalendarEventHaveSettingsChanged()")
        .unwrap());
    // A server refresh does not undo the edit.
    view.event.as_mut().unwrap().revision = 1;
    s.set_wrath_calendar(Arc::new(view));
    s.run("CalendarUpdateEvent()").unwrap();
    let requests = s.take_wrath_calendar_requests();
    let [WrathCalendarRequest::UpdateEvent {
        event_id: 100,
        invite_id: 200,
        fields,
    }] = requests.as_slice()
    else {
        panic!("{requests:?}");
    };
    assert_eq!(
        (fields.title.as_str(), fields.date.hour, fields.date.minute),
        ("Naxx 25", 21, 15)
    );
    assert!(s.eval::<bool>("return CalendarIsActionPending()").unwrap());
    // `0x5f1ef0` sends only changed settings.
    s.set_wrath_calendar_action_pending(false);
    s.run("CalendarUpdateEvent()").unwrap();
    assert_eq!(s.take_wrath_calendar_requests(), []);
    s.run("CalendarEventSetTitle('Naxx 10') CalendarUpdateEvent()")
        .unwrap();
    assert_eq!(s.take_wrath_calendar_requests().len(), 1);
    // Nothing opens while an action is pending.
    s.run("CalendarOpenEvent(0, 15, 1)").unwrap();
    assert_eq!(s.take_wrath_calendar_requests(), []);
    // A holiday opens at once with its type and selection, the open event kept (`0x5b93c2`);
    // `0x5b84a0` clears only the selection.
    s.run(
        "opened = nil
         local f = CreateFrame('Frame')
         f:RegisterEvent('CALENDAR_OPEN_EVENT')
         f:SetScript('OnEvent', function(_, _, kind) opened = kind end)
         CalendarOpenEvent(0, 16, 1)",
    )
    .unwrap();
    s.set_wrath_calendar_action_pending(false);
    s.run("CalendarOpenEvent(0, 16, 1)").unwrap();
    assert_eq!(index(&s), (0, 16, 1));
    s.run("CalendarCloseEvent()").unwrap();
    assert_eq!(s.take_wrath_calendar_requests(), []);
    assert_eq!(index(&s), (0, 0, 0));
    assert_eq!(
        s.eval::<(String, String, i64)>(
            "return opened, CalendarEventGetCalendarType(), CalendarEventGetNumInvites()"
        )
        .unwrap(),
        ("HOLIDAY".into(), "PLAYER".into(), 2)
    );
    assert_eq!(
        s.eval::<String>("return (CalendarGetEventInfo())").unwrap(),
        "Naxx 10"
    );
    // `0x5f2070` refuses a name already on the list; `0x5bc3a6` throttles a second invite.
    s.set_wrath_calendar_action_pending(false);
    s.run(
        "CALENDAR_ERROR_ALREADY_INVITED_TO_EVENT_S = '%s is already invited.'
         local e = CreateFrame('Frame')
         e:RegisterEvent('CALENDAR_UPDATE_ERROR')
         e:SetScript('OnEvent', function(_, _, msg) err = msg end)
         CalendarEventInvite('Me')",
    )
    .unwrap();
    assert_eq!(
        s.eval::<String>("return err").unwrap(),
        "Me is already invited."
    );
    s.run("CalendarEventInvite('Bob')").unwrap();
    s.set_wrath_calendar_action_pending(false);
    s.run("CalendarEventInvite('Cid')").unwrap();
    assert_eq!(
        s.eval::<String>("return err").unwrap(),
        "CALENDAR_ERROR_INVITE_THROTTLED"
    );
    let sent = s.take_wrath_calendar_requests();
    assert!(matches!(&sent[..], [WrathCalendarRequest::Invite { name, .. }] if name == "Bob"));
}

/// A draft begins with the gesture, takes its fields and is added once.
#[test]
fn a_draft_is_added_with_its_fields() {
    let mut s = vm(base());
    // `0x5f1ba9`: a draft without a title is refused (`0x5b9960` also clears the bit).
    s.run(
        "local e = CreateFrame('Frame')
         e:RegisterEvent('CALENDAR_UPDATE_ERROR')
         e:SetScript('OnEvent', function(_, _, msg) err = msg end)
         CalendarNewEvent() CalendarAddEvent()",
    )
    .unwrap();
    let refused =
        |s: &UiScript, key: &str| s.eval::<bool>(&format!("return err == {key}")).unwrap();
    assert!(refused(
        &s,
        "(CALENDAR_ERROR_NEEDS_TITLE or 'CALENDAR_ERROR_NEEDS_TITLE')"
    ));
    s.take_wrath_calendar_requests();
    s.set_guild(crate::script::GuildState {
        in_guild: true,
        ..Default::default()
    });
    s.run(
        "pending = {}
         local f = CreateFrame('Frame')
         f:RegisterEvent('CALENDAR_ACTION_PENDING')
         f:SetScript('OnEvent', function(_, _, on) pending[#pending + 1] = tostring(on) end)
         CalendarNewGuildEvent() CalendarEventSetTitle('Raid night')
         CalendarEventSetType(1) CalendarEventSetDate(3, 20, 2010) CalendarEventSetLocked()",
    )
    .unwrap();
    assert!(s.eval::<bool>("return CalendarCanAddEvent()").unwrap());
    s.run("CalendarAddEvent()").unwrap();
    let requests = s.take_wrath_calendar_requests();
    assert_eq!(
        requests[0],
        WrathCalendarRequest::NewDraft {
            flags: 0x400,
            arena_slot: None
        }
    );
    let WrathCalendarRequest::AddEvent(fields) = &requests[1] else {
        panic!("{requests:?}");
    };
    assert_eq!(
        (
            fields.title.as_str(),
            fields.event_type,
            fields.date.day,
            fields.flags
        ),
        ("Raid night", 0, 20, 0x410)
    );
    // Pending until App clears it, so a second add waits; `0x5b7e20` announces both edges.
    assert!(!s.eval::<bool>("return CalendarCanAddEvent()").unwrap());
    s.set_wrath_calendar_action_pending(false);
    assert!(!s.eval::<bool>("return CalendarCanAddEvent()").unwrap());
    assert_eq!(
        s.eval::<String>("return table.concat(pending, ' ')")
            .unwrap(),
        "true false"
    );
    // A second add within 5 s is throttled (`0x5bbdf4`).
    s.run("CalendarAddEvent()").unwrap();
    assert!(refused(
        &s,
        "(CALENDAR_ERROR_EVENT_THROTTLED or 'CALENDAR_ERROR_EVENT_THROTTLED')"
    ));
    assert_eq!(s.take_wrath_calendar_requests(), []);
    // `0x5bc1be`: only a player draft takes a guild mass invite, its rank less one unclamped.
    s.run("CalendarMassInviteGuild(1, 80, 1)").unwrap();
    assert_eq!(s.take_wrath_calendar_requests(), []);
    s.run("CalendarNewEvent() CalendarMassInviteGuild(1, 80, 0)")
        .unwrap();
    assert_eq!(
        s.take_wrath_calendar_requests()[1..],
        [WrathCalendarRequest::MassInviteGuild {
            min_level: 1,
            max_level: 80,
            min_rank: u32::MAX
        }]
    );
    // `0x5b8a50`: an announcement needs the guild right, even as a draft.
    s.run("CalendarNewGuildAnnouncement()").unwrap();
    assert!(!s.eval::<bool>("return CalendarEventCanEdit()").unwrap());
    s.take_wrath_calendar_requests();
    // `0x5f1130` marks a change only for a different title, capped by `0x7e1830`; a bad time is
    // refused (`0x5bb61e`); a setter without its arguments raises its usage.
    s.run("CalendarNewEvent() CalendarEventSetTitle('')")
        .unwrap();
    assert!(!s
        .eval::<bool>("return CalendarEventHaveSettingsChanged()")
        .unwrap());
    s.run("CalendarEventSetTitle(('x'):rep(80)) CalendarEventSetTime(25, 0)")
        .unwrap();
    assert_eq!(
        s.eval::<(i64, bool)>(
            "return #(CalendarGetEventInfo()), err == (CALENDAR_ERROR_INVALID_TIME or 'CALENDAR_ERROR_INVALID_TIME')"
        )
        .unwrap(),
        (31, true)
    );
    assert!(s.run("CalendarEventSetDate()").is_err());
    s.take_wrath_calendar_requests();
    // A player draft carries 0x1 (`0x5f2abd`), which its add sends.
    s.run("CalendarNewEvent()").unwrap();
    assert_eq!(
        s.take_wrath_calendar_requests(),
        [WrathCalendarRequest::NewDraft {
            flags: 1,
            arena_slot: None
        }]
    );
}

/// The invite list sorts locally and fires its update; a non-moderator is refused the status
/// options.
#[test]
fn invites_sort_and_status_options_need_a_moderator() {
    let mut view = base();
    view.entries = vec![event_row(15, 19, 0)];
    view.event = Some(WrathCalendarEvent {
        event_id: 100,
        creator: 9,
        invitees: ["Zed", "Amy"]
            .iter()
            .enumerate()
            .map(|(k, n)| WrathCalendarInvitee {
                guid: if k == 0 { ME } else { 20 },
                name: Some(n.to_string()),
                level: 70 + k as u8,
                status: k as u8,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    });
    let mut s = vm(view.clone());
    s.run(
        "updates = 0
         local f = CreateFrame('Frame')
         f:RegisterEvent('CALENDAR_UPDATE_INVITE_LIST')
         f:RegisterEvent('CALENDAR_UPDATE_ERROR')
         f:RegisterEvent('CALENDAR_ACTION_PENDING')
         cleared = 0
         f:SetScript('OnEvent', function(_, e, msg)
           if e == 'CALENDAR_UPDATE_ERROR' then err = msg
           elseif e == 'CALENDAR_ACTION_PENDING' then cleared = cleared + (msg == false and 1 or 0)
           else updates = updates + 1 end
         end)
         CalendarOpenEvent(0, 15, 1)",
    )
    .unwrap();
    // `0x5b8690`: no open event, no criterion.
    let criterion = |s: &UiScript| -> (String, bool) {
        s.eval("return CalendarEventGetInviteSortCriterion()")
            .unwrap()
    };
    assert_eq!(criterion(&s), ("".into(), false));
    // The reply.
    s.set_wrath_calendar(Arc::new(view));
    // An absent flag is true and flips the reverse byte (`0x5f19a1`); a false one keeps it; the
    // name matches without case; each sort fires the list event.
    let first = |s: &UiScript, sort: &str| -> String {
        s.run(sort).unwrap();
        s.eval("return (CalendarEventGetInvite(1))").unwrap()
    };
    assert_eq!(first(&s, "CalendarEventSortInvites('name')"), "Zed");
    assert_eq!(first(&s, "CalendarEventSortInvites('NAME', false)"), "Zed");
    assert_eq!(criterion(&s), ("name".into(), true));
    assert_eq!(first(&s, "CalendarEventSortInvites('name', true)"), "Amy");
    // `0xad16cc` ranks an accepted invite before an unanswered one.
    assert_eq!(
        first(&s, "CalendarEventSortInvites('status', false)"),
        "Amy"
    );
    assert_eq!(s.eval::<i64>("return updates").unwrap(), 4);
    assert!(s
        .eval::<bool>("return select(7, CalendarEventGetInvite(2))")
        .unwrap());
    assert_eq!(
        s.eval::<i64>("return select('#', CalendarEventGetStatusOptions(1))")
            .unwrap(),
        0
    );
    assert!(s
        .eval::<bool>("return err == (CALENDAR_ERROR_PERMISSIONS or 'CALENDAR_ERROR_PERMISSIONS')")
        .unwrap());
    // `0x5b9960` clears the pending bit after the error line and announces it.
    assert_eq!(s.eval::<i64>("return cleared").unwrap(), 1);
    // `0x5b7d20`: an invitee without the moderator rank sets nothing.
    s.run("CalendarEventSetTitle('Mine')").unwrap();
    assert!(!s
        .eval::<bool>("return CalendarEventHaveSettingsChanged()")
        .unwrap());
    // `0x5f23f0` answers with the player's own invite and sets the pending bit; nothing more
    // answers while it is set.
    s.take_wrath_calendar_requests();
    s.run("CalendarEventAvailable() CalendarEventDecline()")
        .unwrap();
    assert!(matches!(
        s.take_wrath_calendar_requests().as_slice(),
        [WrathCalendarRequest::Rsvp { status: 1, .. }]
    ));
    s.set_wrath_calendar_action_pending(false);
    // `0x5b96de`: without the edit right, removing the event removes the player's own invite.
    s.run("CalendarRemoveEvent()").unwrap();
    assert!(matches!(
        s.take_wrath_calendar_requests().as_slice(),
        [WrathCalendarRequest::RemoveInvite { invitee: ME, .. }]
    ));
}
