//! Build 12340's talent preview and talent links, over the player's pushed pages
//! ([`super::TalentUiState`]). Evidence: one headless Ghidra round
//! (`/tmp/azb-wire-ghidra/census2`).
//!
//! - `AddPreviewTalentPoints(tab, talent, points[, isPet[, group]])` (`0x5c9590`) moves a talent's
//!   preview rank. When the group's preview total changes it fires
//!   `PREVIEW_TALENT_POINTS_CHANGED(talent, tab, group, delta)` (`0x274`); a pet's is `0x275`.
//! - `ResetPreviewTalentPoints(tab[, isPet[, group]])` (`0x5c7130`) and
//!   `ResetGroupPreviewTalentPoints([isPet[, group]])` (`0x5c7200`) put preview ranks back to the
//!   current ones and fire the same event with talent 0, and also tab 0 for the group.
//! - `LearnPreviewTalents([isPet])` (`0x5c6a10`) sends, for the active group only and only with a
//!   positive preview total, `CMSG_LEARN_PREVIEW_TALENTS` (`0x4C1`): a count, then
//!   `(talent id, preview rank 0-based)` for each talent whose preview passes its rank.
//! - `GetTalentLink(tab, talent[, isInspect[, isPet[, group[, isPreview]]]])` (`0x5c7ce0`) is
//!   `|cff4e96f7|Htalent:<id>:<rank>|h[<name>]|h|r` (`0x61a550`). `rank` is the current or preview
//!   rank 0-based, so -1 while unlearned. The name is that rank's spell, rank 1's when unlearned.
//!   A talent with no spell row answers nil.
//!
//! A preview rank is kept absolute and reads as `max(kept, current)`, so a learned preview
//! settles once the server's ranks catch up. The packet lists talents by tab, tier, then column,
//! so a prerequisite and lower tiers come first, as AzerothCore's `HandleLearnPreviewTalents`
//! learns them in packet order (`Player::LearnTalent`).
//!
//! Labelled:
//! - The legality check `0x5c73d0` was not traced. A preview step here follows
//!   `Player::LearnTalent`'s gates: the required spell, the prerequisite's preview rank, and
//!   `tier * 5` points in the tab counting previews in lower tiers. It is clamped to the max rank
//!   and the unspent points. A removal never goes below the current rank and is refused when it
//!   would strand a previewed talent.
//! - A rank drop (a respec) clears the preview.
//! - Pet talents and a second group are not carried, so their calls change nothing and their
//!   links are nil.

use std::collections::HashMap;

use mlua::{Lua, MultiValue};

use super::talent::TalentView;
use super::{Model, ScriptValue, TalentUiState};

const PREVIEW_TALENT_POINTS_CHANGED: &str = "PREVIEW_TALENT_POINTS_CHANGED";

/// The kept preview ranks by 0-based (tab, talent), and the queued `LearnPreviewTalents` lists.
#[derive(Clone, Debug, Default)]
pub(crate) struct TalentPreview {
    ranks: HashMap<(usize, usize), u32>,
    learns: Vec<Vec<(u32, u32)>>,
}

impl super::UiScript {
    /// Drain the `LearnPreviewTalents` lists, each `CMSG_LEARN_PREVIEW_TALENTS`'
    /// `(talent id, rank 0-based)` pairs in send order.
    pub fn take_talent_preview_learns(&mut self) -> Vec<Vec<(u32, u32)>> {
        std::mem::take(&mut self.model_mut().talent_preview.learns)
    }
}

/// A rank drop clears the preview; the pages are replaced by the caller.
pub(super) fn on_pages(model: &mut Model, fresh: &TalentUiState) {
    let dropped = model
        .talents
        .talents
        .iter()
        .zip(&fresh.talents)
        .any(|(old, new)| old.iter().zip(new).any(|(a, b)| b.rank < a.rank));
    if dropped {
        model.talent_preview.ranks.clear();
    }
}

/// The player's preview rank of a talent, `max(kept, current)`.
pub(super) fn preview_rank(model: &Model, tab: usize, i: usize, t: &TalentView) -> u32 {
    model
        .talent_preview
        .ranks
        .get(&(tab, i))
        .map_or(t.rank, |&r| r.clamp(t.rank, t.max_rank.max(t.rank)))
}

/// A tab's preview points past its current ones, `GetTalentTabInfo`'s fifth return.
pub(super) fn tab_preview_spent(model: &Model, tab: usize) -> u32 {
    model.talents.talents.get(tab).map_or(0, |list| {
        list.iter()
            .enumerate()
            .map(|(i, t)| preview_rank(model, tab, i, t) - t.rank)
            .sum()
    })
}

/// The group's preview points past its current ones.
pub(super) fn group_preview_spent(model: &Model) -> u32 {
    (0..model.talents.talents.len())
        .map(|tab| tab_preview_spent(model, tab))
        .sum()
}

/// A prerequisite `(tier, column)` in tab `tab`, by index.
fn talent_at_seat(p: &TalentUiState, tab: usize, tier: u32, column: u32) -> Option<usize> {
    p.talents
        .get(tab)?
        .iter()
        .position(|t| t.tier == tier && t.column == column)
}

/// `isPreviewLearnable` of a prerequisite edge: its preview rank reaches the required rank.
pub(super) fn prereq_preview_met(
    model: &Model,
    tab: usize,
    tier: u32,
    column: u32,
    need: u32,
) -> bool {
    talent_at_seat(&model.talents, tab, tier, column).is_some_and(|i| {
        let t = &model.talents.talents[tab][i];
        preview_rank(model, tab, i, t) >= need
    })
}

/// The tab's points that count toward tier `tier` (1-based): every current rank, plus the preview
/// points of the lower tiers.
fn points_toward(ranks: &[(u32, u32, u32)], tier: u32) -> u32 {
    ranks
        .iter()
        .map(|&(t, current, preview)| if t < tier { preview } else { current })
        .sum()
}

/// Whether a preview state holds: every previewed talent meets its tier, its prerequisite and its
/// required spell.
fn holds(p: &TalentUiState, ranks: &HashMap<(usize, usize), u32>) -> bool {
    for (tab, list) in p.talents.iter().enumerate() {
        let rank_of = |i: usize| -> u32 {
            let t = &list[i];
            ranks.get(&(tab, i)).copied().unwrap_or(t.rank).max(t.rank)
        };
        let tiers: Vec<(u32, u32, u32)> = list
            .iter()
            .enumerate()
            .map(|(i, t)| (t.tier, t.rank, rank_of(i)))
            .collect();
        for (i, t) in list.iter().enumerate() {
            if rank_of(i) <= t.rank {
                continue;
            }
            if !t.meets_prereq || points_toward(&tiers, t.tier) < (t.tier - 1) * 5 {
                return false;
            }
            for e in &t.prereqs {
                let met =
                    talent_at_seat(p, tab, e.tier, e.column).is_some_and(|j| rank_of(j) >= e.rank);
                if !met {
                    return false;
                }
            }
        }
    }
    true
}

/// Apply `points` to one talent's preview rank; the change actually made.
fn add_points(model: &mut Model, tab: usize, i: usize, points: i64) -> i64 {
    let Some(t) = model
        .talents
        .talents
        .get(tab)
        .and_then(|l| l.get(i))
        .cloned()
    else {
        return 0;
    };
    let current = preview_rank(model, tab, i, &t);
    let available = i64::from(model.talents.points.0) - i64::from(group_preview_spent(model));
    let delta = if points > 0 {
        points
            .min(i64::from(t.max_rank) - i64::from(current))
            .min(available)
            .max(0)
    } else {
        points.max(i64::from(t.rank) - i64::from(current)).min(0)
    };
    if delta == 0 {
        return 0;
    }
    let mut next = model.talent_preview.ranks.clone();
    next.insert((tab, i), (i64::from(current) + delta) as u32);
    if !holds(&model.talents, &next) {
        return 0;
    }
    model.talent_preview.ranks = next;
    delta
}

/// The `(talent id, rank 0-based)` pairs `LearnPreviewTalents` sends, by tab, tier, then column.
fn learn_list(model: &Model) -> Vec<(u32, u32)> {
    let mut rows = Vec::new();
    for (tab, list) in model.talents.talents.iter().enumerate() {
        for (i, t) in list.iter().enumerate() {
            let preview = preview_rank(model, tab, i, t);
            if preview > t.rank {
                rows.push(((tab, t.tier, t.column), (t.id, preview - 1)));
            }
        }
    }
    rows.sort_by_key(|(k, _)| *k);
    rows.into_iter().map(|(_, r)| r).collect()
}

/// The 1-based group argument at `i`, the active group (1) when absent (`0x5c57d0`).
fn group_arg(lua: &Lua, args: &MultiValue, i: usize) -> i64 {
    args.get(i)
        .cloned()
        .and_then(|v| lua.coerce_integer(v).ok().flatten())
        .unwrap_or(1)
}

fn truth(args: &MultiValue, i: usize) -> bool {
    super::binding_abi::bool_or_default(args.get(i), false)
}

fn fire_changed(lua: &Lua, talent: i64, tab: i64, group: i64, delta: i64) {
    super::tick::fire_event_into(
        lua,
        PREVIEW_TALENT_POINTS_CHANGED,
        vec![
            ScriptValue::Int(talent),
            ScriptValue::Int(tab),
            ScriptValue::Int(group),
            ScriptValue::Int(delta),
        ],
    );
}

/// `GetTalentLink`'s string.
pub(super) fn talent_link(t: &TalentView, rank: u32) -> Option<String> {
    (!t.name.is_empty()).then(|| {
        format!(
            "|cff4e96f7|Htalent:{}:{}|h[{}]|h|r",
            t.id,
            i64::from(rank) - 1,
            t.name
        )
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "AddPreviewTalentPoints",
        lua.create_function(|lua, args: MultiValue| {
            let int = |i: usize| {
                args.get(i)
                    .cloned()
                    .and_then(|v| lua.coerce_integer(v).ok().flatten())
            };
            let (Some(tab), Some(talent), Some(points)) = (int(0), int(1), int(2)) else {
                return Err(mlua::Error::runtime(
                    "Usage: AddPreviewTalentPoints(tabIndex, talentIndex, points[, isPet[, groupIndex]])",
                ));
            };
            let group = group_arg(lua, &args, 4);
            if truth(&args, 3) || group != 1 {
                return Ok(());
            }
            let delta = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                match (usize::try_from(tab - 1), usize::try_from(talent - 1)) {
                    (Ok(t), Ok(i)) => add_points(&mut model, t, i, points),
                    _ => 0,
                }
            };
            if delta != 0 {
                fire_changed(lua, talent, tab, group, delta);
            }
            Ok(())
        })?,
    )?;

    g.set(
        "ResetPreviewTalentPoints",
        lua.create_function(|lua, args: MultiValue| {
            let Some(tab) = args
                .front()
                .cloned()
                .and_then(|v| lua.coerce_integer(v).ok().flatten())
            else {
                return Err(mlua::Error::runtime(
                    "Usage: ResetPreviewTalentPoints(tabIndex[, isPet[, groupIndex]])",
                ));
            };
            let group = group_arg(lua, &args, 2);
            if truth(&args, 1) || group != 1 {
                return Ok(());
            }
            let delta = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let before = group_preview_spent(&model);
                if let Ok(t) = usize::try_from(tab - 1) {
                    model.talent_preview.ranks.retain(|&(at, _), _| at != t);
                }
                i64::from(group_preview_spent(&model)) - i64::from(before)
            };
            if delta != 0 {
                fire_changed(lua, 0, tab, group, delta);
            }
            Ok(())
        })?,
    )?;

    g.set(
        "ResetGroupPreviewTalentPoints",
        lua.create_function(|lua, args: MultiValue| {
            let group = group_arg(lua, &args, 1);
            if truth(&args, 0) || group != 1 {
                return Ok(());
            }
            let delta = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let before = group_preview_spent(&model);
                model.talent_preview.ranks.clear();
                -i64::from(before)
            };
            if delta != 0 {
                fire_changed(lua, 0, 0, group, delta);
            }
            Ok(())
        })?,
    )?;

    // GetGroupPreviewTalentPointsSpent([isPet[, group]]) (`0x5c6420`).
    g.set(
        "GetGroupPreviewTalentPointsSpent",
        lua.create_function(|lua, args: MultiValue| {
            let group = group_arg(lua, &args, 1);
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(if truth(&args, 0) || group != 1 {
                0
            } else {
                i64::from(group_preview_spent(&model))
            })
        })?,
    )?;

    g.set(
        "LearnPreviewTalents",
        lua.create_function(|lua, args: MultiValue| {
            if truth(&args, 0) {
                return Ok(());
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if group_preview_spent(&model) == 0 {
                return Ok(());
            }
            let list = learn_list(&model);
            if !list.is_empty() {
                model.talent_preview.learns.push(list);
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::talent::{TalentPrereqView, TalentTabView, TalentView};
    use crate::script::{TalentUiState, UiScript};

    fn talent(id: u32, tier: u32, column: u32, rank: u32, max_rank: u32) -> TalentView {
        TalentView {
            id,
            name: format!("T{id}"),
            tier,
            column,
            rank,
            max_rank,
            meets_prereq: true,
            ..Default::default()
        }
    }

    /// Tier 1: A (5 ranks, 2 learned) and B (1 rank); tier 2: C (3 ranks) needing A at 5.
    fn script(unspent: u32) -> UiScript {
        let mut s = UiScript::new().unwrap();
        let mut c = talent(30, 2, 1, 0, 3);
        c.prereqs = vec![TalentPrereqView {
            tier: 1,
            column: 1,
            learnable: false,
            rank: 5,
        }];
        s.set_talents(TalentUiState {
            tabs: vec![TalentTabView {
                name: "Arms".into(),
                background: "WarriorArms".into(),
                icon: None,
                points_spent: 2,
            }],
            talents: vec![vec![talent(10, 1, 1, 2, 5), talent(20, 1, 2, 0, 1), c]],
            points: (unspent, 0),
        });
        s.run(
            "FIRED = {} \
             local f = CreateFrame('Frame') \
             f:RegisterEvent('PREVIEW_TALENT_POINTS_CHANGED') \
             f:SetScript('OnEvent', function(_, _, ...) FIRED[#FIRED + 1] = table.concat({...}, ',') end)",
        )
        .unwrap();
        s
    }

    #[test]
    fn preview_points_gate_like_learning_and_fire_on_change() {
        let mut s = script(4);
        // C is locked: tier 2 needs 5 points below it, and A at 5.
        s.run("AddPreviewTalentPoints(1, 3, 1)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            0
        );
        // A to 5 (clamped from 9 to the max rank), then C opens.
        s.run("AddPreviewTalentPoints(1, 1, 9)").unwrap();
        s.run("AddPreviewTalentPoints(1, 3, 1)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            4
        );
        assert_eq!(
            s.eval::<i64>("return select(5, GetTalentTabInfo(1))")
                .unwrap(),
            4
        );
        assert_eq!(
            s.eval::<(i64, i64)>("local _,_,_,_,r,_,_,_,p = GetTalentInfo(1, 1) return r, p")
                .unwrap(),
            (2, 5)
        );
        assert!(s.eval::<bool>("return select(4, GetTalentPrereqs(1, 3)) == 1 or select(4, GetTalentPrereqs(1, 3)) == true").unwrap());
        // Removing A's preview would strand C.
        s.run("AddPreviewTalentPoints(1, 1, -1)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            4
        );
        // Out of points: B stays.
        s.run("AddPreviewTalentPoints(1, 2, 1)").unwrap();
        s.run("AddPreviewTalentPoints(1, 1, 1, true)").unwrap();
        assert_eq!(
            s.eval::<Vec<String>>("return FIRED").unwrap(),
            vec!["1,1,1,3".to_string(), "3,1,1,1".to_string()]
        );
        s.run("LearnPreviewTalents()").unwrap();
        assert_eq!(s.take_talent_preview_learns(), vec![vec![(10, 4), (30, 0)]]);
        s.run("ResetGroupPreviewTalentPoints()").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            0
        );
        assert_eq!(s.eval::<String>("return FIRED[3]").unwrap(), "0,0,1,-4");
        s.run("LearnPreviewTalents()").unwrap();
        assert!(s.take_talent_preview_learns().is_empty());
    }

    #[test]
    fn a_learned_preview_settles_and_a_respec_clears_it() {
        let mut s = script(3);
        s.run("AddPreviewTalentPoints(1, 1, 2)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            2
        );
        // The server learns A to 4.
        let mut fresh = TalentUiState {
            tabs: vec![TalentTabView::default()],
            talents: vec![vec![
                talent(10, 1, 1, 4, 5),
                talent(20, 1, 2, 0, 1),
                talent(30, 2, 1, 0, 3),
            ]],
            points: (1, 0),
        };
        s.set_talents(fresh.clone());
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            0
        );
        s.run("AddPreviewTalentPoints(1, 1, 1)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            1
        );
        fresh.talents[0][0].rank = 0;
        fresh.points = (6, 0);
        s.set_talents(fresh);
        assert_eq!(
            s.eval::<i64>("return GetGroupPreviewTalentPointsSpent()")
                .unwrap(),
            0
        );
    }

    #[test]
    fn talent_links_name_the_current_or_preview_rank() {
        let s = script(5);
        assert_eq!(
            s.eval::<String>("return GetTalentLink(1, 1)").unwrap(),
            "|cff4e96f7|Htalent:10:1|h[T10]|h|r"
        );
        assert_eq!(
            s.eval::<String>("return GetTalentLink(1, 2)").unwrap(),
            "|cff4e96f7|Htalent:20:-1|h[T20]|h|r"
        );
        s.run("AddPreviewTalentPoints(1, 1, 1)").unwrap();
        assert_eq!(
            s.eval::<String>("return GetTalentLink(1, 1, false, false, 1, true)")
                .unwrap(),
            "|cff4e96f7|Htalent:10:2|h[T10]|h|r"
        );
        assert!(s.eval::<bool>("return GetTalentLink(1, 9) == nil").unwrap());
        assert!(s
            .eval::<bool>("return GetTalentLink(1, 1, false, true) == nil")
            .unwrap());
        assert!(s.run("GetTalentLink(1)").is_err());
    }
}
