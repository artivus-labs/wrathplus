//! Which receipts clear the action-pending bit, as build 12340's calendar dispatcher (`0x5c3fe0`)
//! does after its handler, with `CALENDAR_ACTION_PENDING(false)` (`0x5c4037`).

use wrathplus_protocol::wrath_calendar::{Command, Packet};

/// Always after the calendar, an event, a member list, the lockouts, the count and the explicit
/// clear; never after an invite alert or its removal; after an invite, its removal, a status and
/// the removed, updated and moderator alerts only when their clear byte is set (`0x5bf0a3`,
/// `0x5bf28a`, `0x5bf4a9`, `0x5c0d06`, `0x5c3d48`, `0x5c0ea8`; on a status, the byte our parser
/// names the rank); after a command result when `0x6cd380` formats its error (`0x5b9960`).
pub(super) fn clears(packet: &Packet, has_line: impl Fn(u32) -> bool) -> bool {
    match packet {
        Packet::Invite {
            raw_not_signup: b, ..
        }
        | Packet::InviteRemoved { raw_byte: b, .. }
        | Packet::Status { rank: b, .. }
        | Packet::RemovedAlert { raw_byte: b, .. }
        | Packet::UpdatedAlert { raw_byte: b, .. }
        | Packet::ModeratorStatusAlert { raw_byte: b, .. } => *b != 0,
        Packet::CommandResult { error, .. } => has_line(*error),
        Packet::InviteAlert { .. } | Packet::InviteRemovedAlert { .. } => false,
        _ => true,
    }
}

/// `0x5be30b`: the calendar's arrival asks for the pending count (`0x5bbbb0`,
/// CMSG_CALENDAR_GET_NUM_PENDING). Labelled: the dispatcher's other ask (`0x5c3f85`) is untraced.
pub(super) fn follow_up(packet: &Packet) -> Option<Command> {
    matches!(packet, Packet::Calendar(_)).then_some(Command::GetNumPending)
}
