//! `SMSG_CALENDAR_COMMAND_RESULT`'s line: build 12340's error switch (`0x6cd380`) and the
//! formatting `0x5b9960` gives it.

use wrathplus_ui::script::UiScript;

/// What `SMSG_CALENDAR_COMMAND_RESULT`'s line formats its key with (`0x5b9960`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ErrorArg {
    /// The bare template.
    None,
    /// A constant the client passes for its `%d` (the limits it knows).
    Count(u32),
    /// The result's name, for its `%s`.
    Name,
}

/// Build 12340's error switch (`0x6cd380`): AzerothCore's `CalendarError` to the GlobalStrings key
/// and the argument `0x5b9960` formats it with; codes it has no arm for raise nothing.
pub(super) fn error_key(error: u32) -> Option<(&'static str, ErrorArg)> {
    use ErrorArg::{Count, Name, None as Bare};
    Some(match error {
        1 => ("CALENDAR_ERROR_GUILD_EVENTS_EXCEEDED", Count(100)),
        2 => ("CALENDAR_ERROR_EVENTS_EXCEEDED", Count(30)),
        3 => ("CALENDAR_ERROR_SELF_INVITES_EXCEEDED", Bare),
        4 => ("CALENDAR_ERROR_OTHER_INVITES_EXCEEDED", Name),
        5 => ("CALENDAR_ERROR_PERMISSIONS", Bare),
        6 => ("CALENDAR_ERROR_EVENT_INVALID", Bare),
        7 => ("CALENDAR_ERROR_NOT_INVITED", Bare),
        8 => ("CALENDAR_ERROR_INTERNAL", Bare),
        9 => ("ERR_GUILD_PLAYER_NOT_IN_GUILD", Bare),
        10 => ("CALENDAR_ERROR_ALREADY_INVITED_TO_EVENT_S", Name),
        11 => ("PLAYER_NOT_FOUND", Bare),
        12 => ("CALENDAR_ERROR_NOT_ALLIED", Bare),
        13 => ("ERR_IGNORING_YOU_S", Name),
        14 => ("CALENDAR_ERROR_INVITES_EXCEEDED", Count(100)),
        16 => ("CALENDAR_ERROR_INVALID_DATE", Bare),
        17 => ("CALENDAR_ERROR_INVALID_TIME", Bare),
        19 => ("CALENDAR_ERROR_NEEDS_TITLE", Bare),
        20 => ("CALENDAR_ERROR_EVENT_PASSED", Bare),
        21 => ("CALENDAR_ERROR_EVENT_LOCKED", Bare),
        22 => ("CALENDAR_ERROR_DELETE_CREATOR_FAILED", Bare),
        24 => ("ERR_SYSTEM_DISABLED", Bare),
        25 => ("ERR_RESTRICTED_ACCOUNT", Bare),
        26 => ("CALENDAR_ERROR_ARENA_EVENTS_EXCEEDED", Bare),
        27 => ("CALENDAR_ERROR_RESTRICTED_LEVEL", Bare),
        28 => ("ERR_USER_SQUELCHED", Bare),
        29 => ("CALENDAR_ERROR_NO_INVITE", Bare),
        36 => ("CALENDAR_ERROR_EVENT_WRONG_SERVER", Bare),
        37 => ("CALENDAR_ERROR_INVITE_WRONG_SERVER", Bare),
        38 => ("CALENDAR_ERROR_NO_GUILD_INVITES", Bare),
        39 => ("CALENDAR_ERROR_INVALID_SIGNUP", Bare),
        40 => ("CALENDAR_ERROR_NO_MODERATOR", Bare),
        _ => return None,
    })
}

/// `key`'s template, its `%s` or `%d` filled with the argument after a NUL when there is one (a
/// bare key is the template as is, `0x5b9960`'s no-argument leg). Labelled: the `|4one:many;`
/// plural escapes are left to the text renderer.
pub(super) fn error_line(script: &UiScript, packed: &str) -> String {
    let (key, arg) = match packed.split_once('\0') {
        Some((key, arg)) => (key, Some(arg)),
        None => (packed, None),
    };
    let template = script
        .lua()
        .globals()
        .get::<Option<String>>(key)
        .ok()
        .flatten()
        .unwrap_or_else(|| key.to_string());
    match arg {
        Some(arg) if template.contains("%d") => template.replacen("%d", arg, 1),
        Some(arg) => template.replacen("%s", arg, 1),
        None => template,
    }
}
