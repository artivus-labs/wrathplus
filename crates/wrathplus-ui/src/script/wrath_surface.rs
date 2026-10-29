//! The Wrath 3.3.5a global API surface.
//!
//! Construction installs the mature Vanilla implementation first; this module then replaces the
//! bindings whose Wrath ABI differs and installs the trusted secure environment.

use mlua::{Lua, MultiValue, Value};

#[path = "wrath_attributes.rs"]
mod wrath_attributes;

#[path = "wrath_cooldown.rs"]
mod wrath_cooldown;
pub(super) use wrath_cooldown::REG_COOLDOWN_METHODS;

#[path = "wrath_loot.rs"]
mod wrath_loot;

#[path = "wrath_power.rs"]
mod wrath_power;

#[path = "wrath_stats.rs"]
mod wrath_stats;

#[path = "wrath_group.rs"]
pub(crate) mod wrath_group;

#[path = "wrath_party_assignment.rs"]
mod wrath_party_assignment;

#[path = "wrath_spell_queries.rs"]
pub(crate) mod wrath_spell_queries;

use super::Model;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // Blizzard_TimeManager.lua:91 reads minutes since midnight during OnLoad.
    super::cvars::install_wrath_engine_cvars(lua)?;
    install_truthful_readers(lua)?;
    super::mail::wrath::install(lua)?;
    super::wrath_lfg::install(lua)?;
    wrath_spell_queries::install(lua)?;
    wrath_stats::install(lua)?;
    wrath_loot::install(lua)?;
    super::loot_roll::wrath::install_readers(lua)?;
    super::loot_roll::wrath::install_actions(lua)?;
    wrath_power::install(lua)?;
    wrath_group::install(lua)?;
    wrath_party_assignment::install(lua)?;
    super::wrath_channel_bindings::install(lua)?;
    super::aura::wrath::install(lua)?;
    install_trusted_secure_environment(lua)?;
    install_frame_methods(lua)?;
    wrath_attributes::install(lua)?;
    wrath_cooldown::install(lua)?;
    super::wrath_quest_poi::blob::install(lua)?;
    super::wrath_quest_poi::order::install(lua)?;
    super::wrath_input_verbs::install(lua)?;
    install_compat(lua)
}

/// The build-12340 engine functions the 1.12 surface lacks (`wrath_compat.lua`).
fn install_compat(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "__azb_fire_event",
        lua.create_function(|lua, (event, args): (String, mlua::Variadic<String>)| {
            let args = args.into_iter().map(super::ScriptValue::Str).collect();
            super::tick::fire_event_into(lua, &event, args);
            Ok(())
        })?,
    )?;
    lua.load(include_str!("wrath_compat.lua"))
        .set_name("=[AzerothBench Wrath compat]")
        .exec()
}

/// Wrath-era methods every frame kind shares: `HookScript` and the "not protected" queries.
fn install_frame_methods(lua: &Lua) -> mlua::Result<()> {
    let methods: mlua::Table = lua.named_registry_value(super::REG_FRAME_METHODS)?;
    let table = |name: &str| lua.named_registry_value::<mlua::Table>(name);
    let install: mlua::Function = lua
        .load(
            r##"
            local methods, regions, fontstrings, buttons, editboxes, minimaps, sliders, textures,
                tooltips = ...
            -- A tooltip remembers its owner.
            local owners = setmetatable({}, { __mode = "k" })
            local setOwner = tooltips.SetOwner
            function tooltips.SetOwner(self, owner, ...)
                owners[self] = owner
                return setOwner(self, owner, ...)
            end
            function tooltips.GetOwner(self) return owners[self] end
            -- Sliders enable and disable like buttons.
            local disabled = setmetatable({}, { __mode = "k" })
            function sliders.Enable(self)
                disabled[self] = nil
                self:EnableMouse(true)
            end
            function sliders.Disable(self)
                disabled[self] = true
                self:EnableMouse(false)
            end
            function sliders.IsEnabled(self) return not disabled[self] and 1 or nil end
            -- Layout shorthands every frame and region has.
            for _, t in ipairs({ methods, regions, textures, fontstrings }) do
                function t.SetSize(self, width, height)
                    self:SetWidth(width)
                    self:SetHeight(height)
                end
                function t.GetSize(self) return self:GetWidth(), self:GetHeight() end
                function t.GetRect(self)
                    local left, bottom = self:GetLeft(), self:GetBottom()
                    if not left then return end
                    return left, bottom, self:GetWidth(), self:GetHeight()
                end
            end
            for _, t in ipairs({ fontstrings, buttons, editboxes }) do
                function t.SetFormattedText(self, fmt, ...) self:SetText(format(fmt, ...)) end
            end
            buttons.SetNormalFontObject = buttons.SetTextFontObject
            buttons.GetNormalFontObject = buttons.GetTextFontObject
            fontstrings.SetSpacing = fontstrings.SetSpacing or function() end
            fontstrings.GetSpacing = fontstrings.GetSpacing or function() return 0 end
            minimaps.SetPlayerTextureHeight = minimaps.SetPlayerTextureHeight or function() end
            minimaps.SetPlayerTextureWidth = minimaps.SetPlayerTextureWidth or function() end
            function methods.IgnoreDepth() end
            local clamp = setmetatable({}, { __mode = "k" })
            function methods.HookScript(self, kind, hook)
                local original = self:GetScript(kind)
                if original then
                    self:SetScript(kind, function(...)
                        original(...)
                        hook(...)
                    end)
                else
                    self:SetScript(kind, hook)
                end
            end
            function methods.IsProtected() return nil, nil end
            function methods.IsForbidden() return nil end
            function methods.CanChangeAttribute() return 1 end
            function methods.CanChangeProtectedState() return 1 end
            function methods.SetClampRectInsets(self, left, right, top, bottom)
                clamp[self] = { left, right, top, bottom }
            end
            function methods.GetClampRectInsets(self)
                local insets = clamp[self]
                if insets then return unpack(insets, 1, 4) end
                return 0, 0, 0, 0
            end
            "##,
        )
        .set_name("=[AzerothBench Wrath frame methods]")
        .into_function()?;
    install.call::<()>((
        methods,
        table(super::REG_REGION_METHODS)?,
        table(super::REG_FONTSTRING_METHODS)?,
        table(super::button::REG_BUTTON_METHODS)?,
        table(super::editbox::REG_EDITBOX_METHODS)?,
        table(super::minimap::REG_MINIMAP_METHODS)?,
        table(super::slider::REG_SLIDER_METHODS)?,
        table(super::REG_TEXTURE_METHODS)?,
        table(super::tooltip::REG_TOOLTIP_METHODS)?,
    ))
}

/// AzerothBench runs every UI chunk as trusted: there is no taint, nothing is protected from
/// addon code, and secure templates build as ordinary frames. Agents are meant to be able to
/// automate the interface, so Blizzard's anti-automation layer is deliberately not reproduced.
/// These are the language-level entry points stock FrameXML expects of that layer.
fn install_trusted_secure_environment(lua: &Lua) -> mlua::Result<()> {
    lua.load(
        r##"
        function issecure() return 1 end
        function issecurevariable() return 1, nil end
        function forceinsecure() end
        function InCombatLockdown() return nil end
        function securecall(f, ...)
            if type(f) == "string" then f = _G[f] end
            if type(f) == "function" then return f(...) end
        end
        function hooksecurefunc(t, name, hook)
            if type(t) ~= "table" then t, name, hook = _G, t, name end
            local original = t[name]
            if type(original) ~= "function" then
                error("hooksecurefunc(): " .. tostring(name) .. " is not a function", 2)
            end
            t[name] = function(...)
                local results = { n = 0 }
                local function keep(...)
                    results.n = select("#", ...)
                    for i = 1, results.n do results[i] = (select(i, ...)) end
                end
                keep(original(...))
                hook(...)
                return unpack(results, 1, results.n)
            end
        end
        "##,
    )
    .set_name("=[AzerothBench trusted secure environment]")
    .exec()
}

fn install_truthful_readers(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    // `0x54a2c0`: the arena feed's leading slots (`UnitGuids::arena_count`, `0x549630`).
    let arena = lua.create_function(super::UnitGuids::arena_lua)?;
    globals.set("GetNumArenaOpponents", arena)?;

    // `IsMouseButtonDown([button])`: a named button ("LeftButton", or 1-5), else any button.
    globals.set(
        "IsMouseButtonDown",
        lua.create_function(|lua, button: Option<Value>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let name = match button {
                Some(Value::String(s)) => Some(s.to_string_lossy()),
                Some(Value::Integer(n)) => Some(match n {
                    1 => "LeftButton".to_string(),
                    2 => "RightButton".to_string(),
                    3 => "MiddleButton".to_string(),
                    n => format!("Button{n}"),
                }),
                _ => None,
            };
            let held = match name {
                Some(name) => model
                    .buttons_down
                    .iter()
                    .any(|b| b.eq_ignore_ascii_case(&name)),
                None => !model.buttons_down.is_empty(),
            };
            Ok(held.then_some(1))
        })?,
    )?;

    // `GetActionInfo(slot)` (2.0+): `"spell", bookIndex, "spell", spellId`, `"item", itemId` or
    // `"macro", macroIndex`; nil for an empty slot. A spell not in the book has no index.
    globals.set(
        "GetActionInfo",
        lua.create_function(|lua, slot: u32| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(a) = model.actions.get(&slot) else {
                return Ok(MultiValue::new());
            };
            let s = |t: &str| lua.create_string(t).map(Value::String);
            let values = match a.kind & 0xf0 {
                super::action::ACTION_KIND_SPELL => {
                    let index = model
                        .spellbook
                        .slots
                        .iter()
                        .position(|v| v.spell_id == a.action)
                        .map_or(Value::Nil, |i| Value::Integer(i as i64 + 1));
                    vec![
                        s("spell")?,
                        index,
                        s("spell")?,
                        Value::Integer(a.action.into()),
                    ]
                }
                super::action::ACTION_KIND_ITEM => {
                    vec![s("item")?, Value::Integer(a.action.into())]
                }
                // The macro's index now, from the slot's id.
                super::action::ACTION_KIND_MACRO => {
                    let id = super::macro_action_id(a.kind, a.action).unwrap_or(0);
                    let index = model.macros.index_of_id(id).map_or(0, |i| i as i64);
                    vec![s("macro")?, Value::Integer(index)]
                }
                _ => vec![],
            };
            Ok(MultiValue::from_vec(values))
        })?,
    )?;

    globals.set(
        "GetBuildInfo",
        lua.create_function(|lua, ()| {
            Ok((
                lua.create_string("3.3.5")?,
                lua.create_string("12340")?,
                lua.create_string("Jun 24 2010")?,
                30300u32,
            ))
        })?,
    )?;

    // `0x60c2a0`: the number 1 or nil, never a boolean (`/tmp/azb-app-mig/ghidra/unitexists.log`).
    // A token is observable only when the engine supplied an existing unit snapshot; no
    // placeholder or guessed identity is manufactured. Labelled: with no object the native asks
    // `0x512a30` before answering nil (unread; the snapshot's `exists` stands in for it).
    globals.set(
        "UnitExists",
        lua.create_function(|lua, token: Option<String>| {
            let exists = token.as_deref().is_some_and(|token| {
                lua.app_data_ref::<Model>()
                    .expect("model app_data")
                    .unit(token)
                    .is_some_and(|unit| unit.exists)
            });
            Ok(exists.then_some(1))
        })?,
    )?;
    globals.set(
        "UnitGUID",
        lua.create_function(|lua, token: Option<String>| {
            let guid = token.as_deref().and_then(|token| {
                lua.app_data_ref::<Model>()
                    .expect("model app_data")
                    .unit(token)
                    .filter(|unit| unit.exists && unit.guid != 0)
                    .map(|unit| unit.guid)
            });
            match guid {
                Some(guid) => Ok(Value::String(lua.create_string(format!("0x{guid:016X}"))?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::UiScript;

    #[test]
    fn wrath_format_supports_localized_positional_conversions_in_both_aliases() {
        let s = UiScript::new().unwrap();
        s.run(r#"
            assert(format('%2$s hit %1$s for %3$02d (%%)', 'target', 'source', 7) == 'source hit target for 07 (%)')
            assert(string.format('%2$s %2$s %1$d', 3, 'repeat') == 'repeat repeat 3')
            assert(format('%s %d', 'plain', 4) == 'plain 4')
            assert(format('%2$s', nil, 'second') == 'second')
            assert(not pcall(format, '%1$s %s', 'mixed', 'bad'))
        "#).unwrap();
    }

    #[test]
    fn wrath_dynamic_fontstring_labels_clear_until_updated_without_calling_functions() {
        let s = UiScript::new().unwrap();
        s.run("local f = CreateFrame('Frame'):CreateFontString() f:SetText('old') f:SetText(function() error('must not invoke') end) assert(f:GetText() == nil) f:SetText(42) assert(f:GetText() == '42') f:SetText({}) assert(f:GetText() == nil)").unwrap();
    }

    #[test]
    fn wrath_world_arrow_creates_one_effect_child_before_onload_uses_it() {
        let s = UiScript::new().unwrap();
        s.run("local parent = CreateFrame('Frame') CreateWorldMapArrowFrame(parent) assert(PlayerArrowEffectFrame ~= nil) PlayerArrowEffectFrame:SetAlpha(.65) PlayerArrowEffectFrame:SetFrameLevel(110) local effect = PlayerArrowEffectFrame CreateWorldMapArrowFrame(parent) assert(effect == PlayerArrowEffectFrame)").unwrap();
    }

    #[test]
    fn wrath_alarm_time_has_a_numeric_default_and_is_writable() {
        let s = UiScript::new().unwrap();
        s.run("assert(tonumber(GetCVar('timeMgrAlarmTime')) == 0) SetCVar('timeMgrAlarmTime', '420') assert(tonumber(GetCVar('timeMgrAlarmTime')) == 420)").unwrap();
    }

    #[test]
    fn arena_opponent_count_is_zero_without_an_arena() {
        let s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetNumArenaOpponents()").unwrap(), 0);
    }

    #[test]
    fn wrath_keeps_the_full_engine_api() {
        let script = UiScript::new().unwrap();
        assert!(script
            .eval::<bool>("return ceil == math.ceil and type(UseContainerItem) == 'function'")
            .unwrap());
    }

    #[test]
    fn secure_templates_build_as_ordinary_trusted_frames() {
        let script = UiScript::new().unwrap();
        let doc = crate::framexml::parse(
            r#"<Ui><Button name="SecureActionButtonTemplate" virtual="true"/></Ui>"#,
        )
        .unwrap();
        let report = crate::loader::load(&script, &doc, &|_| None);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        script
            .run("assert(CreateFrame('Button', 'B', nil, 'SecureActionButtonTemplate'))")
            .unwrap();
    }

    #[test]
    fn secure_entry_points_are_trusted_pass_throughs() {
        let script = UiScript::new().unwrap();
        assert!(script
            .eval::<bool>(
                "local calls = 0 \
                 function Probe(a) calls = calls + a return a * 2 end \
                 hooksecurefunc('Probe', function(a) calls = calls + 10 end) \
                 return issecure() and securecall(Probe, 3) == 6 and securecall('Probe', 1) == 2 \
                    and calls == 24"
            )
            .unwrap());
    }

    /// Every global function the Wrath compat chunk still serves, the shim burn-down list
    /// (`docs/WRATH-COMPAT-SHIM-AUDIT.md`): a native binding installed after it drops its name here,
    /// and a shim line a native binding already replaces is dead and goes.
    const COMPAT_SERVED: &[&str] = &[
        "BNConnected",
        "BNFeaturesEnabled",
        "BNFeaturesEnabledAndConnected",
        "BNGetConversationInfo",
        "BNGetCustomMessageTable",
        "BNGetFriendInfo",
        "BNGetFriendInfoByID",
        "BNGetFriendInviteInfo",
        "BNGetInfo",
        "BNGetMaxPlayersInConversation",
        "BNGetNumBlocked",
        "BNGetNumBlockedToons",
        "BNGetNumConversationMembers",
        "BNGetNumFriendInvites",
        "BNGetNumFriends",
        "BNGetToonInfo",
        "GetActiveVoiceChannel",
        "GetNumVoiceSessionMembersBySessionID",
        "GetNumVoiceSessions",
        "GetVoiceCurrentSessionID",
        "GetVoiceSessionInfo",
        "GetVoiceStatus",
        "IsPlayerResolutionAvailable",
        "IsStereoVideoAvailable",
        "IsVoiceChatAllowed",
        "IsVoiceChatAllowedByServer",
        "IsVoiceChatEnabled",
        "Sound_ChatSystem_GetInputDriverNameByIndex",
        "Sound_ChatSystem_GetNumInputDrivers",
        "Sound_ChatSystem_GetNumOutputDrivers",
        "Sound_ChatSystem_GetOutputDriverNameByIndex",
        "Sound_GameSystem_GetNumOutputDrivers",
        "Sound_GameSystem_GetOutputDriverNameByIndex",
        "UnitIsTalking",
        "VoiceIsDisabledByClient",
    ];

    /// A shim line `X = X or Y` may fall back only to the chunk's own `none`/`zero` or a function
    /// literal: an alias to another native (`IsStackableAction = … or IsConsumableAction`) serves
    /// the other native's source, so the served-list census reads it as native and its removal
    /// leaves the name unbound.
    #[test]
    fn the_compat_chunk_aliases_no_other_native() {
        for line in include_str!("wrath_compat.lua").lines() {
            let Some((lhs, rhs)) = line.split_once(" = ") else {
                continue;
            };
            if lhs.starts_with([' ', '-']) {
                continue;
            }
            for (name, value) in lhs.split(", ").zip(rhs.split(", ")) {
                let mut alternatives = value.split(" or ").map(str::trim);
                if alternatives.next() != Some(name) {
                    continue;
                }
                for alt in alternatives {
                    assert!(
                        matches!(alt, "none" | "zero") || alt.starts_with("function"),
                        "{name} falls back to another binding, {alt}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_compat_chunk_serves_exactly_the_listed_globals() {
        let s = UiScript::new().unwrap();
        let mut served = Vec::new();
        for pair in s.lua.globals().pairs::<String, mlua::Value>() {
            let (name, value) = pair.unwrap();
            if let mlua::Value::Function(f) = value {
                if f.info().source.as_deref() == Some("=[AzerothBench Wrath compat]") {
                    served.push(name);
                }
            }
        }
        served.sort();
        assert_eq!(served, COMPAT_SERVED);
    }
}
