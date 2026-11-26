//! The chat windows' saved state, the whole `chat-cache.txt` record: the per-type colours and each
//! window's tint, alpha, font size, lock, dock, shown flag, name, message groups and channels. It
//! lives in the VM, written and read by the stock verbs (`SetChatWindow*`, `ChangeChatColor`,
//! `GetChatWindowInfo` and kin); this module loads it at login and saves it at logout, in the
//! grammar of the reference's `WTF/Account/<ACC>/<REALM>/<CHAR>/chat-cache.txt` (written whole by
//! `0x499a80`, read by `0x498a60`):
//!
//! ```text
//! VERSION 5
//! ADDEDVERSION 13
//! OPTION_GUILD_RECRUITMENT_CHANNEL AUTO
//! CHANNELS … END           ← the custom channels, one per line, re-joined at login
//! ZONECHANNELS 18874371    ← the joined zone channels, as bits 1<<(ChannelID-1)
//! COLORS … END             ← the chat-type registry, one SAY 255 255 255 N row per type, the
//!                            trailing Y/N its colorNameByClass
//! WINDOW 1                 ← then per window:
//! NAME General             ← only when a name was stored
//! SIZE 0
//! COLOR 0 0 0 0            ← R G B A bytes, from the record's packed BGRA quad
//! LOCKED 1
//! UNINTERACTABLE 0
//! DOCKED 1
//! SHOWN 1
//! POSITION TOPLEFT 0 0     ← only once a position was saved: the anchor, its screen ratios,
//! DIMENSIONS 430 120       ← then the size, each `%f` (`0x4fd870`)
//! MESSAGES … END           ← the enabled groups of the 46-entry CHATMSGGROUP table, in order
//! CHANNELS … END           ← this window's custom channels; its zone channels are bits
//! ZONECHANNELS 18874371    ← this window's zone channels, masked by the joined set
//! END
//! ```
//!
//! Build 12340's grammar is taken from a real 12340-written `chat-cache.txt` (VERSION 5,
//! ADDEDVERSION 13, the `Y`/`N` colour flag, the `UNINTERACTABLE` line); the 1.12 addresses below
//! name the loader and writer whose shape it keeps. The loader zeroes every window's flags first,
//! so a `MESSAGES` block is the whole set; a window the file omits keeps its stock record
//! ([`ChatWindowLook::stock`]), and a file older than `ADDEDVERSION 13` gets the `defaultOn` groups
//! added since: general groups into window 1, combat-log groups into window 2. Label: the 12340
//! back-fill rule is inferred from the group table's version words, not audited.
//! A cache WrathPlus wrote under the 1.12 grammar says `ADDEDVERSION 2`, so its first load under
//! the 12340 tables back-fills every `defaultOn` group newer than version 2 and drops names the
//! 46-entry table lacks: a one-time migration that can bring back groups a player had switched off. `LOCKED` is an `i32` (`CHATWINDOW+0x8c`) written
//! through `setne`, so only 0 and 1 round-trip. Deviation: `set_chat_window_looks` drops an
//! out-of-range window, where the reference's `WINDOW` bound is off by one (`0x498d1c`, `ja` where
//! the array needs `jae`), because that write would land past the array.
//!
//! Ours is `wrathplus-config/chat/<realm>-<character>.txt`
//! ([`crate::local_state::chat_character_path`]); its reader is as lenient as the reference's
//! (keys case-insensitive, unknown keys and blank lines skipped) and also reads WrathPlus's older
//! one-line `WINDOW 1  SIZE 0  COLOR …` rows. `UPDATE_CHAT_WINDOWS`, then `UPDATE_CHAT_COLOR` per
//! registry entry, fire file or no file (`0x4996b9`, `0x499934`). Saves wait one quiet second, as
//! the opacity slider writes on every drag step, and flush at the session end and at `AppExit`.

use std::path::PathBuf;

use bevy::prelude::*;

use wrathplus_ui::script::{
    ChatTypeColor, ChatWindowLook, UiScript, COMBAT_LOG_GROUPS_START, MESSAGE_GROUPS,
};

use super::edit::zone_bit;
use crate::app_core::vm_memo::VmMemo;
use crate::net::{ClientCommand, NetCommands};

/// How long a dirty look sits before the save fires, as in `crate::cvars`.
const SAVE_QUIET: std::time::Duration = std::time::Duration::from_secs(1);

/// The file's header, as comment lines: the reference's reader has no comments, ours skips them.
const HEADER: &str = "\
# WrathPlus chat cache, in the reference's chat-cache.txt grammar (written by
# 0x499a80, read by 0x498a60): the custom channels to re-join, the joined zone channels as bits, the
# per-type COLORS table (r g b and the Y/N colorNameByClass), then one WINDOW block per chat frame —
# NAME (when one was set), SIZE, COLOR r g b a as bytes, LOCKED, UNINTERACTABLE, DOCKED, SHOWN,
# POSITION and DIMENSIONS once a position was saved, the MESSAGES … END list of the groups the window
# shows, its CHANNELS … END list of custom channels, and its zone channels as ZONECHANNELS bits.
# Written whole; the tab menu, /join and ChangeChatColor are what move it.
";

/// WrathPlus's repair marker, a header comment line, not the reference's grammar. A file without it
/// may hold `ZONECHANNELS 0` words from an empty roster, which strip window 1 of its channels;
/// [`restore_chat_looks`] re-seeds such a file once, and the next save stamps the marker. It is
/// matched with `contains`, so a marker line with more text after it still counts.
const WRITER_GENERATION: &str = "# wrathplus-writer 2";

/// Which character's file we are on, where it lives, and whether it is owed a write.
#[derive(Resource, Default)]
pub(super) struct ChatWindowFile {
    path: Option<PathBuf>,
    /// The `(realm, character)` [`Self::path`] was built for, per VM: a fresh VM restores again.
    identity: VmMemo<Option<(String, String)>>,
    /// Whether this VM has unsaved Lua writes, per VM: a replaced VM's stock table must not save.
    dirty: VmMemo<bool>,
    last_change: Option<std::time::Instant>,
}

/// A parsed file: its windows (0-based), its `COLORS` rows in file order, its custom channels.
#[derive(Debug, PartialEq)]
pub(super) struct Parsed {
    pub(super) looks: Vec<(usize, ChatWindowLook)>,
    /// `(name, rgb, colorNameByClass)`; a row without the flag reads `N`.
    pub(super) colors: Vec<(String, [u8; 3], bool)>,
    pub(super) joined: Vec<String>,
    /// The header's `ZONECHANNELS` word, the reference's `[0xb6e5e0]`, overwritten from the file
    /// (`0x498d83`); `None` without the line, so the loader seeds from the DBC rather than 0.
    pub(super) zone_mask: Option<u32>,
    /// `OPTION_GUILD_RECRUITMENT_CHANNEL`, the recruitment auto-join latch: `STANDARD`
    /// is 0 and any other word, or none, is 1 (`0x49ea70`, `ecx=0` only for `STANDARD`).
    pub(super) guild_recruitment_auto: bool,
}

impl Default for Parsed {
    fn default() -> Self {
        Self {
            looks: Vec::new(),
            colors: Vec::new(),
            joined: Vec::new(),
            zone_mask: None,
            // The boot value: the reference writes `AUTO` for characters that never set it.
            guild_recruitment_auto: true,
        }
    }
}

/// Render the file as the writer does (`0x499a80`). `custom` fills the header's `CHANNELS`, custom
/// channels only: a zone channel travels as its bit (`0x499ba1`). `zone_mask` goes raw into the
/// header (`0x499c19`) and is ANDed with each window's bits (`0x49a133`, `0x49a138`). Both are the
/// durable lists, never the live roster, which the session end clears before the flush reads it.
fn render(
    looks: &[ChatWindowLook],
    colors: &[ChatTypeColor],
    custom: &[String],
    zone_mask: u32,
    guild_recruitment_auto: bool,
) -> String {
    let mut out = String::from(HEADER);
    // The repair marker, a comment line.
    out.push_str(WRITER_GENERATION);
    out.push('\n');
    // The recruitment latch; this is the one place it persists.
    let recruitment = if guild_recruitment_auto {
        "AUTO"
    } else {
        "STANDARD"
    };
    out.push_str(&format!(
        "\nVERSION 5\n\nADDEDVERSION 13\n\nOPTION_GUILD_RECRUITMENT_CHANNEL {recruitment}\n\nCHANNELS\n"
    ));
    for name in custom {
        out.push_str(name);
        out.push('\n');
    }
    out.push_str(&format!("END\n\nZONECHANNELS {zone_mask}\n\nCOLORS\n"));
    for c in colors {
        out.push_str(&format!(
            "{} {} {} {} {}\n",
            c.name,
            c.rgb[0],
            c.rgb[1],
            c.rgb[2],
            if c.name_by_class { 'Y' } else { 'N' }
        ));
    }
    out.push_str("END\n\n");
    for (i, l) in looks.iter().enumerate() {
        out.push_str(&format!("WINDOW {}\n", i + 1));
        if !l.name.is_empty() {
            out.push_str(&format!("NAME {}\n", l.name));
        }
        out.push_str(&format!(
            "SIZE {}\nCOLOR {} {} {} {}\nLOCKED {}\nUNINTERACTABLE {}\nDOCKED {}\nSHOWN {}\n",
            l.font_size,
            l.r,
            l.g,
            l.b,
            l.a,
            i32::from(l.locked),
            i32::from(l.uninteractable),
            l.docked.map_or(0, |d| d as i32),
            i32::from(l.shown),
        ));
        // Both geometry lines once the saved flag is set (`0x4fd870`), `%f` of each float.
        let geo = &l.geometry;
        if geo.saved {
            out.push_str(&format!(
                "POSITION {} {:.6} {:.6}\nDIMENSIONS {:.6} {:.6}\n",
                geo.point_name(),
                f64::from(geo.x),
                f64::from(geo.y),
                f64::from(geo.width),
                f64::from(geo.height),
            ));
        }
        out.push_str("\nMESSAGES\n");
        for m in &l.messages {
            out.push_str(m);
            out.push('\n');
        }
        out.push_str("END\n\nCHANNELS\n");
        let mut window_mask = 0;
        for (name, id) in &l.channels {
            if *id == 0 {
                out.push_str(name);
                out.push('\n');
            } else {
                window_mask |= zone_bit(*id);
            }
        }
        out.push_str(&format!(
            "END\n\nZONECHANNELS {}\n\nEND\n\n",
            window_mask & zone_mask
        ));
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    /// Top level, or inside a `WINDOW` whose keys arrive one per line.
    Top,
    /// The header's `CHANNELS … END`: the custom channels to re-join.
    Joined,
    Colors,
    Messages,
    Channels,
}

/// Parse a file. `rows` is `ChatChannels.dbc` as `(id, Shortcut)`, which the in-window
/// `ZONECHANNELS` arm needs to turn bits back into `(Shortcut, id)` channel rows.
fn parse(text: &str, rows: &[(u32, String)]) -> Parsed {
    let mut out = Parsed::default();
    let mut current: Option<(usize, ChatWindowLook)> = None;
    let mut block = Block::Top;
    let mut added_version: u8 = 0;
    let byte = |s: Option<&str>| -> u8 { s.and_then(|v| v.parse::<u8>().ok()).unwrap_or(0) };
    let flush = |current: &mut Option<(usize, ChatWindowLook)>, out: &mut Parsed| {
        if let Some(w) = current.take() {
            out.looks.push(w);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(head) = it.next() else { continue };
        let is_end = head.eq_ignore_ascii_case("END");
        match block {
            Block::Colors => {
                if is_end {
                    block = Block::Top;
                } else {
                    let rgb = [byte(it.next()), byte(it.next()), byte(it.next())];
                    let by_class = it.next().is_some_and(|f| f.eq_ignore_ascii_case("Y"));
                    out.colors.push((head.to_ascii_uppercase(), rgb, by_class));
                }
                continue;
            }
            Block::Joined => {
                if is_end {
                    block = Block::Top;
                } else {
                    out.joined.push(line.to_string());
                }
                continue;
            }
            Block::Messages => {
                if is_end {
                    block = Block::Top;
                } else if let Some((_, look)) = current.as_mut() {
                    look.messages.push(head.to_ascii_uppercase());
                }
                continue;
            }
            Block::Channels => {
                if is_end {
                    block = Block::Top;
                } else if let Some((_, look)) = current.as_mut() {
                    // A word that is a `ChatChannels.dbc` Shortcut is a zone channel that lost its
                    // id: the reference never stores one by name (`AddChatWindowChannel`
                    // `0x4a1000` resolves shortcuts first), and `ChatFrame.lua:1379` matches ids.
                    let id = rows
                        .iter()
                        .find(|(_, shortcut)| shortcut.eq_ignore_ascii_case(head))
                        .map_or(0, |(id, _)| *id);
                    look.channels.push((head.to_string(), id));
                }
                continue;
            }
            Block::Top => {}
        }
        if head.eq_ignore_ascii_case("COLORS") {
            block = Block::Colors;
            continue;
        }
        if head.eq_ignore_ascii_case("WINDOW") {
            flush(&mut current, &mut out);
            let Some(index) = it.next().and_then(|n| n.parse::<usize>().ok()) else {
                warn!("chat cache: WINDOW line with no number ignored: {line}");
                continue;
            };
            if index == 0 {
                continue;
            }
            let mut look = ChatWindowLook::stock(index - 1);
            // WrathPlus's older one-line row: the keys follow on the same line.
            while let Some(key) = it.next() {
                apply_key(&mut look, key, &mut it, rows);
            }
            current = Some((index - 1, look));
            continue;
        }
        let Some((_, look)) = current.as_mut() else {
            if head.eq_ignore_ascii_case("CHANNELS") {
                block = Block::Joined;
            } else if head.eq_ignore_ascii_case("ADDEDVERSION") {
                added_version = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            } else if head.eq_ignore_ascii_case("ZONECHANNELS") {
                // Overwrites `[0xb6e5e0]` (`0x498d83`), never ORs: the file holds the whole mask.
                out.zone_mask = it.next().and_then(|v| v.trim().parse::<u32>().ok());
            } else if head.eq_ignore_ascii_case("OPTION_GUILD_RECRUITMENT_CHANNEL") {
                // The reference's test, whole-word and case-folded: only `STANDARD` means 0.
                out.guild_recruitment_auto = !it
                    .next()
                    .is_some_and(|w| w.eq_ignore_ascii_case("STANDARD"));
            }
            // `VERSION` is not read.
            continue;
        };
        if is_end {
            flush(&mut current, &mut out);
        } else if head.eq_ignore_ascii_case("MESSAGES") {
            look.messages.clear();
            block = Block::Messages;
        } else if head.eq_ignore_ascii_case("CHANNELS") {
            look.channels.retain(|(_, id)| *id != 0);
            block = Block::Channels;
        } else if head.eq_ignore_ascii_case("NAME") {
            look.name = line[4..].trim().to_string();
        } else {
            apply_key(look, head, &mut it, rows);
        }
    }
    flush(&mut current, &mut out);
    // The loader's EOF back-fill (`0x49967c` in 1.12): an older file gets the `defaultOn` groups
    // added since, general groups into window 1 and combat-log groups into window 2.
    if added_version < 13 {
        for (i, (name, on, ver)) in MESSAGE_GROUPS.iter().enumerate() {
            if *on && *ver > added_version {
                let target = usize::from(i >= COMBAT_LOG_GROUPS_START);
                if let Some((_, look)) = out.looks.iter_mut().find(|(w, _)| *w == target) {
                    if !look.messages.iter().any(|m| m == name) {
                        look.messages.push((*name).to_string());
                    }
                }
            }
        }
    }
    for (_, look) in &mut out.looks {
        look.normalize_messages();
    }
    out
}

/// One `KEY value…` of a window block, whichever line it arrived on.
fn apply_key<'a>(
    look: &mut ChatWindowLook,
    key: &str,
    it: &mut impl Iterator<Item = &'a str>,
    rows: &[(u32, String)],
) {
    let byte = |s: Option<&str>| -> u8 { s.and_then(|v| v.parse::<u8>().ok()).unwrap_or(0) };
    let flag = |s: Option<&str>| -> bool { s.is_none_or(|v| v.trim() != "0") };
    if key.eq_ignore_ascii_case("SIZE") {
        look.font_size = it
            .next()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
            .max(0);
    } else if key.eq_ignore_ascii_case("COLOR") {
        look.r = byte(it.next());
        look.g = byte(it.next());
        look.b = byte(it.next());
        look.a = byte(it.next());
    } else if key.eq_ignore_ascii_case("LOCKED") {
        look.locked = flag(it.next());
    } else if key.eq_ignore_ascii_case("UNINTERACTABLE") {
        look.uninteractable = it.next().is_some_and(|v| v.trim() != "0");
    } else if key.eq_ignore_ascii_case("DOCKED") {
        look.docked = it
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok().map(|d| d as u32))
            .filter(|p| *p != 0);
    } else if key.eq_ignore_ascii_case("SHOWN") {
        look.shown = flag(it.next());
    } else if key.eq_ignore_ascii_case("POSITION") || key.eq_ignore_ascii_case("DIMENSIONS") {
        // The reader (`0x508fcc`, `0x509089`): an unknown point keeps the stored one, and either
        // line sets the saved flag.
        let float = |s: Option<&str>| s.and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let geo = &mut look.geometry;
        if key.eq_ignore_ascii_case("POSITION") {
            geo.set_point(it.next().unwrap_or(""));
            (geo.x, geo.y) = (float(it.next()), float(it.next()));
        } else {
            (geo.width, geo.height) = (float(it.next()), float(it.next()));
        }
        geo.saved = true;
    } else if key.eq_ignore_ascii_case("ZONECHANNELS") {
        // The in-window arm: every DBC row whose bit is set joins the window as (Shortcut, id).
        let mask = it
            .next()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        for (id, shortcut) in rows {
            if mask & zone_bit(*id) != 0
                && !look
                    .channels
                    .iter()
                    .any(|(c, _)| c.eq_ignore_ascii_case(shortcut))
            {
                look.channels.push((shortcut.clone(), *id));
            }
        }
    }
}

/// The `(id, Shortcut)` rows the parser needs, off the loaded `ChatChannels.dbc`.
fn shortcut_rows(channels: &super::edit::ChannelState) -> Vec<(u32, String)> {
    channels
        .channels
        .rows()
        .iter()
        .map(|r| (r.id, r.shortcut.clone()))
        .collect()
}

/// Restore the file into a fresh VM, once per character per VM, and fire the loader's two events,
/// file or no file. Called from the world-entry UI load, not `Update`, for those events:
/// `UPDATE_CHAT_WINDOWS` registers a window for any `CHAT_MSG_*` (`ChatFrame.lua:1261-1273`), so
/// it must precede the server's MOTD after `SMSG_LOGIN_VERIFY_WORLD`, and the `UPDATE_CHAT_COLOR`
/// burst must precede `PLAYER_LOGIN`: its `WHISPER` row also repaints `ChatTypeInfo["REPLY"]`
/// (`ChatFrame.lua:1357-1365`), whose `.id` is 0, and `UpdateColorByID(0, …)` recolours every line
/// printed with no explicit colour, an addon's login `Print` among them.
pub(crate) fn restore_chat_looks(world: &mut World, script: &mut UiScript) {
    let Some(id) = world
        .get_resource::<crate::app_core::state::Roster>()
        .and_then(crate::app_core::state::identity)
    else {
        return;
    };
    // Owned rows up front: `ChatWindowFile` is borrowed mutably below.
    let Some(channels) = world.get_resource::<super::edit::ChannelState>() else {
        return;
    };
    let rows = shortcut_rows(channels);
    let auto_rows: Vec<(String, u32)> = channels
        .channels
        .auto_join_rows()
        .map(|r| (r.shortcut.clone(), r.id))
        .collect();
    let commands = world.get_resource::<NetCommands>().map(|c| c.0.clone());
    // The DBC seed: every `ChatChannels.dbc` row the client joins by itself (`flags & 1`).
    let seed_mask = auto_rows.iter().fold(0, |m, (_, id)| m | zone_bit(*id));
    // Scoped, so `ChannelState` can take the mask afterwards.
    let mut parsed;
    {
        let Some(mut file) = world.get_resource_mut::<ChatWindowFile>() else {
            return;
        };
        if file.identity.get(script).as_ref() == Some(&id) {
            return; // already restored for this character into this VM
        }
        let who = format!("{} on {}", id.1, id.0);
        file.path = crate::local_state::chat_character_path(&id.0, &id.1);
        *file.identity.get(script) = Some(id);
        *file.dirty.get(script) = false;
        file.last_change = None;
        let text = file
            .path
            .as_ref()
            .and_then(|path| match std::fs::read_to_string(path) {
                Ok(t) => Some(t),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    warn!("chat cache: cannot read {}: {e}", path.display());
                    None
                }
            });
        let had_file = text.is_some();
        // No marker: `ZONECHANNELS` may be empty. Repaired once.
        let damaged = text
            .as_deref()
            .is_some_and(|t| !t.contains(WRITER_GENERATION));
        parsed = text.map(|t| parse(&t, &rows)).unwrap_or_default();
        if !had_file {
            // The loader's no-file path (`0x4997ad`): window 1 gets each seed row as
            // `(Shortcut, id)`, which `ChatFrame_RegisterForChannels` matches zone speech by.
            let mut general = ChatWindowLook::stock(0);
            general.channels = auto_rows.clone();
            parsed.looks.push((0, general));
        }
        if !parsed.looks.is_empty() || !parsed.colors.is_empty() {
            info!(
                "chat cache: {} windows, {} colour rows, {} custom channels restored",
                parsed.looks.len(),
                parsed.colors.len(),
                parsed.joined.len()
            );
        }
        if damaged {
            // The one-time repair: re-seed window 1 and OR the seed into the mask, additive and
            // deduplicated like the in-window `ZONECHANNELS` arm (`0x499332`-`0x4994e7`).
            if let Some((_, general)) = parsed.looks.iter_mut().find(|(w, _)| *w == 0) {
                for (shortcut, id) in &auto_rows {
                    if !general
                        .channels
                        .iter()
                        .any(|(c, _)| c.eq_ignore_ascii_case(shortcut))
                    {
                        general.channels.push((shortcut.clone(), *id));
                    }
                }
            }
            parsed.zone_mask = Some(parsed.zone_mask.unwrap_or(0) | seed_mask);
            // Owed a write, which stamps the marker; it saves next frame (`last_change` is `None`).
            *file.dirty.get(script) = true;
            info!("chat cache: repaired an unmarked file's zone channels for {who}");
        }
    }
    // The durable mask: the file's word, else the DBC seed; confirmed joins OR into it after.
    let mask = parsed.zone_mask.unwrap_or(seed_mask);
    if let Some(mut channels) = world.get_resource_mut::<super::edit::ChannelState>() {
        // `Some` is the reference's "chat system ready" flag (`0x499a18`): the zone walk and the
        // guild-recruitment cascade wait for it.
        channels.zone_mask = Some(mask);
        // The custom re-join list, seated here, then moved only by a confirmed join or a leave.
        channels.custom = parsed.joined.clone();
    }
    script.set_guild_recruitment_mode(u8::from(parsed.guild_recruitment_auto));
    script.set_chat_colors(parsed.colors);
    script.set_chat_window_looks(parsed.looks);
    // The auto-join rows window 1 gets back on `ResetChatWindows` (`0x501a70`). The startup
    // `load_chat_channels` has read them before any in-game UI load, and every load re-seeds.
    script.set_chat_channel_seed(auto_rows);
    // `UPDATE_CHAT_WINDOWS` once, then `UPDATE_CHAT_COLOR` for every registry entry, file or no
    // file (`0x4996b9`, `0x499934`). Each colour is followed by its
    // `UPDATE_CHAT_COLOR_NAME_BY_CLASS`, the pair 12340's ResetChatWindows fires (`0x501f00`), so a
    // restored flag reaches `ChatTypeInfo`. Label: the loader's own pair was not audited.
    script.fire_event("UPDATE_CHAT_WINDOWS", vec![]);
    let renorm = |b: u8| f64::from(b as f32 * (1.0f32 / 255.0f32));
    for entry in script.chat_colors() {
        script.fire_event(
            "UPDATE_CHAT_COLOR",
            vec![
                wrathplus_ui::script::ScriptValue::Str(entry.name.clone()),
                wrathplus_ui::script::ScriptValue::Number(renorm(entry.rgb[0])),
                wrathplus_ui::script::ScriptValue::Number(renorm(entry.rgb[1])),
                wrathplus_ui::script::ScriptValue::Number(renorm(entry.rgb[2])),
            ],
        );
        script.fire_event(
            "UPDATE_CHAT_COLOR_NAME_BY_CLASS",
            vec![
                wrathplus_ui::script::ScriptValue::Str(entry.name),
                wrathplus_ui::script::ScriptValue::Bool(entry.name_by_class),
            ],
        );
    }
    // The header's `CHANNELS` re-join at login, as in the reference; zone channels are the walk's.
    if let Some(commands) = commands {
        for name in parsed.joined {
            // Saved custom channels; zone channels re-join through the walk with their row.
            let _ = commands.send(ClientCommand::JoinChannel {
                name,
                password: String::new(),
                channel_id: 0,
            });
        }
    }
}

/// Arm the debounce when Lua writes the chat state.
fn watch_chat_looks(script: Option<NonSendMut<UiScript>>, mut file: ResMut<ChatWindowFile>) {
    let Some(mut script) = script else { return };
    let moved = !script.take_chat_window_changes().is_empty();
    let coloured = script.take_chat_color_changes();
    // The recruitment latch persists here too; a manual join or leave arms it, the login seat never.
    let recruitment = script.take_guild_recruitment_change();
    if !(moved || coloured || recruitment) {
        return;
    }
    *file.dirty.get(&script) = true;
    file.last_change = Some(std::time::Instant::now());
}

fn write(script: &UiScript, channels: &super::edit::ChannelState, path: &std::path::Path) {
    // Never write from an unseated mask: `None` means the file was never read, and writing would
    // zero every block's `ZONECHANNELS`.
    let Some(zone_mask) = channels.zone_mask else {
        warn!(
            "chat cache: refusing to write {} — the zone mask has not been seated",
            path.display()
        );
        return;
    };
    let body = render(
        &script.chat_window_looks(),
        &script.chat_colors(),
        &channels.custom,
        zone_mask,
        script.guild_recruitment_mode() != 0,
    );
    if let Err(e) = crate::local_state::write_atomic(path, &body) {
        warn!("chat cache: cannot write {}: {e}", path.display());
    }
}

/// Whether a write is owed now.
///
/// - A flush (`exiting`: session end or window close) writes whenever this VM restored the file
///   (`restored`, never a fresh VM's stock table), as the reference rewrites it whole at chat
///   teardown (`0x499a80` from `0x490c55`) with no dirty check (`[0xb6e5c4]` is never read); host
///   state such as the zone mask never marks the file dirty.
/// - Deviation: the debounce also writes Lua's changes (`dirty`) after the quiet time, where the
///   reference writes only at teardown, so a crash loses at most a second of changes.
fn owes_write(exiting: bool, restored: bool, dirty: bool, quiet: bool) -> bool {
    if exiting {
        restored
    } else {
        dirty && quiet
    }
}

/// The write itself, on the terms [`owes_write`] set.
fn flush(
    script: &UiScript,
    channels: &super::edit::ChannelState,
    file: &mut ChatWindowFile,
    exiting: bool,
) {
    let restored = file.identity.get(script).is_some();
    let dirty = *file.dirty.get(script);
    let quiet = file.last_change.is_none_or(|t| t.elapsed() >= SAVE_QUIET);
    if !owes_write(exiting, restored, dirty, quiet) {
        return;
    }
    if let Some(path) = file.path.clone() {
        write(script, channels, &path);
    }
    *file.dirty.get(script) = false;
}

/// The debounced save, and the `AppExit` flush.
fn save_chat_looks(
    script: Option<NonSendMut<UiScript>>,
    channels: Res<super::edit::ChannelState>,
    mut file: ResMut<ChatWindowFile>,
    mut exits: MessageReader<AppExit>,
) {
    let exiting = exits.read().next().is_some();
    let Some(script) = script else { return };
    flush(&script, &channels, &mut file, exiting);
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<ChatWindowFile>()
        // The restore runs from the world-entry UI load, not here: its events must precede the
        // first chat line and `PLAYER_LOGIN`, which no `Update` ordering guarantees.
        .add_systems(
            Update,
            // After the tick: every write it watches is Lua's.
            watch_chat_looks
                .after(crate::app_core::schedule::UiInput)
                .in_set(crate::app_core::schedule::InWorldGated),
        )
        .add_systems(
            Update,
            save_chat_looks
                .after(crate::app_core::schedule::UiInput)
                .in_set(crate::app_core::schedule::InWorldGated),
        );
    // The session-end flush is `fold_dying_vm_chat_cache`; the quit flush rides the exit edge,
    // as the close button's `AppExit` is written in `PostUpdate`, after an `Update` save.
    crate::shutdown::on_app_exit(app, save_chat_looks.into_configs());
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
