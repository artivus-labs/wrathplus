//! The talent window feed: the class's pages from `Talent.dbc` and `TalentTab.dbc`, the known
//! spells and `PLAYER_CHARACTER_POINTS1/2`, `LearnTalent` clicks as `CMSG_LEARN_TALENT` and
//! `LearnPreviewTalents` as `CMSG_LEARN_PREVIEW_TALENTS`. A
//! talent's rank and unspent points follow the active `SMSG_TALENTS_INFO` group once received,
//! falling back to known spells before it arrives; an unlearned one wears rank 1's name and icon.
//!
//! The red requirement lines are the three keys `0x52b390` emits, in its order:
//! `TOOLTIP_TALENT_TIER_POINTS` (`0x854a40`), `TOOLTIP_TALENT_PREREQ[_P1]` (`0x854a5c`) and
//! `ITEM_REQ_SKILL` (`0x84e338`), resolved off the install's `GlobalStrings.lua`; a key it lacks
//! renders no line.

use std::collections::BTreeSet;

use bevy::prelude::*;

use wrathplus_formats::{Talent, TalentCatalog};
use wrathplus_ui::script::{
    ScriptValue, TalentPrereqView, TalentTabView, TalentUiState, TalentView, UiScript,
    WrathTalentGroups,
};
use wrathplus_ui::strings::{fill, Arg};

use crate::app_core::schedule::UiInput;
use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::PlayerActions;
use crate::app_core::state::Spells;
use crate::net::wrath_notices::WrathProgress;
use crate::net::{ClientCommand, NetCommands};
use crate::ui_script::gate;
use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};

/// The talent catalog (`Talent.dbc`, `TalentTab.dbc`), absent when the client data failed to load.
#[derive(Resource)]
pub(crate) struct Talents {
    pub(crate) catalog: TalentCatalog,
}

pub struct UiTalentPlugin;

impl Plugin for UiTalentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_talents.after(AssetSet::Open))
            .add_systems(
                Update,
                (
                    // Feed before the input pass, so an open this frame sees a populated
                    // window; drain after it, so a click sends the same frame.
                    feed_talents.in_set(UnitFeed),
                    drain_talent_learns.after(UiInput),
                ),
            );
    }
}

fn load_talents(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_talent_catalog(&mut chain)
    };
    match loaded {
        Ok(catalog) => {
            info!("ui_talent: {} talent(s) in the catalog", catalog.len());
            commands.insert_resource(Talents { catalog });
        }
        Err(e) => warn!("ui_talent: Talent.dbc failed to load — no talent window: {e:#}"),
    }
}

/// The last pushed snapshot, and the points pair `CHARACTER_POINTS_CHANGED`'s deltas come from.
#[derive(Default)]
struct FeedMemory {
    pushed: TalentUiState,
    groups: WrathTalentGroups,
    /// `None` until the first read, which gives zero deltas.
    points: Option<(u32, u32)>,
}

#[allow(clippy::too_many_arguments)]
fn feed_talents(
    script: Option<NonSendMut<UiScript>>,
    talents: Option<Res<Talents>>,
    actions: Res<PlayerActions>,
    progress: Option<Res<WrathProgress>>,
    spells: Option<Res<Spells>>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    changed_self: Query<(), (With<SelfPlayer>, Changed<ObjectStore>)>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<FeedMemory>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let (memory, vm_reset) = memory.get_reset(&script);
    // Reopened by every input the build reads: the known spells, both catalogs (their landing
    // counts as a change) and the self descriptor.
    let self_changed = !changed_self.is_empty();
    let actions_changed = actions.is_changed();
    let progress_changed = progress.as_ref().is_some_and(|r| r.is_changed());
    let talents_changed = talents.as_ref().is_some_and(|r| r.is_changed());
    let spells_changed = spells.as_ref().is_some_and(|r| r.is_changed());
    gate::trace(
        "feed_talents",
        &[
            ("vm_reset", vm_reset),
            ("self", self_changed),
            ("actions", actions_changed),
            ("progress", progress_changed),
            ("talents", talents_changed),
            ("spells", spells_changed),
        ],
    );
    let gate = gate::Gate::new(
        vm_reset
            || self_changed
            || actions_changed
            || progress_changed
            || talents_changed
            || spells_changed,
    );
    if gate.skip() {
        return;
    }
    let groups = player_groups(progress.as_deref());
    let mut group_change = None;
    if groups != memory.groups {
        // `0x5c9ed7`: a packet that moves the active group fires (new, old), each 1-based or 0
        // without groups (`0x5c9edb`-`0x5c9ef2`); a reloaded VM is seeded silently.
        let packet = progress
            .as_ref()
            .is_some_and(|p| p.talents.contains_key(&false));
        if packet && !vm_reset && groups.active != memory.groups.active {
            let n = |g: &WrathTalentGroups| (g.count > 0).then_some(i64::from(g.active) + 1);
            group_change = Some((n(&groups).unwrap_or(0), n(&memory.groups).unwrap_or(0)));
        }
        script.set_talent_groups(groups.clone());
        memory.groups = groups;
    }
    'pages: {
        let (Some(talents), Some(spells)) = (talents.as_deref(), spells.as_deref()) else {
            break 'pages;
        };
        let Ok(store) = self_q.single() else {
            break 'pages;
        };
        let race = store.0.unit_race().unwrap_or(0);
        let class = store.0.unit_class().unwrap_or(0);
        let points = (
            player_talent_state(progress.as_deref()).map_or_else(
                || store.0.player_talent_points().unwrap_or(0),
                |(unspent, _)| unspent,
            ),
            store.0.player_free_professions().unwrap_or(0),
        );
        // Scoped: the `GlobalStrings.lua` lookup borrows the VM, which the push needs mutably.
        let fresh = {
            let get = |key: &str| {
                script
                    .lua()
                    .globals()
                    .get::<String>(key)
                    .ok()
                    .filter(|t| !t.is_empty())
            };
            build_pages_by(
                &talents.catalog,
                &|t| player_rank(t, &actions.spells, progress.as_deref()),
                &|spell| actions.spells.contains(&spell),
                &spells.catalog,
                race,
                class,
                points,
                &get,
            )
        };
        if fresh != memory.pushed {
            gate.audit("feed_talents", "the talent pages");
            debug!(
                "ui_talent: fed {} tab(s), {} points",
                fresh.tabs.len(),
                points.0
            );
            script.set_talents(fresh.clone());
            memory.pushed = fresh;
            // Build-12340 PlayerTalentFrame repaints on this event, after its getters are updated.
            script.fire_event("PLAYER_TALENT_UPDATE", vec![]);
            // The talent and profession point deltas, as the reference's `SignalEvent2` sends them:
            // `ChatFrame.lua:1326` tests `arg2 > 0` unguarded, so an argless fire raises.
            // `memory.points` still holds the previous pair here.
            let (talent_delta, profession_delta) = match memory.points {
                Some((old_talent, old_professions)) => (
                    i64::from(points.0) - i64::from(old_talent),
                    i64::from(points.1) - i64::from(old_professions),
                ),
                None => (0, 0),
            };
            script.fire_event(
                "CHARACTER_POINTS_CHANGED",
                vec![
                    ScriptValue::Int(talent_delta),
                    ScriptValue::Int(profession_delta),
                ],
            );
        }
        // The free-professions line must not be composed here: the stock `ChatFrame_OnEvent` arm
        // (`ChatFrame.lua:1324-1334`) prints `LEVEL_UP_SKILL_POINTS` itself on `arg2 > 0`.
        if memory.points != Some(points) {
            gate.audit("feed_talents", "the points memo");
            memory.points = Some(points);
        }
    }
    // After `PLAYER_TALENT_UPDATE`, as the handler signals `0x271` before `0x272`.
    if let Some((new, old)) = group_change {
        let args = vec![ScriptValue::Int(new), ScriptValue::Int(old)];
        script.fire_event("ACTIVE_TALENT_GROUP_CHANGED", args);
    }
}

/// AzerothCore's BuildPlayerTalentsInfoData supplies the active spec and unspent points.
fn player_talent_state(
    progress: Option<&WrathProgress>,
) -> Option<(u32, &wrathplus_protocol::events::TalentGroup)> {
    let (unspent, active, groups) = progress?.talents.get(&false)?;
    Some((*unspent, groups.get(usize::from(*active))?))
}

/// The player's group count and active group off the talents packet, zero without one.
fn player_groups(progress: Option<&WrathProgress>) -> WrathTalentGroups {
    progress.and_then(|p| p.talents.get(&false)).map_or_else(
        Default::default,
        |(_, active, groups)| WrathTalentGroups {
            count: groups.len() as u32,
            active: u32::from(*active),
        },
    )
}

fn player_rank(t: &Talent, known: &BTreeSet<u32>, progress: Option<&WrathProgress>) -> u32 {
    match player_talent_state(progress) {
        Some((_, group)) => group
            .talents
            .iter()
            .find(|&&(id, _)| id == t.id)
            .map_or(0, |&(_, rank)| (u32::from(rank) + 1).min(t.max_rank())),
        None => rank_of(t, known),
    }
}

/// A talent's rank: the highest rank whose spell is known.
fn rank_of(t: &Talent, known: &BTreeSet<u32>) -> u32 {
    t.ranks
        .iter()
        .enumerate()
        .filter(|(_, &s)| s != 0 && known.contains(&s))
        .map(|(i, _)| i as u32 + 1)
        .max()
        .unwrap_or(0)
}

/// Builds the pushed snapshot.
pub fn build_pages(
    catalog: &TalentCatalog,
    known: &BTreeSet<u32>,
    // `Spell.dbc` alone, for names and icons, so the addon-corpus survey can build the snapshot
    // without the other spell catalogs.
    spells: &wrathplus_formats::SpellCatalog,
    race: u8,
    class: u8,
    points: (u32, u32),
    // The VM's `GlobalStrings.lua`, for the red requirement lines.
    get: &dyn Fn(&str) -> Option<String>,
) -> TalentUiState {
    build_pages_by(
        catalog,
        &|t| rank_of(t, known),
        &|spell| known.contains(&spell),
        spells,
        race,
        class,
        points,
        get,
    )
}

/// [`build_pages`] over any rank source: our own known spells, or an inspected player's
/// `SMSG_INSPECT_TALENT` ranks ([`crate::ui_inspect_wrath`]). `knows` answers a talent's
/// `requiredSpell`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pages_by(
    catalog: &TalentCatalog,
    rank_of: &dyn Fn(&Talent) -> u32,
    knows: &dyn Fn(u32) -> bool,
    spells: &wrathplus_formats::SpellCatalog,
    race: u8,
    class: u8,
    points: (u32, u32),
    get: &dyn Fn(&str) -> Option<String>,
) -> TalentUiState {
    let tabs = catalog.tabs_for_class(race, class);
    build_tab_pages(catalog, tabs, rank_of, knows, spells, points, get)
}

/// [`build_pages_by`] over given tabs: a class's, or the hunter pet's one.
pub(crate) fn build_tab_pages(
    catalog: &TalentCatalog,
    tab_list: Vec<&wrathplus_formats::TalentTabInfo>,
    rank_of: &dyn Fn(&Talent) -> u32,
    knows: &dyn Fn(u32) -> bool,
    spells: &wrathplus_formats::SpellCatalog,
    points: (u32, u32),
    get: &dyn Fn(&str) -> Option<String>,
) -> TalentUiState {
    let mut tabs = Vec::new();
    let mut pages = Vec::new();
    for tab in tab_list {
        let list = catalog.talents_in_tab(tab.id);
        let spent: u32 = list.iter().map(rank_of).sum();
        let mut views = Vec::with_capacity(list.len());
        for t in list {
            let rank = rank_of(t);
            let max_rank = t.max_rank();
            let display_spell = t.ranks[rank.max(1) as usize - 1];
            let next_spell = if rank > 0 && rank < max_rank {
                t.ranks[rank as usize]
            } else {
                0
            };
            let d = spells.get(display_spell);
            // `meetsPrereq` is the `requiredSpell` check alone (`GetTalentInfo` `0x4f3200`).
            let meets_prereq = t.required_spell == 0 || knows(t.required_spell);
            // The tier gate reads the tab's own spent sum; prereqs read the prereq's rank.
            let tier_unlocked = t.row * 5 <= spent;
            let mut req_lines = Vec::new();
            if !tier_unlocked {
                req_lines.extend(
                    get("TOOLTIP_TALENT_TIER_POINTS")
                        .map(|f| fill(&f, &[Arg::D(i64::from(t.row * 5)), Arg::S(&tab.name)])),
                );
            }
            // The item tooltip's `ITEM_REQ_SKILL`, not the `LOCKED_WITH_*` or `SPELL_FAILED_*`
            // keys that read the same in enUS.
            if !meets_prereq {
                if let Some(req) = spells.get(t.required_spell) {
                    req_lines.extend(get("ITEM_REQ_SKILL").map(|f| fill(&f, &[Arg::S(&req.name)])));
                }
            }
            let mut prereqs = Vec::new();
            if t.prereq_talent != 0 {
                if let Some(p) = catalog.talent(t.tab, t.prereq_talent) {
                    let need = t.prereq_rank + 1;
                    let learnable = rank_of(p) >= need;
                    prereqs.push(TalentPrereqView {
                        tier: p.row + 1,
                        column: p.col + 1,
                        learnable,
                        rank: need,
                    });
                    if !learnable {
                        let p_name = spells
                            .get(p.ranks[0])
                            .map(|d| d.name.clone())
                            .unwrap_or_default();
                        req_lines.extend(
                            wrathplus_ui::strings::plural("TOOLTIP_TALENT_PREREQ", Some(need), get)
                                .map(|f| fill(&f, &[Arg::D(i64::from(need)), Arg::S(&p_name)])),
                        );
                    }
                }
            }
            // The green hint is points left and not maxed alone (`SetTalent` `0x535170`); the
            // frame's gold, green and gray come from the stock Lua.
            let learnable = rank < max_rank && points.0 > 0;
            views.push(TalentView {
                id: t.id,
                name: d.map(|d| d.name.clone()).unwrap_or_default(),
                texture: d.and_then(|d| d.icon.clone()),
                tier: t.row + 1,
                column: t.col + 1,
                rank,
                max_rank,
                exceptional: t.exceptional,
                meets_prereq,
                prereqs,
                display_spell,
                next_spell,
                req_lines,
                learnable,
            });
        }
        tabs.push(TalentTabView {
            name: tab.name.clone(),
            background: tab.background.clone(),
            points_spent: spent,
        });
        pages.push(views);
    }
    TalentUiState {
        tabs,
        talents: pages,
        points,
    }
}

/// Drains `LearnTalent(tab, index)` clicks into `CMSG_LEARN_TALENT` for the current rank count,
/// the 0-based next rank, which vmangos learns up to.
#[allow(clippy::too_many_arguments)]
fn drain_talent_learns(
    script: Option<NonSendMut<UiScript>>,
    talents: Option<Res<Talents>>,
    actions: Res<PlayerActions>,
    progress: Option<Res<WrathProgress>>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    commands: Res<NetCommands>,
    pet: (
        Option<Res<crate::ui_pet_talent_tree::PetTalentPages>>,
        Option<Res<crate::app_core::state::PetBar>>,
    ),
) {
    let Some(mut script) = script else {
        return;
    };
    crate::ui_chat_verbs::drain(&mut script, self_q.single().ok(), &commands);
    // `LearnPreviewTalents` (`0x5c6a10`): the VM built the pairs in send order.
    for talents in script.take_talent_preview_learns() {
        debug!("ui_talent: learn {} preview talent(s)", talents.len());
        let _ = commands.0.send(ClientCommand::NativeInspect(
            wrathplus_protocol::wrath_inspect::Command::LearnPreviewTalents { talents },
        ));
    }
    let (clicks, pet_clicks): (Vec<_>, Vec<_>) = script
        .take_talent_learns()
        .into_iter()
        .partition(|&(_, _, pet)| !pet);
    if let (Some(pages), Some(bar)) = (pet.0.as_deref(), pet.1.as_deref()) {
        for (tab, index, _) in pet_clicks {
            crate::ui_pet_talent_tree::learn(pages, bar.guid(), tab, index, &commands);
        }
    }
    if clicks.is_empty() {
        return;
    }
    let Some(talents) = talents.as_deref() else {
        return;
    };
    let Ok(store) = self_q.single() else {
        return;
    };
    let race = store.0.unit_race().unwrap_or(0);
    let class = store.0.unit_class().unwrap_or(0);
    let tabs = talents.catalog.tabs_for_class(race, class);
    for (tab, index, _) in clicks {
        // 1-based Lua-facing pair; a 0 from a stray script is a miss, never an underflow.
        let Some(t) = (tab as usize)
            .checked_sub(1)
            .and_then(|i| tabs.get(i))
            .and_then(|tab| {
                (index as usize)
                    .checked_sub(1)
                    .and_then(|i| talents.catalog.talents_in_tab(tab.id).get(i))
            })
        else {
            continue;
        };
        let rank = player_rank(t, &actions.spells, progress.as_deref());
        // Not at max is the client's only send gate (`LearnTalent` `0x4f36a0`); points and
        // prereqs are the server's to enforce.
        if rank >= t.max_rank() {
            continue;
        }
        debug!("ui_talent: learn talent {} rank {}", t.id, rank);
        let _ = commands.0.send(ClientCommand::LearnTalent {
            talent_id: t.id,
            rank,
        });
    }
}

#[cfg(test)]
#[path = "ui_talent_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ui_talent_refresh_tests.rs"]
mod refresh_tests;
