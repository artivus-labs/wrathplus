//! The bounded Wrath 3.3.5a world login and character-select protocol.
//!
//! Typed support intentionally stops after bounded character creation / initial-login observation;
//! gameplay consumers use the lossless raw framed transport and supply their own Wrath codecs.
//! The layouts come from AzerothCore's `WorldSocket.cpp`, `ServerPktHeader.h`, and
//! `Player::BuildEnumData`.

use std::io::{self, Cursor, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use flate2::write::ZlibEncoder;
use sha1::{Digest, Sha1};
use wrathplus_srp::wrath_header::{DecrypterHalf, EncrypterHalf, HeaderCrypto};
use wrathplus_srp::{NormalizedString, SESSION_KEY_LENGTH};

/// The 3.3.5a client build accepted by AzerothCore.
pub const WRATH_CLIENT_BUILD: u32 = 12_340;
/// AzerothCore's usual world-server port.
pub const WRATH_WORLD_PORT: u16 = 8085;

const SMSG_AUTH_CHALLENGE: u16 = 0x1ec;
const CMSG_AUTH_SESSION: u32 = 0x1ed;
const SMSG_AUTH_RESPONSE: u16 = 0x1ee;
const SMSG_WARDEN_DATA: u16 = 0x2e6;
const CMSG_CHAR_ENUM: u32 = 0x037;
const SMSG_CHAR_ENUM: u16 = 0x03b;
const CMSG_CHAR_CREATE: u32 = 0x036;
const SMSG_CHAR_CREATE: u16 = 0x03a;
const CMSG_CHAR_DELETE: u32 = 0x038;
const SMSG_CHAR_DELETE: u16 = 0x03c;
const CMSG_PLAYER_LOGIN: u32 = 0x03d;
const SMSG_CHARACTER_LOGIN_FAILED: u16 = 0x041;
const SMSG_LOGIN_SETTIMESPEED: u16 = 0x042;
const SMSG_TUTORIAL_FLAGS: u16 = 0x0fd;
const SMSG_ACCOUNT_DATA_TIMES: u16 = 0x209;
const SMSG_LOGIN_VERIFY_WORLD: u16 = 0x236;
const AUTH_OK: u8 = 0x0c;
const AUTH_WAIT_QUEUE: u8 = 0x1b;
pub const CHAR_CREATE_SUCCESS: u8 = 0x2f;
/// AzerothCore `ResponseCodes::CHAR_CREATE_ERROR`, "Error creating character".
pub const CHAR_CREATE_ERROR: u8 = 0x30;
pub const CHAR_DELETE_SUCCESS: u8 = 0x47;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DIAL_ENDPOINTS: usize = 16;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const QUEUE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_STARTUP_PACKETS: usize = 64;
const MAX_QUEUE_UPDATES: usize = 64;
const MAX_ENUM_PACKETS: usize = 64;
const MAX_CREATE_PACKETS: usize = 64;
const MAX_INITIAL_LOGIN_PACKETS: usize = 512;
const MAX_CHARACTERS: usize = 50;
const MAX_SERVER_PACKET_SIZE: usize = 0x7f_ffff;
const MAX_CLIENT_PACKET_BODY_SIZE: usize = u16::MAX as usize - 4;

/// The world server requires Warden, which this initial Wrath protocol slice does not implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrathWardenRequired;

impl std::fmt::Display for WrathWardenRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "this Wrath server requires Warden, which is not implemented"
        )
    }
}

impl std::error::Error for WrathWardenRequired {}

/// An AzerothCore world-authentication result other than `AUTH_OK` or `AUTH_WAIT_QUEUE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrathWorldAuthReject {
    pub code: u8,
}

impl std::fmt::Display for WrathWorldAuthReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Wrath world server refused the session: result {:#04x}",
            self.code
        )
    }
}

impl std::error::Error for WrathWorldAuthReject {}

/// A normal 3.3.5a `CMSG_CHAR_CREATE` request.
///
/// Appearance values, the race/class combination and every name rule are the server's: the name
/// goes out as the C string 12340's `0x4e0380` copies, cut at a NUL, and an empty one draws
/// AzerothCore's `CHAR_NAME_NO_NAME`. This layer only rejects unplayable ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrathCreateCharacter {
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    /// The final byte read by AzerothCore. Normal character creation uses outfit zero.
    pub outfit_id: u8,
}

/// Local rejection of a malformed or out-of-range character-create request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrathCreateValidationError {
    reason: &'static str,
}

impl std::fmt::Display for WrathCreateValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid Wrath character creation request: {}",
            self.reason
        )
    }
}

impl std::error::Error for WrathCreateValidationError {}

/// A typed non-success result from `SMSG_CHAR_CREATE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrathCreateCharacterReject {
    pub code: u8,
}

impl std::fmt::Display for WrathCreateCharacterReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Wrath server refused character creation: result {:#04x}",
            self.code
        )
    }
}

impl std::error::Error for WrathCreateCharacterReject {}

/// Metadata for one packet observed during the bounded initial-login exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrathInitialPacket {
    pub opcode: u16,
    /// Payload bytes, excluding the server header and two-byte opcode.
    pub body_size: usize,
}

/// Packet evidence collected while entering the world.
///
/// The counters prove only that packets with these opcodes were received. In particular, this is
/// not an object model or evidence that the full world state was decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrathInitialLoginObservation {
    pub packets: Vec<WrathInitialPacket>,
    pub login_verify_world_count: usize,
    pub account_data_times_count: usize,
    pub tutorial_flags_count: usize,
    pub login_set_time_speed_count: usize,
}

/// A typed `SMSG_CHARACTER_LOGIN_FAILED` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrathPlayerLoginReject {
    pub code: u8,
}

impl std::fmt::Display for WrathPlayerLoginReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Wrath server refused player login: result {:#04x}",
            self.code
        )
    }
}

impl std::error::Error for WrathPlayerLoginReject {}

/// One of the 23 character-enumeration equipment slots, including the four bag slots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WrathCharEnumItem {
    pub display_id: u32,
    pub inventory_type: u8,
    /// The visual spell aura chosen from the permanent or temporary enchantment.
    pub enchantment_aura: u32,
}

/// One Wrath `SMSG_CHAR_ENUM` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct WrathCharacter {
    pub guid: u64,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
    pub level: u8,
    pub zone: u32,
    pub map: u32,
    pub position: [f32; 3],
    pub guild_id: u32,
    pub flags: u32,
    /// `CHAR_CUSTOMIZE_FLAG_*`, distinct from the character flags.
    pub customization_flags: u32,
    pub first_login: bool,
    pub pet_display_id: u32,
    pub pet_level: u32,
    pub pet_family: u32,
    pub equipment: [WrathCharEnumItem; 23],
}

/// One complete server-to-client Wrath world packet, without its encrypted framing header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrathServerPacket {
    pub opcode: u16,
    pub body: Vec<u8>,
}

type RawPacket = WrathServerPacket;

/// Read half of a split [`WrathWorldSession`], retaining the server-to-client RC4 stream.
pub struct WrathWorldReader {
    stream: TcpStream,
    decrypter: DecrypterHalf,
    failed: bool,
}

/// Write half of a split [`WrathWorldSession`], retaining the client-to-server RC4 stream.
pub struct WrathWorldWriter {
    stream: TcpStream,
    encrypter: EncrypterHalf,
    failed: bool,
}

/// A receive failed after consuming bytes from the current encrypted frame.
///
/// The reader is poisoned after this error; its source retains the framing or socket failure that
/// made continuing unsafe.
#[derive(Debug)]
pub struct WrathPartialFrameRead {
    pub consumed: usize,
}

impl std::fmt::Display for WrathPartialFrameRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Wrath partial frame read failed after {} byte(s)",
            self.consumed
        )
    }
}

impl std::error::Error for WrathPartialFrameRead {}

/// An authenticated Wrath world connection with bounded character-select and login observation.
pub struct WrathWorldSession {
    account_expansion: Option<u8>,
    stream: TcpStream,
    crypto: HeaderCrypto,
    read_failed: bool,
    write_failed: bool,
    /// The auth bootstrap's `SMSG_ADDON_INFO` / `SMSG_CLIENTCACHE_VERSION`, which AzerothCore sends
    /// right after `SMSG_AUTH_RESPONSE` (`WorldSession::SendAddonsInfo`, `SendClientCacheVersion`)
    /// and the startup and roster loops would otherwise skip ([`crate::wrath_bootstrap`]).
    bootstrap: Vec<WrathServerPacket>,
    /// The addons this session's `SMSG_ADDON_INFO` hid, once it arrived; kept across re-logins.
    addon_reply: Option<Vec<String>>,
}

impl WrathWorldSession {
    /// Connect, authenticate as `username`, and wait for admission to `realm_id`.
    pub fn connect(
        addr: impl ToSocketAddrs,
        username: &str,
        session_key: [u8; SESSION_KEY_LENGTH],
        realm_id: u32,
    ) -> Result<Self> {
        let username = NormalizedString::new(username)
            .map_err(|e| anyhow!("invalid username for Wrath world login: {e}"))?;
        let client_seed = wrathplus_srp::world_client_seed();
        Self::connect_with_seed(addr, &username, session_key, realm_id, client_seed)
    }

    fn connect_with_seed(
        addr: impl ToSocketAddrs,
        username: &NormalizedString,
        session_key: [u8; SESSION_KEY_LENGTH],
        realm_id: u32,
        client_seed: u32,
    ) -> Result<Self> {
        let mut stream = connect_bounded(addr)?;
        stream
            .set_nodelay(true)
            .context("disabling Nagle on Wrath world socket")?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .context("setting Wrath world read timeout")?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .context("setting Wrath world write timeout")?;

        let challenge = recv_raw(&mut stream, None).context("reading SMSG_AUTH_CHALLENGE")?;
        if challenge.opcode != SMSG_AUTH_CHALLENGE {
            bail!(
                "expected SMSG_AUTH_CHALLENGE, got opcode {:#x}",
                challenge.opcode
            );
        }
        if challenge.body.len() != 40 {
            bail!(
                "SMSG_AUTH_CHALLENGE has {} bytes, expected 40",
                challenge.body.len()
            );
        }
        let server_seed = u32::from_le_bytes(challenge.body[4..8].try_into().unwrap());
        let proof = world_proof(username.as_ref(), client_seed, server_seed, &session_key);
        let body = auth_session_body(username.as_ref(), realm_id, client_seed, &proof)?;
        send_raw(&mut stream, None, CMSG_AUTH_SESSION, &body)
            .context("sending CMSG_AUTH_SESSION")?;

        let crypto = HeaderCrypto::from_session_key(session_key);
        let mut session = Self {
            account_expansion: None,
            stream,
            crypto,
            read_failed: false,
            write_failed: false,
            bootstrap: Vec::new(),
            addon_reply: None,
        };
        let mut queue_updates = 0usize;
        for _ in 0..MAX_STARTUP_PACKETS {
            let packet = session.recv_raw()?;
            match packet.opcode {
                SMSG_AUTH_RESPONSE => {
                    let result = *packet
                        .body
                        .first()
                        .ok_or_else(|| anyhow!("empty SMSG_AUTH_RESPONSE"))?;
                    session.account_expansion =
                        auth_response_expansion(&packet.body).or(session.account_expansion);
                    match result {
                        AUTH_OK => {
                            if packet.body.len() != 11 {
                                bail!("AUTH_OK has {} bytes, expected 11", packet.body.len());
                            }
                            return Ok(session);
                        }
                        AUTH_WAIT_QUEUE => {
                            parse_queue_position(&packet.body)?;
                            queue_updates += 1;
                            if queue_updates > MAX_QUEUE_UPDATES {
                                bail!("Wrath login queue exceeded {MAX_QUEUE_UPDATES} updates");
                            }
                            session
                                .stream
                                .set_read_timeout(Some(QUEUE_TIMEOUT))
                                .context("setting bounded Wrath login-queue timeout")?;
                        }
                        code => return Err(WrathWorldAuthReject { code }.into()),
                    }
                }
                SMSG_WARDEN_DATA => return Err(WrathWardenRequired.into()),
                // AzerothCore may interleave cache/account setup packets.
                _ => session.keep_bootstrap(packet),
            }
        }
        bail!("no SMSG_AUTH_RESPONSE within {MAX_STARTUP_PACKETS} startup packets")
    }

    /// Keep a bootstrap packet the select-screen loops would skip; drop anything else.
    fn keep_bootstrap(&mut self, packet: WrathServerPacket) {
        if crate::wrath_bootstrap::handles(packet.opcode) {
            if packet.opcode == crate::wrath_bootstrap::SMSG_ADDON_INFO {
                self.addon_reply = Some(crate::wrath_bootstrap::secure_addons(&packet.body));
            }
            self.bootstrap.push(packet);
        }
    }

    /// The bootstrap packets kept so far, in arrival order, for the gameplay stream to publish.
    pub fn take_bootstrap_packets(&mut self) -> Vec<WrathServerPacket> {
        std::mem::take(&mut self.bootstrap)
    }

    /// The addons this session's `SMSG_ADDON_INFO` marked secure; `None` if it never arrived.
    pub fn addon_info(&self) -> Option<Vec<String>> {
        self.addon_reply.clone()
    }

    /// Raw authenticated account expansion; not inferred from client build or character level.
    pub fn account_expansion(&self) -> Option<u8> {
        self.account_expansion
    }

    /// Request and parse one bounded Wrath character roster.
    pub fn enumerate_characters(&mut self) -> Result<Vec<WrathCharacter>> {
        self.stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .context("restoring Wrath character-enumeration timeout")?;
        self.send_raw(CMSG_CHAR_ENUM, &[])
            .context("sending CMSG_CHAR_ENUM")?;

        for _ in 0..MAX_ENUM_PACKETS {
            let packet = self.recv_raw()?;
            match packet.opcode {
                SMSG_CHAR_ENUM => return parse_character_enum(&packet.body),
                SMSG_WARDEN_DATA => return Err(WrathWardenRequired.into()),
                // Only a bounded set of startup packets is ignored.
                _ => self.keep_bootstrap(packet),
            }
        }
        bail!("no SMSG_CHAR_ENUM within {MAX_ENUM_PACKETS} packets")
    }

    /// Alias matching the existing vanilla session API.
    pub fn char_enum(&mut self) -> Result<Vec<WrathCharacter>> {
        self.enumerate_characters()
    }

    /// Validate and submit a normal character-create request, waiting for its one-byte result.
    pub fn create_character(&mut self, request: &WrathCreateCharacter) -> Result<()> {
        let code = self.create_character_result(request)?;
        parse_create_character_result(&[code])
    }

    /// General select-screen operation: server refusals are raw results, not transport errors. A
    /// request this layer cannot encode sends nothing and answers `CHAR_CREATE_ERROR`, a refusal
    /// like the server's, never a session error.
    pub fn create_character_result(&mut self, request: &WrathCreateCharacter) -> Result<u8> {
        let Ok(body) = create_character_body(request) else {
            return Ok(CHAR_CREATE_ERROR);
        };
        self.send_raw(CMSG_CHAR_CREATE, &body)
            .context("sending CMSG_CHAR_CREATE")?;
        self.character_action_result(SMSG_CHAR_CREATE)
    }

    /// Delete uses a full little-endian GUID (AzerothCore CharacterHandler.cpp).
    pub fn delete_character_result(&mut self, guid: u64) -> Result<u8> {
        self.send_raw(CMSG_CHAR_DELETE, &delete_character_body(guid))
            .context("sending CMSG_CHAR_DELETE")?;
        self.character_action_result(SMSG_CHAR_DELETE)
    }

    fn character_action_result(&mut self, opcode: u16) -> Result<u8> {
        for _ in 0..MAX_CREATE_PACKETS {
            let packet = self.recv_raw()?;
            match packet.opcode {
                op if op == opcode => {
                    return parse_single_result(&packet.body, "character action result")
                }
                SMSG_WARDEN_DATA => return Err(WrathWardenRequired.into()),
                _ => self.keep_bootstrap(packet),
            }
        }
        bail!("no character action result {opcode:#x} within {MAX_CREATE_PACKETS} packets")
    }

    /// Send `CMSG_PLAYER_LOGIN` and observe the bounded initial packet sequence.
    ///
    /// Collection ends at the first `SMSG_LOGIN_SETTIMESPEED` after
    /// `SMSG_LOGIN_VERIFY_WORLD`, the AzerothCore initial-packet marker. Bodies are intentionally
    /// not decoded as vanilla objects; only opcode and payload size are retained.
    pub fn observe_initial_login(&mut self, guid: u64) -> Result<WrathInitialLoginObservation> {
        self.begin_player_login(guid)?;

        let mut observation = WrathInitialLoginObservation::new();
        for _ in 0..MAX_INITIAL_LOGIN_PACKETS {
            let packet = self.recv_raw()?;
            if packet.opcode == SMSG_WARDEN_DATA {
                return Err(WrathWardenRequired.into());
            }
            if packet.opcode == SMSG_CHARACTER_LOGIN_FAILED {
                let code = parse_single_result(&packet.body, "SMSG_CHARACTER_LOGIN_FAILED")?;
                return Err(WrathPlayerLoginReject { code }.into());
            }
            if observation.observe(&packet)? {
                return Ok(observation);
            }
        }
        bail!(
            "initial Wrath login did not reach SMSG_LOGIN_VERIFY_WORLD and subsequent \
             SMSG_LOGIN_SETTIMESPEED within {MAX_INITIAL_LOGIN_PACKETS} packets"
        )
    }

    /// Send `CMSG_PLAYER_LOGIN` without reading or discarding any server packets.
    ///
    /// Callers that need the lossless gameplay stream should call this and then use [`Self::recv_raw`]
    /// or [`Self::into_split`]. [`Self::observe_initial_login`] remains the compatibility helper that
    /// consumes the bounded initial sequence.
    pub fn begin_player_login(&mut self, guid: u64) -> Result<()> {
        self.send_raw(CMSG_PLAYER_LOGIN, &guid.to_le_bytes())
            .context("sending CMSG_PLAYER_LOGIN")
    }

    /// Receive one complete framed packet, rejecting Warden rather than exposing an unusable session.
    ///
    /// A timeout before any byte of a new frame is consumed may be retried. Any error after a partial
    /// header or body permanently fails this receive side: Wrath's RC4 header stream and TCP framing
    /// cannot safely be treated as a fresh packet after such an error.
    pub fn recv_raw(&mut self) -> Result<WrathServerPacket> {
        let Some(timeout) = self
            .stream
            .read_timeout()
            .context("reading Wrath world read timeout")?
        else {
            let packet = recv_raw_fail_closed(
                &mut self.stream,
                Some(self.crypto.decrypter()),
                &mut self.read_failed,
            )?;
            return reject_warden(packet);
        };
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| anyhow!("Wrath world read timeout exceeds monotonic clock range"))?;
        self.recv_raw_until(deadline)
    }

    /// Receive one complete framed packet before an absolute monotonic deadline.
    ///
    /// The same deadline bounds the complete encrypted header and body. A timeout after any frame
    /// byte is consumed permanently fails the receive side; a timeout while still waiting for the
    /// first byte may be retried.
    pub fn recv_raw_until(&mut self, deadline: Instant) -> Result<WrathServerPacket> {
        let packet = recv_raw_fail_closed_until(
            &mut self.stream,
            Some(self.crypto.decrypter()),
            &mut self.read_failed,
            deadline,
        )?;
        reject_warden(packet)
    }

    /// Frame and send one raw client packet with the Wrath client-to-server header cipher.
    ///
    /// Bodies are bounded by the protocol's 16-bit client size field. A failed socket write poisons
    /// this write side because the number of encrypted header bytes accepted by TCP is unknowable.
    pub fn send_raw(&mut self, opcode: u32, body: &[u8]) -> Result<()> {
        send_raw_fail_closed(
            &mut self.stream,
            Some(self.crypto.encrypter()),
            &mut self.write_failed,
            opcode,
            body,
        )
    }

    /// Set the socket read timeout. The authenticated session defaults to ten seconds.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.stream
            .set_read_timeout(timeout)
            .context("setting Wrath world read timeout")
    }

    /// Set the socket write timeout. The authenticated session defaults to ten seconds.
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.stream
            .set_write_timeout(timeout)
            .context("setting Wrath world write timeout")
    }

    /// Split the socket and directional Wrath crypto without resetting either RC4 stream.
    pub fn into_split(self) -> Result<(WrathWorldReader, WrathWorldWriter)> {
        let read_stream = self
            .stream
            .try_clone()
            .context("cloning Wrath world socket for split")?;
        let (encrypter, decrypter) = self.crypto.split();
        Ok((
            WrathWorldReader {
                stream: read_stream,
                decrypter,
                failed: self.read_failed,
            },
            WrathWorldWriter {
                stream: self.stream,
                encrypter,
                failed: self.write_failed,
            },
        ))
    }
}

impl WrathWorldReader {
    /// Receive one complete raw server packet; see [`WrathWorldSession::recv_raw`] for retry safety.
    pub fn recv_raw(&mut self) -> Result<WrathServerPacket> {
        let Some(timeout) = self
            .stream
            .read_timeout()
            .context("reading split Wrath world read timeout")?
        else {
            let packet = recv_raw_fail_closed(
                &mut self.stream,
                Some(&mut self.decrypter),
                &mut self.failed,
            )?;
            return reject_warden(packet);
        };
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| anyhow!("Wrath world read timeout exceeds monotonic clock range"))?;
        self.recv_raw_until(deadline)
    }

    /// Receive one complete framed packet before an absolute monotonic deadline.
    ///
    /// The same deadline bounds the complete encrypted header and body. See
    /// [`WrathWorldSession::recv_raw_until`] for retry and poisoning semantics.
    pub fn recv_raw_until(&mut self, deadline: Instant) -> Result<WrathServerPacket> {
        let packet = recv_raw_fail_closed_until(
            &mut self.stream,
            Some(&mut self.decrypter),
            &mut self.failed,
            deadline,
        )?;
        reject_warden(packet)
    }

    /// Receive one packet while keeping the socket timeout as an idle poll before the first byte.
    ///
    /// Once any frame byte arrives, `frame_timeout` replaces that poll timeout and bounds the whole
    /// header and body. This lets a caller poll side channels frequently without treating an
    /// ordinarily fragmented packet as if every frame had the same short deadline.
    pub fn recv_raw_with_frame_timeout(
        &mut self,
        frame_timeout: Duration,
    ) -> Result<WrathServerPacket> {
        let idle_timeout = self
            .stream
            .read_timeout()
            .context("reading split Wrath world idle timeout")?;
        let packet = recv_raw_fail_closed_with_frame_timeout(
            &mut self.stream,
            Some(&mut self.decrypter),
            &mut self.failed,
            idle_timeout,
            frame_timeout,
        )?;
        reject_warden(packet)
    }

    /// Set the read timeout; `None` permits an indefinitely quiet gameplay connection.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.stream
            .set_read_timeout(timeout)
            .context("setting split Wrath world read timeout")
    }
}

impl WrathWorldWriter {
    /// Frame and send one raw client packet while preserving the split encryption stream.
    pub fn send_raw(&mut self, opcode: u32, body: &[u8]) -> Result<()> {
        send_raw_fail_closed(
            &mut self.stream,
            Some(&mut self.encrypter),
            &mut self.failed,
            opcode,
            body,
        )
    }

    /// Set the socket write timeout. The authenticated session defaults to ten seconds.
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        self.stream
            .set_write_timeout(timeout)
            .context("setting split Wrath world write timeout")
    }
}

fn reject_warden(packet: WrathServerPacket) -> Result<WrathServerPacket> {
    if packet.opcode == SMSG_WARDEN_DATA {
        Err(WrathWardenRequired.into())
    } else {
        Ok(packet)
    }
}

impl WrathInitialLoginObservation {
    fn new() -> Self {
        Self {
            packets: Vec::new(),
            login_verify_world_count: 0,
            account_data_times_count: 0,
            tutorial_flags_count: 0,
            login_set_time_speed_count: 0,
        }
    }

    fn observe(&mut self, packet: &RawPacket) -> Result<bool> {
        match packet.opcode {
            SMSG_LOGIN_VERIFY_WORLD => {
                require_body_size(packet, 20, "SMSG_LOGIN_VERIFY_WORLD")?;
                self.login_verify_world_count += 1;
            }
            SMSG_ACCOUNT_DATA_TIMES => {
                validate_account_data_times(&packet.body)?;
                self.account_data_times_count += 1;
            }
            SMSG_TUTORIAL_FLAGS => {
                require_body_size(packet, 32, "SMSG_TUTORIAL_FLAGS")?;
                self.tutorial_flags_count += 1;
            }
            SMSG_LOGIN_SETTIMESPEED => {
                require_body_size(packet, 12, "SMSG_LOGIN_SETTIMESPEED")?;
                self.login_set_time_speed_count += 1;
            }
            _ => {}
        }
        self.packets.push(WrathInitialPacket {
            opcode: packet.opcode,
            body_size: packet.body.len(),
        });
        Ok(self.login_verify_world_count != 0 && self.login_set_time_speed_count != 0)
    }
}

fn create_character_body(request: &WrathCreateCharacter) -> Result<Vec<u8>> {
    let name = request
        .name
        .as_bytes()
        .split(|&b| b == 0)
        .next()
        .unwrap_or_default();
    let refusals = [
        (
            !matches!(request.race, 1..=8 | 10 | 11),
            "race is not a playable 3.3.5 race",
        ),
        (
            !matches!(request.class, 1..=9 | 11),
            "class is not a playable 3.3.5 class",
        ),
        (request.gender > 1, "gender must be 0 or 1"),
        (
            request.outfit_id != 0,
            "normal character creation requires outfit_id 0",
        ),
    ];
    if let Some(&(_, reason)) = refusals.iter().find(|(bad, _)| *bad) {
        return Err(WrathCreateValidationError { reason }.into());
    }

    let mut body = Vec::with_capacity(name.len() + 10);
    body.extend_from_slice(name);
    body.push(0);
    body.extend_from_slice(&[
        request.race,
        request.class,
        request.gender,
        request.skin,
        request.face,
        request.hair_style,
        request.hair_color,
        request.facial_hair,
        request.outfit_id,
    ]);
    Ok(body)
}

fn delete_character_body(guid: u64) -> [u8; 8] {
    guid.to_le_bytes()
}

fn parse_single_result(body: &[u8], packet_name: &str) -> Result<u8> {
    if body.len() != 1 {
        bail!("{packet_name} has {} bytes, expected 1", body.len());
    }
    Ok(body[0])
}

fn parse_create_character_result(body: &[u8]) -> Result<()> {
    let code = parse_single_result(body, "SMSG_CHAR_CREATE")?;
    if code == CHAR_CREATE_SUCCESS {
        Ok(())
    } else {
        Err(WrathCreateCharacterReject { code }.into())
    }
}

fn require_body_size(packet: &RawPacket, expected: usize, packet_name: &str) -> Result<()> {
    if packet.body.len() != expected {
        bail!(
            "{packet_name} has {} bytes, expected {expected}",
            packet.body.len()
        );
    }
    Ok(())
}

fn validate_account_data_times(body: &[u8]) -> Result<()> {
    if body.len() < 9 {
        bail!("truncated SMSG_ACCOUNT_DATA_TIMES: {} bytes", body.len());
    }
    let mask = u32::from_le_bytes(body[5..9].try_into().unwrap());
    let expected = 9 + mask.count_ones() as usize * 4;
    if body.len() != expected {
        bail!(
            "SMSG_ACCOUNT_DATA_TIMES has {} bytes, expected {expected} for mask {mask:#010x}",
            body.len()
        );
    }
    Ok(())
}

fn connect_bounded(addr: impl ToSocketAddrs) -> Result<TcpStream> {
    let addrs: Vec<_> = addr
        .to_socket_addrs()
        .context("resolving Wrath world address")?
        .take(MAX_DIAL_ENDPOINTS)
        .collect();
    if addrs.is_empty() {
        bail!("Wrath world address resolved to no endpoints");
    }
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some((addr, error)),
        }
    }
    let (addr, error) = last.expect("non-empty address list");
    Err(error).with_context(|| format!("connecting to Wrath world server at {addr}"))
}

fn world_proof(
    account: &str,
    client_seed: u32,
    server_seed: u32,
    session_key: &[u8; SESSION_KEY_LENGTH],
) -> [u8; 20] {
    let mut sha = Sha1::new();
    sha.update(account.as_bytes());
    sha.update(0u32.to_le_bytes());
    sha.update(client_seed.to_le_bytes());
    sha.update(server_seed.to_le_bytes());
    sha.update(session_key);
    sha.finalize().into()
}

/// The stock client's addon block, its 23 secure built-ins, which `SMSG_ADDON_INFO` answers record
/// for record ([`crate::wrath_bootstrap`]); the banned-cache trailer is 0, as on a fresh install.
fn stock_addon_block() -> Result<Vec<u8>> {
    let plain = crate::types::addons::addon_block(&crate::types::STOCK_SECURE_ADDONS, 0);
    let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain)?;
    let compressed = encoder.finish()?;
    let mut out = Vec::with_capacity(4 + compressed.len());
    out.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

fn auth_session_body(
    account: &str,
    realm_id: u32,
    client_seed: u32,
    proof: &[u8; 20],
) -> Result<Vec<u8>> {
    let addons = stock_addon_block()?;
    let mut body = Vec::with_capacity(64 + account.len() + addons.len());
    body.extend_from_slice(&WRATH_CLIENT_BUILD.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // LoginServerID
    body.extend_from_slice(account.as_bytes());
    body.push(0);
    body.extend_from_slice(&0u32.to_le_bytes()); // LoginServerType
    body.extend_from_slice(&client_seed.to_le_bytes()); // LocalChallenge
    body.extend_from_slice(&0u32.to_le_bytes()); // RegionID
    body.extend_from_slice(&0u32.to_le_bytes()); // BattlegroupID
    body.extend_from_slice(&realm_id.to_le_bytes());
    body.extend_from_slice(&0u64.to_le_bytes()); // DosResponse
    body.extend_from_slice(proof);
    body.extend_from_slice(&addons);
    Ok(body)
}

fn send_raw(
    stream: &mut impl Write,
    encrypter: Option<&mut EncrypterHalf>,
    opcode: u32,
    body: &[u8],
) -> Result<()> {
    let size = body
        .len()
        .checked_add(4)
        .ok_or_else(|| anyhow!("client packet size overflow"))?;
    let size = u16::try_from(size)
        .map_err(|_| anyhow!("client packet body too large: {} bytes", body.len()))?;
    let size = size.to_be_bytes();
    let opcode = opcode.to_le_bytes();
    let mut header = [size[0], size[1], opcode[0], opcode[1], opcode[2], opcode[3]];
    if let Some(encrypter) = encrypter {
        encrypter.encrypt(&mut header);
    }
    stream.write_all(&header)?;
    stream.write_all(body)?;
    Ok(())
}

fn send_raw_fail_closed(
    stream: &mut impl Write,
    encrypter: Option<&mut EncrypterHalf>,
    failed: &mut bool,
    opcode: u32,
    body: &[u8],
) -> Result<()> {
    if *failed {
        bail!("Wrath world writer is unusable after a previous frame write failed");
    }
    if body.len() > MAX_CLIENT_PACKET_BODY_SIZE {
        bail!(
            "client packet body too large: {} bytes (maximum {MAX_CLIENT_PACKET_BODY_SIZE})",
            body.len()
        );
    }
    let result = send_raw(stream, encrypter, opcode, body);
    if result.is_err() {
        *failed = true;
    }
    result
}

fn recv_raw(stream: &mut impl Read, decrypter: Option<&mut DecrypterHalf>) -> Result<RawPacket> {
    let mut first = [0u8; 4];
    stream
        .read_exact(&mut first)
        .context("reading Wrath server header")?;

    let (size, opcode) = if let Some(decrypter) = decrypter {
        // The fifth byte, when present, belongs to the same RC4 stream and must be consumed only
        // after the first four reveal the large-packet high bit.
        decrypter.decrypt(&mut first);
        decode_server_header(stream, first, Some(decrypter))?
    } else {
        decode_server_header(stream, first, None)?
    };
    if size < 2 {
        bail!("Wrath server header size {size} is smaller than its opcode");
    }
    if size > MAX_SERVER_PACKET_SIZE {
        bail!("Wrath server packet size {size} exceeds protocol limit");
    }
    let body_len = size - 2;
    let mut body = vec![0u8; body_len];
    stream
        .read_exact(&mut body)
        .with_context(|| format!("reading Wrath body for opcode {opcode:#x} ({body_len} bytes)"))?;
    Ok(RawPacket { opcode, body })
}

fn recv_raw_fail_closed(
    stream: &mut impl Read,
    decrypter: Option<&mut DecrypterHalf>,
    failed: &mut bool,
) -> Result<RawPacket> {
    if *failed {
        bail!("Wrath world reader is unusable after a partial frame read failed");
    }

    let mut reader = FrameReader::new(stream);
    let result = recv_raw(&mut reader, decrypter);
    match &result {
        Err(error) if reader.consumed != 0 => {
            *failed = true;
            if error.downcast_ref::<io::Error>().is_some() {
                return result.context(WrathPartialFrameRead {
                    consumed: reader.consumed,
                });
            }
        }
        _ => {}
    }
    result
}

fn recv_raw_fail_closed_until(
    stream: &mut TcpStream,
    decrypter: Option<&mut DecrypterHalf>,
    failed: &mut bool,
    deadline: Instant,
) -> Result<RawPacket> {
    let original_timeout = stream
        .read_timeout()
        .context("reading Wrath world socket timeout before deadline receive")?;
    let result = {
        let mut reader = DeadlineReader::new(stream, deadline);
        recv_raw_fail_closed(&mut reader, decrypter, failed)
    };
    if let Err(error) = stream.set_read_timeout(original_timeout) {
        *failed = true;
        return Err(error).context("restoring Wrath world socket timeout after deadline receive");
    }
    result
}

fn recv_raw_fail_closed_with_frame_timeout(
    stream: &mut TcpStream,
    decrypter: Option<&mut DecrypterHalf>,
    failed: &mut bool,
    idle_timeout: Option<Duration>,
    frame_timeout: Duration,
) -> Result<RawPacket> {
    let result = {
        let mut reader = FrameDeadlineReader::new(stream, idle_timeout, frame_timeout);
        recv_raw_fail_closed(&mut reader, decrypter, failed)
    };
    if let Err(error) = stream.set_read_timeout(idle_timeout) {
        *failed = true;
        return Err(error).context("restoring Wrath world socket idle timeout after frame receive");
    }
    result
}

struct DeadlineReader<'a> {
    stream: &'a mut TcpStream,
    deadline: Instant,
}

impl<'a> DeadlineReader<'a> {
    fn new(stream: &'a mut TcpStream, deadline: Instant) -> Self {
        Self { stream, deadline }
    }
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "Wrath frame deadline elapsed")
            })?;
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(buffer)
    }
}

struct FrameDeadlineReader<'a> {
    stream: &'a mut TcpStream,
    idle_timeout: Option<Duration>,
    frame_timeout: Duration,
    deadline: Option<Instant>,
}

impl<'a> FrameDeadlineReader<'a> {
    fn new(
        stream: &'a mut TcpStream,
        idle_timeout: Option<Duration>,
        frame_timeout: Duration,
    ) -> Self {
        Self {
            stream,
            idle_timeout,
            frame_timeout,
            deadline: None,
        }
    }
}

impl Read for FrameDeadlineReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let timeout = match self.deadline {
            Some(deadline) => Some(
                deadline
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "Wrath frame deadline elapsed")
                    })?,
            ),
            None => self.idle_timeout,
        };
        self.stream.set_read_timeout(timeout)?;
        let read = self.stream.read(buffer)?;
        if read != 0 && self.deadline.is_none() {
            self.deadline = Some(Instant::now().checked_add(self.frame_timeout).ok_or_else(
                || {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Wrath frame timeout exceeds monotonic clock range",
                    )
                },
            )?);
        }
        Ok(read)
    }
}

struct FrameReader<'a, R> {
    inner: &'a mut R,
    consumed: usize,
}

impl<'a, R> FrameReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self { inner, consumed: 0 }
    }
}

impl<R: Read> Read for FrameReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.consumed += read;
        Ok(read)
    }
}

fn decode_server_header(
    stream: &mut impl Read,
    first: [u8; 4],
    decrypter: Option<&mut DecrypterHalf>,
) -> Result<(usize, u16)> {
    if first[0] & 0x80 == 0 {
        let size = usize::from(u16::from_be_bytes([first[0], first[1]]));
        return Ok((size, u16::from_le_bytes([first[2], first[3]])));
    }

    let mut last = [0u8; 1];
    stream
        .read_exact(&mut last)
        .context("reading fifth byte of large Wrath server header")?;
    if let Some(decrypter) = decrypter {
        decrypter.decrypt(&mut last);
    }
    let size =
        (usize::from(first[0] & 0x7f) << 16) | (usize::from(first[1]) << 8) | usize::from(first[2]);
    Ok((size, u16::from_le_bytes([first[3], last[0]])))
}

/// AzerothCore AuthHandler.cpp::SendAuthResponse: result + billing u32/u8/u32 + expansion.
/// Short queue updates do not carry billing/expansion; retain absence rather than zero.
fn auth_response_expansion(body: &[u8]) -> Option<u8> {
    match body.first().copied()? {
        AUTH_OK if body.len() >= 11 => body.get(10).copied(),
        AUTH_WAIT_QUEUE if body.len() >= 16 => body.get(10).copied(),
        _ => None,
    }
}

fn parse_queue_position(body: &[u8]) -> Result<u32> {
    // Full AUTH_WAIT_QUEUE: result, billing remaining/flags/rested/expansion, position, migration.
    if body.len() < 16 {
        bail!("truncated AUTH_WAIT_QUEUE: {} bytes", body.len());
    }
    Ok(u32::from_le_bytes(body[11..15].try_into().unwrap()))
}

fn parse_character_enum(body: &[u8]) -> Result<Vec<WrathCharacter>> {
    let (&count, entries) = body
        .split_first()
        .ok_or_else(|| anyhow!("empty SMSG_CHAR_ENUM"))?;
    let count = usize::from(count);
    if count > MAX_CHARACTERS {
        bail!("SMSG_CHAR_ENUM count {count} exceeds {MAX_CHARACTERS}");
    }
    let mut cursor = Cursor::new(entries);
    let mut characters = Vec::with_capacity(count);
    for index in 0..count {
        characters.push(
            read_character(&mut cursor)
                .with_context(|| format!("parsing Wrath character {index}"))?,
        );
    }
    if cursor.position() != entries.len() as u64 {
        bail!(
            "SMSG_CHAR_ENUM has {} trailing bytes",
            entries.len() as u64 - cursor.position()
        );
    }
    Ok(characters)
}

fn read_character(r: &mut Cursor<&[u8]>) -> Result<WrathCharacter> {
    let guid = read_u64(r)?;
    let name = read_cstring(r, 63)?;
    let race = read_u8(r)?;
    let class = read_u8(r)?;
    let gender = read_u8(r)?;
    let skin = read_u8(r)?;
    let face = read_u8(r)?;
    let hair_style = read_u8(r)?;
    let hair_color = read_u8(r)?;
    let facial_hair = read_u8(r)?;
    let level = read_u8(r)?;
    let zone = read_u32(r)?;
    let map = read_u32(r)?;
    let position = [read_f32(r)?, read_f32(r)?, read_f32(r)?];
    let guild_id = read_u32(r)?;
    let flags = read_u32(r)?;
    let customization_flags = read_u32(r)?;
    let first_login = read_u8(r)? != 0;
    let pet_display_id = read_u32(r)?;
    let pet_level = read_u32(r)?;
    let pet_family = read_u32(r)?;
    let mut equipment = [WrathCharEnumItem::default(); 23];
    for item in &mut equipment {
        item.display_id = read_u32(r)?;
        item.inventory_type = read_u8(r)?;
        item.enchantment_aura = read_u32(r)?;
    }
    Ok(WrathCharacter {
        guid,
        name,
        race,
        class,
        gender,
        skin,
        face,
        hair_style,
        hair_color,
        facial_hair,
        level,
        zone,
        map,
        position,
        guild_id,
        flags,
        customization_flags,
        first_login,
        pet_display_id,
        pet_level,
        pet_family,
        equipment,
    })
}

fn read_exact<const N: usize>(r: &mut impl Read) -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    r.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_u8(r: &mut impl Read) -> Result<u8> {
    Ok(read_exact::<1>(r)?[0])
}

fn read_u32(r: &mut impl Read) -> Result<u32> {
    Ok(u32::from_le_bytes(read_exact(r)?))
}

fn read_u64(r: &mut impl Read) -> Result<u64> {
    Ok(u64::from_le_bytes(read_exact(r)?))
}

fn read_f32(r: &mut impl Read) -> Result<f32> {
    Ok(f32::from_le_bytes(read_exact(r)?))
}

fn read_cstring(r: &mut impl Read, max_len: usize) -> Result<String> {
    let mut bytes = Vec::new();
    loop {
        let byte = read_u8(r).context("truncated character name")?;
        if byte == 0 {
            return String::from_utf8(bytes).context("character name is not UTF-8");
        }
        if bytes.len() == max_len {
            bail!("character name exceeds {max_len} bytes");
        }
        bytes.push(byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn auth_expansion_keeps_absence_zero_and_unknown_server_values() {
        assert_eq!(auth_response_expansion(&[]), None);
        for code in [AUTH_OK, AUTH_WAIT_QUEUE] {
            let len = if code == AUTH_OK { 11 } else { 16 };
            let mut body = vec![0; len];
            body[0] = code;
            for byte in [0, 1, 2, 255] {
                body[10] = byte;
                assert_eq!(auth_response_expansion(&body), Some(byte));
            }
            for end in 0..len {
                assert_eq!(auth_response_expansion(&body[..end]), None);
            }
            body[0] = 0x0d;
            assert_eq!(auth_response_expansion(&body), None);
        }
        assert_eq!(loopback_session().0.account_expansion(), None);
    }

    const TRANSPORT_KEY: [u8; SESSION_KEY_LENGTH] = [0x5a; SESSION_KEY_LENGTH];

    fn loopback_session() -> (WrathWorldSession, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        client.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        client.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        (
            WrathWorldSession {
                account_expansion: None,
                stream: client,
                crypto: HeaderCrypto::from_session_key(TRANSPORT_KEY),
                read_failed: false,
                write_failed: false,
                bootstrap: Vec::new(),
                addon_reply: None,
            },
            server,
        )
    }

    fn send_server_packet(
        stream: &mut TcpStream,
        crypto: &mut DecrypterHalf,
        opcode: u16,
        body: &[u8],
    ) {
        let size = body.len() + 2;
        let mut header = if size > 0x7fff {
            vec![
                0x80 | ((size >> 16) as u8),
                (size >> 8) as u8,
                size as u8,
                opcode as u8,
                (opcode >> 8) as u8,
            ]
        } else {
            let size = (size as u16).to_be_bytes();
            vec![size[0], size[1], opcode as u8, (opcode >> 8) as u8]
        };
        crypto.decrypt(&mut header);
        stream.write_all(&header).unwrap();
        stream.write_all(body).unwrap();
    }

    fn server_packet_bytes(crypto: &mut DecrypterHalf, opcode: u16, body: &[u8]) -> Vec<u8> {
        let size = body.len() + 2;
        assert!(size <= 0x7fff);
        let size = (size as u16).to_be_bytes();
        let mut wire = vec![size[0], size[1], opcode as u8, (opcode >> 8) as u8];
        crypto.decrypt(&mut wire);
        wire.extend_from_slice(body);
        wire
    }

    fn recv_client_packet(stream: &mut TcpStream, crypto: &mut EncrypterHalf) -> (u32, Vec<u8>) {
        let mut header = [0; 6];
        stream.read_exact(&mut header).unwrap();
        crypto.encrypt(&mut header);
        let size = usize::from(u16::from_be_bytes([header[0], header[1]]));
        let opcode = u32::from_le_bytes(header[2..6].try_into().unwrap());
        let mut body = vec![0; size - 4];
        stream.read_exact(&mut body).unwrap();
        (opcode, body)
    }

    fn enum_fixture() -> Vec<u8> {
        let mut body = vec![1];
        body.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        body.extend_from_slice(b"Arthas\0");
        body.extend_from_slice(&[1, 6, 0, 2, 3, 4, 5, 6, 80]);
        body.extend_from_slice(&12u32.to_le_bytes());
        body.extend_from_slice(&571u32.to_le_bytes());
        for value in [1.5f32, -2.25, 3.75] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(&44u32.to_le_bytes());
        body.extend_from_slice(&0x0400u32.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.push(1);
        body.extend_from_slice(&123u32.to_le_bytes());
        body.extend_from_slice(&80u32.to_le_bytes());
        body.extend_from_slice(&15u32.to_le_bytes());
        for slot in 0..23u32 {
            body.extend_from_slice(&(1000 + slot).to_le_bytes());
            body.push(slot as u8);
            body.extend_from_slice(&(2000 + slot).to_le_bytes());
        }
        body
    }

    #[test]
    fn proof_matches_fixed_sha1_vector() {
        let key = [0x5au8; SESSION_KEY_LENGTH];
        assert_eq!(
            world_proof("ACCOUNT", 0x1122_3344, 0xaabb_ccdd, &key),
            [
                0x08, 0x08, 0x5a, 0x52, 0xa9, 0xbf, 0xb4, 0x83, 0x3f, 0x9a, 0x74, 0x23, 0x03, 0x39,
                0x18, 0xc0, 0xbc, 0x78, 0x43, 0xf9,
            ]
        );
    }

    #[test]
    fn auth_session_has_exact_fixed_fields_and_the_stock_addons() {
        let proof = [0xabu8; 20];
        let body = auth_session_body("ACCOUNT", 7, 0x1122_3344, &proof).unwrap();
        assert_eq!(&body[0..4], &WRATH_CLIENT_BUILD.to_le_bytes());
        assert_eq!(&body[4..8], &[0; 4]);
        assert_eq!(&body[8..16], b"ACCOUNT\0");
        assert_eq!(&body[16..20], &[0; 4]);
        assert_eq!(&body[20..24], &0x1122_3344u32.to_le_bytes());
        assert_eq!(&body[24..32], &[0; 8]);
        assert_eq!(&body[32..36], &7u32.to_le_bytes());
        assert_eq!(&body[36..44], &[0; 8]);
        assert_eq!(&body[44..64], &proof);
        let stock = crate::types::addons::addon_block(&crate::types::STOCK_SECURE_ADDONS, 0);
        assert_eq!(&body[64..68], &(stock.len() as u32).to_le_bytes());
        let mut plain = Vec::new();
        ZlibDecoder::new(&body[68..])
            .read_to_end(&mut plain)
            .unwrap();
        assert_eq!(plain, stock);
    }

    #[test]
    fn server_headers_support_small_and_large_sizes() {
        let mut small = Cursor::new(Vec::<u8>::new());
        assert_eq!(
            decode_server_header(&mut small, [0x01, 0x02, 0x34, 0x12], None).unwrap(),
            (0x102, 0x1234)
        );
        let mut fifth = Cursor::new(vec![0x12]);
        assert_eq!(
            decode_server_header(&mut fifth, [0x81, 0x23, 0x45, 0x34], None).unwrap(),
            (0x01_2345, 0x1234)
        );
        assert!(
            decode_server_header(&mut Cursor::new(Vec::<u8>::new()), [0x80, 0, 2, 0x34], None)
                .is_err()
        );
    }

    #[test]
    fn client_header_rejects_u16_size_overflow() {
        let error = send_raw(
            &mut Vec::new(),
            None,
            CMSG_CHAR_ENUM,
            &vec![0; usize::from(u16::MAX) - 3],
        )
        .unwrap_err();
        assert!(error.to_string().contains("too large"));
    }

    #[test]
    fn character_fixture_covers_wrath_only_fields_and_23_slots() {
        let characters = parse_character_enum(&enum_fixture()).unwrap();
        let character = &characters[0];
        assert_eq!(character.name, "Arthas");
        assert_eq!(character.customization_flags, 2);
        assert!(character.first_login);
        assert_eq!(character.pet_display_id, 123);
        assert_eq!(character.equipment.len(), 23);
        assert_eq!(character.equipment[22].display_id, 1022);
        assert_eq!(character.equipment[22].enchantment_aura, 2022);
    }

    #[test]
    fn malformed_or_truncated_character_enums_fail() {
        assert!(parse_character_enum(&[]).is_err());
        assert!(parse_character_enum(&[51]).is_err());
        let fixture = enum_fixture();
        for cut in [1, 12, fixture.len() - 1] {
            assert!(parse_character_enum(&fixture[..cut]).is_err(), "cut {cut}");
        }
        let mut trailing = fixture;
        trailing.push(0xff);
        assert!(parse_character_enum(&trailing).is_err());
    }

    #[test]
    fn malformed_packet_sizes_and_truncated_bodies_fail() {
        assert!(recv_raw(&mut Cursor::new(vec![0, 1, 0x3b, 0]), None).is_err());
        assert!(recv_raw(&mut Cursor::new(vec![0, 5, 0x3b, 0, 1]), None).is_err());
    }

    fn create_fixture() -> WrathCreateCharacter {
        WrathCreateCharacter {
            name: "Arthas".into(),
            race: 1,
            class: 2,
            gender: 0,
            skin: 3,
            face: 4,
            hair_style: 5,
            hair_color: 6,
            facial_hair: 7,
            outfit_id: 0,
        }
    }

    #[test]
    fn character_create_has_exact_wrath_wire_order_and_client_header() {
        let body = create_character_body(&create_fixture()).unwrap();
        assert_eq!(body, b"Arthas\0\x01\x02\x00\x03\x04\x05\x06\x07\x00");

        let mut wire = Vec::new();
        send_raw(&mut wire, None, CMSG_CHAR_CREATE, &body).unwrap();
        assert_eq!(&wire[..6], &[0, 0x14, 0x36, 0, 0, 0]);
        assert_eq!(&wire[6..], body);
    }

    #[test]
    fn character_create_names_are_c_strings_and_bad_ids_are_rejected() {
        // The server judges names: an empty one goes out for `CHAR_NAME_NO_NAME`, a NUL ends it.
        for (name, wire) in [("", &b"\0"[..]), ("Bad\0name", &b"Bad\0"[..])] {
            let mut request = create_fixture();
            request.name = name.into();
            let body = create_character_body(&request).unwrap();
            assert_eq!(&body[..wire.len()], wire, "{name:?}");
        }
        for (field, value) in [("race", 9), ("class", 10), ("gender", 2), ("outfit", 1)] {
            let mut request = create_fixture();
            match field {
                "race" => request.race = value,
                "class" => request.class = value,
                "gender" => request.gender = value,
                "outfit" => request.outfit_id = value,
                _ => unreachable!(),
            }
            assert!(
                create_character_body(&request).is_err(),
                "accepted {field}={value}"
            );
        }
    }

    #[test]
    fn character_names_are_server_policy_and_delete_is_full_guid() {
        for name in [
            "A".to_string(),
            "Åke".into(),
            "Bad-name".into(),
            "X".repeat(256),
        ] {
            let mut request = create_fixture();
            request.name = name.clone();
            let body = create_character_body(&request).unwrap();
            assert_eq!(&body[..name.len()], name.as_bytes());
            assert_eq!(body[name.len()], 0);
        }
        assert_eq!(
            delete_character_body(0x0807060504030201),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        for code in 0..=255 {
            assert_eq!(parse_single_result(&[code], "result").unwrap(), code);
        }
        assert!(parse_single_result(&[], "result").is_err());
        assert!(parse_single_result(&[0x47, 0], "result").is_err());
        assert_ne!(CHAR_CREATE_SUCCESS, crate::types::CHAR_CREATE_SUCCESS);
        assert_ne!(CHAR_DELETE_SUCCESS, crate::types::CHAR_DELETE_SUCCESS);
    }

    #[test]
    fn character_mutations_use_native_opcodes_and_keep_refusals_as_results() {
        let (mut session, mut server) = loopback_session();
        let thread = thread::spawn(move || {
            let (mut enc, mut dec) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
            assert_eq!(
                recv_client_packet(&mut server, &mut enc),
                (
                    CMSG_CHAR_CREATE,
                    create_character_body(&create_fixture()).unwrap()
                )
            );
            send_server_packet(&mut server, &mut dec, SMSG_CHAR_CREATE, &[0x59]);
            assert_eq!(
                recv_client_packet(&mut server, &mut enc),
                (
                    CMSG_CHAR_DELETE,
                    0x0102030405060708u64.to_le_bytes().to_vec()
                )
            );
            send_server_packet(&mut server, &mut dec, SMSG_CHAR_DELETE, &[0x48]);
        });
        assert_eq!(
            session.create_character_result(&create_fixture()).unwrap(),
            0x59
        );
        assert_eq!(
            session.delete_character_result(0x0102030405060708).unwrap(),
            0x48
        );
        thread.join().unwrap();
    }

    #[test]
    fn character_create_result_is_typed_and_rejects_truncation() {
        assert!(parse_create_character_result(&[CHAR_CREATE_SUCCESS]).is_ok());
        let error = parse_create_character_result(&[0x32]).unwrap_err();
        assert_eq!(
            error.downcast_ref::<WrathCreateCharacterReject>(),
            Some(&WrathCreateCharacterReject { code: 0x32 })
        );
        assert!(parse_create_character_result(&[]).is_err());
        assert!(parse_create_character_result(&[CHAR_CREATE_SUCCESS, 0]).is_err());
    }

    #[test]
    fn player_login_is_full_little_endian_guid_packet() {
        let guid = 0x0102_0304_0506_0708u64;
        let mut wire = Vec::new();
        send_raw(&mut wire, None, CMSG_PLAYER_LOGIN, &guid.to_le_bytes()).unwrap();
        assert_eq!(&wire[..6], &[0, 12, 0x3d, 0, 0, 0]);
        assert_eq!(&wire[6..], &guid.to_le_bytes());
    }

    #[test]
    fn initial_login_observation_counts_evidence_without_parsing_objects() {
        let mut observation = WrathInitialLoginObservation::new();
        assert!(!observation
            .observe(&RawPacket {
                opcode: SMSG_TUTORIAL_FLAGS,
                body: vec![0; 32],
            })
            .unwrap());
        let mut account_data = vec![0; 9 + 3 * 4];
        account_data[5..9].copy_from_slice(&0b1000_0011u32.to_le_bytes());
        assert!(!observation
            .observe(&RawPacket {
                opcode: SMSG_ACCOUNT_DATA_TIMES,
                body: account_data,
            })
            .unwrap());
        assert!(!observation
            .observe(&RawPacket {
                opcode: SMSG_LOGIN_VERIFY_WORLD,
                body: vec![0; 20],
            })
            .unwrap());
        assert!(observation
            .observe(&RawPacket {
                opcode: SMSG_LOGIN_SETTIMESPEED,
                body: vec![0; 12],
            })
            .unwrap());

        assert_eq!(observation.login_verify_world_count, 1);
        assert_eq!(observation.account_data_times_count, 1);
        assert_eq!(observation.tutorial_flags_count, 1);
        assert_eq!(observation.login_set_time_speed_count, 1);
        assert_eq!(observation.packets.len(), 4);
        assert_eq!(observation.packets[2].body_size, 20);
    }

    #[test]
    fn initial_login_known_packets_reject_truncated_or_inconsistent_bodies() {
        for (opcode, size) in [
            (SMSG_LOGIN_VERIFY_WORLD, 19),
            (SMSG_TUTORIAL_FLAGS, 31),
            (SMSG_LOGIN_SETTIMESPEED, 11),
        ] {
            assert!(WrathInitialLoginObservation::new()
                .observe(&RawPacket {
                    opcode,
                    body: vec![0; size],
                })
                .is_err());
        }
        assert!(validate_account_data_times(&[0; 8]).is_err());
        let mut inconsistent = vec![0; 9];
        inconsistent[5..9].copy_from_slice(&1u32.to_le_bytes());
        assert!(validate_account_data_times(&inconsistent).is_err());
    }

    #[test]
    fn queue_response_layout_is_bounded_and_complete() {
        assert!(parse_queue_position(&[0; 15]).is_err());
        let mut body = vec![0; 16];
        body[0] = AUTH_WAIT_QUEUE;
        body[11..15].copy_from_slice(&1234u32.to_le_bytes());
        assert_eq!(parse_queue_position(&body).unwrap(), 1234);
    }

    #[test]
    fn split_loopback_preserves_both_crypto_streams_and_extended_headers() {
        let (mut session, mut server) = loopback_session();
        session.send_raw(0x1122_3344, b"before split").unwrap();
        let (mut reader, mut writer) = session.into_split().unwrap();

        let server_thread = thread::spawn(move || {
            let (mut client_encrypt, mut server_decrypt) =
                HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
            assert_eq!(
                recv_client_packet(&mut server, &mut client_encrypt),
                (0x1122_3344, b"before split".to_vec())
            );
            assert_eq!(
                recv_client_packet(&mut server, &mut client_encrypt),
                (0x5566_7788, b"after split".to_vec())
            );

            send_server_packet(&mut server, &mut server_decrypt, 0x1234, b"small");
            let large = vec![0xa5; 0x8_000];
            send_server_packet(&mut server, &mut server_decrypt, 0x5678, &large);
        });

        writer.send_raw(0x5566_7788, b"after split").unwrap();
        assert_eq!(
            reader.recv_raw().unwrap(),
            WrathServerPacket {
                opcode: 0x1234,
                body: b"small".to_vec(),
            }
        );
        let large = reader.recv_raw().unwrap();
        assert_eq!(large.opcode, 0x5678);
        assert_eq!(large.body, vec![0xa5; 0x8_000]);
        server_thread.join().unwrap();
    }

    #[test]
    fn begin_player_login_does_not_consume_an_already_arrived_packet() {
        let (mut session, mut server) = loopback_session();
        let server_thread = thread::spawn(move || {
            let (mut client_encrypt, mut server_decrypt) =
                HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
            send_server_packet(&mut server, &mut server_decrypt, 0x1234, b"queued");
            assert_eq!(
                recv_client_packet(&mut server, &mut client_encrypt),
                (
                    CMSG_PLAYER_LOGIN,
                    0x0102_0304_0506_0708u64.to_le_bytes().to_vec()
                )
            );
        });

        session.begin_player_login(0x0102_0304_0506_0708).unwrap();
        assert_eq!(
            session.recv_raw().unwrap(),
            WrathServerPacket {
                opcode: 0x1234,
                body: b"queued".to_vec(),
            }
        );
        server_thread.join().unwrap();
    }

    #[test]
    fn raw_transport_rejects_warden_and_oversized_client_bodies() {
        let (mut session, mut server) = loopback_session();
        let server_thread = thread::spawn(move || {
            let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
            send_server_packet(&mut server, &mut server_decrypt, SMSG_WARDEN_DATA, &[]);
        });

        let error = session.recv_raw().unwrap_err();
        assert!(error.downcast_ref::<WrathWardenRequired>().is_some());
        server_thread.join().unwrap();

        let error = session
            .send_raw(0x1234, &vec![0; MAX_CLIENT_PACKET_BODY_SIZE + 1])
            .unwrap_err();
        assert!(error.to_string().contains("too large"));
    }

    #[test]
    fn partial_encrypted_header_timeout_or_body_truncation_poison_reader() {
        let (mut session, mut server) = loopback_session();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let mut header = [0, 2, 0x34, 0x12];
        server_decrypt.decrypt(&mut header);
        server.write_all(&header[..2]).unwrap();
        let timeout = session
            .recv_raw_until(Instant::now() + Duration::from_millis(100))
            .unwrap_err();
        let io_error = timeout.downcast_ref::<io::Error>().unwrap();
        assert!(matches!(
            io_error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        let second = session.recv_raw().unwrap_err();
        assert!(second.to_string().contains("partial frame"));

        let (mut session, mut server) = loopback_session();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let mut header = [0, 6, 0x34, 0x12];
        server_decrypt.decrypt(&mut header);
        server.write_all(&header).unwrap();
        server.write_all(&[1, 2]).unwrap();
        server.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(session.recv_raw().is_err());
        assert!(session
            .recv_raw()
            .unwrap_err()
            .to_string()
            .contains("partial frame"));
    }

    #[test]
    fn continuously_arriving_frame_expires_at_absolute_deadline_and_poisons() {
        let (mut session, mut server) = loopback_session();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let wire = server_packet_bytes(&mut server_decrypt, 0x1234, &[0xa5; 32]);
        let server_thread = thread::spawn(move || {
            for byte in wire {
                if server.write_all(&[byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });

        let started = Instant::now();
        let error = session
            .recv_raw_until(started + Duration::from_millis(120))
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(350));
        assert!(matches!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)
        ));
        assert!(session
            .recv_raw_until(Instant::now() + Duration::from_millis(20))
            .unwrap_err()
            .to_string()
            .contains("partial frame"));
        drop(session);
        server_thread.join().unwrap();
    }

    #[test]
    fn header_and_body_share_deadline_and_partial_body_timeout_poisons() {
        let (mut session, mut server) = loopback_session();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let wire = server_packet_bytes(&mut server_decrypt, 0x1234, b"body");
        let server_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(70));
            server.write_all(&wire[..4]).unwrap();
            thread::sleep(Duration::from_millis(70));
            let _ = server.write_all(&wire[4..]);
        });

        let started = Instant::now();
        let error = session
            .recv_raw_until(started + Duration::from_millis(110))
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(matches!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)
        ));
        assert!(session
            .recv_raw_until(Instant::now() + Duration::from_millis(20))
            .unwrap_err()
            .to_string()
            .contains("partial frame"));
        drop(session);
        server_thread.join().unwrap();
    }

    #[test]
    fn decrypted_invalid_header_poisons_reader() {
        let (mut session, mut server) = loopback_session();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let mut header = [0, 1, 0x34, 0x12];
        server_decrypt.decrypt(&mut header);
        server.write_all(&header).unwrap();

        let error = session
            .recv_raw_until(Instant::now() + Duration::from_secs(1))
            .unwrap_err();
        assert!(error.to_string().contains("smaller than its opcode"));
        assert!(session
            .recv_raw_until(Instant::now() + Duration::from_millis(20))
            .unwrap_err()
            .to_string()
            .contains("partial frame"));
    }

    #[test]
    fn no_byte_deadline_timeout_is_retryable() {
        let (mut session, mut server) = loopback_session();
        let error = session
            .recv_raw_until(Instant::now() + Duration::from_millis(20))
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)
        ));

        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        send_server_packet(&mut server, &mut server_decrypt, 0x1234, b"retry");
        assert_eq!(
            session
                .recv_raw_until(Instant::now() + Duration::from_secs(1))
                .unwrap(),
            WrathServerPacket {
                opcode: 0x1234,
                body: b"retry".to_vec(),
            }
        );
    }

    #[test]
    fn short_idle_poll_does_not_become_the_partial_frame_deadline() {
        let (session, mut server) = loopback_session();
        let (mut reader, _writer) = session.into_split().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let wire = server_packet_bytes(&mut server_decrypt, 0x1234, b"slow");
        let server_thread = thread::spawn(move || {
            for byte in wire {
                server.write_all(&[byte]).unwrap();
                thread::sleep(Duration::from_millis(30));
            }
        });

        assert_eq!(
            reader
                .recv_raw_with_frame_timeout(Duration::from_millis(300))
                .unwrap(),
            WrathServerPacket {
                opcode: 0x1234,
                body: b"slow".to_vec(),
            }
        );
        server_thread.join().unwrap();
    }

    #[test]
    fn partial_frame_reports_the_poisoning_error_before_unusable_reader() {
        let (session, mut server) = loopback_session();
        let (mut reader, _writer) = session.into_split().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let wire = server_packet_bytes(&mut server_decrypt, 0x1234, b"late");
        server.write_all(&wire[..2]).unwrap();

        let first = reader
            .recv_raw_with_frame_timeout(Duration::from_millis(40))
            .unwrap_err();
        let partial = first.downcast_ref::<WrathPartialFrameRead>().unwrap();
        assert_eq!(partial.consumed, 2);
        assert!(first.to_string().contains("partial frame read failed"));
        assert!(reader
            .recv_raw_with_frame_timeout(Duration::from_secs(1))
            .unwrap_err()
            .to_string()
            .contains("reader is unusable"));
    }

    #[test]
    fn invalid_extended_size_fails_closed_without_allocating_body() {
        let (mut session, mut server) = loopback_session();
        let (_, mut server_decrypt) = HeaderCrypto::from_session_key(TRANSPORT_KEY).split();
        let mut header = [0x80, 0, 1, 0x34, 0x12];
        server_decrypt.decrypt(&mut header);
        server.write_all(&header).unwrap();
        let error = session.recv_raw().unwrap_err();
        assert!(error.to_string().contains("smaller than its opcode"));
        assert!(session.recv_raw().is_err());
    }
}
