//! Realm selection: the realm list dialog and the policy that decides whether the player sees it.
//! Not [`crate::realmlist`], which is the logon server's address (the `realmList` CVar).
//!
//! `RealmList` is a `frameStrata="DIALOG"` frame, not a glue screen (`GlueParent.lua`'s
//! `GlueScreenInfo` has no entry for it): it is shown over the current screen and hidden again,
//! and that screen is where the player returns.
//!
//! The IO thread publishes the list from two parks (`crate::net::io`), the login-side realm park
//! and the character park, and takes one answer. The policy, in order:
//!
//! 1. `--realm` (or `WOW_REALM`), matched case-insensitively; a name not on the list falls through.
//! 2. The realm this session is on, so a logout or a dropped connection returns to it.
//! 3. The remembered realm, the persisted `realmName` CVar.
//! 4. An unattended run (`crate::run_mode::unattended`) takes the first realm that is up.
//! 5. Otherwise the dialog.
//!
//! Change Realm costs a world dial, not a login. `RequestRealmList` (`0x46ecf0` -> `0x46b8d0`)
//! begins a realm query on the realmd socket, which stays open until world entry (`0x46b70c`), and
//! names no screen; only `ConnectToRealm` (`0x46b210`) drops the world connection, once a realm is
//! chosen. So the character park serves the list in place and Cancel there does nothing.

mod category;
mod glue;
mod input;
pub(crate) mod load;
mod screen;
mod smoke;

pub(crate) use glue::{glue_cancel, glue_change_realm, glue_refresh, glue_tabs, GLUE_UPDATE_RATE};
pub(crate) use load::pvp_rp;

use bevy::prelude::*;

use crate::net::{RealmChoice, RealmListMessage, RealmRequest};
pub(crate) use category::RealmsCategory;
use wrathplus_protocol::RealmInfo;

/// The persisted 1.12 CVar naming the last realm connected to (`0x83f2d0`); the SavedVariables
/// path (`0x8539a8`) is built from it.
pub(crate) const CVAR_REALM_NAME: &str = "realmName";

/// `REALM_LIST_REFRESH_TIME` (`RealmList.lua:3`): the re-request interval while the
/// dialog is up.
const REFRESH_SECS: f32 = 5.0;

#[cfg(test)]
use crate::app_core::ui_flags::Sort;
pub(crate) use crate::app_core::ui_flags::{Realms, SortKey};

/// `GetRealmInfo`'s `realmDown`: flag bit 0x02, greyed and not enterable; the reference's
/// auto-pick (`0x46ea20`) skips a realm on these low bits.
pub(crate) fn is_down(realm: &RealmInfo) -> bool {
    realm.flags & 0x02 != 0
}

/// `GetRealmInfo`'s `invalidRealm`: flag bit 0x01, painted red but still enterable.
pub(super) fn is_invalid(realm: &RealmInfo) -> bool {
    realm.flags & 0x01 != 0
}

/// The realm-selection subsystem: the park's policy plus the screen.
pub struct RealmSelectPlugin;

impl Plugin for RealmSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Realms>()
            .add_systems(
                Startup,
                category::load_categories.after(wrathplus_assets::AssetSet::Open),
            )
            .add_systems(
                Update,
                (
                    apply_realm_policy,
                    // Ungated by state: the dialog stands over login and character select alike,
                    // except the login screen the stock glue hosts, whose RealmList is its own.
                    screen::drive_screen.run_if(not(crate::ui_script::glue::stock_glue_up)),
                    // Input before the row refresh, so a click shows on the frame it landed.
                    smoke::debug_realm_smoke,
                    (
                        input::clicks,
                        input::keys,
                        tick_refresh,
                        screen::refresh_list,
                    )
                        .chain()
                        .run_if(|realms: Res<Realms>| realms.shown)
                        .run_if(not(crate::ui_script::glue::stock_glue_up)),
                )
                    .chain()
                    .after(wrathplus_world::schedule::WorldStage::Net)
                    .before(crate::glue::GlueVisuals),
            );
        hide_at_world_entry(app);
    }
}

/// World entry takes the realm list down with the rest of the glue: the reference builds the
/// world's interface on a Lua state of its own (`crate::ui_script`'s entry load), so no glue
/// frame is left to draw, whatever was open. The stock Okay hides the frame before it calls
/// `ChangeRealm`; a script calling `ChangeRealm` alone leaves the frame up, and its `OnUpdate`
/// goes on asking for the list, which opens it over character select again (#70). Left set in
/// the world, [`Realms::shown`] drew the native dialog over it (the stock glue no longer hosts
/// the screen there) and made the next published list a refresh, never answered.
pub(crate) fn hide_at_world_entry(app: &mut App) {
    app.add_systems(
        OnEnter(crate::app_core::state::ClientState::InWorld),
        |mut realms: ResMut<Realms>| realms.hide(),
    );
}

/// Answer the realm park, or raise the dialog; the policy for both parks, in every state.
fn apply_realm_policy(
    mut msgs: MessageReader<RealmListMessage>,
    mut realms: ResMut<Realms>,
    choice: Res<RealmChoice>,
    cvars: Res<crate::cvars::Cvars>,
) {
    if !realms.env_read {
        realms.env_read = true;
        realms.env_realm = crate::launch_plan::plan().realm;
    }
    for msg in msgs.read() {
        realms.realms = msg.realms.clone();
        realms.refresh_in = REFRESH_SECS;
        // The registered default is empty: a client that has never connected.
        let remembered = cvars
            .get(CVAR_REALM_NAME)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if realms.current.is_none() {
            realms.current = remembered.clone();
        }
        // The rebuild (`0x46e510`): the tab in front is kept as its ordinal, as the Lua keeps it.
        realms.store_current_category();
        realms.clamp_offset();

        // A list published while the dialog is up is a refresh, never a question.
        if realms.shown {
            continue;
        }
        match auto_answer(
            &realms,
            remembered.as_deref(),
            crate::run_mode::unattended(),
        ) {
            Some(name) => {
                realms.select(&name);
                realms.enter(&choice, name);
            }
            None => realms.open(),
        }
    }
}

/// The realm that answers this list without the player, or `None` to show the dialog: the policy
/// ladder in the module doc. Every arm requires the realm to be up.
fn auto_answer(realms: &Realms, remembered: Option<&str>, unattended: bool) -> Option<String> {
    // Case-insensitive: vmangos does not normalise realm names. `current` outranks the persisted
    // CVar, which WrathPlus writes only at world entry, so a logout before it still returns here.
    let named = [
        realms.env_realm.as_deref(),
        realms.current.as_deref(),
        remembered,
    ]
    .into_iter()
    .flatten()
    .find_map(|want| {
        realms
            .realms
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(want) && !is_down(r))
            .map(|r| r.name.clone())
    });
    named.or_else(|| {
        // Nobody to click Okay: the first realm up, in drawn order.
        unattended
            .then(|| {
                realms
                    .rows()
                    .into_iter()
                    .map(|i| &realms.realms[i])
                    .find(|r| !is_down(r))
                    .map(|r| r.name.clone())
            })
            .flatten()
    })
}

/// Change Realm: raise the dialog over character select and ask for a fresh list; the session is
/// untouched.
///
/// Deviation: the reference shows the frame when `OPEN_REALM_LIST` arrives; ours shows at once
/// against the list already in hand, so the click does not wait a round trip.
pub fn open_over_char_select(realms: &mut Realms, choice: &RealmChoice) {
    realms.open();
    let _ = choice.0.send(RealmRequest::Refresh);
}

/// `RealmList_OnUpdate`: re-request the list every `REALM_LIST_REFRESH_TIME` while shown
/// (`RealmList_OnHide` cancels the query).
fn tick_refresh(time: Res<Time>, mut realms: ResMut<Realms>, choice: Res<RealmChoice>) {
    realms.refresh_in -= time.delta_secs();
    if realms.refresh_in <= 0.0 {
        realms.refresh_in = REFRESH_SECS;
        let _ = choice.0.send(RealmRequest::Refresh);
    }
}

/// `Realms`'s methods that need this module (the type lives in `app_core` for the crate split).
pub(crate) trait RealmsSelect {
    /// The rows the screen draws: indices into [`Self::realms`] for the tab in front, sorted per
    /// category (`0x46e750`).
    fn rows(&self) -> Vec<usize>;
    /// The load distribution, over every realm, not the selected category: `0x46e510` walks the
    /// flat all-categories array, so switching tabs moves no band.
    fn stats(&self) -> (f32, f32);
    /// Whether Okay is live: `RealmListUpdate` disables it and only a highlighted, online row
    /// re-enables it.
    fn can_enter(&self) -> bool;
    /// `RealmList:Show()`: front the stored category's tab, reset the scroll, highlight
    /// [`Self::current`], start the refresh timer.
    fn open(&mut self);
    /// `GlueScrollFrame_Update` clamps the scroll bar to the tab's row count, and the bar's
    /// change handler writes `RealmList.offset` back.
    fn clamp_offset(&mut self);
    /// Answer a park with a realm and remember it as [`Self::current`].
    fn enter(&mut self, choice: &RealmChoice, name: String);
    /// `RealmListUpdate` with no clicked name: the highlight is the row `GetRealmInfo` flags
    /// `currentRealm`, [`Self::current`] case-folded, on the tab in front, or none.
    fn highlight_current(&mut self);
}

impl RealmsSelect for Realms {
    fn highlight_current(&mut self) {
        self.selected = self.current.as_deref().and_then(|want| {
            self.rows()
                .into_iter()
                .map(|i| &self.realms[i])
                .find(|r| r.name.eq_ignore_ascii_case(want))
                .map(|r| r.name.clone())
        });
    }

    fn rows(&self) -> Vec<usize> {
        let (mean, stddev) = self.stats();
        let mut rows = self.category_realms();
        rows.sort_by(|&a, &b| {
            let (ra, rb) = (&self.realms[a], &self.realms[b]);
            for (key, descending) in self.sort.0 {
                let ord = match key {
                    // Higher count first.
                    SortKey::Characters => rb.characters.cmp(&ra.characters),
                    SortKey::Load => {
                        let la = load::realm_load_classify(ra.flags, ra.population, mean, stddev);
                        let lb = load::realm_load_classify(rb.flags, rb.population, mean, stddev);
                        la.total_cmp(&lb)
                    }
                    SortKey::Name => ra.name.to_lowercase().cmp(&rb.name.to_lowercase()),
                    SortKey::Type => load::pvp_rp(ra.realm_type).cmp(&load::pvp_rp(rb.realm_type)),
                };
                if ord != std::cmp::Ordering::Equal {
                    return if descending { ord.reverse() } else { ord };
                }
            }
            // The reference's characters case answers +1 both ways on equal nonzero counts (its
            // tie-break strcmp is clobbered), a defect not reproduced: all four tied is Equal.
            std::cmp::Ordering::Equal
        });
        rows
    }

    fn stats(&self) -> (f32, f32) {
        let pops: Vec<f32> = self.realms.iter().map(|r| r.population).collect();
        load::realm_load_stats(&pops)
    }

    fn can_enter(&self) -> bool {
        self.selected().is_some_and(|r| !is_down(r))
    }

    fn open(&mut self) {
        self.shown = true;
        self.front_selected_category();
        self.offset = 0;
        self.highlight_current();
        self.refresh_in = REFRESH_SECS;
    }

    fn clamp_offset(&mut self) {
        let max = self.rows().len().saturating_sub(screen::MAX_ROWS);
        self.offset = self.offset.min(max);
    }

    fn enter(&mut self, choice: &RealmChoice, name: String) {
        self.current = Some(name.clone());
        let _ = choice.0.send(RealmRequest::Enter(name));
    }
}

#[cfg(test)]
mod tests;
