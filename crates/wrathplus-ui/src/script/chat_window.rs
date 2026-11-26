//! The chat window records and the window verbs that set them; `GetChatWindowInfo`, the saved
//! geometry and `SetChatWindowUninteractable` are in `wrath_chat_window`.
//!
//! Build 12340 keeps one record per window (`0xbcf0b0`, stride 0x100, ten slots; the layout is
//! `wrath_chat_window`'s header), loaded from and saved to `chat-cache.txt`. Every window verb tests
//! its arguments' types first and raises its own `Usage:`, then reads the id by `fistp` truncation
//! and does nothing, answering nothing, for an id outside 1..10 (`0x4fbdc7`-`0x4fbdf4`): stock's
//! temporary windows are 11 up (`FloatingChatFrame.lua:703-713`) and call the same setters.
//!
//! `SetChatWindowColor` (`0x4fc470`) and `SetChatWindowAlpha` (`0x4fc5b0`) store `fistp(x · 255.0)`
//! truncated (`[0x9eaf78]` is the 255.0), so a round trip quantises: alpha 0.4 reads back 102/255,
//! 0.5 reads 127/255. Deviation: a value outside 0..1 clamps, where the reference keeps the low byte
//! (alpha 2.0 stores 254, -1.0 stores 1), because no FrameXML or addon caller passes one.

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{bool_or_default, coerced_number, ftol, is_number, is_string};
use super::channel::ZoneChannelRow;
use super::Model;
use crate::layout::Point;

/// The reference's record slots (`0xbcf0b0`), one per `ChatFrame1`..`ChatFrame10`.
pub(super) const ENGINE_CHAT_WINDOW_SLOTS: usize = 10;

/// `CHATMSGGROUP`, build 12340's 46-entry message-group table (`0x9fb7c0`, stride 0xc):
/// `(name, defaultOn, addedVersion)` in the order of the record's flags
/// (`/tmp/azb-ui-chat-tables*.log`). The first 40 are the general window's: ResetChatWindows
/// (`0x501f00`) enables their `defaultOn` ones in window 1. The last six are the combat log's,
/// all enabled in window 2. A real 12340 `chat-cache.txt` lists exactly these names.
pub const MESSAGE_GROUPS: [(&str, bool, u8); 46] = [
    ("SYSTEM", true, 0),
    ("SYSTEM_NOMENU", true, 5),
    ("SAY", true, 0),
    ("EMOTE", true, 7),
    ("YELL", true, 0),
    ("WHISPER", true, 0),
    ("PARTY", true, 0),
    ("PARTY_LEADER", true, 10),
    ("RAID", true, 7),
    ("RAID_LEADER", true, 7),
    ("RAID_WARNING", true, 7),
    ("BATTLEGROUND", true, 7),
    ("BATTLEGROUND_LEADER", true, 7),
    ("GUILD", true, 0),
    ("OFFICER", true, 7),
    ("MONSTER_SAY", true, 7),
    ("MONSTER_YELL", true, 7),
    ("MONSTER_EMOTE", true, 7),
    ("MONSTER_WHISPER", true, 7),
    ("MONSTER_BOSS_EMOTE", true, 7),
    ("MONSTER_BOSS_WHISPER", true, 7),
    ("ERRORS", true, 7),
    ("AFK", true, 7),
    ("DND", true, 7),
    ("IGNORED", true, 7),
    ("BG_HORDE", true, 7),
    ("BG_ALLIANCE", true, 7),
    ("BG_NEUTRAL", true, 7),
    ("COMBAT_FACTION_CHANGE", true, 1),
    ("SKILL", true, 0),
    ("LOOT", true, 0),
    ("MONEY", true, 2),
    ("CHANNEL", true, 0),
    ("ACHIEVEMENT", true, 8),
    ("GUILD_ACHIEVEMENT", true, 8),
    ("TARGETICONS", false, 11),
    ("BN_WHISPER", true, 12),
    ("BN_WHISPER_INFORM", true, 12),
    ("BN_CONVERSATION", true, 12),
    ("BN_INLINE_TOAST_ALERT", true, 13),
    ("OPENING", true, 6),
    ("TRADESKILLS", true, 6),
    ("PET_INFO", true, 6),
    ("COMBAT_XP_GAIN", true, 0),
    ("COMBAT_HONOR_GAIN", true, 0),
    ("COMBAT_MISC_INFO", true, 0),
];

/// The first combat-log group: groups before it are window 1's, the rest window 2's.
pub const COMBAT_LOG_GROUPS_START: usize = 40;

/// The flag index of a group name, case-folded as the loader's `SStrCmpI` matches it.
pub fn message_group_index(name: &str) -> Option<usize> {
    MESSAGE_GROUPS
        .iter()
        .position(|(n, _, _)| n.eq_ignore_ascii_case(name))
}

/// A window's saved geometry, record `+0xe8..+0xfc`: the anchor `0x814ed0` names, x and y as
/// screen ratios, width and height, and the `+0xf4` flag that `SetChatWindowSavedPosition` and
/// either cache line set (`0x4fc97c`, `0x50907d`, `0x509104`); the cache writes both lines only
/// under it (`0x4fd870`). The default is the static zeros (`0x9c9360`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ChatWindowGeometry {
    pub saved: bool,
    pub point: Point,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ChatWindowGeometry {
    const ZERO: Self = Self {
        saved: false,
        point: Point::TopLeft,
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    /// The anchor's name, as `0x814f20` turns the enum back.
    pub fn point_name(&self) -> &'static str {
        super::object::point_name(self.point)
    }

    /// The anchor `name` names, case-folded (`0x814ed0`); false, changing nothing, for none.
    pub fn set_point(&mut self, name: &str) -> bool {
        super::object::point_from_str(name)
            .map(|p| self.point = p)
            .is_some()
    }
}

/// One window's record as `chat-cache.txt` persists it, in the reference's own field types.
#[derive(Clone, PartialEq, Debug)]
pub struct ChatWindowLook {
    /// Background tint, one byte per channel.
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Background alpha byte, `0x28` in the default record (`0x501813`) until a setter moves it.
    pub a: u8,
    /// `SIZE`: the font height, or 0 for the font's own; the setter never stores `<= 0`.
    pub font_size: i32,
    /// `LOCKED`, stock true for every window; the tab menu's `FCF_ToggleLock` moves it.
    pub locked: bool,
    /// `DOCKED` (`+0xdc`), the dock position `FCF_DockFrame(frame, docked)` takes, `None` for 0:
    /// a dword, which `GetChatWindowInfo` reads unsigned and the cache writes signed.
    pub docked: Option<u32>,
    /// `+0x00`: `""` until `SetChatWindowName` stores one; `NAME` is written only when non-empty.
    pub name: String,
    /// `+0xe0`, `SHOWN`.
    pub shown: bool,
    /// `+0x80`: the enabled message-group names, the `MESSAGES … END` block.
    pub messages: Vec<String>,
    /// `+0xb8`/`+0xc8`: `(name, zone channel id)` pairs, the `CHANNELS … END` block.
    pub channels: Vec<(String, u32)>,
    /// `+0xe4`, `SetChatWindowUninteractable`; `GetChatWindowInfo`'s tenth value.
    pub uninteractable: bool,
    /// `+0xe8..+0xfc`, the `POSITION` and `DIMENSIONS` lines.
    pub geometry: ChatWindowGeometry,
}

impl Default for ChatWindowLook {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl ChatWindowLook {
    /// The default record (`0x501800`) of windows 3 to 10, which [`Self::stock`] builds 1 and 2 on.
    pub(super) const DEFAULT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0x28,
        font_size: 0,
        locked: true,
        docked: None,
        name: String::new(),
        shown: false,
        messages: Vec::new(),
        channels: Vec::new(),
        uninteractable: false,
        geometry: ChatWindowGeometry::ZERO,
    };

    /// Window `index`'s (0-based) stock record, what ResetChatWindows (`0x501f00`) leaves and a
    /// client with no `chat-cache.txt` runs from: window 1 shown and docked at 1 with the
    /// `defaultOn` general groups; window 2 shown and docked at 2 with the six combat-log groups;
    /// the rest hidden with no groups. Label: 12340's boot init was not audited separately and is
    /// taken to equal the reset.
    pub fn stock(index: usize) -> Self {
        let names = |range: std::ops::Range<usize>, all: bool| -> Vec<String> {
            MESSAGE_GROUPS[range]
                .iter()
                .filter(|(_, on, _)| all || *on)
                .map(|(n, _, _)| (*n).to_string())
                .collect()
        };
        match index {
            0 => Self {
                shown: true,
                docked: Some(1),
                messages: names(0..COMBAT_LOG_GROUPS_START, false),
                ..Self::DEFAULT
            },
            1 => Self {
                shown: true,
                docked: Some(2),
                messages: names(COMBAT_LOG_GROUPS_START..MESSAGE_GROUPS.len(), true),
                ..Self::DEFAULT
            },
            _ => Self::DEFAULT,
        }
    }

    /// Drops unknown names and duplicates and puts the rest in table order, so
    /// `GetChatWindowMessages` (`0x4fc0c0`) answers as the reference's flag walk does.
    pub fn normalize_messages(&mut self) {
        let mut flags = [false; MESSAGE_GROUPS.len()];
        for m in &self.messages {
            if let Some(i) = message_group_index(m) {
                flags[i] = true;
            }
        }
        self.messages = MESSAGE_GROUPS
            .iter()
            .zip(flags)
            .filter(|(_, on)| *on)
            .map(|((n, _, _), _)| (*n).to_string())
            .collect();
    }
}

/// The leading decimal integer after optional whitespace and a sign, 0 when there is none. The
/// reference's `SStrToInt` (`0x64ac60`) takes neither the whitespace nor a `+`.
pub(super) fn leading_int(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let n: i64 = digits
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .fold(0i64, |acc, c| {
            acc.saturating_mul(10)
                .saturating_add(i64::from(c as u8 - b'0'))
        });
    if neg {
        -n
    } else {
        n
    }
}

/// The byte `fistp(x · 255.0)` stores, clamped (the module's deviation); non-finite stores 0.
fn to_byte(x: f64) -> u8 {
    if !x.is_finite() {
        return 0;
    }
    (x * 255.0).trunc().clamp(0.0, 255.0) as u8
}

/// The low dword of `fistp m64` under the truncating control word (RC=11, `or eax,0xc00`): NaN or
/// a value past ±2^63 stores the integer indefinite, whose low dword is 0.
fn fistp_low(n: f64) -> u32 {
    let t = n.trunc();
    if t.abs() < 2f64.powi(63) {
        t as i64 as u32
    } else {
        0
    }
}

/// The 0-based window a verb's argument 1 names: that dword less one, unsigned below ten
/// (`sub 1; cmp 0xa; jae`, `0x4fbde2`); `None` for an id outside 1..10 or a non-number.
pub(super) fn window_index(lua: &Lua, v: Option<&Value>) -> Option<usize> {
    let n = lua.coerce_number(v?.clone()).ok().flatten()?;
    let i = fistp_low(n).wrapping_sub(1) as usize;
    (i < ENGINE_CHAT_WINDOW_SLOTS).then_some(i)
}

/// A window verb's opening: its own argument checks (`ok`) or its `Usage:`, then the window.
pub(super) fn checked_window(
    lua: &Lua,
    args: &MultiValue,
    ok: bool,
    usage: &'static str,
) -> mlua::Result<Option<usize>> {
    if !ok {
        return Err(mlua::Error::runtime(usage));
    }
    Ok(window_index(lua, args.front()))
}

/// `f` on window `i`'s record; a change cues the persist.
pub(super) fn edit_look<T>(lua: &Lua, i: usize, f: impl FnOnce(&mut ChatWindowLook) -> T) -> T {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let before = model.chat_window_looks[i].clone();
    let out = f(&mut model.chat_window_looks[i]);
    if model.chat_window_looks[i] != before {
        model.chat_window_changes.insert(i);
    }
    out
}

impl super::Model {
    /// Appends `(name, id)` to window `window`'s channels unless the name is there (case-folded):
    /// the shared tail of `JoinChannelByName` (`0x49ec24`-`0x49ede7`) and `AddChatWindowChannel`
    /// (`0x4a1000`). `name` is the DBC Shortcut for a matched row, else the argument verbatim.
    pub(super) fn register_window_channel(&mut self, window: usize, name: String, id: u32) -> bool {
        let Some(look) = self.chat_window_looks.get_mut(window) else {
            return false;
        };
        if look
            .channels
            .iter()
            .any(|(c, _)| c.eq_ignore_ascii_case(&name))
        {
            return false;
        }
        look.channels.push((name, id));
        self.chat_window_changes.insert(window);
        true
    }

    /// Removes the first entry named `key` (case-folded) from every window, as leave-by-name
    /// `0x49ee70` does (`0x49f001`-`0x49f085`). `key` is the DBC Shortcut for a matched row, else
    /// the argument; a numeric leave strips nothing, as a slot name never equals a window entry.
    pub(super) fn strip_window_channel(&mut self, key: &str) -> bool {
        let mut stripped = false;
        for (i, look) in self.chat_window_looks.iter_mut().enumerate() {
            if let Some(at) = look
                .channels
                .iter()
                .position(|(c, _)| c.eq_ignore_ascii_case(key))
            {
                look.channels.remove(at);
                self.chat_window_changes.insert(i);
                stripped = true;
            }
        }
        stripped
    }
}

impl super::UiScript {
    /// Host-side [`Model::register_window_channel`], for the guild-recruitment join, which
    /// registers `GuildRecruitment` in window 1 as `0x49eb70(frameIdx = 0)` does.
    pub fn register_chat_window_channel(&mut self, window: usize, name: &str, id: u32) -> bool {
        self.model_mut()
            .register_window_channel(window, name.to_string(), id)
    }

    /// Seeds the records from the host's persisted store. A load queues no change: an echo would
    /// re-dirty the file just read.
    pub fn set_chat_window_looks(
        &mut self,
        looks: impl IntoIterator<Item = (usize, ChatWindowLook)>,
    ) {
        let mut model = self.model_mut();
        for (i, look) in looks {
            if let Some(slot) = model.chat_window_looks.get_mut(i) {
                *slot = look;
                slot.normalize_messages();
            }
        }
    }

    /// Every window's record, index 0 being `ChatFrame1`, for the saver.
    pub fn chat_window_looks(&self) -> Vec<ChatWindowLook> {
        self.model_mut().chat_window_looks.to_vec()
    }

    /// Drains the 0-based windows Lua changed since the last call, deduplicated and ascending.
    pub fn take_chat_window_changes(&mut self) -> Vec<usize> {
        let mut v: Vec<usize> = std::mem::take(&mut self.model_mut().chat_window_changes)
            .into_iter()
            .collect();
        v.sort_unstable();
        v
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // SetChatWindowColor(index, r, g, b) (`0x4fc470`) only stores: `FCF_SetWindowColor` tints the
    // textures itself.
    g.set(
        "SetChatWindowColor",
        lua.create_function(|lua, args: MultiValue| {
            let ok = (0..4).all(|n| is_number(lua, args.get(n)));
            let usage = "Usage: SetChatWindowColor(index, r, g, b)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let [r, g, b] = [1, 2, 3].map(|n| to_byte(coerced_number(lua, args.get(n).cloned())));
            edit_look(lua, i, |l| (l.r, l.g, l.b) = (r, g, b));
            Ok(())
        })?,
    )?;

    // SetChatWindowAlpha(index, alpha) (`0x4fc5b0`), called on every opacity-slider step.
    g.set(
        "SetChatWindowAlpha",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front()) && is_number(lua, args.get(1));
            let usage = "Usage: SetChatWindowAlpha(index, alpha)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let a = to_byte(coerced_number(lua, args.get(1).cloned()));
            edit_look(lua, i, |l| l.a = a);
            Ok(())
        })?,
    )?;

    // SetChatWindowLocked(index, locked) (`0x4fc660`) and SetChatWindowShown(index, shown)
    // (`0x4fc810`): the flag coercion `0x815500` with default 0 and 1, so `0` and "no" unlock or
    // hide, and an absent `shown` shows.
    type Flag = fn(&mut ChatWindowLook) -> &mut bool;
    let flags: [(&str, &'static str, bool, Flag); 2] = [
        (
            "SetChatWindowLocked",
            "Usage: SetChatWindowLocked(index, locked)",
            false,
            |l| &mut l.locked,
        ),
        (
            "SetChatWindowShown",
            "Usage: SetChatWindowShown(index, shown)",
            true,
            |l| &mut l.shown,
        ),
    ];
    for (name, usage, default, field) in flags {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let ok = is_number(lua, args.front());
                let Some(i) = checked_window(lua, &args, ok, usage)? else {
                    return Ok(());
                };
                let on = bool_or_default(args.get(1), default);
                edit_look(lua, i, |l| *field(l) = on);
                Ok(())
            })?,
        )?;
    }

    // SetChatWindowDocked(index, docked) (`0x4fc760`): a number's truncated dword, any other
    // argument 0; `FCF_SaveDock` writes each docked window's position, `FCF_UnDockFrame` nil.
    g.set(
        "SetChatWindowDocked",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front());
            let usage = "Usage: SetChatWindowDocked(index, docked)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let docked = lua
                .coerce_number(args.get(1).cloned().unwrap_or(Value::Nil))?
                .map(fistp_low)
                .filter(|d| *d != 0);
            edit_look(lua, i, |l| l.docked = docked);
            Ok(())
        })?,
    )?;

    // SetChatWindowSize(index, size) (`0x4fc3d0`) only stores; FrameXML sets the font itself. A
    // size `__ftol2_sse` makes `<= 0` is dropped (`0x4fc439 jle`), one past `i32` included.
    g.set(
        "SetChatWindowSize",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front()) && is_number(lua, args.get(1));
            let usage = "Usage: SetChatWindowSize(index, size)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let size = ftol(coerced_number(lua, args.get(1).cloned()));
            if size > 0 {
                edit_look(lua, i, |l| l.font_size = size);
            }
            Ok(())
        })?,
    )?;

    // SetChatWindowName(index, name) (`0x4fc330`): `SStrCopy(record, name, 0x80)`, 127 bytes and
    // the NUL; a name that is neither string nor number stores "".
    g.set(
        "SetChatWindowName",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front());
            let usage = "Usage: SetChatWindowName(index, \"name\")";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let mut name = args
                .get(1)
                .and_then(|v| super::binding_abi::optional_string(lua, v))
                .unwrap_or_default();
            let mut cut = name.len().min(127);
            while !name.is_char_boundary(cut) {
                cut -= 1;
            }
            name.truncate(cut);
            edit_look(lua, i, |l| l.name = name);
            Ok(())
        })?,
    )?;

    // Add/RemoveChatWindowMessages(index, "messageGroup") (`0x4fc170`/`0x4fc250`): one group name,
    // matched whole and case-folded (`0x4fc200`); an unknown name does nothing.
    for (name, usage, add) in [
        (
            "AddChatWindowMessages",
            "Usage: AddChatWindowMessages(index, \"messageGroup\")",
            true,
        ),
        (
            "RemoveChatWindowMessages",
            "Usage: RemoveChatWindowMessages(index, \"messageGroup\")",
            false,
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let ok = is_number(lua, args.front()) && is_string(args.get(1));
                let Some(i) = checked_window(lua, &args, ok, usage)? else {
                    return Ok(());
                };
                let group = args
                    .get(1)
                    .and_then(|v| super::binding_abi::optional_string(lua, v))
                    .and_then(|g| message_group_index(&g))
                    .map(|g| MESSAGE_GROUPS[g].0);
                let Some(group) = group else {
                    return Ok(());
                };
                edit_look(lua, i, |l| {
                    l.messages.retain(|m| m != group);
                    if add {
                        l.messages.push(group.to_string());
                        l.normalize_messages();
                    }
                });
                Ok(())
            })?,
        )?;
    }

    // GetChatWindowMessages(index) (`0x4fc0c0`): the enabled group names as varargs, in table order.
    g.set(
        "GetChatWindowMessages",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front());
            let usage = "Usage: GetChatWindowMessages(index)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            model.chat_window_looks[i]
                .messages
                .iter()
                .map(|n| lua.create_string(n).map(Value::String))
                .collect()
        })?,
    )?;

    // AddChatWindowChannel(index, "channel") (`0x501db0`): a whole, case-folded match on a
    // `ChatChannels.dbc` Shortcut stores the DBC's spelling and answers the row id; no match stores
    // the name and answers 0, which `ChatFrame_AddChannel` reads as truthy; a matched row with no
    // zone text yet answers and stores nothing. A name already there only answers the id.
    g.set(
        "AddChatWindowChannel",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front()) && is_string(args.get(1));
            let usage = "Usage: AddChatWindowChannel(index, \"channel\")";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(MultiValue::new());
            };
            let Some(name) = args
                .get(1)
                .and_then(|v| super::binding_abi::optional_string(lua, v))
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
            else {
                return Ok(MultiValue::new());
            };
            let matched = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                model
                    .zone_channel_catalog
                    .iter()
                    .find(|r| r.shortcut.eq_ignore_ascii_case(&name))
                    .cloned()
            };
            let (name, zone_id) = match matched {
                None => (name, 0),
                Some(ZoneChannelRow { resolved: None, .. }) => return Ok(MultiValue::new()),
                Some(row) => (row.shortcut, row.id),
            };
            edit_look(lua, i, |look| {
                if !look
                    .channels
                    .iter()
                    .any(|(c, _)| c.eq_ignore_ascii_case(&name))
                {
                    look.channels.push((name, zone_id));
                }
            });
            Ok(MultiValue::from_vec(vec![Value::Integer(i64::from(
                zone_id,
            ))]))
        })?,
    )?;

    // RemoveChatWindowChannel(index, "channel") (`0x4ff0d0`): a numeric key names the joined
    // channel in slot n, and an empty slot removes nothing; the key then resolves to its DBC
    // Shortcut as above, and the entry of that name is freed.
    g.set(
        "RemoveChatWindowChannel",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front()) && is_string(args.get(1));
            let usage = "Usage: RemoveChatWindowChannel(index, \"channel\")";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(());
            };
            let key = args
                .get(1)
                .and_then(|v| super::binding_abi::optional_string(lua, v))
                .unwrap_or_default();
            let key = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let n = leading_int(&key);
                let key = if n > 0 {
                    match usize::try_from(n)
                        .ok()
                        .and_then(|n| model.joined_channels.get(n - 1))
                        .and_then(|c| c.as_ref().filter(|c| c.confirmed))
                        .map(|c| c.name.clone())
                    {
                        Some(name) => name,
                        None => return Ok(()),
                    }
                } else {
                    key.trim().to_string()
                };
                match model
                    .zone_channel_catalog
                    .iter()
                    .find(|r| r.shortcut.eq_ignore_ascii_case(&key))
                {
                    Some(ZoneChannelRow { resolved: None, .. }) => return Ok(()),
                    Some(row) => row.shortcut.clone(),
                    None => key,
                }
            };
            edit_look(lua, i, |look| {
                look.channels.retain(|(c, _)| !c.eq_ignore_ascii_case(&key));
            });
            Ok(())
        })?,
    )?;

    // GetChatWindowChannels(index) (`0x4feff0`): `name, zoneId` pairs in slot order.
    g.set(
        "GetChatWindowChannels",
        lua.create_function(|lua, args: MultiValue| {
            let ok = is_number(lua, args.front());
            let usage = "Usage: GetChatWindowChannels(index)";
            let Some(i) = checked_window(lua, &args, ok, usage)? else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::new();
            for (name, zone_id) in &model.chat_window_looks[i].channels {
                out.push(Value::String(lua.create_string(name)?));
                out.push(Value::Integer(i64::from(*zone_id)));
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
#[path = "chat_window_tests.rs"]
mod tests;

#[cfg(test)]
mod record_tests {
    use super::{ChatWindowLook, MESSAGE_GROUPS};
    use crate::script::{UiScript, ZoneChannelRow};

    /// The stock records (`0x501f00`): window 1 has the `defaultOn` general groups (all but
    /// TARGETICONS), window 2 the six combat-log groups.
    #[test]
    fn the_stock_records_are_the_reset_records() {
        let s = UiScript::new().unwrap();
        let looks = s.chat_window_looks();
        assert!(looks[0].shown && looks[1].shown && !looks[2].shown);
        assert_eq!((looks[0].docked, looks[1].docked), (Some(1), Some(2)));
        assert_eq!(looks[0].messages.len(), 39);
        assert_eq!(looks[0].messages[0], "SYSTEM");
        assert!(looks[0].messages.iter().any(|m| m == "ACHIEVEMENT"));
        assert!(!looks[0].messages.iter().any(|m| m == "TARGETICONS"));
        assert_eq!(
            looks[1].messages,
            [
                "OPENING",
                "TRADESKILLS",
                "PET_INFO",
                "COMBAT_XP_GAIN",
                "COMBAT_HONOR_GAIN",
                "COMBAT_MISC_INFO"
            ]
        );
        assert!(looks[2].messages.is_empty());
        assert!(looks
            .iter()
            .all(|l| l.channels.is_empty() && l.name.is_empty()));
        assert_eq!(
            s.eval::<Vec<String>>("return {GetChatWindowMessages(1)}")
                .unwrap(),
            looks[0].messages
        );
        assert_eq!(MESSAGE_GROUPS.len(), 46);
    }

    #[test]
    fn name_and_shown_round_trip_through_the_getter_and_cue_the_persist() {
        let mut s = UiScript::new().unwrap();
        s.run("SetChatWindowName(3, 'Loot') SetChatWindowShown(3) SetChatWindowShown(1, nil)")
            .unwrap();
        assert_eq!(
            s.eval::<(String, Option<i64>)>(
                "local n, _, _, _, _, _, sh = GetChatWindowInfo(3) return n, sh"
            )
            .unwrap(),
            ("Loot".to_string(), Some(1))
        );
        assert!(s
            .eval::<bool>("local _, _, _, _, _, _, sh = GetChatWindowInfo(1) return sh == nil")
            .unwrap());
        assert_eq!(s.take_chat_window_changes(), vec![0, 2]);
        s.run("SetChatWindowName(3, 'Loot')").unwrap();
        assert!(s.take_chat_window_changes().is_empty(), "no move, no cue");
        // The flag coercion (`0x815500`, default 1 for shown, 0 for locked) and the 127-byte cap.
        s.run(
            "SetChatWindowShown(4, 'off') SetChatWindowShown(5, '1') SetChatWindowShown(6, 0) \
             SetChatWindowShown(7, 'no') SetChatWindowLocked(4, '0') \
             SetChatWindowName(4, string.rep('x', 200))",
        )
        .unwrap();
        let looks = s.chat_window_looks();
        assert_eq!(
            (
                looks[3].shown,
                looks[4].shown,
                looks[5].shown,
                looks[6].shown
            ),
            (false, true, false, false)
        );
        assert!(!looks[3].locked, "'0' unlocks");
        assert_eq!(looks[3].name.len(), 127, "127 bytes and the NUL");
    }

    #[test]
    fn message_types_are_flags_answered_in_table_order() {
        let mut s = UiScript::new().unwrap();
        // One group name per call, matched whole: a second name and a padded one do nothing.
        s.run(
            "AddChatWindowMessages(3, 'yell', 'MONEY') AddChatWindowMessages(3, 'SAY') \
             AddChatWindowMessages(3, 'BOGUS') AddChatWindowMessages(3, ' loot') \
             AddChatWindowMessages(3, 'money') RemoveChatWindowMessages(1, 'Say') \
             RemoveChatWindowMessages(1, 'LOOT') RemoveChatWindowMessages(1, 'NOPE')",
        )
        .unwrap();
        assert_eq!(
            s.eval::<Vec<String>>("return {GetChatWindowMessages(3)}")
                .unwrap(),
            vec!["SAY".to_string(), "YELL".to_string(), "MONEY".to_string()]
        );
        let general = s.chat_window_looks()[0].messages.clone();
        assert!(!general.iter().any(|m| m == "SAY" || m == "LOOT"));
        assert_eq!(general.len(), 37);
        assert_eq!(s.take_chat_window_changes(), vec![0, 2]);
        s.set_chat_window_looks([(
            4,
            ChatWindowLook {
                messages: vec!["money".into(), "bogus".into(), "SAY".into(), "say".into()],
                ..Default::default()
            },
        )]);
        assert_eq!(
            s.chat_window_looks()[4].messages,
            vec!["SAY".to_string(), "MONEY".to_string()]
        );
    }

    #[test]
    fn channels_carry_the_zone_id_and_answer_it_on_add() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(vec![
            ZoneChannelRow {
                id: 1,
                shortcut: "General".into(),
                resolved: Some("General - Elwynn Forest".into()),
                listed: true,
            },
            ZoneChannelRow {
                id: 2,
                shortcut: "Trade".into(),
                resolved: None,
                listed: false,
            },
        ]);
        s.run(
            "A = {AddChatWindowChannel(1, 'general')} \
             B = {AddChatWindowChannel(1, 'MyChan')} \
             C = {AddChatWindowChannel(1, 'mychan')} \
             D = {AddChatWindowChannel(1, 'Trade')} \
             E = {AddChatWindowChannel(1, '')}",
        )
        .unwrap();
        assert_eq!(
            s.eval::<(i64, i64, i64)>("return A[1], B[1], C[1]")
                .unwrap(),
            (1, 0, 0)
        );
        assert!(s
            .eval::<bool>("return table.getn(D) == 0 and table.getn(E) == 0")
            .unwrap());
        assert_eq!(
            s.eval::<Vec<mlua::Value>>("return {GetChatWindowChannels(1)}")
                .unwrap()
                .len(),
            4,
            "name, id, name, id — the duplicate was not stored twice"
        );
        assert_eq!(
            s.chat_window_looks()[0].channels,
            vec![("General".to_string(), 1), ("MyChan".to_string(), 0)],
            "the DBC's spelling for the shortcut, the typed one for the custom channel"
        );
        s.run("RemoveChatWindowChannel(1, 'MYCHAN') RemoveChatWindowChannel(1, '3')")
            .unwrap();
        assert_eq!(
            s.chat_window_looks()[0].channels,
            vec![("General".to_string(), 1)],
            "a number names a joined slot; with none joined it removes nothing"
        );
        assert_eq!(s.take_chat_window_changes(), vec![0]);
    }

    #[test]
    fn a_host_loaded_record_is_what_the_verbs_read() {
        let mut s = UiScript::new().unwrap();
        s.set_chat_window_looks([(
            4,
            ChatWindowLook {
                name: "Trade".into(),
                shown: true,
                messages: vec!["CHANNEL".into()],
                channels: vec![("Trade".into(), 2)],
                ..Default::default()
            },
        )]);
        assert_eq!(
            s.eval::<Vec<String>>("return {GetChatWindowMessages(5)}")
                .unwrap(),
            vec!["CHANNEL".to_string()]
        );
        assert_eq!(
            s.eval::<(String, i64)>("return GetChatWindowChannels(5)")
                .unwrap(),
            ("Trade".to_string(), 2)
        );
        assert!(
            s.take_chat_window_changes().is_empty(),
            "the load path never echoes"
        );
    }
}
