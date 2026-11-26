//! `GameTooltip:SetHyperlink`'s quest and achievement links, as build 12340 builds them (dispatch
//! `0x62dae0`): the quest core `0x622960` and the achievement core `0x627220`. Each clears the
//! tooltip, writes its lines, runs its content script (`OnTooltipSetQuest` `+0x504`,
//! `OnTooltipSetAchievement` `+0x50c`) and shows.
//!
//! The colours are the 12340 tooltip table: gold `0xad2d2c` ffffd200, white `0xad2d30`, green
//! `0xad2d40` ff00ff00, and the criteria pair gray `0xad2d54` ff808080 / done `0xad2d58` ff40c040.
//! The blank separator is the one-space string `0x9e1050`.

use std::collections::HashMap;

use mlua::{Lua, Table};

use super::object::frame_handle_of;
use super::tooltip::{
    append_line, clear_content, finish_content, fire_cleared, show_or_hide_empty,
};
use super::Model;
use crate::widget::FrameHandle;

const GOLD: [f32; 4] = [1.0, 210.0 / 255.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const CRITERION: [f32; 4] = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0];
const CRITERION_DONE: [f32; 4] = [64.0 / 255.0, 192.0 / 255.0, 64.0 / 255.0, 1.0];
const BLANK: &str = " ";

/// One quest's link tooltip content, from the host's quest template cache
/// (`SMSG_QUEST_QUERY_RESPONSE`): the title, the objectives text, and the requirement rows.
///
/// Labelled: 12340 composes each requirement row itself from the template and the player's
/// progress (`0x5e1300` counts them, `0x5e1460` writes each); the host supplies the finished text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestLinkView {
    pub title: String,
    pub objectives: String,
    pub requirements: Vec<String>,
}

/// The quest templates the host has pushed, the ids asked for and not yet answered, and which
/// tooltip waits on which quest (the core stores it at `+0x368`).
#[derive(Default)]
struct State {
    views: HashMap<u32, QuestLinkView>,
    requests: Vec<u32>,
    pending: HashMap<FrameHandle, u32>,
    talents: HashMap<u32, TalentLinkView>,
    glyphs: GlyphLinkCatalog,
}

/// One Talent.dbc row as a `talent:` link reads it (`0x65c290` returns the row).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TalentLinkView {
    /// The rank spells in rank order, up to the last non-zero one.
    pub ranks: Vec<u32>,
    /// Flags bit 0: the exceptional talents 12340 hands to the spell builder instead.
    pub exceptional: bool,
}

/// What a `glyph:<slot>:<glyph>` link reads: GlyphProperties.dbc's spell per glyph id, and
/// GlyphSlot.dbc's `(type, tooltip order)` per slot id, type 0 major and 1 minor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphLinkCatalog {
    pub glyph_spells: HashMap<u32, u32>,
    pub slots: HashMap<u32, (u32, u32)>,
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    Ok(())
}

impl super::UiScript {
    /// Install one quest's link content. A tooltip still waiting on this quest is rebuilt, as the
    /// cache callback `0x61de20` re-runs the core for the id it stored.
    pub fn set_quest_link_view(&mut self, quest_id: u32, view: QuestLinkView) {
        let waiting: Vec<FrameHandle> = {
            let Some(mut state) = self.lua.app_data_mut::<State>() else {
                return;
            };
            state.views.insert(quest_id, view);
            state.requests.retain(|&q| q != quest_id);
            state
                .pending
                .iter()
                .filter(|(_, &q)| q == quest_id)
                .map(|(&h, _)| h)
                .collect()
        };
        for h in waiting {
            let wrapper = super::object::frame_wrapper(&self.lua, {
                let mut model = self.model_mut();
                model.frame_id(h)
            });
            if let Ok(wrapper) = wrapper {
                if let Err(e) = set_quest(&self.lua, &wrapper, quest_id) {
                    self.model_mut().record_script_error(e.to_string());
                }
            }
        }
    }

    /// Install Talent.dbc for `talent:` links, keyed by talent id; links name any class's talents.
    pub fn set_talent_link_catalog(&mut self, talents: HashMap<u32, TalentLinkView>) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            state.talents = talents;
        }
    }

    /// Install GlyphProperties.dbc and GlyphSlot.dbc for `glyph:` links.
    pub fn set_glyph_link_catalog(&mut self, catalog: GlyphLinkCatalog) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            state.glyphs = catalog;
        }
    }

    /// Drain the quest ids a link tooltip asked for: the host sends `CMSG_QUEST_QUERY` for each it
    /// has not cached and answers with [`Self::set_quest_link_view`].
    pub fn take_quest_link_requests(&mut self) -> Vec<u32> {
        self.lua
            .app_data_mut::<State>()
            .map(|mut s| std::mem::take(&mut s.requests))
            .unwrap_or_default()
    }
}

/// The payload of a `<kind>:` link, bare (`quest:123:80`) or escaped (`|Hquest:123:80|h[…]|h`),
/// up to the closing `|h`.
pub(super) fn link_payload<'a>(link: &'a str, kind: &str) -> Option<&'a str> {
    let at = if link.starts_with(kind) {
        kind.len()
    } else {
        link.find(&format!("|H{kind}"))? + 2 + kind.len()
    };
    let tail = &link[at..];
    Some(&tail[..tail.find("|h").unwrap_or(tail.len())])
}

/// The quest core `0x622960`: title (gold), `QUEST_TOOLTIP_ACTIVE` (green) while the quest is in
/// the log, a blank, the objectives (white, wrapped), then `QUEST_TOOLTIP_REQUIREMENTS` (gold) and
/// a ` - ` row per requirement (white) after a blank. An uncached quest is asked for and leaves
/// the tooltip empty until its answer rebuilds it.
pub(super) fn set_quest(lua: &Lua, this: &Table, quest_id: u32) -> mlua::Result<()> {
    let h = frame_handle_of(lua, this)?;
    {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        clear_content(&mut model, h);
    }
    fire_cleared(lua, h);
    let view = {
        let mut state = lua.app_data_mut::<State>().expect("link tooltip state");
        state.pending.insert(h, quest_id);
        let view = state.views.get(&quest_id).cloned();
        if view.is_none() && !state.requests.contains(&quest_id) {
            state.requests.push(quest_id);
        }
        view
    };
    let Some(view) = view else {
        show_or_hide_empty(lua, h);
        return Ok(());
    };
    let in_log = lua
        .app_data_ref::<Model>()
        .expect("model app_data")
        .quest_log
        .entries
        .iter()
        .any(|e| !e.is_header && e.quest_id == quest_id);
    let global = |name: &str| -> mlua::Result<String> {
        Ok(lua
            .globals()
            .get::<Option<String>>(name)?
            .unwrap_or_default())
    };
    append_line(lua, this, (view.title.clone(), GOLD), None, false)?;
    if in_log {
        append_line(
            lua,
            this,
            (global("QUEST_TOOLTIP_ACTIVE")?, GREEN),
            None,
            false,
        )?;
    }
    append_line(lua, this, (BLANK.to_string(), GOLD), None, false)?;
    append_line(lua, this, (view.objectives.clone(), WHITE), None, true)?;
    if !view.requirements.is_empty() {
        append_line(lua, this, (BLANK.to_string(), GOLD), None, false)?;
        append_line(
            lua,
            this,
            (global("QUEST_TOOLTIP_REQUIREMENTS")?, GOLD),
            None,
            false,
        )?;
        for row in &view.requirements {
            append_line(lua, this, (format!(" - {row}"), WHITE), None, false)?;
        }
    }
    finish_content(lua, h, "OnTooltipSetQuest");
    Ok(())
}

/// `achievement:<id>:<guid hex>:<completed>:<month>:<day>:<year>:<c1>:<c2>:<c3>:<c4>`, as
/// `0x62dae0` reads it; a missing field is 0.
struct AchievementLink {
    id: u32,
    guid: u64,
    completed: bool,
    month: i64,
    day: i64,
    year: i64,
    criteria: [u32; 4],
}

fn parse_achievement(payload: &str) -> Option<AchievementLink> {
    let mut fields = payload.split(':');
    let id = fields.next()?.trim().parse().ok()?;
    let guid = u64::from_str_radix(fields.next().unwrap_or("0").trim(), 16).unwrap_or(0);
    let mut num = || {
        fields
            .next()
            .and_then(|f| f.trim().parse::<i64>().ok())
            .unwrap_or(0)
    };
    let completed = num() != 0;
    let (month, day, year) = (num(), num(), num());
    let criteria = [num() as u32, num() as u32, num() as u32, num() as u32];
    Some(AchievementLink {
        id,
        guid,
        completed,
        month,
        day,
        year,
        criteria,
    })
}

/// A player's name as the client holds it: our own record, else a unit we hold by guid.
pub(super) fn known_name(model: &Model, guid: u64) -> Option<String> {
    if guid != 0 && guid == model.unit_guids.player {
        Some(model.player_record.name.clone()).filter(|n| !n.is_empty())
    } else {
        model.units_by_guid.get(&guid).and_then(|u| u.name.clone())
    }
}

/// The achievement core `0x627220`: the title (white), a blank, `ACHIEVEMENT_TOOLTIP_COMPLETE` or
/// `_IN_PROGRESS` with the linker (green), a blank, the description (white, wrapped), a blank, then
/// the visible criteria two to a line, each gray or, when its bit in the link's masks is set,
/// green. An id with no Achievement.dbc row writes nothing.
///
/// Labelled: the linker's name comes from the units the VM holds (the player, then any unit with
/// that guid), else `UNKNOWN`; 12340 also asks its name cache, and withholds an in-progress
/// tooltip until the name arrives. The criteria list is drawn whenever visible criteria exist.
pub(super) fn set_achievement(lua: &Lua, this: &Table, payload: &str) -> mlua::Result<()> {
    let h = frame_handle_of(lua, this)?;
    let Some(link) = parse_achievement(payload) else {
        return Ok(());
    };
    let found = {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        model.wrath_achievements.as_ref().and_then(|meta| {
            let row = meta.achievements.get(&link.id)?.clone();
            let owner = meta.criteria_owner(&row).unwrap_or(row.id);
            let criteria: Vec<String> = meta
                .linked_criteria
                .get(&owner)
                .into_iter()
                .flatten()
                .filter_map(|c| meta.criteria.get(c))
                .filter(|c| c.flags & 2 == 0)
                .map(|c| c.description.clone())
                .collect();
            Some((row, criteria, known_name(&model, link.guid)))
        })
    };
    let Some((row, criteria, name)) = found else {
        return Ok(());
    };
    {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        clear_content(&mut model, h);
    }
    fire_cleared(lua, h);
    let name = match name {
        Some(name) => name,
        None => lua
            .globals()
            .get::<Option<String>>("UNKNOWN")?
            .unwrap_or_default(),
    };
    let status: String = if link.completed {
        lua.load("return format(ACHIEVEMENT_TOOLTIP_COMPLETE or '', ...)")
            .call((name, link.month, link.day, link.year))?
    } else {
        lua.load("return format(ACHIEVEMENT_TOOLTIP_IN_PROGRESS or '', ...)")
            .call(name)?
    };
    append_line(lua, this, (row.name.clone(), WHITE), None, false)?;
    append_line(lua, this, (BLANK.to_string(), GOLD), None, false)?;
    append_line(lua, this, (status, GREEN), None, false)?;
    append_line(lua, this, (BLANK.to_string(), GOLD), None, false)?;
    append_line(lua, this, (row.description.clone(), WHITE), None, true)?;
    append_line(lua, this, (BLANK.to_string(), GOLD), None, false)?;
    let done = |i: usize| {
        link.criteria
            .get(i / 32)
            .is_some_and(|m| m & (1 << (i % 32)) != 0)
    };
    let colour = |i: usize| if done(i) { CRITERION_DONE } else { CRITERION };
    for (pair, chunk) in criteria.chunks(2).enumerate() {
        let left = (chunk[0].clone(), colour(pair * 2));
        let right = chunk.get(1).map(|r| (r.clone(), colour(pair * 2 + 1)));
        append_line(lua, this, left, right, false)?;
    }
    finish_content(lua, h, "OnTooltipSetAchievement");
    Ok(())
}

/// `talent:<id>:<rank>` (`0x62dae0` → `0x65c290`, then `0x626e20(row, 0, 0, 1, rank)`): the rank
/// in range is shown, else none; the shown rank's spell (the next one's when none) names the
/// tooltip, then `TOOLTIP_TALENT_RANK` as rank+1 of max+1, its description, and the next rank's
/// under `TOOLTIP_TALENT_NEXT_RANK`, through the shared talent renderer. A talent with no row
/// writes nothing.
///
/// Labelled: the exceptional-talent spell builder and the requirement lines (`0x6224f0`,
/// `0x622800`) are not reproduced; the shared renderer draws the talent shape for every row.
pub(super) fn set_talent(lua: &Lua, this: &Table, payload: &str) -> mlua::Result<()> {
    let mut fields = payload.split(':');
    let Some(id) = fields.next().and_then(|f| f.trim().parse::<u32>().ok()) else {
        return Ok(());
    };
    let rank = fields
        .next()
        .and_then(|f| f.trim().parse::<i64>().ok())
        .unwrap_or(-1);
    let row = lua
        .app_data_ref::<State>()
        .and_then(|s| s.talents.get(&id).cloned());
    let Some(row) = row else {
        return Ok(());
    };
    let ranks: Vec<u32> = {
        let last = row.ranks.iter().rposition(|&s| s != 0);
        last.map_or_else(Vec::new, |n| row.ranks[..=n].to_vec())
    };
    if ranks.is_empty() {
        return Ok(());
    }
    let top = ranks.len() as i64 - 1;
    let shown = (0..=top).contains(&rank).then_some(rank as usize);
    let current = shown.map(|r| ranks[r]);
    let next = ranks.get(shown.map_or(0, |r| r + 1)).copied();
    let Some(display) = current.or(next) else {
        return Ok(());
    };
    let next_spell = current.and(next).unwrap_or(0);
    let next_desc = (next_spell != 0)
        .then(|| {
            lua.app_data_ref::<Model>()
                .expect("model app_data")
                .spell_tooltips
                .get(&next_spell)
                .map(|v| v.description.clone())
        })
        .flatten();
    let rank_line = crate::strings::global(lua, "TOOLTIP_TALENT_RANK").map(|tmpl| {
        crate::strings::fill(
            &tmpl,
            &[
                crate::strings::Arg::D(shown.map_or(0, |r| r as i64 + 1)),
                crate::strings::Arg::D(top + 1),
            ],
        )
    });
    super::tooltip_spell::set_spell_with_talent(
        lua,
        this,
        display,
        super::tooltip_spell::TalentLines {
            exceptional: row.exceptional,
            rank_line,
            reqs: Vec::new(),
            next_spell,
            next_desc,
            learn: false,
        },
    )
}

/// `glyph:<slot>:<glyph>` (`0x62dae0` → `0x622ba0(slot, glyph, 1, 1, 0)`): the glyph's spell name
/// (white), else `GLYPH_INACTIVE` (gray); `MAJOR_GLYPH` or `MINOR_GLYPH` (glyph blue); then the
/// spell's description (gold, wrapped), or `GLYPH_EMPTY_DESC` for no glyph. A link draws no
/// remove line, and 12340 runs no content script here.
pub(super) fn set_glyph(lua: &Lua, this: &Table, payload: &str) -> mlua::Result<()> {
    let mut fields = payload
        .split(':')
        .map(|f| f.trim().parse::<u32>().unwrap_or(0));
    let (slot, glyph) = (fields.next().unwrap_or(0), fields.next().unwrap_or(0));
    let (spell, slot_row) = {
        let state = lua.app_data_ref::<State>().expect("link tooltip state");
        (
            state.glyphs.glyph_spells.get(&glyph).copied(),
            state.glyphs.slots.get(&slot).copied(),
        )
    };
    let h = frame_handle_of(lua, this)?;
    {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        clear_content(&mut model, h);
    }
    fire_cleared(lua, h);
    let view = spell.and_then(|s| super::tooltip_spell::spell_view_of(lua, s, false));
    let text = |key: &str| {
        lua.globals()
            .get::<Option<String>>(key)
            .map(Option::unwrap_or_default)
    };
    match &view {
        Some(v) => append_line(lua, this, (v.name.clone(), WHITE), None, false)?,
        None => append_line(lua, this, (text("GLYPH_INACTIVE")?, CRITERION), None, false)?,
    }
    let minor = slot_row.is_some_and(|(kind, _)| kind != 0);
    let kind = if minor { "MINOR_GLYPH" } else { "MAJOR_GLYPH" };
    append_line(
        lua,
        this,
        (text(kind)?, super::wrath_glyphs::GLYPH_BLUE),
        None,
        false,
    )?;
    match &view {
        Some(v) if !v.description.is_empty() => {
            append_line(lua, this, (v.description.clone(), GOLD), None, true)?
        }
        Some(_) => {}
        None => append_line(lua, this, (text("GLYPH_EMPTY_DESC")?, GOLD), None, true)?,
    }
    show_or_hide_empty(lua, h);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::QuestLinkView;
    use crate::script::{
        PlayerRecord, QuestLogEntryView, QuestLogState, UiScript, UnitGuids, WrathAchievement,
        WrathAchievementCriterion, WrathAchievementMetadata,
    };

    fn tooltip(s: &UiScript) {
        s.run(
            r#"
            local a = CreateFrame("Button", "LinkOwner"); a:SetPoint("CENTER", 0, 0)
            a:SetWidth(10); a:SetHeight(10)
            local tt = CreateFrame("GameTooltip", "TT")
            tt:SetOwner(a, "ANCHOR_RIGHT")
            QUEST_TOOLTIP_ACTIVE = "You are on this quest"
            QUEST_TOOLTIP_REQUIREMENTS = "Requirements:"
            ACHIEVEMENT_TOOLTIP_COMPLETE = "Achievement earned by %1$s on %2$d/%3$02d/20%4$02d"
            ACHIEVEMENT_TOOLTIP_IN_PROGRESS = "Achievement in progress by %s"
            UNKNOWN = "Unknown"
            RUNS = 0
            "#,
        )
        .unwrap();
    }

    fn lines(s: &UiScript) -> Vec<String> {
        s.eval::<Vec<String>>(
            "local t = {} for i = 1, TT:NumLines() do t[i] = _G['TTTextLeft' .. i]:GetText() or '' end return t",
        )
        .unwrap()
    }

    /// An uncached quest asks once and stays empty; the answer rebuilds the waiting tooltip in the
    /// 12340 shape and runs `OnTooltipSetQuest`.
    #[test]
    fn a_quest_link_asks_then_builds_the_native_lines() {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(800.0, 600.0);
        tooltip(&s);
        s.run(
            r#"
            TT:SetScript("OnTooltipSetQuest", function(self) RUNS = RUNS + 1 end)
            TT:SetHyperlink("|cffffff00|Hquest:176:10|h[Wanted: Hogger]|h|r")
            "#,
        )
        .unwrap();
        assert_eq!(s.take_quest_link_requests(), vec![176]);
        assert!(lines(&s).is_empty());
        assert_eq!(s.eval::<i64>("return RUNS").unwrap(), 0);
        s.set_quest_log(QuestLogState {
            entries: vec![QuestLogEntryView {
                quest_id: 176,
                title: "Wanted: Hogger".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        s.set_quest_link_view(
            176,
            QuestLinkView {
                title: "Wanted: Hogger".into(),
                objectives: "Kill Hogger.".into(),
                requirements: vec!["Huge Gnoll Claw".into()],
            },
        );
        assert_eq!(
            lines(&s),
            [
                "Wanted: Hogger",
                "You are on this quest",
                " ",
                "Kill Hogger.",
                " ",
                "Requirements:",
                " - Huge Gnoll Claw"
            ]
        );
        assert_eq!(s.eval::<i64>("return RUNS").unwrap(), 1);
        assert!(s.take_quest_link_requests().is_empty());
    }

    fn metadata() -> WrathAchievementMetadata {
        let mut meta = WrathAchievementMetadata::default();
        meta.achievements.insert(
            6,
            WrathAchievement {
                id: 6,
                name: "Level 10".into(),
                description: "Reach level 10.".into(),
                ..Default::default()
            },
        );
        for (id, text, flags) in [
            (1, "First", 0),
            (2, "Hidden", 2),
            (3, "Second", 0),
            (4, "Third", 0),
        ] {
            meta.criteria.insert(
                id,
                WrathAchievementCriterion {
                    id,
                    achievement_id: 6,
                    criteria_type: 0,
                    asset_id: 0,
                    required_quantity: 1,
                    description: text.into(),
                    flags,
                    ..Default::default()
                },
            );
        }
        meta.linked_criteria.insert(6, vec![1, 2, 3, 4]);
        meta
    }

    /// A completed achievement link: title, the dated earned line naming the linker, the
    /// description, then the visible criteria two to a line, coloured by the link's mask, and
    /// `OnTooltipSetAchievement`.
    #[test]
    fn an_achievement_link_builds_the_native_lines() {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(800.0, 600.0);
        s.set_wrath_achievement_metadata(metadata());
        s.set_unit_guids(&UnitGuids {
            player: 0x42,
            ..Default::default()
        });
        s.set_player_record(PlayerRecord {
            name: "Benchone".into(),
            ..Default::default()
        });
        tooltip(&s);
        s.run(
            r#"
            TT:SetScript("OnTooltipSetAchievement", function(self) RUNS = RUNS + 1 end)
            TT:SetHyperlink("|cffffff00|Hachievement:6:0000000000000042:1:4:11:9:5:0:0:0|h[Level 10]|h|r")
            "#,
        )
        .unwrap();
        assert_eq!(
            lines(&s),
            [
                "Level 10",
                " ",
                "Achievement earned by Benchone on 4/11/2009",
                " ",
                "Reach level 10.",
                " ",
                "First",
                "Third"
            ]
        );
        assert_eq!(
            s.eval::<String>("return TTTextRight7:GetText()").unwrap(),
            "Second"
        );
        // Mask 5: criteria 0 and 2 done (First, Third); Second is not.
        let colour = |region: &str| {
            let (r, g, b) = s
                .eval::<(f64, f64, f64)>(&format!(
                    "local r, g, b = {region}:GetTextColor() return r, g, b"
                ))
                .unwrap();
            [r, g, b].map(|c| (c * 255.0).round() as u8)
        };
        assert_eq!(colour("TTTextLeft7"), [64, 192, 64]);
        assert_eq!(colour("TTTextRight7"), [128, 128, 128]);
        assert_eq!(colour("TTTextLeft8"), [64, 192, 64]);
        assert_eq!(s.eval::<i64>("return RUNS").unwrap(), 1);
        // An unknown id writes nothing.
        s.run("TT:SetHyperlink('achievement:999:0:0:0:0:0:0:0:0:0')")
            .unwrap();
        assert_eq!(s.eval::<i64>("return RUNS").unwrap(), 1);
    }

    /// `talent:<id>:<rank>` draws the shown rank's spell and the next; `glyph:<slot>:<glyph>` the
    /// glyph's spell with its type; `unit:<guid>` a held unit; an unknown link type raises.
    #[test]
    fn talent_glyph_unit_and_unknown_links() {
        use crate::script::{SpellTooltipView, UnitState};
        use std::collections::HashMap;
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(800.0, 600.0);
        for (id, name, desc) in [
            (10, "Improved Fireball", "Rank one."),
            (11, "Improved Fireball", "Rank two."),
            (20, "Glyph of Fireball", "Faster fireballs."),
        ] {
            s.set_spell_tooltip(
                id,
                SpellTooltipView {
                    name: name.into(),
                    description: desc.into(),
                    ..Default::default()
                },
            );
        }
        s.set_talent_link_catalog(HashMap::from([(
            31,
            super::TalentLinkView {
                ranks: vec![10, 11, 0],
                exceptional: false,
            },
        )]));
        s.set_glyph_link_catalog(super::GlyphLinkCatalog {
            glyph_spells: HashMap::from([(5, 20)]),
            slots: HashMap::from([(21, (0, 1))]),
        });
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                guid: 0xF130_0000_4500_0001,
                name: Some("Timber Wolf".into()),
                ..Default::default()
            }),
        );
        tooltip(&s);
        s.run(
            r#"
            TOOLTIP_TALENT_RANK = "Rank %d/%d"
            TOOLTIP_TALENT_NEXT_RANK = "Next rank:"
            MAJOR_GLYPH = "Major Glyph"
            TT:SetScript("OnTooltipSetSpell", function() RUNS = RUNS + 1 end)
            TT:SetHyperlink("|cff4e96f7|Htalent:31:0|h[Improved Fireball]|h|r")
            "#,
        )
        .unwrap();
        let talent = lines(&s);
        assert_eq!(
            talent.first().map(String::as_str),
            Some("Improved Fireball")
        );
        assert!(talent.iter().any(|l| l == "Rank 1/2"), "{talent:?}");
        assert_eq!(s.eval::<i64>("return RUNS").unwrap(), 1);
        s.run(r#"TT:SetHyperlink("|cff66bbff|Hglyph:21:5|h[Glyph of Fireball]|h|r")"#)
            .unwrap();
        assert_eq!(
            lines(&s),
            ["Glyph of Fireball", "Major Glyph", "Faster fireballs."]
        );
        assert_eq!(
            s.eval::<i64>("return RUNS").unwrap(),
            1,
            "the glyph core runs no script"
        );
        s.run(r#"TT:SetHyperlink("unit:F130000045000001")"#)
            .unwrap();
        assert_eq!(lines(&s).first().map(String::as_str), Some("Timber Wolf"));
        let err = s
            .run(r#"TT:SetHyperlink("bogus:1")"#)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("TT:SetHyperlink(): Unknown link type"),
            "{err}"
        );
    }
}
