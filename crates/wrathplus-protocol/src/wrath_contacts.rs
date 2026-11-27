//! Build-12340 contacts and duels as the shared events their feeds own. Framing is the existing
//! lossless `wrath_social` and `wrath_duel` codecs (AzerothCore
//! 7f12e89ee5f467a50e62eba1d525eac7dc953d03 SocialMgr.cpp, SocialHandler.cpp, Player.cpp
//! `DuelComplete`/`SendDuelCountdown`, SpellEffects.cpp `EffectDuel`); this module only maps.
//!
//! One `SMSG_CONTACT_LIST` becomes the shared friend list and ignore list, split by the contact
//! flag bits (1 friend, 2 ignored), each only when the header's flags carry its bit, and the
//! friends' notes ride `WrathContactNotes`; the mute bit has no place there. The duel countdown is AC's milliseconds (3000) as the shared whole
//! seconds; a winner mode of 1 is a retreat.
use crate::types::{FriendEntry, FriendOnline, FriendStatusUpdate};
use crate::SessionEvent;
use std::io;

pub const OPCODES: [u16; 8] = [
    0x0067, 0x0068, 0x0167, 0x0168, 0x0169, 0x016a, 0x016b, 0x02b7,
];

const CONTACT_FRIEND: u32 = 0x01;
const CONTACT_IGNORED: u32 = 0x02;

fn lossy(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn decode(opcode: u16, body: &[u8]) -> io::Result<Vec<SessionEvent>> {
    use crate::wrath_duel::Packet as D;
    use crate::wrath_social::Packet as S;
    if matches!(opcode, 0x0067 | 0x0068) {
        return Ok(match crate::wrath_social::decode(opcode, body)? {
            S::ContactList { flags, contacts } => {
                let friends = contacts
                    .iter()
                    .filter_map(|c| {
                        let friend = c
                            .friend
                            .as_ref()
                            .filter(|_| c.flags & CONTACT_FRIEND != 0)?;
                        let presence = friend.presence.as_ref();
                        Some(FriendEntry {
                            guid: c.guid,
                            status: friend.status,
                            area: presence.map_or(0, |p| p.area),
                            level: presence.map_or(0, |p| p.level),
                            class: presence.map_or(0, |p| p.class),
                        })
                    })
                    .collect();
                let guids = contacts
                    .iter()
                    .filter(|c| c.flags & CONTACT_IGNORED != 0)
                    .map(|c| c.guid)
                    .collect();
                let notes = contacts
                    .iter()
                    .filter(|c| c.flags & CONTACT_FRIEND != 0 && !c.note.is_empty())
                    .map(|c| (c.guid, lossy(c.note.clone())))
                    .collect();
                // 12340 replaces only the lists the header names: friends on bit 1 (`0x6b7e01`),
                // ignores on bit 2 (`0x6b7e3f`).
                let mut events = Vec::new();
                if flags & CONTACT_FRIEND != 0 {
                    events.push(SessionEvent::FriendList { friends });
                    events.push(SessionEvent::WrathContactNotes {
                        notes,
                        replace: true,
                    });
                }
                if flags & CONTACT_IGNORED != 0 {
                    events.push(SessionEvent::IgnoreList { guids });
                }
                events
            }
            S::FriendStatus {
                result,
                guid,
                note,
                online,
            } => {
                let note = note.map(|note| SessionEvent::WrathContactNotes {
                    notes: vec![(guid, lossy(note))],
                    replace: false,
                });
                let status = SessionEvent::FriendStatus(FriendStatusUpdate {
                    result,
                    guid,
                    online: online.and_then(|f| {
                        let p = f.presence?;
                        Some(FriendOnline {
                            status: f.status,
                            area: p.area,
                            level: p.level,
                            class: p.class,
                        })
                    }),
                });
                std::iter::once(status).chain(note).collect()
            }
        });
    }
    Ok(vec![match crate::wrath_duel::decode(opcode, body)? {
        D::Requested {
            arbiter,
            challenger,
        } => SessionEvent::DuelRequested {
            arbiter,
            challenger,
        },
        D::OutOfBounds => SessionEvent::DuelOutOfBounds,
        D::InBounds => SessionEvent::DuelInBounds,
        D::Complete { started } => SessionEvent::DuelComplete {
            started: started != 0,
        },
        D::Winner {
            mode,
            winner,
            loser,
        } => SessionEvent::DuelWinner {
            fled: mode == 1,
            winner: lossy(winner),
            loser: lossy(loser),
        },
        D::Countdown { counter } => SessionEvent::DuelCountdown {
            seconds: counter / 1000,
        },
    }])
}

#[cfg(test)]
mod tests;
