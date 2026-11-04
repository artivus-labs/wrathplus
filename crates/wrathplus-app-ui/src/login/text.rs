//! The login screen's dialog text: the connecting stages and every refusal, from the glue strings.

use wrathplus_protocol::{DialFailure, LoginRefusal, LoginStage};

use crate::glue_strings::GlueStrings;

/// The connecting dialog's `LOGIN_STATE_*` text for a stage.
pub(crate) fn stage_text(strings: &GlueStrings, stage: LoginStage) -> &str {
    match stage {
        LoginStage::Connecting => strings.text("LOGIN_STATE_CONNECTING", "Connecting"),
        LoginStage::Authenticating => strings.text("LOGIN_STATE_AUTHENTICATING", "Authenticating"),
        LoginStage::Handshaking => strings.text("LOGIN_STATE_HANDSHAKING", "Handshaking"),
    }
}

/// The failure text for a login result; a transport failure (`None`) reads `LOGIN_FAILED`.
///
/// Deviation: a dial that never opened a socket reads `AUTH_LOGIN_SERVER_NOT_FOUND` or
/// `LOGIN_SERVER_DOWN` plus the address, because with an editable realmlist "Unable to connect"
/// cannot tell a bad name from a server that is down.
pub(crate) fn fail_text(
    strings: &GlueStrings,
    refusal: Option<LoginRefusal>,
    dial: Option<&DialFailure>,
) -> String {
    if let Some(dial) = dial {
        let headline = if dial.unresolved {
            strings.text("AUTH_LOGIN_SERVER_NOT_FOUND", "Invalid Login Server")
        } else {
            strings.text("LOGIN_SERVER_DOWN", "Login Server Down")
        };
        return format!("{headline}\n{}", dial.address);
    }
    let authored = match refusal {
        Some(LoginRefusal::World(code)) => world_refusal_text(strings, code),
        Some(LoginRefusal::Logon(code)) => logon_refusal_text(strings, Some(code)),
        None => logon_refusal_text(strings, None),
    };
    without_dead_url(authored).into_owned()
}

/// The world server's `SMSG_AUTH_RESPONSE` refusal text: the client's dispatch (`0x5aa960`,
/// key table `0x85cae8`), numbered as `AuthResponseCodes` in cmangos `SharedDefines.h:1721+`.
fn world_refusal_text(strings: &GlueStrings, code: u8) -> &str {
    use wrathplus_protocol::types as m;
    let (key, fallback): (&str, &str) = match code {
        m::AUTH_FAILED => ("AUTH_FAILED", "Authentication failed"),
        m::AUTH_REJECT => ("AUTH_REJECT", "Login unavailable"),
        m::AUTH_BAD_SERVER_PROOF => ("AUTH_BAD_SERVER_PROOF", "Server is not valid"),
        m::AUTH_UNAVAILABLE => (
            "AUTH_UNAVAILABLE",
            "System unavailable - Please try again later",
        ),
        m::AUTH_SYSTEM_ERROR => ("AUTH_SYSTEM_ERROR", "System Error"),
        m::AUTH_BILLING_ERROR => ("AUTH_BILLING_ERROR", "Billing system error"),
        m::AUTH_BILLING_EXPIRED => ("AUTH_BILLING_EXPIRED", "Account billing has expired"),
        m::AUTH_VERSION_MISMATCH => ("AUTH_VERSION_MISMATCH", "Wrong client version"),
        m::AUTH_UNKNOWN_ACCOUNT => ("AUTH_UNKNOWN_ACCOUNT", "Unknown account"),
        m::AUTH_INCORRECT_PASSWORD => ("AUTH_INCORRECT_PASSWORD", "Incorrect Password"),
        m::AUTH_SESSION_EXPIRED => ("AUTH_SESSION_EXPIRED", "Session Expired"),
        m::AUTH_SERVER_SHUTTING_DOWN => ("AUTH_SERVER_SHUTTING_DOWN", "Server Shutting Down"),
        m::AUTH_ALREADY_LOGGING_IN => ("AUTH_ALREADY_LOGGING_IN", "Already Logging In"),
        m::AUTH_LOGIN_SERVER_NOT_FOUND => ("AUTH_LOGIN_SERVER_NOT_FOUND", "Invalid Login Server"),
        // All but `AUTH_ALREADY_ONLINE` below are the rows of the client's `OKAY_WITH_URL` table
        // (`0x803740`). Deviation: no URL button, because `LaunchURL` is a shell-out WrathPlus does
        // not make and its whitelist (`0x85cc34`) names only Blizzard domains that no longer
        // serve these pages.
        m::AUTH_BANNED => (
            "AUTH_BANNED",
            "This account has been banned for violating the Terms of Use Agreement",
        ),
        m::AUTH_ALREADY_ONLINE => ("AUTH_ALREADY_ONLINE", "This character is still logged on"),
        m::AUTH_NO_TIME => (
            "AUTH_NO_TIME",
            "Your World of Warcraft subscription has expired",
        ),
        m::AUTH_DB_BUSY => ("AUTH_DB_BUSY", "This session has timed out"),
        m::AUTH_SUSPENDED => (
            "AUTH_SUSPENDED",
            "This account has been temporarily suspended for violating the Terms of Use Agreement",
        ),
        m::AUTH_PARENTAL_CONTROL => (
            "AUTH_PARENTAL_CONTROL",
            "Access to this account has been blocked by parental controls.",
        ),
        // `AUTH_OK` and `AUTH_WAIT_QUEUE` never reach here; anything else is unknown.
        _ => ("AUTH_FAILED", "Authentication failed"),
    };
    strings.text(key, fallback)
}

/// Cut a failure string at the clause (comma or full stop) before its first web address.
/// Deviation: the reference prints the address, but those Blizzard pages no longer serve and a
/// private realm's player is not served by them. With no boundary before the address (as in
/// `AUTH_BANNED`) the string is left whole.
pub(super) fn without_dead_url(text: &str) -> std::borrow::Cow<'_, str> {
    let Some(url) = ["www.", "http://", "https://"]
        .iter()
        .filter_map(|p| text.find(p))
        .min()
    else {
        return std::borrow::Cow::Borrowed(text);
    };
    let head = &text[..url];
    let Some(cut) = head.rfind([',', '.']) else {
        return std::borrow::Cow::Borrowed(text);
    };
    let kept = head[..cut].trim_end();
    if kept.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(format!("{kept}."))
}

/// The realmd logon-proof refusal text, the long `LOGIN_*` family: realmd results resolve
/// through key table `0x836b78` (`Logon::OnAuthResult` `0x5b2c90`, byte-index table `0x5b2ea4`,
/// jump table `0x5b2e78`), the world server's through `0x85cae8` ([`world_refusal_text`]).
///
/// 0x04 and 0x05 share one arm, `LOGIN_UNKNOWN_ACCOUNT`: the reference never shows
/// `LOGIN_INCORRECT_PASSWORD` and has no lockout. Codes past 0x0F saturate to the arm that shows
/// `DISCONNECTED`.
fn logon_refusal_text(strings: &GlueStrings, code: Option<u8>) -> &str {
    let (key, fallback): (&str, &str) = match code {
        // No code, unknown0/1, invalid server and no access all read `LOGIN_FAILED`.
        None | Some(0x01) | Some(0x02) | Some(0x0B) | Some(0x0D) => {
            ("LOGIN_FAILED", "Unable to connect")
        }
        Some(0x03) => (
            "LOGIN_BANNED",
            "This World of Warcraft account has been closed and is no longer available for use.  \
             Please go to http://www.worldofwarcraft.com/misc/banned.html for further information. ",
        ),
        // The unknown account and the wrong password alike.
        Some(0x04) | Some(0x05) => (
            "LOGIN_UNKNOWN_ACCOUNT",
            "The information you have entered is not valid.  Please check the spelling of the \
             account name and password.  If you need help in retrieving a lost or stolen password \
             and account, see www.worldofwarcraft.com for more information.",
        ),
        Some(0x06) => (
            "LOGIN_ALREADYONLINE",
            "This account is already logged into World of Warcraft.  Please check the spelling and \
             try again.",
        ),
        Some(0x07) => (
            "LOGIN_NOTIME",
            "You have used up your prepaid time for this account. Please purchase more to continue \
             playing",
        ),
        Some(0x08) => (
            "LOGIN_DBBUSY",
            "Could not log in to World of Warcraft at this time.  Please try again later.",
        ),
        Some(0x09) => (
            "LOGIN_BADVERSION",
            "Unable to validate game version.  This may be caused by file corruption or the \
             interference of another program.  Please visit www.blizzard.com/support/wow/ for more \
             information and possible solutions to this issue.",
        ),
        Some(0x0C) => (
            "LOGIN_SUSPENDED",
            "This World of Warcraft account has been temporarily suspended.  Please go to \
             http://www.worldofwarcraft.com/misc/banned.html for further information.",
        ),
        Some(0x0F) => (
            "LOGIN_PARENTALCONTROL",
            "Access to this account has been blocked by parental controls.  Your settings may be \
             changed in your account preferences at http://www.worldofwarcraft.com.",
        ),
        // 0x0A (version update) is ignored on the proof (`0x5bada2`); every remaining code takes
        // the disconnect arm.
        Some(_) => ("DISCONNECTED", "Disconnected from server"),
    };
    strings.text(key, fallback)
}
