//! Wrath of the Lich King 3.3.5a (build 12340) login protocol support.
//!
//! The realmd logon: SRP6 over `wrathplus-srp`, then the realm list.

use std::io::{Cursor, Read, Write};
use std::net::{Ipv4Addr, TcpStream};

use anyhow::{anyhow, bail, Context, Result};
use sha1::{Digest, Sha1};
use wrathplus_srp::{NormalizedString, PublicKey, SrpClientChallenge, SESSION_KEY_LENGTH};

use crate::wire::{read_array, read_f32_le, read_u16_le, read_u32_le, read_u8};
use crate::{host_port, AuthReject, AUTH_PORT};

const CMD_AUTH_LOGON_CHALLENGE: u8 = 0x00;
const CMD_AUTH_LOGON_PROOF: u8 = 0x01;
const CMD_REALM_LIST: u8 = 0x10;

/// Wrath's login protocol byte and client build.
pub const PROTOCOL_VERSION: u8 = 8;
pub const CLIENT_BUILD: u16 = 12_340;
pub const CLIENT_VERSION: (u8, u8, u8) = (3, 3, 5);

const GAME_NAME_WOW: u32 = 0x0057_6f57;
const PLATFORM_X86: u32 = 0x0078_3836;
// Every platform's build signs in as the Mac 12340 client. AzerothCore starts Warden for an
// account whose client says `Win` (`Warden.Enabled = 1` is its default) and has no Mac Warden;
// this client has no Warden, so a `Win` sign-in is refused at the world handshake.
const OS_MACOS: u32 = 0x004f_5358;
const LOCALE_EN_US: u32 = 0x656e_5553;

const SECURITY_PIN: u8 = 0x01;
const SECURITY_MATRIX: u8 = 0x02;
const SECURITY_TOKEN: u8 = 0x04;
const KNOWN_SECURITY_FLAGS: u8 = SECURITY_PIN | SECURITY_MATRIX | SECURITY_TOKEN;
const REALM_FLAG_SPECIFY_BUILD: u8 = 0x04;

// The frame is length-prefixed by u16. Keeping an explicit bound makes the allocation and all
// C-string searches subordinate to that framing even when the peer is hostile.
const MAX_REALM_LIST_FRAME: usize = u16::MAX as usize;
const MIN_REALM_LIST_FRAME: usize = 8; // u32 header, u16 count, u16 footer

/// SRP inputs from a successful Wrath logon challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeReply {
    pub server_public_key: [u8; 32],
    pub generator: u8,
    pub large_safe_prime: [u8; 32],
    pub salt: [u8; 32],
    pub crc_salt: [u8; 16],
}

/// The additional fields in a successful post-BC logon-proof response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofReply {
    pub server_proof: [u8; 20],
    pub account_flags: u32,
    pub survey_id: u32,
    pub login_flags: u16,
}

/// A build advertised for a realm whose `REALM_FLAG_SPECIFYBUILD` bit is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealmBuild {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    pub build: u16,
}

/// A realm record in the Wrath realm-list format.
#[derive(Debug, Clone, PartialEq)]
pub struct RealmInfo {
    pub icon: u8,
    pub locked: bool,
    pub flags: u8,
    pub name: String,
    pub address: String,
    pub population: f32,
    pub characters: u8,
    pub timezone: u8,
    pub id: u8,
    pub specified_build: Option<RealmBuild>,
}

/// A successful Wrath auth login. Credentials are consumed by [`logon`] and are never retained.
pub struct Logon {
    pub session_key: [u8; SESSION_KEY_LENGTH],
    pub account_flags: u32,
    pub survey_id: u32,
    pub login_flags: u16,
    pub realms: Vec<RealmInfo>,
    stream: Option<TcpStream>,
}

impl Logon {
    /// Refresh the framed realm list, retaining the old list if the auth socket fails.
    pub fn refresh_realms(&mut self, timeout: std::time::Duration) -> bool {
        let Some(stream) = self.stream.as_mut() else {
            return false;
        };
        let refreshed = (|| -> Result<Vec<RealmInfo>> {
            stream.set_read_timeout(Some(timeout))?;
            write_realm_list_request(stream)?;
            read_realm_list(stream)
        })();
        match refreshed {
            Ok(realms) => {
                self.realms = realms;
                true
            }
            Err(_) => {
                self.stream = None;
                false
            }
        }
    }
}

/// Write a 3.3.5a/build-12340 `CMD_AUTH_LOGON_CHALLENGE_Client` packet.
pub fn write_logon_challenge(w: &mut impl Write, account_name: &str) -> Result<()> {
    let account_len =
        u8::try_from(account_name.len()).context("account name exceeds wire length")?;
    let mut body = Vec::with_capacity(34 + account_name.len());
    body.extend_from_slice(&GAME_NAME_WOW.to_le_bytes());
    body.extend_from_slice(&[CLIENT_VERSION.0, CLIENT_VERSION.1, CLIENT_VERSION.2]);
    body.extend_from_slice(&CLIENT_BUILD.to_le_bytes());
    body.extend_from_slice(&PLATFORM_X86.to_le_bytes());
    body.extend_from_slice(&OS_MACOS.to_le_bytes());
    body.extend_from_slice(&LOCALE_EN_US.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
    body.push(account_len);
    body.extend_from_slice(account_name.as_bytes());

    let body_len = u16::try_from(body.len()).context("logon challenge exceeds wire length")?;
    w.write_all(&[CMD_AUTH_LOGON_CHALLENGE, PROTOCOL_VERSION])?;
    w.write_all(&body_len.to_le_bytes())?;
    w.write_all(&body)?;
    Ok(())
}

/// Read a Wrath challenge. Any requested PIN, matrix or token authentication is rejected rather
/// than partially consumed and allowed to desynchronise the following proof exchange.
pub fn read_challenge_reply(r: &mut impl Read) -> Result<ChallengeReply> {
    expect_opcode(r, CMD_AUTH_LOGON_CHALLENGE, "CMD_AUTH_LOGON_CHALLENGE")?;
    let protocol_status = read_u8(r)?;
    if protocol_status != 0 {
        bail!("unsupported challenge protocol status {protocol_status:#04x}");
    }
    reject_result(read_u8(r)?)?;

    let server_public_key = read_array::<32>(r)?;
    let generator_len = read_u8(r)? as usize;
    if generator_len != 1 {
        bail!("generator was {generator_len} bytes, expected 1");
    }
    let generator = read_u8(r)?;
    let prime_len = read_u8(r)? as usize;
    if prime_len != 32 {
        bail!("large safe prime was {prime_len} bytes, expected 32");
    }
    let large_safe_prime = read_array::<32>(r)?;
    let salt = read_array::<32>(r)?;
    let crc_salt = read_array::<16>(r)?;
    let security_flags = read_u8(r)?;
    if security_flags != 0 {
        let detail = if security_flags & !KNOWN_SECURITY_FLAGS != 0 {
            "unknown security bits"
        } else {
            "MFA/security challenge"
        };
        bail!("unsupported {detail}: {security_flags:#04x}");
    }

    Ok(ChallengeReply {
        server_public_key,
        generator,
        large_safe_prime,
        salt,
        crc_salt,
    })
}

// AzerothCore's build_info hash for the Mac build 12340, matching `OS_MACOS`. It is deliberately
// local to Wrath: the build-5875 integrity material in auth.rs is not valid for StrictVersionCheck
// here.
const INTEGRITY_HASH_12340_MACOS: [u8; 20] = [
    0xb7, 0x06, 0xd1, 0x3f, 0xf2, 0xf4, 0x01, 0x88, 0x39, 0x72, 0x94, 0x61, 0xe3, 0xf8, 0xa0, 0xe2,
    0xb5, 0xfd, 0xc0, 0x34,
];

/// Compute AzerothCore's strict build-12340 version proof, `SHA1(A[32] || build_hash[20])`.
pub fn version_proof(client_public_key: &[u8; 32]) -> [u8; 20] {
    let mut sha = Sha1::new();
    sha.update(client_public_key);
    sha.update(INTEGRITY_HASH_12340_MACOS);
    sha.finalize().into()
}

pub fn write_logon_proof(
    w: &mut impl Write,
    client_public_key: &[u8; 32],
    client_proof: &[u8; 20],
) -> std::io::Result<()> {
    w.write_all(&[CMD_AUTH_LOGON_PROOF])?;
    w.write_all(client_public_key)?;
    w.write_all(client_proof)?;
    w.write_all(&version_proof(client_public_key))?;
    w.write_all(&[0, 0]) // telemetry-key count, security flags
}

/// Read the 32-byte successful proof response used by 2.x/3.x clients.
pub fn read_proof_reply(r: &mut impl Read) -> Result<ProofReply> {
    expect_opcode(r, CMD_AUTH_LOGON_PROOF, "CMD_AUTH_LOGON_PROOF")?;
    reject_result(read_u8(r)?)?;
    Ok(ProofReply {
        server_proof: read_array::<20>(r)?,
        account_flags: read_u32_le(r)?,
        survey_id: read_u32_le(r)?,
        login_flags: read_u16_le(r)?,
    })
}

pub fn write_realm_list_request(w: &mut impl Write) -> std::io::Result<()> {
    w.write_all(&[CMD_REALM_LIST, 0, 0, 0, 0])
}

/// Parse one length-framed Wrath realm list. Reads exactly the advertised frame before parsing, so
/// malformed counts, strings and optional-build records cannot consume a subsequent packet.
pub fn read_realm_list(r: &mut impl Read) -> Result<Vec<RealmInfo>> {
    expect_opcode(r, CMD_REALM_LIST, "CMD_REALM_LIST")?;
    let frame_len = read_u16_le(r)? as usize;
    if !(MIN_REALM_LIST_FRAME..=MAX_REALM_LIST_FRAME).contains(&frame_len) {
        bail!("realm-list frame length {frame_len} is out of bounds");
    }
    let mut frame = vec![0u8; frame_len];
    r.read_exact(&mut frame)
        .context("truncated realm-list frame")?;
    parse_realm_frame(&frame)
}

fn parse_realm_frame(frame: &[u8]) -> Result<Vec<RealmInfo>> {
    let mut cursor = Cursor::new(frame);
    let _header = read_u32_le(&mut cursor)?;
    let count = read_u16_le(&mut cursor)? as usize;
    // Every record needs at least icon, lock, flags, two NULs, population, chars, timezone and id.
    const MIN_RECORD: usize = 12;
    let records_available = frame.len().saturating_sub(8);
    if count > records_available / MIN_RECORD {
        bail!(
            "realm count {count} cannot fit in {}-byte frame",
            frame.len()
        );
    }

    let mut realms = Vec::with_capacity(count);
    for _ in 0..count {
        let icon = read_u8(&mut cursor)?;
        let locked = match read_u8(&mut cursor)? {
            0 => false,
            1 => true,
            value => bail!("invalid realm locked byte {value:#04x}"),
        };
        let flags = read_u8(&mut cursor)?;
        let name = read_bounded_cstring(&mut cursor)?;
        let address = read_bounded_cstring(&mut cursor)?;
        let population = read_f32_le(&mut cursor)?;
        let characters = read_u8(&mut cursor)?;
        let timezone = read_u8(&mut cursor)?;
        let id = read_u8(&mut cursor)?;
        let specified_build = if flags & REALM_FLAG_SPECIFY_BUILD != 0 {
            Some(RealmBuild {
                major: read_u8(&mut cursor)?,
                minor: read_u8(&mut cursor)?,
                patch: read_u8(&mut cursor)?,
                build: read_u16_le(&mut cursor)?,
            })
        } else {
            None
        };
        realms.push(RealmInfo {
            icon,
            locked,
            flags,
            name,
            address,
            population,
            characters,
            timezone,
            id,
            specified_build,
        });
    }

    let footer = read_u16_le(&mut cursor)?;
    if footer != 0x0010 {
        bail!("invalid Wrath realm-list footer {footer:#06x}");
    }
    if cursor.position() != frame.len() as u64 {
        bail!("realm-list frame has trailing bytes");
    }
    Ok(realms)
}

fn read_bounded_cstring(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    let start = cursor.position() as usize;
    let relative_end = cursor.get_ref()[start..]
        .iter()
        .position(|&byte| byte == 0)
        .context("unterminated string in realm-list frame")?;
    let end = start + relative_end;
    let value = cursor.get_ref()[start..end].to_vec();
    cursor.set_position((end + 1) as u64);
    String::from_utf8(value).context("realm-list string is not UTF-8")
}

fn expect_opcode(r: &mut impl Read, expected: u8, name: &str) -> Result<()> {
    let opcode = read_u8(r)?;
    if opcode != expected {
        bail!("expected {name} ({expected:#04x}), got {opcode:#04x}");
    }
    Ok(())
}

fn reject_result(result: u8) -> Result<()> {
    if result != 0 {
        return Err(AuthReject { code: result }.into());
    }
    Ok(())
}

fn dial_wrath(host: &str, port: u16) -> Result<TcpStream> {
    use std::net::ToSocketAddrs;
    use std::time::Duration;
    let timeout = Duration::from_secs(10);
    let addresses = (host, port)
        .to_socket_addrs()
        .context("resolving Wrath auth server")?;
    let mut last_error = None;
    for address in addresses.take(16) {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(anyhow!(
        "connecting to Wrath auth server failed: {:?}",
        last_error
    ))
}

/// Perform the explicit Wrath 3.3.5a login flow and fetch the initial realm list.
pub fn logon(host: &str, username: &str, password: &str) -> Result<Logon> {
    let (host, port) = host_port(host, AUTH_PORT);
    let username = NormalizedString::new(username).map_err(|e| anyhow!("invalid username: {e}"))?;
    let password = NormalizedString::new(password).map_err(|e| anyhow!("invalid password: {e}"))?;

    let mut stream = dial_wrath(host, port)?;
    write_logon_challenge(&mut stream, username.as_ref())
        .context("sending Wrath logon challenge")?;
    let reply = read_challenge_reply(&mut stream).context("reading Wrath logon challenge")?;
    let server_public_key = PublicKey::from_le_bytes(reply.server_public_key)
        .map_err(|e| anyhow!("invalid server public key: {e}"))?;

    // AzerothCore hashes A, B, K and proofs at their declared widths. Its SHA1Interleave trims a
    // leading-zero pair from S; the shared engine avoids that ambiguous draw, so no vanilla B redial
    // workaround belongs in this path.
    let challenge = SrpClientChallenge::new(
        username,
        password,
        reply.generator,
        reply.large_safe_prime,
        server_public_key,
        reply.salt,
    );
    write_logon_proof(
        &mut stream,
        challenge.client_public_key(),
        challenge.client_proof(),
    )
    .context("sending Wrath logon proof")?;
    let proof = read_proof_reply(&mut stream).context("reading Wrath logon proof")?;
    let client = challenge
        .verify_server_proof(proof.server_proof)
        .map_err(|e| anyhow!("server proof mismatch (wrong password?): {e}"))?;
    let session_key = *client.session_key();

    write_realm_list_request(&mut stream).context("requesting Wrath realm list")?;
    let realms = read_realm_list(&mut stream).context("reading Wrath realm list")?;
    Ok(Logon {
        session_key,
        account_flags: proof.account_flags,
        survey_id: proof.survey_id,
        login_flags: proof.login_flags,
        realms,
        stream: Some(stream),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitted_challenge_is_protocol_eight_build_12340_version_335() {
        let mut packet = Vec::new();
        write_logon_challenge(&mut packet, "TEST").unwrap();
        let mut expected = vec![0x00, 0x08, 0x22, 0x00, b'W', b'o', b'W', 0];
        expected.extend_from_slice(&[3, 3, 5, 0x34, 0x30, b'6', b'8', b'x', 0]);
        expected.extend_from_slice(b"XSO\0");
        expected.extend_from_slice(b"SUne");
        expected.extend_from_slice(&[0, 0, 0, 0]); // timezone
        expected.extend_from_slice(&[127, 0, 0, 1, 4, b'T', b'E', b'S', b'T']);
        assert_eq!(packet, expected);
    }

    #[test]
    fn version_proof_uses_build_12340_integrity_hash() {
        let a: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(9));
        let expected = "8d575a807990bf3d47041e02c48311adfb5732c2";
        assert_eq!(
            version_proof(&a)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            expected
        );
    }

    #[test]
    fn proof_success_reads_all_post_bc_fields() {
        let mut fixture = vec![CMD_AUTH_LOGON_PROOF, 0];
        fixture.extend(0x40..0x54);
        fixture.extend_from_slice(&0x1122_3344u32.to_le_bytes());
        fixture.extend_from_slice(&0x5566_7788u32.to_le_bytes());
        fixture.extend_from_slice(&0x99aau16.to_le_bytes());
        let proof = read_proof_reply(&mut fixture.as_slice()).unwrap();
        assert_eq!(proof.server_proof, std::array::from_fn(|i| 0x40 + i as u8));
        assert_eq!(proof.account_flags, 0x1122_3344);
        assert_eq!(proof.survey_id, 0x5566_7788);
        assert_eq!(proof.login_flags, 0x99aa);
    }

    fn realm_packet(frame: Vec<u8>) -> Vec<u8> {
        let mut packet = vec![CMD_REALM_LIST];
        packet.extend_from_slice(&(frame.len() as u16).to_le_bytes());
        packet.extend(frame);
        packet
    }

    #[test]
    fn realm_fixture_has_u16_count_lock_and_optional_build() {
        let mut frame = 0u32.to_le_bytes().to_vec();
        frame.extend_from_slice(&1u16.to_le_bytes());
        frame.extend_from_slice(&[1, 1, REALM_FLAG_SPECIFY_BUILD]);
        frame.extend_from_slice(b"Azeroth\0realm.invalid:8085\0");
        frame.extend_from_slice(&0.75f32.to_le_bytes());
        frame.extend_from_slice(&[2, 1, 7, 3, 3, 5]);
        frame.extend_from_slice(&CLIENT_BUILD.to_le_bytes());
        frame.extend_from_slice(&0x0010u16.to_le_bytes());
        let realms = read_realm_list(&mut realm_packet(frame).as_slice()).unwrap();
        assert_eq!(realms.len(), 1);
        assert_eq!(realms[0].icon, 1);
        assert!(realms[0].locked);
        assert_eq!(realms[0].name, "Azeroth");
        assert_eq!(realms[0].id, 7);
        assert_eq!(
            realms[0].specified_build,
            Some(RealmBuild {
                major: 3,
                minor: 3,
                patch: 5,
                build: 12_340
            })
        );
    }

    #[test]
    fn realm_frame_rejects_truncation_impossible_count_and_bad_flags() {
        let truncated = vec![CMD_REALM_LIST, 8, 0, 0, 0];
        assert!(read_realm_list(&mut truncated.as_slice()).is_err());

        let mut impossible = 0u32.to_le_bytes().to_vec();
        impossible.extend_from_slice(&2u16.to_le_bytes());
        impossible.extend_from_slice(&0x0010u16.to_le_bytes());
        assert!(read_realm_list(&mut realm_packet(impossible).as_slice()).is_err());

        let mut bad_lock = 0u32.to_le_bytes().to_vec();
        bad_lock.extend_from_slice(&1u16.to_le_bytes());
        bad_lock.extend_from_slice(&[0, 2, 0]);
        bad_lock.extend_from_slice(b"x\0y\0");
        bad_lock.extend_from_slice(&0f32.to_le_bytes());
        bad_lock.extend_from_slice(&[0, 0, 0]);
        bad_lock.extend_from_slice(&0x0010u16.to_le_bytes());
        assert!(read_realm_list(&mut realm_packet(bad_lock).as_slice()).is_err());
    }

    #[test]
    fn challenge_rejects_all_security_modes_and_unknown_bits() {
        for security_flags in [SECURITY_PIN, SECURITY_MATRIX, SECURITY_TOKEN, 0x80] {
            let mut fixture = vec![CMD_AUTH_LOGON_CHALLENGE, 0, 0];
            fixture.extend_from_slice(&[1; 32]);
            fixture.extend_from_slice(&[1, 7, 32]);
            fixture.extend_from_slice(&[2; 32]);
            fixture.extend_from_slice(&[3; 32]);
            fixture.extend_from_slice(&[4; 16]);
            fixture.push(security_flags);
            assert!(read_challenge_reply(&mut fixture.as_slice()).is_err());
        }
    }

    #[test]
    fn response_parsers_reject_truncated_fixtures() {
        let proof = [CMD_AUTH_LOGON_PROOF, 0, 1, 2, 3];
        assert!(read_proof_reply(&mut proof.as_slice()).is_err());

        let challenge = [CMD_AUTH_LOGON_CHALLENGE, 0, 0, 1, 2, 3];
        assert!(read_challenge_reply(&mut challenge.as_slice()).is_err());
    }
}
