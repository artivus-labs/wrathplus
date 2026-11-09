//! The glue API's Lua registration ([`install`]): each call reads or queues on the VM's
//! [`super::GlueState`].

use mlua::{Lua, MultiValue, Value};

use super::{GlueRequest, GlueState};

/// The glue VM's state, read side.
fn glue(lua: &Lua) -> mlua::AppDataRef<'_, GlueState> {
    lua.app_data_ref::<GlueState>().expect("glue state")
}

/// The glue VM's state, write side.
fn glue_mut(lua: &Lua) -> mlua::AppDataRefMut<'_, GlueState> {
    lua.app_data_mut::<GlueState>().expect("glue state")
}

fn push(lua: &Lua, r: GlueRequest) {
    glue_mut(lua).requests.push(r);
}

fn text(lua: &Lua, v: Option<&Value>) -> mlua::Result<String> {
    Ok(v.cloned()
        .map(|v| lua.coerce_string(v))
        .transpose()?
        .flatten()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

/// A function that returns `v` (an integer, or nil for `None`) and does nothing else.
fn constant(lua: &Lua, name: &str, v: Option<i64>) -> mlua::Result<()> {
    set(lua, name, move |_, _| {
        one(v.map_or(Value::Nil, Value::Integer))
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // `CSimpleModelFFX`'s own verbs, which `GlueParent.lua`'s `SetLighting` calls on the glue
    // screens' backdrops, added to this VM's Model table only. Labelled inert: the glue booth
    // stands the scene's glow and lights (`portrait::glue_booth`).
    let model: mlua::Table =
        lua.named_registry_value(crate::script::modelframe::REG_MODEL_METHODS)?;
    for name in [
        "SetGlow",
        "ResetLights",
        "AddCharacterLight",
        "AddLight",
        "AddPetLight",
    ] {
        model.set(name, lua.create_function(|_, _: MultiValue| Ok(()))?)?;
    }
    set(lua, "DefaultServerLogin", |lua, args| {
        let account = text(lua, args.front())?;
        let password = text(lua, args.get(1))?;
        push(lua, GlueRequest::Login { account, password });
        none()
    })?;
    set(lua, "CancelLogin", |lua, _| {
        push(lua, GlueRequest::CancelLogin);
        none()
    })?;
    set(lua, "QuitGame", |lua, _| {
        push(lua, GlueRequest::Quit);
        none()
    })?;
    set(lua, "StatusDialogClick", |lua, _| {
        push(lua, GlueRequest::StatusDialogClick);
        none()
    })?;
    set(lua, "GetSavedAccountName", |lua, _| {
        let name = glue(lua).saved_account.clone();
        one(Value::String(lua.create_string(&name)?))
    })?;
    set(lua, "SetSavedAccountName", |lua, args| {
        let name = text(lua, args.front())?;
        glue_mut(lua).saved_account = name.clone();
        push(lua, GlueRequest::SaveAccountName(name));
        none()
    })?;
    set(lua, "IsConnectedToServer", |lua, _| {
        let connected = glue(lua).connected;
        one(crate::script::binding_abi::flag(connected))
    })?;
    // `serverName, isPVP, isRP`, which CharacterSelect's realm label reads.
    set(lua, "GetServerName", |lua, _| {
        let (name, pvp, rp) = {
            let g = glue(lua);
            (g.server_name.clone(), g.server_pvp, g.server_rp)
        };
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(&name)?),
            crate::script::binding_abi::flag(pvp),
            crate::script::binding_abi::flag(rp),
        ]))
    })?;
    set(lua, "SetCurrentScreen", |lua, args| {
        let name = text(lua, args.front())?;
        glue_mut(lua).current_screen = name;
        none()
    })?;
    set(lua, "GetNumCharacters", |lua, _| {
        let n = glue(lua).num_characters;
        one(Value::Integer(i64::from(n)))
    })?;
    // ── The realm list (`RealmList.lua`) ──
    set(lua, "GetRealmCategories", |lua, _| {
        let names: Vec<String> = glue(lua)
            .realm_tabs
            .iter()
            .filter(|t| !t.name.is_empty())
            .map(|t| t.name.clone())
            .collect();
        Ok(MultiValue::from_vec(
            names
                .iter()
                .map(|n| lua.create_string(n).map(Value::String))
                .collect::<mlua::Result<_>>()?,
        ))
    })?;
    set(lua, "GetNumRealms", |lua, args| {
        let category = lua
            .coerce_integer(args.front().cloned().unwrap_or(Value::Nil))?
            .unwrap_or(1);
        let n = glue(lua).tab(category).map_or(0, |t| t.realms.len());
        one(Value::Integer(n as i64))
    })?;
    set(lua, "GetRealmInfo", |lua, args| {
        let category = lua
            .coerce_integer(args.front().cloned().unwrap_or(Value::Nil))?
            .unwrap_or(1);
        let index = lua
            .coerce_integer(args.get(1).cloned().unwrap_or(Value::Nil))?
            .unwrap_or(0);
        let realm = {
            let m = glue(lua);
            let i = usize::try_from(index).ok().and_then(|i| i.checked_sub(1));
            m.tab(category)
                .zip(i)
                .and_then(|(t, i)| t.realms.get(i).cloned())
        };
        let Some(r) = realm else {
            return none();
        };
        let flag = |b: bool| crate::script::binding_abi::flag(b);
        // Labelled: the realm's build (major, minor, revision, build) is not on our realm record.
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(&r.name)?),
            Value::Integer(i64::from(r.num_characters)),
            flag(r.invalid),
            flag(r.down),
            flag(r.current),
            flag(r.pvp),
            flag(r.rp),
            Value::Number(f64::from(r.load)),
            flag(r.locked),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Integer(i64::from(r.realm_type)),
        ]))
    })?;
    set(lua, "GetSelectedCategory", |lua, _| {
        let c = glue(lua).realm_selected_category;
        one(Value::Integer(c as i64))
    })?;
    set(lua, "RealmListUpdateRate", |lua, _| {
        let s = glue(lua).realm_update_rate;
        one(Value::Number(f64::from(s)))
    })?;
    set(lua, "ChangeRealm", |lua, args| {
        let int = |v: Option<&Value>| -> mlua::Result<usize> {
            Ok(lua
                .coerce_integer(v.cloned().unwrap_or(Value::Nil))?
                .and_then(|i| usize::try_from(i).ok())
                .unwrap_or(0))
        };
        let (category, index) = (int(args.front())?, int(args.get(1))?);
        push(lua, GlueRequest::ChangeRealm { category, index });
        none()
    })?;
    set(lua, "RequestRealmList", |lua, _| {
        push(lua, GlueRequest::RequestRealmList);
        none()
    })?;
    set(lua, "RealmListDialogCancelled", |lua, _| {
        push(lua, GlueRequest::RealmListCancelled);
        none()
    })?;
    // `RealmList_OnHide` calls it, on OK too, so it only stops the refresh, which the list's own
    // timer already owns; the sort is App's (the list's order is the native sort).
    for name in ["CancelRealmListQuery", "SortRealms"] {
        set(lua, name, |_, _| none())?;
    }
    // No locale-locked or tournament realms on an AzerothCore list.
    for name in [
        "IsInvalidLocale",
        "IsTournamentRealmCategory",
        "IsInvalidTournamentRealmCategory",
    ] {
        constant(lua, name, None)?;
    }

    // ── Character select (`CharacterSelect.lua`) ──
    set(lua, "GetCharacterInfo", |lua, args| {
        let index = lua
            .coerce_integer(args.front().cloned().unwrap_or(Value::Nil))?
            .unwrap_or(0);
        let row = {
            let m = glue(lua);
            usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| m.characters.get(i).cloned())
        };
        let Some(c) = row else {
            return none();
        };
        let s = |t: &str| lua.create_string(t).map(Value::String);
        let flag = |b: bool| crate::script::binding_abi::flag(b);
        Ok(MultiValue::from_vec(vec![
            s(&c.name)?,
            s(&c.race)?,
            s(&c.class)?,
            Value::Integer(i64::from(c.level)),
            s(&c.zone)?,
            Value::Integer(i64::from(c.sex)),
            flag(c.ghost),
            flag(c.customize),
            flag(c.race_change),
            flag(c.faction_change),
        ]))
    })?;
    set(lua, "GetSelectBackgroundModel", |lua, args| {
        let index = lua
            .coerce_integer(args.front().cloned().unwrap_or(Value::Nil))?
            .unwrap_or(0);
        let stage = {
            let m = glue(lua);
            usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| m.characters.get(i))
                .map_or_else(|| m.empty_stage.clone(), |c| c.stage.clone())
        };
        one(Value::String(lua.create_string(&stage)?))
    })?;
    let index_request = |name: &'static str, make: fn(usize) -> GlueRequest| -> mlua::Result<()> {
        set(lua, name, move |lua, args| {
            let i = lua
                .coerce_integer(args.front().cloned().unwrap_or(Value::Nil))?
                .and_then(|i| usize::try_from(i).ok())
                .unwrap_or(0);
            push(lua, make(i));
            none()
        })
    };
    index_request("SelectCharacter", GlueRequest::SelectCharacter)?;
    index_request("DeleteCharacter", GlueRequest::DeleteCharacter)?;
    for (name, r) in [
        ("EnterWorld", GlueRequest::EnterWorld),
        ("DisconnectFromServer", GlueRequest::Disconnect),
        ("GetCharacterListUpdate", GlueRequest::RefreshCharacters),
    ] {
        set(lua, name, move |lua, _| {
            push(lua, r.clone());
            none()
        })?;
    }
    set(lua, "GetCharacterSelectFacing", |lua, _| {
        let d = glue(lua).select_facing;
        one(Value::Number(f64::from(d)))
    })?;
    set(lua, "SetCharacterSelectFacing", |lua, args| {
        let d = lua
            .coerce_number(args.front().cloned().unwrap_or(Value::Nil))?
            .unwrap_or(0.0) as f32;
        glue_mut(lua).select_facing = d;
        push(lua, GlueRequest::SetSelectFacing(d));
        none()
    })?;
    for name in ["SetCharSelectBackground", "SetCharCustomizeBackground"] {
        set(lua, name, |lua, args| {
            let path = text(lua, args.front())?;
            push(lua, GlueRequest::SetBackground(path));
            none()
        })?;
    }
    // Inert on an AzerothCore realm: no billing plan, account-data times are App's, no realm split
    // or account messages; the DK swap scene is the booth's.
    constant(lua, "GetBillingPlan", Some(0))?;
    constant(lua, "GetBillingTimeRemaining", Some(0))?;
    for name in [
        "ReadyForAccountDataTimes",
        "RequestRealmSplitInfo",
        "UpdateSelectionCustomizationScene",
    ] {
        set(lua, name, |_, _| none())?;
    }
    constant(lua, "AccountMsg_GetBody", None)?;

    // ── AzerothBench's own, not stock 3.3.5: the login screen's Server field
    // (`assets/ui/glue/ServerField.xml`) reads and sets the address through these. ──
    set(lua, "WrathPlusGetRealmlist", |lua, _| {
        let address = glue(lua).realmlist.clone();
        Ok(MultiValue::from_vec(vec![Value::String(
            lua.create_string(&address)?,
        )]))
    })?;
    set(lua, "WrathPlusSetRealmlist", |lua, args| {
        let text = text(lua, args.front())?;
        push(lua, GlueRequest::SetRealmlist(text));
        none()
    })?;

    // Wrath of the Lich King: expansion level 2.
    constant(lua, "GetClientExpansionLevel", Some(2))?;
    // The glue state's own `GetBuildInfo`, five values for `VERSION_TEMPLATE`
    // (`"%s %s (%s) (%s)\n%s"`), which `AccountLogin_OnLoad` reads as versionType, buildType,
    // version, internalVersion, date and prints "Version 3.3.5 (12340) (Release x86)" over the
    // date. Labelled: the strings are the shipped login screen's, not read off the binding.
    set(lua, "GetBuildInfo", |lua, _| {
        Ok(MultiValue::from_vec(
            ["Version", "Release x86", "3.3.5", "12340", "Jun 24 2010"]
                .into_iter()
                .map(|s| lua.create_string(s).map(Value::String))
                .collect::<mlua::Result<_>>()?,
        ))
    })?;
    // The SSE check `AccountLogin`'s `OnShow` asks: a machine running WrathPlus is supported.
    constant(lua, "IsSystemSupported", Some(1))?;
    // The select and create screens' model frames: the booth stands their stage, so the names
    // need nothing kept.
    for name in ["SetCharSelectModelFrame", "SetCharCustomizeFrame"] {
        set(lua, name, |_, _| none())?;
    }

    // ── Inert on a 3.3.5a AzerothCore realm (labelled in the module doc) ──
    for name in ["IsTrialAccount", "IsStreamingTrial", "GetUsesToken"] {
        constant(lua, name, None)?;
    }
    // No notice to show: each reads as already accepted.
    for name in [
        "ShowTOSNotice",
        "ShowEULANotice",
        "ShowTerminationWithoutNoticeNotice",
        "ShowScanningNotice",
        "ShowContestNotice",
        "ShowChangedOptionWarnings",
    ] {
        constant(lua, name, None)?;
    }
    constant(lua, "IsScanDLLFinished", Some(1))?;
    for name in ["GetNumGameAccounts", "AccountMsg_GetNumUnreadMsgs"] {
        constant(lua, name, Some(0))?;
    }
    for name in [
        "GetSavedAccountList",
        "GetGameAccountInfo",
        "GetChangedOptionWarnings",
        "AccountMsg_GetIndexNextUnreadMsg",
        "AccountMsg_GetHeaderSubject",
    ] {
        constant(lua, name, None)?;
    }
    // Queries, not setters (`AccountLogin_ShowUserAgreements`): every agreement reads as
    // accepted, so the login panel shows and the TOS frame never does.
    for name in [
        "TOSAccepted",
        "EULAAccepted",
        "TerminationWithoutNoticeAccepted",
        "ScanningAccepted",
        "ContestAccepted",
    ] {
        constant(lua, name, Some(1))?;
    }
    for name in [
        "ScanDLLStart",
        "ScanDLLContinueAnyway",
        "SetSavedAccountList",
        "SetGameAccount",
        "SetUsesToken",
        "TokenEntered",
        "PINEntered",
        "MatrixEntered",
        "MatrixRevert",
        "MatrixCommit",
        "SurveyNotificationDone",
        "AccountMsg_LoadHeaders",
        "AccountMsg_LoadBody",
        "AccountMsg_SetMsgRead",
        "SetClearConfigData",
        "LaunchURL",
        "PlayCreditsMusic",
        "PlayGlueMusic",
        "PlayGlueAmbience",
        "StopGlueMusic",
        "StopGlueAmbience",
        "StopAllSFX",
    ] {
        set(lua, name, |_, _| none())?;
    }
    Ok(())
}
