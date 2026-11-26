//! Build-12340 session metadata: tutorial flags, cinematics, played time, level-up, weather and the
//! account-data channel, plus the matching commands.
//! Wire reference: AzerothCore 7f12e89ee5f467a50e62eba1d525eac7dc953d03: WorldSession.cpp
//! (`SendTutorialsData`, `SendAccountDataTimes`), MiscHandler.cpp (`HandleUpdateAccountData`,
//! `HandleRequestAccountData`, `HandleAreaTriggerOpcode`, `HandleZoneUpdateOpcode`),
//! CharacterHandler.cpp (tutorial handlers), CharacterPackets.cpp (`PlayedTime`), MiscPackets.cpp
//! (`LevelUpInfo`, `Weather`), Player.cpp (`SendCinematicStart`). Strict framing.
//!
//! Tutorial flags, cinematics, played time, level-up and weather mean what 1.12's do and feed the
//! shared events; this build's level-up carries seven power deltas (the rune and runic-power pair
//! are framed and dropped), weather has no sound id (0) and played time a trigger byte (dropped).
//! Account-data times and updates are capture-only.
use crate::types::LevelUpInfo;
use crate::SessionEvent;
use std::io::{self, Cursor, Read};

/// The shared events speak the 1.12 weather types (1 rain, 2 snow, 3 sand, 0 clear), but 3.3.5's
/// `SMSG_WEATHER` carries a `WeatherState` (AzerothCore `Weather.h`, picked by `Weather::GetWeatherState`
/// from type and grade): 3/4/5 light/medium/heavy rain, 6/7/8 snow, 22/41/42 sandstorm, 86 thunders and
/// 90 black rain (rain), 106 black snow, 1 fog (no precipitation). Anything else is clear.
pub(crate) fn weather_kind(state: u32) -> u32 {
    match state {
        3..=5 | 86 | 90 => 1,
        6..=8 | 106 => 2,
        22 | 41 | 42 => 3,
        _ => 0,
    }
}

pub const OPCODES: [u16; 8] = [
    0x00fa, 0x00fd, 0x01cd, 0x01d4, 0x0209, 0x020c, 0x02f4, 0x0463,
];

/// `NUM_ACCOUNT_DATA_TYPES`.
pub const ACCOUNT_DATA_TYPES: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// `SMSG_ACCOUNT_DATA_TIMES`: server time, a raw byte, the type mask and one time per set bit.
    AccountDataTimes {
        server_time: u32,
        raw_byte: u8,
        mask: u32,
        times: Vec<u32>,
    },
    /// `SMSG_UPDATE_ACCOUNT_DATA`: one type's zlib-compressed blob, kept compressed.
    AccountData {
        player: u64,
        kind: u32,
        time: u32,
        decompressed_size: u32,
        compressed: Vec<u8>,
    },
    /// `SMSG_UPDATE_ACCOUNT_DATA_COMPLETE`.
    AccountDataSaved { kind: u32, raw_word: u32 },
}

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

fn u8_(r: &mut Cursor<&[u8]>) -> io::Result<u8> {
    let mut b = [0; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn u32_(r: &mut Cursor<&[u8]>) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn u64_(r: &mut Cursor<&[u8]>) -> io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

pub fn decode(opcode: u16, body: &[u8]) -> io::Result<SessionEvent> {
    let mut r = Cursor::new(body);
    let r = &mut r;
    let capture = |packet| SessionEvent::WrathSessionMeta { packet };
    let event = match opcode {
        0x00fd => {
            let mut flags = vec![0; 32];
            r.read_exact(&mut flags)?;
            SessionEvent::TutorialFlags(flags)
        }
        0x00fa => SessionEvent::CinematicTriggered {
            cinematic_id: u32_(r)?,
        },
        0x01cd => {
            let (total, level) = (u32_(r)?, u32_(r)?);
            // The request's trigger byte, echoed.
            u8_(r)?;
            SessionEvent::PlayedTime { total, level }
        }
        0x01d4 => {
            let (level, health) = (u32_(r)?, u32_(r)?);
            let mut powers = [0; 5];
            for power in &mut powers {
                *power = u32_(r)?;
            }
            // Runes and runic power.
            u32_(r)?;
            u32_(r)?;
            let mut stats = [0; 5];
            for stat in &mut stats {
                *stat = u32_(r)?;
            }
            SessionEvent::LevelUp(LevelUpInfo {
                level,
                health,
                powers,
                stats,
            })
        }
        0x02f4 => SessionEvent::Weather {
            weather_type: weather_kind(u32_(r)?),
            grade: f32::from_bits(u32_(r)?),
            sound_id: 0,
            instant: u8_(r)? != 0,
        },
        0x0209 => {
            let (server_time, raw_byte, mask) = (u32_(r)?, u8_(r)?, u32_(r)?);
            if mask >> ACCOUNT_DATA_TYPES != 0 {
                return Err(invalid("account data mask"));
            }
            let times = (0..mask.count_ones())
                .map(|_| u32_(r))
                .collect::<io::Result<_>>()?;
            capture(Packet::AccountDataTimes {
                server_time,
                raw_byte,
                mask,
                times,
            })
        }
        0x020c => {
            let (player, kind, time, decompressed_size) = (u64_(r)?, u32_(r)?, u32_(r)?, u32_(r)?);
            let compressed = body[r.position() as usize..].to_vec();
            r.set_position(body.len() as u64);
            capture(Packet::AccountData {
                player,
                kind,
                time,
                decompressed_size,
                compressed,
            })
        }
        0x0463 => capture(Packet::AccountDataSaved {
            kind: u32_(r)?,
            raw_word: u32_(r)?,
        }),
        _ => return Err(invalid("session metadata opcode")),
    };
    if r.position() as usize != body.len() {
        return Err(invalid("session metadata extent"));
    }
    Ok(event)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `CMSG_TUTORIAL_FLAG`: a flag index.
    TutorialFlag {
        id: u32,
    },
    TutorialClear,
    TutorialReset,
    /// `CMSG_PLAYED_TIME`: 1 asks for the chat line.
    PlayedTime {
        trigger: u8,
    },
    NextCinematicCamera,
    CompleteCinematic,
    /// `CMSG_AREATRIGGER`.
    AreaTrigger {
        trigger_id: u32,
    },
    /// `CMSG_ZONEUPDATE`: AC uses its own position, not this zone.
    ZoneUpdate {
        zone: u32,
    },
    /// `CMSG_SET_ACTIONBAR_TOGGLES`.
    ActionBarToggles {
        toggles: u8,
    },
    /// `CMSG_REQUEST_ACCOUNT_DATA`: one type, 0..8.
    RequestAccountData {
        kind: u32,
    },
    /// `CMSG_UPDATE_ACCOUNT_DATA`: a blob already zlib-compressed by the caller; size 0 erases.
    UpdateAccountData {
        kind: u32,
        time: u32,
        decompressed_size: u32,
        compressed: Vec<u8>,
    },
    /// `CMSG_READY_FOR_ACCOUNT_DATA_TIMES`: empty.
    ReadyForAccountDataTimes,
    /// `CMSG_SET_TITLE`: a `CharTitles` bit index, -1 clears (AC `HandleSetTitleOpcode`).
    SetTitle {
        title: i32,
    },
}

pub fn encode(command: &Command) -> (u32, Vec<u8>) {
    let word = |opcode, value: u32| (opcode, value.to_le_bytes().to_vec());
    match command {
        Command::TutorialFlag { id } => word(0x00fe, *id),
        Command::TutorialClear => (0x00ff, vec![]),
        Command::TutorialReset => (0x0100, vec![]),
        Command::PlayedTime { trigger } => (0x01cc, vec![*trigger]),
        Command::NextCinematicCamera => (0x00fb, vec![]),
        Command::CompleteCinematic => (0x00fc, vec![]),
        Command::AreaTrigger { trigger_id } => word(0x00b4, *trigger_id),
        Command::ZoneUpdate { zone } => word(0x01f4, *zone),
        Command::ActionBarToggles { toggles } => (0x02bf, vec![*toggles]),
        Command::SetTitle { title } => (0x0374, title.to_le_bytes().to_vec()),
        Command::RequestAccountData { kind } => word(0x020a, *kind),
        Command::UpdateAccountData {
            kind,
            time,
            decompressed_size,
            compressed,
        } => {
            let mut body = Vec::with_capacity(12 + compressed.len());
            for value in [kind, time, decompressed_size] {
                body.extend(value.to_le_bytes());
            }
            body.extend(compressed);
            (0x020b, body)
        }
        Command::ReadyForAccountDataTimes => (0x04ff, vec![]),
    }
}

#[cfg(test)]
mod tests;
