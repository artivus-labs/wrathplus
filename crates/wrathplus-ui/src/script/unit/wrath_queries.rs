//! Build 12340's unit and player readers over the pushed unit snapshots, roster and auras.
//! Evidence: `/tmp/azb-app-mig/ghidra/{audit1,unit1,unit2,unit4}.log`.
//!
//! - `UnitIsAFK` (`0x60cc30`), `UnitIsDND` (`0x60cd50`): a held player's flags `0x2` or `0x4`;
//!   DND without a held player reads roster-record status bit 7.
//! - `IsXPUserDisabled` (`0x614140`): `PLAYER_FLAGS & 0x2000000`, a boolean, nothing without a
//!   player.
//! - `UnitIsTappedByAllThreatList` (`0x611230`): `UNIT_DYNAMIC_FLAGS & 0x80`.
//! - `IsReferAFriendLinked` (`0x51d1b0` → `0x6d7590`): `UNIT_DYNAMIC_FLAGS & 0x40`.
//! - `CanSummonFriend` (`0x51d2b0` → `0x6d7630`): the link, a group member below level 61, and
//!   the summon spell off cooldown.
//! - `GetSummonFriendCooldown` (`0x511d30`): the summon spell's cooldown.
//! - `UnitIsRaidOfficer` (`0x60c8a0`): the raid member's rank above 0.
//! - `UnitInBattleground` (`0x60caa0`): the raid slot, in a battleground.
//! - `UnitIsSameServer` (`0x612670`): 1 unless the two units' realms differ.
//! - `PlayerIsPVPInactive` (`0x612e20`): the unit, a token or a group member's name (`0x60abf0`),
//!   carries aura 43681, a boolean.
//! - `GetAvailableRoles` (`0x5548f0`): the class's role byte (`0xa394af`, by class id), as
//!   booleans for tank `0x2`, healer `0x4` and damage `0x8`.
//! - `GetNumLanguages` (`0x500760`): the known languages.
//!
//! Labelled: `UnitIsAFK`'s two local overrides (`0x729740`, `0x4cee50` with `0xbcefec`) and a
//! roster-only member's AFK bit (`0x513c30`, `+8` bit 6) are not read. The link
//! queries also take a friend's name (stock `FriendsFrame.lua:99`, `:117`): a non-token string
//! reads the held unit of that name, else nil, never raising; their non-held arms (`0x52bee0`,
//! `0x5727b0`) and the summon spell's writer (`0x53ec40`) are not decompiled.
//! `UnitInBattleground`'s battleground test (`0x572f50`) reads the map's `InstanceType` 3, and its
//! slot is the 0-based raid index. The units carry no realm, so `UnitIsSameServer` answers 1 for
//! any pair. `IsThreatWarningEnabled` (`0x517d30`, the flag at `0xbd08c0` that `0x519df0` writes)
//! reads the `threatWarning` CVar.

use mlua::{Lua, MultiValue, Value};

use super::{check_unit_token, with_unit, Model};
use crate::script::binding_abi::flag;

/// Summon Friend, the spell `0xbe7d88` holds.
const SUMMON_FRIEND: u32 = 45927;
/// The battleground "Inactive" aura `PlayerIsPVPInactive` scans for.
const AURA_INACTIVE: u32 = 43681;
/// The LFG auras `GetLFGDeserterExpiration` (`0x5580e0`) and `GetLFGRandomCooldownExpiration`
/// (`0x558060`) look up: Dungeon Deserter and Dungeon Cooldown.
const AURA_DUNGEON_DESERTER: u32 = 71041;
const AURA_DUNGEON_COOLDOWN: u32 = 71328;

const PLAYER_FLAGS_AFK: u32 = 0x2;
const PLAYER_FLAGS_DND: u32 = 0x4;
const PLAYER_FLAGS_NO_XP_GAIN: u32 = 0x0200_0000;
const UNIT_DYNFLAG_REFER_A_FRIEND: u32 = 0x40;
const UNIT_DYNFLAG_TAPPED_BY_ALL_THREAT_LIST: u32 = 0x80;
/// `UNIT_FIELD_FLAGS` byte 3 bit 0 (`0x60d860` reads descriptor `+0xd7 & 1`).
const UNIT_FLAG_POSSESSED: u32 = 0x0100_0000;

/// `0xa394af`: each class id's role bits.
const CLASS_ROLES: [u8; 12] = [
    0x00, 0x0b, 0x0f, 0x09, 0x09, 0x0d, 0x0b, 0x0d, 0x09, 0x09, 0x00, 0x0f,
];

/// The unit a link query names: a token, or a held unit's name (a friends-list row's).
fn linked<'m>(model: &'m Model, t: Option<&str>) -> Option<&'m super::UnitState> {
    let t = t?;
    if super::resolve::token_recognised(t, model.unit_guids.wrath) {
        return model.unit(t);
    }
    let mut held = model.units_by_lower.values();
    held.find(|u| u.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(t)))
}

/// The class id of a `class_file` token.
pub(crate) fn class_id(file: &str) -> usize {
    match file {
        "WARRIOR" => 1,
        "PALADIN" => 2,
        "HUNTER" => 3,
        "ROGUE" => 4,
        "PRIEST" => 5,
        "DEATHKNIGHT" => 6,
        "SHAMAN" => 7,
        "MAGE" => 8,
        "WARLOCK" => 9,
        "DRUID" => 11,
        _ => 0,
    }
}

fn unit_arg(lua: &Lua, v: Value, usage: &'static str) -> mlua::Result<Option<String>> {
    Ok(Some(crate::script::binding_abi::string_arg(lua, v, usage)?))
}

/// The summon spell's `(start, duration, enabled)` off the native `GetSpellCooldown`.
fn summon_cooldown(lua: &Lua) -> (f64, f64, f64) {
    let Ok(f) = lua.globals().get::<mlua::Function>("GetSpellCooldown") else {
        return (0.0, 0.0, 1.0);
    };
    let v: MultiValue = f.call(SUMMON_FRIEND).unwrap_or_default();
    let n = |i: usize| {
        v.get(i)
            .and_then(|x| lua.coerce_number(x.clone()).ok().flatten())
            .unwrap_or(0.0)
    };
    (n(0), n(1), if v.len() > 2 { n(2) } else { 1.0 })
}

impl crate::script::UiScript {
    /// The focus guid `FocusUnit`/`ClearFocus` asked for this frame (0 clears), taken once.
    pub fn take_focus_request(&mut self) -> Option<u64> {
        self.model_mut().focus_request.take()
    }

    /// Publish `ChrClasses.dbc`'s `(file name, male name, female name)` rows, in id order, for
    /// `FillLocalizedClassList`.
    pub fn set_localized_class_names(&mut self, names: Vec<(String, String, String)>) {
        self.lua.set_app_data(ClassNames(names));
    }
}

#[derive(Default)]
struct ClassNames(Vec<(String, String, String)>);

/// `0xbd0c94`: the selection colours by reaction rank 0 (hated) to 7 (exalted), as `(r, g, b)`;
/// `0xbd0c8c`, the friendly player's blue.
const REACTION_COLORS: [(f64, f64, f64); 8] = [
    (1.0, 0.0, 0.0),
    (1.0, 0.0, 0.0),
    (1.0, 128.0 / 255.0, 0.0),
    (1.0, 1.0, 0.0),
    (0.0, 1.0, 0.0),
    (0.0, 1.0, 0.0),
    (0.0, 1.0, 0.0),
    (0.0, 1.0, 0.0),
];
const FRIENDLY_PLAYER_COLOR: (f64, f64, f64) = (0.0, 0.0, 1.0);

/// `0x521bf0` with no party or friend tint, as `UnitSelectionColor` asks it: a player-controlled
/// unit is red while either side can attack the other (`0x729740`), its reaction colour while
/// PvP-flagged, else blue; any other unit its reaction colour; no unit blue. Labelled: WrathPlus's
/// `can_attack` answers both directions alike, so the native's one-way yellow and blue legs fold
/// into red, and the sanctuary terms (`UNIT_FIELD_BYTES_2` `0x08`) are not modelled.
fn selection_color(unit: Option<&super::UnitState>) -> (f64, f64, f64) {
    let reaction = |u: &super::UnitState| REACTION_COLORS[usize::from(u.reaction.clamp(1, 8) - 1)];
    match unit.filter(|u| u.exists) {
        None => FRIENDLY_PLAYER_COLOR,
        Some(u) if u.player_controlled => {
            if u.can_attack {
                REACTION_COLORS[0]
            } else if u.pvp && u.reaction != 0 {
                reaction(u)
            } else {
                FRIENDLY_PLAYER_COLOR
            }
        }
        Some(u) if u.reaction != 0 => reaction(u),
        Some(_) => REACTION_COLORS[3],
    }
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // `UnitIsAFK` (`0x60cc30`) is nil for a player we can attack (`0x729740` at `0x60cccd`), DND
    // never. Labelled: AFK's local override (`[0xbcefec]`), its group-record bit, and
    // `can_attack`, fed for target, targettarget, npc and chain ends only.
    for (name, usage, bit) in [
        ("UnitIsAFK", "Usage: UnitIsAFK(\"unit\")", PLAYER_FLAGS_AFK),
        ("UnitIsDND", "Usage: UnitIsDND(\"unit\")", PLAYER_FLAGS_DND),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, token: Value| {
                let token = unit_arg(lua, token, usage)?;
                let hit = with_unit(lua, &token, false, |u| {
                    if u.has_object && u.is_player {
                        !(bit == PLAYER_FLAGS_AFK && u.can_attack) && u.player_flags & bit != 0
                    } else {
                        // `0x60cdda`: DND falls back to the record's status bit 7.
                        bit == PLAYER_FLAGS_DND && u.roster_status & 0x80 != 0
                    }
                })?;
                Ok(flag(hit))
            })?,
        )?;
    }
    g.set(
        "IsXPUserDisabled",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match model.unit("player").filter(|u| u.has_object) {
                Some(u) => MultiValue::from_vec(vec![Value::Boolean(
                    u.player_flags & PLAYER_FLAGS_NO_XP_GAIN != 0,
                )]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    g.set(
        "UnitIsTappedByAllThreatList",
        lua.create_function(|lua, token: Value| {
            let token = unit_arg(lua, token, "Usage: UnitIsTappedByAllThreatList(\"unit\")")?;
            let hit = with_unit(lua, &token, false, |u| {
                u.has_object && u.dynamic_flags & UNIT_DYNFLAG_TAPPED_BY_ALL_THREAT_LIST != 0
            })?;
            Ok(flag(hit))
        })?,
    )?;
    // UnitSelectionColor(unit) (`0x60da00`): r, g, b, a of `0x521bf0`'s colour; a non-string
    // raises.
    g.set(
        "UnitSelectionColor",
        lua.create_function(|lua, token: Value| {
            let Value::String(token) = token else {
                return Err(mlua::Error::runtime("Usage: UnitSelectionColor(\"unit\")"));
            };
            let token = token.to_str()?.to_string();
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let (r, g, b) = selection_color(model.unit(&token));
            Ok((r, g, b, 1.0))
        })?,
    )?;
    // GetNumFriends() (`0x6b4060`): the friend count, then how many are online; 0, 0 without a
    // list.
    g.set(
        "GetNumFriends",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let friends = &model.social.friends;
            Ok((
                friends.len() as i64,
                friends.iter().filter(|f| f.connected).count() as i64,
            ))
        })?,
    )?;
    // FillLocalizedClassList(classTable[, isFemale]) (`0x60a510`, bound outside the main table at
    // `0xad2718`): every class's file name keyed to its localized name; answers the table.
    g.set(
        "FillLocalizedClassList",
        lua.create_function(|lua, (table, female): (Value, Value)| {
            let Value::Table(table) = table else {
                return Err(mlua::Error::runtime(
                    "Usage: FillLocalizedClassList(classTable[, isFemale])",
                ));
            };
            let female = !matches!(female, Value::Nil | Value::Boolean(false));
            if let Some(names) = lua.app_data_ref::<ClassNames>() {
                for (file, male_name, female_name) in &names.0 {
                    table.set(
                        file.as_str(),
                        if female {
                            female_name.as_str()
                        } else {
                            male_name.as_str()
                        },
                    )?;
                }
            }
            Ok(table)
        })?,
    )?;
    // FocusUnit(unit) (`0x522480`) focuses the unit a token names, and nothing for one that names
    // none; ClearFocus() (`0x5225e0`) passes guid 0. Both through `0x51ff20`, the host's half.
    g.set(
        "FocusUnit",
        lua.create_function(|lua, token: Option<String>| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let guid = token
                .as_deref()
                .and_then(|t| model.unit(t))
                .filter(|u| u.exists && u.guid != 0)
                .map(|u| u.guid);
            if let Some(guid) = guid {
                model.focus_request = Some(guid);
            }
            Ok(())
        })?,
    )?;
    g.set(
        "ClearFocus",
        lua.create_function(|lua, ()| {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .focus_request = Some(0);
            Ok(())
        })?,
    )?;
    // UnitIsPossessed(unit) (`0x60d860`): 1 or nil, no usage error.
    g.set(
        "UnitIsPossessed",
        lua.create_function(|lua, token: Option<String>| {
            let hit = with_unit(lua, &token, false, |u| {
                u.has_object && u.flags & UNIT_FLAG_POSSESSED != 0
            })?;
            Ok(flag(hit))
        })?,
    )?;
    // The player's LFG aura's expiry (the aura's `+0x14` times 0.001: `GetTime` seconds), nothing
    // without the aura.
    for (name, spell) in [
        ("GetLFGDeserterExpiration", AURA_DUNGEON_DESERTER),
        ("GetLFGRandomCooldownExpiration", AURA_DUNGEON_COOLDOWN),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, ()| {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                Ok(
                    match model.player_auras.iter().find(|a| a.spell_id == spell) {
                        Some(a) => MultiValue::from_vec(vec![Value::Number(a.expiration_time)]),
                        None => MultiValue::new(),
                    },
                )
            })?,
        )?;
    }
    g.set(
        "IsReferAFriendLinked",
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let unit = linked(&model, token.as_deref());
            Ok(flag(unit.is_some_and(|u| {
                u.has_object && u.dynamic_flags & UNIT_DYNFLAG_REFER_A_FRIEND != 0
            })))
        })?,
    )?;
    g.set(
        "CanSummonFriend",
        lua.create_function(|lua, token: Option<String>| {
            let eligible = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                linked(&model, token.as_deref()).is_some_and(|u| {
                    let grouped = model.party.members.iter().any(|m| m.guid == u.guid)
                        || model.party.raid.iter().any(|m| m.guid == u.guid);
                    u.has_object
                        && u.dynamic_flags & UNIT_DYNFLAG_REFER_A_FRIEND != 0
                        && grouped
                        && u.level < 61
                })
            };
            let (start, duration, _) = summon_cooldown(lua);
            let now = crate::script::clock::now(lua);
            let ready = duration == 0.0 || start + duration <= now;
            Ok(flag(eligible && ready))
        })?,
    )?;
    g.set(
        "GetSummonFriendCooldown",
        lua.create_function(|lua, ()| {
            let (start, duration, enabled) = summon_cooldown(lua);
            Ok((start, duration, enabled))
        })?,
    )?;
    g.set(
        "UnitIsRaidOfficer",
        lua.create_function(|lua, token: Option<String>| {
            check_unit_token(lua, &token)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let guid = token.as_ref().and_then(|t| model.unit(t)).map(|u| u.guid);
            Ok(flag(guid.is_some_and(|g| {
                model.party.raid.iter().any(|m| m.guid == g && m.rank > 0)
            })))
        })?,
    )?;
    g.set(
        "UnitInBattleground",
        lua.create_function(|lua, token: Option<String>| {
            check_unit_token(lua, &token)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let guid = token.as_ref().and_then(|t| model.unit(t)).map(|u| u.guid);
            let slot = guid.and_then(|g| model.party.raid.iter().position(|m| m.guid == g));
            Ok(match slot {
                Some(i) if model.instance_type == Some(3) => Value::Number(i as f64),
                _ => Value::Nil,
            })
        })?,
    )?;
    g.set(
        "UnitIsSameServer",
        lua.create_function(|lua, (a, b): (Value, Value)| {
            let usage = "Usage: UnitIsSameServer(\"unit\", \"otherUnit\")";
            unit_arg(lua, a, usage)?;
            unit_arg(lua, b, usage)?;
            Ok(1.0)
        })?,
    )?;
    g.set(
        "PlayerIsPVPInactive",
        lua.create_function(|lua, token: Value| {
            let token = unit_arg(lua, token, "Usage: PlayerIsPVPInactive(\"unit\")")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            // A token through the guid table, else a group member's name (`0x60abf0`): the stock
            // battle maps pass a teammate's name. Nobody answers false, never a raise.
            let guid = token
                .as_deref()
                .map_or(0, |t| match model.unit_guids.guid_of(t) {
                    Ok(g) => g.unwrap_or(0),
                    Err(_) => crate::script::wrath_unit_popup::guid_of(&model, t),
                });
            let auras = match guid {
                0 => None,
                g if g == model.unit_guids.player => Some(model.player_auras.as_slice()),
                g => model.unit_auras.get(&g).map(Vec::as_slice),
            };
            Ok(auras.is_some_and(|a| a.iter().any(|x| x.spell_id == AURA_INACTIVE)))
        })?,
    )?;
    g.set(
        "GetAvailableRoles",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let id = model
                .unit("player")
                .and_then(|u| u.class_file.as_deref())
                .map_or(0, class_id);
            let bits = CLASS_ROLES[id];
            Ok((bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
        })?,
    )?;
    g.set(
        "IsThreatWarningEnabled",
        lua.create_function(|lua, ()| {
            let on = lua
                .globals()
                .get::<mlua::Function>("GetCVar")
                .ok()
                .and_then(|f| f.call::<Option<String>>("threatWarning").ok().flatten())
                .is_some_and(|v| v != "0" && !v.is_empty());
            Ok(flag(on))
        })?,
    )?;
    g.set(
        "GetNumLanguages",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .known_languages
                .len() as f64)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_queries_tests.rs"]
mod tests;
