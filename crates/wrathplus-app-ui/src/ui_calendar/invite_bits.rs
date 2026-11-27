//! A listed row's invite bits (`+0x9c`), as build 12340 sets them from the calendar's invites
//! (`0x5c2cd9`-`0x5c2f8d`): 4 for the player's own event or a creator's invite, 2 for a moderator's
//! or creator's, 8 for a sign-up invite or a guild event without one, and 1 (pending) for an
//! unanswered invite to an event still ahead of the game clock and not locked (flag `0x10`).

use wrathplus_protocol::wrath_calendar::EventRow;
use wrathplus_ui::script::WrathCalendarDate;

/// The bits for `row` and the player's invite (status, rank, type); `None` where 12340 lists no
/// row: no invite, and neither an announcement (`0x40`) nor a guild event (`0x400`, `0x5c2f5d`).
/// Labelled: 12340 sets them at receipt; here each view build re-reads `now`, so a pending pip
/// also clears once its event passes.
pub(super) fn invite_bits(
    row: &EventRow,
    invite: Option<(u8, u8, u8)>,
    me: u64,
    now: WrathCalendarDate,
) -> Option<u8> {
    let mine = if me != 0 && row.creator == me { 4 } else { 0 };
    if row.flags & 0x40 != 0 {
        return Some(mine);
    }
    let Some((status, rank, kind)) = invite else {
        return (row.flags & 0x400 != 0).then_some(mine | 8);
    };
    let ahead = WrathCalendarDate::from_packed(row.packed_time).is_some_and(|t| t > now);
    let pending = status == 0 && ahead && row.flags & 0x10 == 0;
    Some(
        mine | u8::from(pending)
            | if rank != 0 { 2 } else { 0 }
            | if rank == 2 { 4 } else { 0 }
            | if kind == 1 { 8 } else { 0 },
    )
}
