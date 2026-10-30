//! The trainer bindings the stock `Blizzard_TrainerUI` window calls, over the snapshot the app
//! pushes ([`UiScript::set_trainer`]). `SMSG_TRAINER_LIST` is flat; the client builds a collapsible
//! tree of header and service rows over it, every Lua index is 1-based into that tree, and the
//! trainer type (`ds:0xb73a08`) picks the row comparator (`0x4d8561`), the group key and the
//! header comparator (`0x4d7786`, `0x4d79eb`).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi;
use super::Model;

mod filter;
mod selection;

/// A service's state (the wire's `TrainerSpellState`): green learnable, red gated, gray known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrainerServiceCategory {
    #[default]
    Available,
    Unavailable,
    Used,
}

impl TrainerServiceCategory {
    /// `GetTrainerServiceInfo`'s type string, which the stock window colours each row by.
    fn era_str(self) -> &'static str {
        match self {
            TrainerServiceCategory::Available => "available",
            TrainerServiceCategory::Unavailable => "unavailable",
            TrainerServiceCategory::Used => "used",
        }
    }

    /// The filter-flag slot this category occupies ([`Model::trainer_filter`]).
    fn filter_slot(self) -> usize {
        match self {
            TrainerServiceCategory::Available => 0,
            TrainerServiceCategory::Unavailable => 1,
            TrainerServiceCategory::Used => 2,
        }
    }

    fn from_filter_str(s: &str) -> Option<Self> {
        match s {
            "available" => Some(TrainerServiceCategory::Available),
            "unavailable" => Some(TrainerServiceCategory::Unavailable),
            "used" => Some(TrainerServiceCategory::Used),
            _ => None,
        }
    }
}

/// A service's skill requirement (`GetTrainerServiceSkillReq`). The wire has no per-gate bit, so
/// `met` comes from the service's `category`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrainerSkillReq {
    pub name: String,
    pub rank: u32,
    pub met: bool,
}

/// A prerequisite ability (`GetTrainerServiceAbilityReq`); `met` is whether the player knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrainerAbilityReq {
    pub name: String,
    pub met: bool,
}

/// What `GameTooltip:SetTrainerService` (`0x5338b0`) describes for a row, resolved by the app; the
/// subject is independent of the icon: wrappers describe the taught spell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrainerTooltip {
    /// Direct trade recipe: spell/reagents followed by the crafted item (12340 `0x62526a`).
    Recipe { spell_id: u32, item_id: u32 },
    /// The item builder on this item id, for a taught spell with `SPELL_ATTR_IS_TRADESKILL`; 0 or
    /// an uncached template renders empty, as the reference's builder does on a cache miss.
    Item(u32),
    /// The spell builder on the taught spell, or on the wire spell when no learn slot resolved.
    Spell {
        spell_id: u32,
        /// The builder's altCaster argument, which drops the totem and reagent lines (`0x52ed43`,
        /// `0x52f393`); set when the matched slot is `SPELL_EFFECT_LEARN_PET_SPELL`.
        alt_caster: bool,
    },
}

impl Default for TrainerTooltip {
    fn default() -> Self {
        TrainerTooltip::Spell {
            spell_id: 0,
            alt_caster: false,
        }
    }
}

/// One service, resolved by the app from the wire `TrainerSpell`; its place in
/// [`TrainerState::services`] is wire order, not display order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrainerService {
    /// The wire spell id, which `CMSG_TRAINER_BUY_SPELL` names.
    pub spell_id: u32,
    /// The wire spell's name; `None` (before `Spell.dbc` loads) answers nil, shown as `UNKNOWN`.
    pub name: Option<String>,
    /// The wire spell's rank text, such as "Rank 2".
    pub subtext: Option<String>,
    pub texture: Option<String>,
    pub description: String,
    /// In copper, already reputation-discounted by the server.
    pub cost: u32,
    /// A primary profession's first rank: the stock window's `cpCost2 > 0`, which asks to confirm.
    pub prof_first_rank: bool,
    pub category: TrainerServiceCategory,
    pub level_req: u32,
    pub skill_req: Option<TrainerSkillReq>,
    pub ability_reqs: Vec<TrainerAbilityReq>,
    pub is_trade_skill: bool,
    /// The tree group, 12340's builder `0x596450`: at type 2, 3 for a passive spell, 1 for a
    /// `SKILL_STEP` (44) one, else 2, so a tradeskill trainer drops nothing; at every other type the
    /// taught spell's `SkillLine` (`0x594ae0`), where 0 (unresolved) drops the service.
    pub group_key: u32,
    /// The header text: the skill line's name, or at type 2 `TRADESKILL_SERVICE_STEP`, `_LEARN`
    /// or `_PASSIVE`.
    pub group_name: String,
    pub tooltip: TrainerTooltip,
    /// Build 12340's `GetTrainerServiceItemLink` (`0x596290`): at a tradeskill trainer, the link
    /// of the item the spell's first effect creates, once its template is cached.
    pub item_link: Option<String>,
}

/// One header and its services, built by [`UiScript::set_trainer`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrainerGroup {
    /// Also the collapse key.
    pub key: u32,
    pub name: String,
    /// Positions into [`TrainerState::services`], in display order.
    pub services: Vec<usize>,
}

/// The open trainer, pushed whole by the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrainerState {
    pub services: Vec<TrainerService>,
    /// `SMSG_TRAINER_LIST`'s trailing string.
    pub greeting: String,
    /// `SMSG_TRAINER_LIST`'s trainer type (`ds:0xb73a08`): 0 class, 1 mount, 2 tradeskill, 3 pet.
    pub trainer_type: u32,
    /// Built by [`UiScript::set_trainer`], not pushed by the app.
    pub groups: Vec<TrainerGroup>,
}

impl super::UiScript {
    /// Push the open trainer, or clear it with `None`. A push builds the tree ([`build_groups`])
    /// and keeps only the collapsed groups that still exist, so a fold survives a content update.
    pub fn set_trainer(&mut self, state: Option<TrainerState>) {
        let mut model = self.model_mut();
        match state {
            None => {
                model.trainer_selection = None;
                model.trainer_collapsed.clear();
                model.trainer = None;
            }
            Some(mut s) => {
                s.groups = build_groups(&s.services, s.trainer_type);
                let live: std::collections::HashSet<u32> = s.groups.iter().map(|g| g.key).collect();
                model.trainer_collapsed.retain(|sl| live.contains(sl));
                model.trainer = Some(s);
                if model.trainer_selection.is_none() {
                    selection::seat_first(&mut model);
                }
            }
        }
    }

    /// Reset what each `SMSG_TRAINER_LIST` resets: the collapse set and the selection. The filter
    /// is a CVar no list packet writes in 12340 ([`filter`]); the collapse reset is 1.12's
    /// (`0x4d7560`), untraced in 12340. Call it per packet, never per snapshot: the next snapshot
    /// re-seats the selection ([`selection`]).
    pub fn reset_trainer_list_state(&mut self) {
        let mut model = self.model_mut();
        model.trainer_collapsed.clear();
        model.trainer_selection = None;
    }

    /// Drain the spell ids `BuyTrainerService` queued, each already resolved from its row.
    pub fn take_trainer_buys(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().trainer_buys)
    }

    /// Whether `CloseTrainer` was called since the last drain; the close sends no packet.
    pub fn take_trainer_close(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().trainer_close)
    }
}

/// One row: a header by group index, or a service by its position in [`TrainerState::services`].
#[derive(Clone, Copy)]
enum Row {
    Header(usize),
    Service(usize),
}

/// 12340's `0x76ec80`, the row and group comparators' compare: per code point, upper-cased
/// (`0x76e7a0`) and folded to its base letter (`0x76eab0`), Æ, ß and Œ matching AE, SS and OE
/// (`0x76eb80`); the first raw difference breaks a tie.
fn collate(a: &str, b: &str) -> std::cmp::Ordering {
    fold(a).cmp(&fold(b)).then_with(|| a.cmp(b))
}

fn fold(s: &str) -> Vec<u32> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let c = c as u32;
        let upper = match c {
            0x61..=0x7a | 0xe0..=0xfe | 0x430..=0x44f => c - 0x20,
            0x153 => 0x152,
            0x451 => 0x401,
            _ => c,
        };
        match upper {
            0xc6 => out.extend([0x41, 0x45]),
            0xdf => out.extend([0x53, 0x53]),
            0x152 => out.extend([0x4f, 0x45]),
            0xc0..=0xc5 => out.push(0x41),
            0xc7 => out.push(0x43),
            0xc8..=0xcb => out.push(0x45),
            0xcc..=0xcf => out.push(0x49),
            0xd1 => out.push(0x4e),
            0xd2..=0xd6 => out.push(0x4f),
            0xd9..=0xdc => out.push(0x55),
            0x401 => out.push(0x415),
            other => out.push(other),
        }
    }
    out
}

const TRAINER_TYPE_TRADESKILL: u32 = 2;

/// The record's required-skill value `[+0x1c]`, 0 with no skill gate.
fn skill_value(s: &TrainerService) -> u32 {
    s.skill_req.as_ref().map_or(0, |r| r.rank)
}

/// Rows at every trainer but a tradeskill one (12340's `0x5947d0`): required level `[+0x14]`,
/// skill value `[+0x1c]`, name, then rank, all ascending.
fn class_order(a: &TrainerService, b: &TrainerService) -> std::cmp::Ordering {
    let name = |s: &TrainerService| s.name.clone().unwrap_or_default();
    let rank = |s: &TrainerService| s.subtext.clone().unwrap_or_default();
    a.level_req
        .cmp(&b.level_req)
        .then_with(|| skill_value(a).cmp(&skill_value(b)))
        .then_with(|| collate(&name(a), &name(b)))
        .then_with(|| collate(&rank(a), &rank(b)))
}

/// Rows at a tradeskill trainer (`0x5949a0`): required skill value ascending, then name. It has
/// no level key and no rank key.
fn tradeskill_order(a: &TrainerService, b: &TrainerService) -> std::cmp::Ordering {
    let name = |s: &TrainerService| s.name.clone().unwrap_or_default();
    skill_value(a)
        .cmp(&skill_value(b))
        .then_with(|| collate(&name(a), &name(b)))
}

/// Group the services by [`TrainerService::group_key`], dropping key 0, and sort each group's rows
/// by the type's comparator; headers sort by raw key at type 2 (`0x593ce0`), else as `0x5941b0`.
fn build_groups(services: &[TrainerService], trainer_type: u32) -> Vec<TrainerGroup> {
    let mut map: std::collections::HashMap<u32, TrainerGroup> = std::collections::HashMap::new();
    for (i, s) in services.iter().enumerate() {
        if s.group_key == 0 {
            continue;
        }
        map.entry(s.group_key)
            .or_insert_with(|| TrainerGroup {
                key: s.group_key,
                name: s.group_name.clone(),
                services: Vec::new(),
            })
            .services
            .push(i);
    }
    let mut groups: Vec<TrainerGroup> = map.into_values().collect();
    let order = match trainer_type {
        TRAINER_TYPE_TRADESKILL => tradeskill_order,
        _ => class_order,
    };
    for g in &mut groups {
        g.services
            .sort_by(|&a, &b| order(&services[a], &services[b]));
    }
    if trainer_type == TRAINER_TYPE_TRADESKILL {
        groups.sort_by_key(|g| g.key);
    } else {
        // `0x5941b0`: a group whose every service carries a profession point cost (`[+0x24]`,
        // cleared by the first service without one) sorts last, then by name.
        let all_cost = |g: &TrainerGroup| g.services.iter().all(|&i| services[i].prof_first_rank);
        groups.sort_by(|a, b| {
            all_cost(a)
                .cmp(&all_cost(b))
                .then_with(|| collate(&a.name, &b.name))
                .then_with(|| a.key.cmp(&b.key))
        });
    }
    groups
}

/// The client's whole row array and its visible count: visible rows in display order, then every
/// hidden row in a tail. The finalizer `0x4d8410` only clears a hidden record's visible flag
/// (`0x4d8546`) and sorts it behind (`0x4d857e`), and the getters bound on the total
/// (`0x4d89b0`, `ds:0xb73a10`), so a hidden row keeps its index. A group the filter empties hides
/// its header too (`0x4d8535`); a collapsed one keeps it (`0x4d853d`). The tail's order here is
/// tree order, not the client's; the stock window reaches the tail only by the selection's index.
fn rows(model: &Model) -> (Vec<Row>, usize) {
    let Some(t) = model.trainer.as_ref() else {
        return (Vec::new(), 0);
    };
    let mut shown = Vec::new();
    let mut tail = Vec::new();
    for (gi, g) in t.groups.iter().enumerate() {
        let (passes, filtered): (Vec<usize>, Vec<usize>) = g
            .services
            .iter()
            .copied()
            .partition(|&si| filter::shows(model, t.services[si].category));
        if passes.is_empty() {
            // The filter hid every service, so the header goes too.
            tail.push(Row::Header(gi));
        } else {
            shown.push(Row::Header(gi));
            // A collapsed group keeps its header on screen and parks its services behind.
            let into = if model.trainer_collapsed.contains(&g.key) {
                &mut tail
            } else {
                &mut shown
            };
            into.extend(passes.into_iter().map(Row::Service));
        }
        tail.extend(filtered.into_iter().map(Row::Service));
    }
    let visible = shown.len();
    shown.extend(tail);
    (shown, visible)
}

/// The service at a 1-based row index; `None` for a header. `GameTooltip:SetTrainerService`
/// resolves its index through this too, never through a raw `services` position.
pub(super) fn service(model: &Model, index: usize) -> Option<&TrainerService> {
    let n = index.checked_sub(1)?;
    match rows(model).0.get(n)? {
        Row::Service(si) => model.trainer.as_ref()?.services.get(*si),
        Row::Header(_) => None,
    }
}

/// `GetNumTrainerServices` (`0x4d8d90`): the visible count `ds:0xb73a18`, not the array's length.
fn num_services(model: &Model) -> usize {
    rows(model).1
}

/// `GetTrainerSelectionIndex` (12340 `0x594430` → `0x594170`): the selected record's 1-based row,
/// past the visible count while it is hidden; nil when nothing is selected or it left the list.
/// The selection is the record id (`0xc0e49c`), so no filter, collapse or update moves it.
fn selected_row(model: &Model) -> Option<usize> {
    let want = model.trainer_selection?;
    let t = model.trainer.as_ref()?;
    rows(model)
        .0
        .iter()
        .position(|r| match r {
            Row::Service(si) => t.services.get(*si).is_some_and(|s| s.spell_id == want),
            Row::Header(_) => false,
        })?
        .checked_add(1)
}

/// Queue `TRAINER_UPDATE`, which the reference fires from its mask commits, not from Lua:
/// the filter commit (12340 `0x594fe0`) writes the CVar, re-runs the rebuild and fires event
/// `0x127`, and the skill-line and expand-mask commits (1.12 `0x4d8cb0`, `0x4d8cd0`, the collapse
/// verbs') do the same. The stock window's filter click (`Blizzard_TrainerUI.lua:496`) does not
/// repaint.
fn queue_trainer_update(model: &mut Model) {
    model
        .pending_events
        .push(("TRAINER_UPDATE".to_string(), Vec::new()));
}

fn set_collapsed(model: &mut Model, id: usize, collapse: bool) {
    if id == 0 && !collapse {
        model.trainer_collapsed.clear();
        return;
    }
    let targets: Vec<u32> = if id == 0 {
        model
            .trainer
            .as_ref()
            .map(|t| t.groups.iter().map(|g| g.key).collect())
            .unwrap_or_default()
    } else {
        match rows(model).0.get(id - 1) {
            Some(Row::Header(gi)) => model
                .trainer
                .as_ref()
                .and_then(|t| t.groups.get(*gi))
                .map(|g| g.key)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    };
    for sl in targets {
        if collapse {
            model.trainer_collapsed.insert(sl);
        } else {
            model.trainer_collapsed.remove(&sl);
        }
    }
}

fn opt_str(lua: &Lua, s: Option<&String>) -> mlua::Result<Value> {
    s.map_or(Ok(Value::Nil), |s| Ok(Value::String(lua.create_string(s)?)))
}

/// Build 12340's `GetTrainerServiceItemLink(index)`: the visible service's product link, else nil.
pub(super) fn install_wrath(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetTrainerServiceItemLink",
        lua.create_function(|lua, args: mlua::MultiValue| {
            let Some(index) = args
                .front()
                .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
            else {
                return Err(mlua::Error::runtime(
                    "Usage: GetTrainerServiceItemLink(index)",
                ));
            };
            let link = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                usize::try_from(index.round() as i64)
                    .ok()
                    .and_then(|i| service(&model, i))
                    .and_then(|s| s.item_link.clone())
            };
            opt_str(lua, link.as_ref())
        })?,
    )
}

/// Register the trainer globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetNumTrainerServices",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(num_services(&model) as i64)
        })?,
    )?;

    // GetTrainerServiceInfo(index) → name, subText, serviceType, isExpanded; one nil past the end.
    // A service's subText is "" with no rank, never nil (labelled: the native push is untraced),
    // as stock `ClassTrainer_SetSelection` formats it unguarded (Blizzard_TrainerUI.lua:269).
    g.set(
        "GetTrainerServiceInfo",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(n) = index.checked_sub(1) else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            let Some(row) = rows(&model).0.get(n).copied() else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            let t = model
                .trainer
                .as_ref()
                .expect("a resolved row ⇒ an open trainer");
            match row {
                Row::Header(gi) => {
                    let g = &t.groups[gi];
                    let expanded = !model.trainer_collapsed.contains(&g.key);
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(&g.name)?),
                        Value::Nil,
                        Value::String(lua.create_string("header")?),
                        binding_abi::flag(expanded),
                    ]))
                }
                Row::Service(si) => {
                    let s = &t.services[si];
                    Ok(MultiValue::from_vec(vec![
                        opt_str(lua, s.name.as_ref())?,
                        Value::String(lua.create_string(s.subtext.as_deref().unwrap_or(""))?),
                        Value::String(lua.create_string(s.category.era_str())?),
                        Value::Nil,
                    ]))
                }
            }
        })?,
    )?;

    // GetTrainerServiceSkillLine(index): the group name, nil for a header. The reference's return
    // (`0x4d9160`) is untraced; the stock `CONFIRM_PROFESSION` dialog shows it as the profession.
    g.set(
        "GetTrainerServiceSkillLine",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(n) = index.checked_sub(1) else {
                return Ok(Value::Nil);
            };
            let Some(Row::Service(si)) = rows(&model).0.get(n).copied() else {
                return Ok(Value::Nil);
            };
            let t = model
                .trainer
                .as_ref()
                .expect("a resolved row ⇒ an open trainer");
            Ok(Value::String(
                lua.create_string(&t.services[si].group_name)?,
            ))
        })?,
    )?;

    g.set(
        "GetTrainerServiceIcon",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            opt_str(lua, service(&model, index).and_then(|s| s.texture.as_ref()))
        })?,
    )?;

    g.set(
        "GetTrainerServiceDescription",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            // `005959d0` answers nil with no description (`0x84e280`), never "".
            match service(&model, index).filter(|s| !s.description.is_empty()) {
                Some(s) => Ok(Value::String(lua.create_string(&s.description)?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;

    // GetTrainerServiceCost(index) → money, talentPointCost, professionPointCost; the talent cost
    // is always 0, the profession cost 1 for a primary profession's first rank.
    g.set(
        "GetTrainerServiceCost",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let (money, prof) = service(&model, index)
                .map(|s| (s.cost, u32::from(s.prof_first_rank)))
                .unwrap_or((0, 0));
            Ok(MultiValue::from_vec(vec![
                Value::Integer(i64::from(money)),
                Value::Integer(0),
                Value::Integer(i64::from(prof)),
            ]))
        })?,
    )?;

    g.set(
        "GetTrainerServiceLevelReq",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(service(&model, index).map_or(0, |s| s.level_req) as i64)
        })?,
    )?;

    g.set(
        "GetTrainerServiceSkillReq",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(req) = service(&model, index).and_then(|s| s.skill_req.as_ref()) else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&req.name)?),
                Value::Integer(i64::from(req.rank)),
                binding_abi::flag(req.met),
            ]))
        })?,
    )?;

    g.set(
        "GetTrainerServiceNumAbilityReq",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(service(&model, index).map_or(0, |s| s.ability_reqs.len()) as i64)
        })?,
    )?;

    g.set(
        "GetTrainerServiceAbilityReq",
        lua.create_function(|lua, (index, i): (usize, usize)| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(req) = service(&model, index)
                .and_then(|s| i.checked_sub(1).and_then(|n| s.ability_reqs.get(n)))
            else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&req.name)?),
                binding_abi::flag(req.met),
            ]))
        })?,
    )?;

    // GetTrainerServiceStepReq(index) (`0x5957d0`) → step, met: two. Labelled: the step text and
    // its skill gate read the spell's skill-step effect (44) against the player's skills, data the
    // wire does not carry, so both answer nil.
    g.set(
        "GetTrainerServiceStepReq",
        lua.create_function(|lua, index: Value| {
            if lua.coerce_number(index)?.is_none() {
                return Err(mlua::Error::runtime(
                    "Usage: GetTrainerServiceStepReq(index)",
                ));
            }
            Ok((Value::Nil, Value::Nil))
        })?,
    )?;

    g.set(
        "IsTrainerServiceTradeSkill",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(binding_abi::flag(
                service(&model, index).is_some_and(|s| s.is_trade_skill),
            ))
        })?,
    )?;

    // IsTrainerServiceLearnSpell(index) → isLearnSpell, isPetLearnSpell: any service that is not a
    // tradeskill is a learn spell; isPetLearnSpell is not built and always answers nil.
    g.set(
        "IsTrainerServiceLearnSpell",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let learn = service(&model, index).is_some_and(|s| !s.is_trade_skill);
            Ok(MultiValue::from_vec(vec![
                binding_abi::flag(learn),
                Value::Nil,
            ]))
        })?,
    )?;

    // IsTradeskillTrainer(): trainer type 2 (`0x4d8ea0`).
    g.set(
        "IsTradeskillTrainer",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(binding_abi::flag(model.trainer.as_ref().is_some_and(|t| {
                t.trainer_type == TRAINER_TYPE_TRADESKILL
            })))
        })?,
    )?;

    // IsTalentTrainer(): trainer type 1 (`0x4d8ed0`), which vmangos gives its mount trainers.
    g.set(
        "IsTalentTrainer",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(binding_abi::flag(
                model.trainer.as_ref().is_some_and(|t| t.trainer_type == 1),
            ))
        })?,
    )?;

    g.set(
        "GetTrainerGreetingText",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let text = model
                .trainer
                .as_ref()
                .map(|t| t.greeting.clone())
                .unwrap_or_default();
            lua.create_string(&text)
        })?,
    )?;

    filter::register(lua, &g)?;

    // Collapse/ExpandTrainerSkillLine(id): fold the group headed by row `id`, or all at 0. `id`
    // spans the whole row array like the getters' gate (`0x4d89b0`); their own bound is untraced.
    g.set(
        "CollapseTrainerSkillLine",
        lua.create_function(|lua, id: usize| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            set_collapsed(&mut model, id, true);
            queue_trainer_update(&mut model);
            Ok(())
        })?,
    )?;
    g.set(
        "ExpandTrainerSkillLine",
        lua.create_function(|lua, id: usize| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            set_collapsed(&mut model, id, false);
            queue_trainer_update(&mut model);
            Ok(())
        })?,
    )?;

    // SelectTrainerService(index) (12340 `0x5943a0`): the row's record id; past the end or on a
    // header it stores 0, which selects nothing and is not a fresh list's unseated `None`.
    g.set(
        "SelectTrainerService",
        lua.create_function(|lua, index: usize| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.trainer_selection = Some(service(&model, index).map_or(0, |s| s.spell_id));
            Ok(())
        })?,
    )?;

    g.set(
        "GetTrainerSelectionIndex",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(selected_row(&model).map(|r| r as i64))
        })?,
    )?;

    // BuyTrainerService(index): queue the row's spell id. `0x4d89d0` bounds through the getters'
    // gate and refuses any state byte but 0 (`0x4d89e5`), so only an available service queues.
    // Deviation: index 0 is a no-op, because one stray call would spend the purse; the reference
    // buys every visible service for any index at or below 0 (`0x4da244`, `0x4d8a70`).
    g.set(
        "BuyTrainerService",
        lua.create_function(|lua, index: usize| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let buyable = service(&model, index)
                .filter(|s| s.category == TrainerServiceCategory::Available)
                .map(|s| s.spell_id);
            if let Some(spell_id) = buyable {
                model.trainer_buys.push(spell_id);
            }
            Ok(())
        })?,
    )?;

    // CloseTrainer(): sends no packet; the app clears its state on the flag.
    g.set(
        "CloseTrainer",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.trainer_close = true;
            Ok(())
        })?,
    )?;

    Ok(())
}
#[cfg(test)]
mod tests;
