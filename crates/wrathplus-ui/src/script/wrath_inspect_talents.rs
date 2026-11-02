//! Build-12340 inspected talents: the host's reduction of `SMSG_INSPECT_TALENT` (`0x3F4`,
//! `https://github.com/artivus-labs/wrathplus/wiki/WRATH-INSPECT-TALENTS`). The host pushes the latest reply for the inspected player,
//! then fires `INSPECT_TALENT_READY`; the talent getters' inspect branches read it.

/// One talent group (spec) as the inspected player has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathInspectGroup {
    /// The class's pages with the inspected ranks; `points` is `(unspent, 0)`, and no talent is
    /// learnable.
    pub pages: super::TalentUiState,
    /// `GlyphProperties.dbc` ids per glyph socket, 0 for an empty socket.
    pub glyphs: Vec<u16>,
}

/// The inspected player's talents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathInspectTalents {
    pub guid: u64,
    /// Unspent talent points.
    pub unspent: u32,
    /// The active group, 1-based, as `GetActiveTalentGroup(true)` answers.
    pub active_group: u32,
    /// Up to two groups; empty when the realm disables talent inspection.
    pub groups: Vec<WrathInspectGroup>,
}

impl super::UiScript {
    /// Replace the inspected talents; `None` once nothing is inspected. The host fires
    /// `INSPECT_TALENT_READY` after a reply.
    pub fn set_wrath_inspect_talents(&mut self, talents: Option<WrathInspectTalents>) {
        self.model_mut().wrath_inspect_talents = talents;
    }

    /// The pushed inspected talents.
    pub fn wrath_inspect_talents(&self) -> Option<WrathInspectTalents> {
        self.model_ref().wrath_inspect_talents.clone()
    }
}

// ── The 12340 talent getters ─────────────────────────────────────────────────────────────────
//
// Build 12340's talent readers take `isInspect`, `isPet` and a 1-based `groupIndex`
// (`/tmp/azb-ui-talent-native.log`): GetNumTalentTabs `0x5c5cc0`, GetTalentTabInfo `0x5c6150`,
// GetNumTalents `0x5c5d40`, GetTalentInfo `0x5c7800`, GetTalentPrereqs `0x5c7ed0`,
// GetNumTalentGroups `0x5c5df0`, GetActiveTalentGroup `0x5c5810`, GetUnspentTalentPoints
// `0x5c6310`, GetTalentLink `0x5c7ce0`. The player has the one group the host pushes
// (`set_talents`), whose preview ranks are [`super::wrath_talent_preview`]'s; the inspect branch
// reads [`WrathInspectTalents`], whose preview ranks are its ranks; the pet branch reads the
// hunter pet's one page (`set_pet_talents`). Labelled: a second player group is not carried.

use mlua::{Lua, MultiValue, Value};

use super::talent::{TalentTabView, TalentView};
use super::wrath_talent_preview as preview;
use super::Model;

/// Which pages a getter reads: the player's (group 1 only), or an inspected group's.
fn pages(
    model: &Model,
    inspect: bool,
    pet: bool,
    group: Option<i64>,
) -> Option<&super::TalentUiState> {
    if pet {
        return (!inspect && group.unwrap_or(1) == 1).then_some(model.pet_talents.as_ref()?);
    }
    if inspect {
        let t = model.wrath_inspect_talents.as_ref()?;
        let g = group.unwrap_or(i64::from(t.active_group.max(1)));
        return usize::try_from(g - 1)
            .ok()
            .and_then(|i| t.groups.get(i))
            .map(|g| &g.pages);
    }
    (group.unwrap_or(1) == 1).then_some(&model.talents)
}

/// The player's own pages, whose preview ranks the preview state holds.
fn own(inspect: bool, pet: bool) -> bool {
    !inspect && !pet
}

fn flag(on: bool) -> Value {
    super::binding_abi::flag(on)
}

fn int(n: impl Into<i64>) -> Value {
    Value::Integer(n.into())
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// A rounded number argument, `None` when not a number.
fn number(lua: &Lua, v: &Value) -> Option<i64> {
    lua.coerce_number(v.clone())
        .ok()
        .flatten()
        .map(|n| n.round_ties_even() as i64)
}

/// The flag coercion with default 0 (`0x815500`).
fn truth(args: &MultiValue, i: usize) -> bool {
    super::binding_abi::bool_or_default(args.get(i), false)
}

fn tab_at(p: &super::TalentUiState, tab: i64) -> Option<&TalentTabView> {
    usize::try_from(tab - 1).ok().and_then(|t| p.tabs.get(t))
}

fn talent_at(p: &super::TalentUiState, tab: i64, i: i64) -> Option<&TalentView> {
    let t = usize::try_from(tab - 1).ok()?;
    let i = usize::try_from(i - 1).ok()?;
    p.talents.get(t)?.get(i)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetNumTalentTabs",
        lua.create_function(|lua, args: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(
                pages(&model, truth(&args, 0), truth(&args, 1), None).map_or(0, |p| p.tabs.len())
                    as i64,
            )
        })?,
    )?;

    // GetTalentTabInfo(tab [, isInspect [, isPet [, group]]]) → name, iconTexture, pointsSpent,
    // background, previewPointsSpent; the miss is nil, nil, 0, nil, 0.
    g.set(
        "GetTalentTabInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some(tab) = number(lua, &arg(&args, 0)) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetTalentTabInfo(tabIndex[, isInspect[, isPet[, groupIndex]]])",
                ));
            };
            let group = number(lua, &arg(&args, 3));
            let (inspect, pet) = (truth(&args, 1), truth(&args, 2));
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(t) = pages(&model, inspect, pet, group).and_then(|p| tab_at(p, tab)) else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    int(0),
                    Value::Nil,
                    int(0),
                ]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&t.name)?),
                Value::Nil,
                int(t.points_spent),
                Value::String(lua.create_string(&t.background)?),
                // The preview points past the spent ones (`0x5c6150`, the tab's `+8`).
                int(if own(inspect, pet) {
                    preview::tab_preview_spent(&model, (tab - 1) as usize)
                } else {
                    0
                }),
            ]))
        })?,
    )?;

    g.set(
        "GetNumTalents",
        lua.create_function(|lua, args: MultiValue| {
            let Some(tab) = number(lua, &arg(&args, 0)) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetNumTalents(tabIndex[, isInspect[, isPet]])",
                ));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let n = pages(&model, truth(&args, 1), truth(&args, 2), None)
                .and_then(|p| usize::try_from(tab - 1).ok().and_then(|t| p.talents.get(t)))
                .map_or(0, Vec::len);
            Ok(n as i64)
        })?,
    )?;

    // GetTalentInfo(tab, index [, isInspect [, isPet [, group]]]) → name, iconTexture, tier,
    // column, rank, maxRank, isExceptional, meetsPrereq, previewRank, meetsPreviewPrereq. An
    // inspected talent meets its prerequisites when it has a rank. The miss is nil, nil, 1, 1, 0,
    // 0, 0, nil, 0, nil.
    g.set(
        "GetTalentInfo",
        lua.create_function(|lua, args: MultiValue| {
            let (Some(tab), Some(i)) = (number(lua, &arg(&args, 0)), number(lua, &arg(&args, 1)))
            else {
                return Err(mlua::Error::runtime(
                    "Usage: GetTalentInfo(tabIndex, talentIndex[, isInspect[, isPet[, groupIndex]]])",
                ));
            };
            let inspect = truth(&args, 2);
            let pet = truth(&args, 3);
            let group = number(lua, &arg(&args, 4));
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(t) = pages(&model, inspect, pet, group).and_then(|p| talent_at(p, tab, i))
            else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    int(1),
                    int(1),
                    int(0),
                    int(0),
                    int(0),
                    Value::Nil,
                    int(0),
                    Value::Nil,
                ]));
            };
            let meets = if inspect { t.rank > 0 } else { t.meets_prereq };
            let preview_rank = if own(inspect, pet) {
                preview::preview_rank(&model, (tab - 1) as usize, (i - 1) as usize, t)
            } else {
                t.rank
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&t.name)?),
                match &t.texture {
                    Some(tex) => Value::String(lua.create_string(tex)?),
                    None => Value::Nil,
                },
                int(t.tier),
                int(t.column),
                int(t.rank),
                int(t.max_rank),
                flag(t.exceptional),
                flag(meets),
                int(preview_rank),
                flag(meets),
            ]))
        })?,
    )?;

    // GetTalentPrereqs(tab, index [, isInspect [, isPet [, group]]]) → per prerequisite: tier,
    // column, isLearnable, isPreviewLearnable.
    g.set(
        "GetTalentPrereqs",
        lua.create_function(|lua, args: MultiValue| {
            let (Some(tab), Some(i)) = (number(lua, &arg(&args, 0)), number(lua, &arg(&args, 1)))
            else {
                return Err(mlua::Error::runtime(
                    "Usage: GetTalentPrereqs(tabIndex, talentIndex[, isInspect[, isPet[, groupIndex]]])",
                ));
            };
            let group = number(lua, &arg(&args, 4));
            let (inspect, pet) = (truth(&args, 2), truth(&args, 3));
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::new();
            if let Some(t) = pages(&model, inspect, pet, group).and_then(|p| talent_at(p, tab, i)) {
                for p in &t.prereqs {
                    let preview_met = if own(inspect, pet) {
                        preview::prereq_preview_met(&model, (tab - 1) as usize, p.tier, p.column, p.rank)
                    } else {
                        p.learnable
                    };
                    out.extend([int(p.tier), int(p.column), flag(p.learnable), flag(preview_met)]);
                }
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // GetNumTalentGroups([isInspect [, isPet]]): the inspected groups; the player's one; the pet's.
    g.set(
        "GetNumTalentGroups",
        lua.create_function(|lua, args: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match (truth(&args, 0), truth(&args, 1)) {
                (true, _) => model
                    .wrath_inspect_talents
                    .as_ref()
                    .map_or(0, |t| t.groups.len()) as i64,
                (false, true) => i64::from(model.pet_talents.is_some()),
                (false, false) => 1,
            })
        })?,
    )?;

    // GetActiveTalentGroup([isInspect [, isPet]]): 1-based.
    g.set(
        "GetActiveTalentGroup",
        lua.create_function(|lua, args: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(if truth(&args, 0) {
                model
                    .wrath_inspect_talents
                    .as_ref()
                    .map_or(1, |t| i64::from(t.active_group.max(1)))
            } else {
                1
            })
        })?,
    )?;

    // GetUnspentTalentPoints([isInspect [, isPet [, group]]]).
    g.set(
        "GetUnspentTalentPoints",
        lua.create_function(|lua, args: MultiValue| {
            let group = number(lua, &arg(&args, 2));
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let inspect = truth(&args, 0);
            let pet = truth(&args, 1);
            Ok(match (inspect, pet) {
                (true, true) => 0,
                (false, true) => model
                    .pet_talents
                    .as_ref()
                    .map_or(0, |p| i64::from(p.points.0)),
                (true, false) => {
                    let Some(t) = model.wrath_inspect_talents.as_ref() else {
                        return Ok(0);
                    };
                    let g = group.unwrap_or(i64::from(t.active_group.max(1)));
                    usize::try_from(g - 1)
                        .ok()
                        .and_then(|i| t.groups.get(i))
                        .map_or(0, |g| i64::from(g.pages.points.0))
                }
                (false, false) => {
                    pages(&model, false, false, group).map_or(0, |p| i64::from(p.points.0))
                }
            })
        })?,
    )?;

    // GetTalentLink(tab, index [, isInspect [, isPet [, group [, isPreview]]]]): the link of the
    // current or the preview rank; nil off the pages.
    g.set(
        "GetTalentLink",
        lua.create_function(|lua, args: MultiValue| {
            let (Some(tab), Some(i)) = (number(lua, &arg(&args, 0)), number(lua, &arg(&args, 1)))
            else {
                return Err(mlua::Error::runtime(
                    "Usage: GetTalentLink(tabIndex, talentIndex[, isInspect[, isPet[, groupIndex[, isPreview]]]])",
                ));
            };
            let (inspect, pet) = (truth(&args, 2), truth(&args, 3));
            let group = number(lua, &arg(&args, 4));
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(t) = pages(&model, inspect, pet, group).and_then(|p| talent_at(p, tab, i))
            else {
                return Ok(Value::Nil);
            };
            let rank = if own(inspect, pet) && truth(&args, 5) {
                preview::preview_rank(&model, (tab - 1) as usize, (i - 1) as usize, t)
            } else {
                t.rank
            };
            match preview::talent_link(t, rank) {
                Some(link) => Ok(Value::String(lua.create_string(&link)?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::talent::{TalentPrereqView, TalentTabView, TalentView};
    use crate::script::{TalentUiState, UiScript};

    fn page(rank: u32, spent: u32, unspent: u32) -> TalentUiState {
        TalentUiState {
            tabs: vec![TalentTabView {
                name: "Arcane".into(),
                background: "MageArcane".into(),
                points_spent: spent,
            }],
            talents: vec![vec![TalentView {
                name: "Arcane Focus".into(),
                tier: 1,
                column: 2,
                rank,
                max_rank: 3,
                exceptional: true,
                meets_prereq: true,
                prereqs: vec![TalentPrereqView {
                    tier: 1,
                    column: 1,
                    learnable: true,
                    rank: 1,
                }],
                ..Default::default()
            }]],
            points: (unspent, 0),
        }
    }

    #[test]
    fn player_and_inspect_branches_answer_the_12340_shapes() {
        let mut s = UiScript::new().unwrap();
        s.set_talents(page(1, 5, 7));
        assert_eq!(
            s.eval::<i64>("return select('#', GetTalentTabInfo(1))")
                .unwrap(),
            5
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetTalentInfo(1, 1))")
                .unwrap(),
            10
        );
        assert_eq!(
            s.eval::<(i64, i64)>("local _,_,_,_,r,m,e,mp,pr = GetTalentInfo(1, 1) return r, pr")
                .unwrap(),
            (1, 1)
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetTalentPrereqs(1, 1))")
                .unwrap(),
            4
        );
        assert_eq!(s.eval::<i64>("return GetUnspentTalentPoints()").unwrap(), 7);
        assert_eq!(s.eval::<i64>("return GetNumTalentGroups()").unwrap(), 1);
        // Inspect: nothing yet, then two groups, the second active.
        assert_eq!(s.eval::<i64>("return GetNumTalentTabs(true)").unwrap(), 0);
        assert_eq!(
            s.eval::<(bool, i64)>("local n, _, p = GetTalentTabInfo(1, true) return n == nil, p")
                .unwrap(),
            (true, 0)
        );
        s.set_wrath_inspect_talents(Some(WrathInspectTalents {
            guid: 9,
            unspent: 2,
            active_group: 2,
            groups: vec![
                WrathInspectGroup {
                    pages: page(0, 0, 4),
                    glyphs: vec![],
                },
                WrathInspectGroup {
                    pages: page(3, 31, 2),
                    glyphs: vec![],
                },
            ],
        }));
        assert_eq!(s.eval::<i64>("return GetNumTalentGroups(true)").unwrap(), 2);
        assert_eq!(
            s.eval::<i64>("return GetActiveTalentGroup(true)").unwrap(),
            2
        );
        assert_eq!(
            s.eval::<i64>("return select(3, GetTalentTabInfo(1, true))")
                .unwrap(),
            31
        );
        assert_eq!(
            s.eval::<i64>("return select(3, GetTalentTabInfo(1, true, false, 1))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<i64>("return select(5, GetTalentInfo(1, 1, true))")
                .unwrap(),
            3
        );
        assert!(
            s.eval::<bool>("return select(8, GetTalentInfo(1, 1, true, false, 1)) == nil")
                .unwrap(),
            "an unranked inspected talent does not meet its prerequisites"
        );
        assert_eq!(
            s.eval::<i64>("return GetUnspentTalentPoints(true)")
                .unwrap(),
            2
        );
        assert_eq!(
            s.eval::<i64>("return GetUnspentTalentPoints(true, false, 1)")
                .unwrap(),
            4
        );
        // Pets carry nothing.
        assert_eq!(
            s.eval::<i64>("return GetNumTalentTabs(false, true)")
                .unwrap(),
            0
        );
        assert!(s.run("GetTalentInfo(1)").is_err());
    }
}
