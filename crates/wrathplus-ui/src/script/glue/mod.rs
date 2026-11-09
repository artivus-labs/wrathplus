//! Build 12340's glue API for the stock GlueXML screens, installed only on the glue VM
//! ([`super::UiScript::install_glue_api`]): the in-game VM never answers these, as the reference's
//! glue Lua state is its own (`CGlueMgr`).
//!
//! This slice is AccountLogin's and GlueParent's. The calls that act go out as [`GlueRequest`]s for
//! App; the reads answer from [`GlueState`], which App seeds. Labelled as inert, because a 3.3.5a
//! AzerothCore realm never raises them: the TOS/EULA/scan/contest notices (shown as accepted), the
//! account messages (none, every agreement already accepted), game-account selection, tokens and
//! PINs, `LaunchURL` (no browser), and
//! the glue music and ambience calls (App owns glue sound). The glue screens' `<ModelFFX>`
//! backdrops parse as `Model` (labelled: `GetObjectType` answers `"Model"`).

mod api;
#[cfg(test)]
mod tests;

/// A glue call App acts on, in call order.
#[derive(Clone, PartialEq)]
pub enum GlueRequest {
    /// `DefaultServerLogin(account, password)`: log in to the realmlist's logon server.
    Login { account: String, password: String },
    /// `CancelLogin()`: abandon the attempt in flight.
    CancelLogin,
    /// `QuitGame()`.
    Quit,
    /// `SetSavedAccountName(name)`; empty clears.
    SaveAccountName(String),
    /// `StatusDialogClick()`: the status dialog's button (Cancel while connecting).
    StatusDialogClick,
    /// `ChangeRealm(category, index)`: enter the `index`-th realm (1-based) of tab `category`.
    ChangeRealm { category: usize, index: usize },
    /// `RequestRealmList()`: the list's refresh timer.
    RequestRealmList,
    /// `RealmListDialogCancelled()`: the list's Cancel, which on the login screen drops the realm
    /// connection.
    RealmListCancelled,
    /// `WrathPlusSetRealmlist(text)`, AzerothBench's own login button (not stock 3.3.5): the typed
    /// realmlist, for App to check and keep.
    SetRealmlist(String),
    /// `SelectCharacter(index)`, 1-based: the row the stage shows.
    SelectCharacter(usize),
    /// `EnterWorld()`, on the selected row.
    EnterWorld,
    /// `DeleteCharacter(index)`, 1-based, after the stock confirm.
    DeleteCharacter(usize),
    /// `DisconnectFromServer()`: select's Back to the login screen.
    Disconnect,
    /// `GetCharacterListUpdate()`: ask for a fresh roster.
    RefreshCharacters,
    /// `SetCharacterSelectFacing(degrees)`: the stage's character facing.
    SetSelectFacing(f32),
    /// `SetCharSelectBackground(path)` / `SetCharCustomizeBackground(path)`: the stage model
    /// `SetBackgroundModel` (`GlueParent.lua`) built from a stage name.
    SetBackground(String),
}

/// One character-select row as `GetCharacterInfo` returns it, and its stage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlueCharacter {
    pub name: String,
    pub race: String,
    pub class: String,
    pub level: u32,
    pub zone: String,
    pub sex: u32,
    pub ghost: bool,
    /// Paid customization, race change and faction change pending (`PCC`, `PRC`, `PFC`).
    pub customize: bool,
    pub race_change: bool,
    pub faction_change: bool,
    /// `GetSelectBackgroundModel`'s stage name for this character.
    pub stage: String,
}

/// One realm row as `GetRealmInfo` returns it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlueRealm {
    pub name: String,
    pub num_characters: u32,
    pub invalid: bool,
    pub down: bool,
    pub current: bool,
    pub pvp: bool,
    pub rp: bool,
    /// The load band, `CGlue::RealmLoadClassify`'s value.
    pub load: f32,
    pub locked: bool,
    /// The realm type, the list's last return.
    pub realm_type: u32,
}

/// One tab of the realm list: its category name and its rows, in the list's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlueRealmTab {
    pub name: String,
    pub realms: Vec<GlueRealm>,
}

impl std::fmt::Debug for GlueRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Never the password.
            GlueRequest::Login { account, .. } => {
                write!(f, "Login {{ account: {account:?}, password: <set> }}")
            }
            GlueRequest::CancelLogin => f.write_str("CancelLogin"),
            GlueRequest::Quit => f.write_str("Quit"),
            GlueRequest::SaveAccountName(n) => write!(f, "SaveAccountName({n:?})"),
            GlueRequest::StatusDialogClick => f.write_str("StatusDialogClick"),
            GlueRequest::ChangeRealm { category, index } => {
                write!(f, "ChangeRealm {{ category: {category}, index: {index} }}")
            }
            GlueRequest::RequestRealmList => f.write_str("RequestRealmList"),
            GlueRequest::RealmListCancelled => f.write_str("RealmListCancelled"),
            GlueRequest::SetRealmlist(text) => write!(f, "SetRealmlist({text:?})"),
            GlueRequest::SelectCharacter(i) => write!(f, "SelectCharacter({i})"),
            GlueRequest::EnterWorld => f.write_str("EnterWorld"),
            GlueRequest::DeleteCharacter(i) => write!(f, "DeleteCharacter({i})"),
            GlueRequest::Disconnect => f.write_str("Disconnect"),
            GlueRequest::RefreshCharacters => f.write_str("RefreshCharacters"),
            GlueRequest::SetSelectFacing(d) => write!(f, "SetSelectFacing({d})"),
            GlueRequest::SetBackground(p) => write!(f, "SetBackground({p:?})"),
        }
    }
}

/// What the glue reads answer from, seeded by App.
#[derive(Default)]
pub(crate) struct GlueState {
    pub(crate) requests: Vec<GlueRequest>,
    pub(crate) saved_account: String,
    pub(crate) connected: bool,
    pub(crate) server_name: String,
    /// `GetServerName`'s PvP and RP flags, the connected realm's type.
    pub(crate) server_pvp: bool,
    pub(crate) server_rp: bool,
    pub(crate) current_screen: String,
    pub(crate) num_characters: u32,
    /// The realm list: every tab, 1-based by ordinal; the first answers an ordinal past the last
    /// (`0x46edf0` has no sentinel).
    pub(crate) realm_tabs: Vec<GlueRealmTab>,
    pub(crate) realm_selected_category: usize,
    pub(crate) realm_update_rate: f32,
    /// The realmlist the login dials.
    pub(crate) realmlist: String,
    /// Character select's rows, and the stage an empty or out-of-range index stands on.
    pub(crate) characters: Vec<GlueCharacter>,
    pub(crate) empty_stage: String,
    /// `GetCharacterSelectFacing`, degrees.
    pub(crate) select_facing: f32,
}

impl GlueState {
    fn tab(&self, category: i64) -> Option<&GlueRealmTab> {
        let i = usize::try_from(category).ok()?.checked_sub(1)?;
        self.realm_tabs.get(i).or_else(|| self.realm_tabs.first())
    }
}

impl super::UiScript {
    /// Install the glue API on this VM: the glue host's VM only.
    pub fn install_glue_api(&self) -> mlua::Result<()> {
        self.lua().set_app_data(GlueState::default());
        api::install(self.lua())?;
        super::glue_create::install(self.lua())
    }

    /// The glue VM's state; `None` on a VM without the glue API (the in-world one), where every
    /// glue setter is a no-op and every getter answers nothing, so a caller scheduled across the
    /// world switch cannot take the client down.
    fn glue_mut(&self) -> Option<mlua::AppDataRefMut<'_, GlueState>> {
        self.lua().app_data_mut::<GlueState>()
    }

    /// Drain the glue calls App acts on, in call order.
    pub fn take_glue_requests(&mut self) -> Vec<GlueRequest> {
        self.glue_mut()
            .map(|mut m| std::mem::take(&mut m.requests))
            .unwrap_or_default()
    }

    /// `GetSavedAccountName`'s answer.
    pub fn set_glue_saved_account(&mut self, name: &str) {
        if let Some(mut m) = self.glue_mut() {
            m.saved_account = name.to_string();
        }
    }

    /// `IsConnectedToServer` and `GetServerName`'s answers.
    pub fn set_glue_connection(&mut self, connected: bool, server_name: &str) {
        if let Some(mut m) = self.glue_mut() {
            m.connected = connected;
            m.server_name = server_name.to_string();
        }
    }

    /// `GetServerName`'s second and third answers: whether the connected realm is PvP and RP.
    pub fn set_glue_server_type(&mut self, pvp: bool, rp: bool) {
        if let Some(mut m) = self.glue_mut() {
            m.server_pvp = pvp;
            m.server_rp = rp;
        }
    }

    /// `GetNumCharacters`'s answer.
    pub fn set_glue_num_characters(&mut self, n: u32) {
        if let Some(mut m) = self.glue_mut() {
            m.num_characters = n;
        }
    }

    /// The realm list `GetRealmCategories`/`GetNumRealms`/`GetRealmInfo` read, the tab
    /// `GetSelectedCategory` names, and the seconds `RealmListUpdateRate` answers.
    pub fn set_glue_realms(
        &mut self,
        tabs: Vec<GlueRealmTab>,
        selected_category: usize,
        update_rate: f32,
    ) {
        if let Some(mut m) = self.glue_mut() {
            m.realm_tabs = tabs;
            m.realm_selected_category = selected_category;
            m.realm_update_rate = update_rate;
        }
    }

    /// Character select's rows (`GetNumCharacters`, `GetCharacterInfo`, `GetSelectBackgroundModel`)
    /// and the stage an empty account stands on.
    pub fn set_glue_characters(&mut self, characters: Vec<GlueCharacter>, empty_stage: &str) {
        if let Some(mut m) = self.glue_mut() {
            m.num_characters = characters.len() as u32;
            m.characters = characters;
            m.empty_stage = empty_stage.to_string();
        }
    }

    /// `WrathPlusGetRealmlist`'s answer.
    pub fn set_glue_realmlist(&mut self, address: &str) {
        if let Some(mut m) = self.glue_mut() {
            m.realmlist = address.to_string();
        }
    }

    /// The launch's prefill (`--fill-only`): the stock login boxes hold `account` and `password`,
    /// which travel as call arguments, never as Lua source, and nothing submits.
    pub fn fill_login(&self, account: &str, password: &str) -> mlua::Result<()> {
        let fill: mlua::Function = self
            .lua()
            .load(
                "return function(a, p) AccountLoginAccountEdit:SetText(a) \
                 AccountLoginPasswordEdit:SetText(p) end",
            )
            .eval()?;
        fill.call((account, password))
    }

    /// The screen `SetCurrentScreen` last named (`"login"`, `"charselect"`, …); `None` off the
    /// glue VM.
    pub fn glue_current_screen(&self) -> Option<String> {
        self.glue_mut().map(|m| m.current_screen.clone())
    }
}
