//! The crafting book feed: the client-built TradeSkill window around `wrathplus_ui::script`'s
//! `tradeskill` module. There is no wire: a profession opener (`SPELL_EFFECT_TRADE_SKILL` in
//! `Effect[0]`) never reaches the send, as `Spell_C::TryCast` (`0x6e4b60`) opens the window
//! client-side, and [`TradeSkillOpens`] is that intercept here.
//!
//! The book (`0x4fca20`) lists each known spell with `SPELL_ATTR_IS_TRADESKILL` (`0x20`, which
//! also hides it from the spellbook) whose `SkillLineAbility` row joins the open line, grouped by
//! the created item's `(ItemClass, ItemSubClass)` named from `ItemSubClass.dbc`. A spell-focus
//! tool never reads red: the reference's `hasTool` for a focus is the literal 1.0 (`0x4ffa8b`),
//! as the client holds no focus id or radius, and the server's refusal is the only feedback.

use std::time::Instant;

use bevy::prelude::*;

use wrathplus_formats::{SpellFocusCatalog, SPELL_ATTR_IS_TRADESKILL, SPELL_EFFECT_CREATE_ITEM};
use wrathplus_protocol::types::PLAYER_SKILL_SLOTS;
use wrathplus_protocol::{SessionEvent, SessionEventKind};
use wrathplus_ui::script::{
    TradeSkillDifficulty, TradeSkillReagent, TradeSkillRecipe, TradeSkillState, UiScript,
};

use crate::app_core::schedule::UiInput;
use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::PlayerActions;
use crate::app_core::state::SkillLines;
use crate::app_core::state::Spells;
use crate::creature_anim::{CastEvent, CastEventKind};
use crate::entities::ItemDisplays;
use crate::inventory::count_of;
use crate::inventory::InventoryScope;
use crate::items::Items;
use crate::net::{NetCommands, NetHandlerApp, Objects};
use crate::spell::{cast_target, CastCommit, CastLadder};
use crate::ui_items::item_icon;
use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};

pub(crate) use crate::app_core::interaction::TradeSkillOpens;
/// A recipe's "Requires:" tools: focus, totems and totem categories.
mod tools;
pub(crate) use tools::TotemCategories;
/// The open book's skill line. The Lua close and the session end clear it: a logout replaces the
/// VM without running the old `OnHide`, so the close alone never comes.
#[derive(Resource, Default)]
pub(crate) struct TradeSkillOpen {
    pub(crate) line: Option<u32>,
    /// The opener spell (`0xc235a0`), our own or the link's: `GetTradeSkillListLink` builds off it.
    pub(crate) spell: u32,
    /// A clicked trade link's book (`0xc235e4`), or `None` for our own.
    pub(crate) link: Option<LinkedBook>,
}

/// A linked window (`0x5de120`): the linker, their rank off the link (`0xc235e8`/`0xc235ec`), and
/// the spells the link's mask marks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinkedBook {
    pub(crate) guid: u64,
    pub(crate) rank: u32,
    pub(crate) max_rank: u32,
    pub(crate) spells: Vec<u32>,
}

impl TradeSkillOpen {
    fn close(&mut self) {
        self.line = None;
        self.spell = 0;
        self.link = None;
    }

    /// Open a clicked trade link's book (`0x5de120`): a line that links, and a mask of the line's
    /// length, else nothing opens. Labelled: the reference resolves the opener's line through the
    /// linker's race and class (`0x812410` over their name-cache row); here the opener's first
    /// `SkillLineAbility` row.
    fn open_link(&mut self, catalog: &wrathplus_formats::SkillLineCatalog, raw: &str) -> bool {
        let Some(link) = wrathplus_formats::parse_trade_link(raw) else {
            return false;
        };
        let Some(line) = catalog.spell_to_line(link.spell_id) else {
            return false;
        };
        if !catalog.line(line).is_some_and(|l| l.can_link) {
            return false;
        }
        let Some(spells) =
            wrathplus_formats::decode_trade_mask(catalog.line_abilities(line), &link.mask)
        else {
            return false;
        };
        self.line = Some(line);
        self.spell = link.spell_id;
        self.link = Some(LinkedBook {
            guid: link.guid,
            rank: link.rank,
            max_rank: link.max_rank,
            spells,
        });
        true
    }
}

/// `GetTradeSkillListLink` (`0x5db060`): our own link for `spell`'s line, when the line links and
/// we hold the skill. The rank is the raw `PLAYER_SKILL_INFO` pair; labelled: the reference's
/// `0x51a250`/`0x594710` may add the skill bonus.
pub(crate) fn own_trade_link(
    catalog: &wrathplus_formats::SkillLineCatalog,
    spells: &Spells,
    known: &std::collections::BTreeSet<u32>,
    store: &ObjectStore,
    guid: u64,
    spell: u32,
) -> Option<String> {
    let def = spells.catalog.get(spell)?;
    if def.effects[0] != wrathplus_formats::SPELL_EFFECT_TRADE_SKILL {
        return None;
    }
    let line = catalog.spell_to_line(spell)?;
    if !catalog.line(line)?.can_link {
        return None;
    }
    let (rank, max_rank) = skill_rank(store, line);
    if max_rank == 0 {
        return None;
    }
    let race = store.0.unit_race().unwrap_or(0);
    let class = store.0.unit_class().unwrap_or(0);
    let mask = wrathplus_formats::encode_trade_mask(
        catalog.line_abilities(line),
        |s| known.contains(&s),
        race,
        class,
    );
    let link = wrathplus_formats::TradeLink {
        spell_id: spell,
        rank,
        max_rank,
        guid,
        mask,
    };
    Some(wrathplus_formats::trade_link(&link, &def.name))
}

/// The Create All repeat (`DoTradeSkill` `0x500280`): latch the count, cast once, and re-cast on
/// each of our own `SMSG_SPELL_GO`s for the spell until dry; a failure, a close or the session end
/// stops it.
#[derive(Resource, Default)]
pub(crate) struct TradeSkillRepeat {
    pub(crate) spell_id: u32,
    pub(crate) remaining: u32,
}

impl TradeSkillRepeat {
    fn clear(&mut self) {
        self.spell_id = 0;
        self.remaining = 0;
    }
}

/// `SpellFocusObject.dbc`, the names on a "Requires: Anvil" line.
#[derive(Resource)]
pub(crate) struct SpellFocus {
    pub(crate) catalog: SpellFocusCatalog,
}

pub struct UiTradeSkillPlugin;

impl Plugin for UiTradeSkillPlugin {
    fn build(&self, app: &mut App) {
        app.net_handler(SessionEventKind::Disconnected, on_session_end);
        app.init_resource::<TradeSkillOpens>()
            .init_resource::<TradeSkillOpen>()
            .init_resource::<TradeSkillRepeat>()
            .add_systems(
                Startup,
                (load_spell_focus, tools::load_totem_categories).after(AssetSet::Open),
            )
            .add_systems(
                Update,
                (
                    // Opens before the feed, so an opener shows the window this frame; the
                    // drain after the input pass, so a Create click casts this frame.
                    open_trade_skill
                        .after(crate::spell::auto_ranged::tick)
                        .before(feed_trade_skill),
                    feed_trade_skill.in_set(UnitFeed),
                    drain_trade_skill.after(UiInput),
                ),
            );
    }
}

/// The book, its repeat and any queued opener end with the session.
fn on_session_end(
    In(_): In<SessionEvent>,
    mut opens: ResMut<TradeSkillOpens>,
    mut open: ResMut<TradeSkillOpen>,
    mut repeat: ResMut<TradeSkillRepeat>,
) {
    opens.0.clear();
    open.close();
    repeat.clear();
}

fn load_spell_focus(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_spell_focus_catalog(&mut chain)
    };
    match loaded {
        Ok(catalog) => {
            debug!("ui_tradeskill: {} spell-focus name(s)", catalog.len());
            commands.insert_resource(SpellFocus { catalog });
        }
        Err(e) => warn!(
            "ui_tradeskill: SpellFocusObject.dbc failed to load — the Requires line drops the \
             focus name: {e:#}"
        ),
    }
}

/// Open the book for each intercepted opener: its line is the opener's `SkillLineAbility` row
/// (3908 Tailoring is line 197), the client's `SkillLineRecIndex_Find` (`0x6de040`).
fn open_trade_skill(
    mut opens: ResMut<TradeSkillOpens>,
    mut open: ResMut<TradeSkillOpen>,
    skill_lines: Option<Res<SkillLines>>,
    mut repeat: ResMut<TradeSkillRepeat>,
) {
    for spell_id in opens.0.drain(..) {
        let Some(skill_lines) = skill_lines.as_deref() else {
            warn!("ui_tradeskill: opener {spell_id} before SkillLineAbility loaded — dropped");
            continue;
        };
        let Some(line) = skill_lines.catalog.spell_to_line(spell_id) else {
            warn!("ui_tradeskill: opener {spell_id} has no SkillLineAbility row — dropped");
            continue;
        };
        // 3.3.5 has no Craft window: every opener, Enchanting's included, opens this book.
        // `0x5de120`: our own opener on our own open book closes it.
        if open.link.is_none() && open.line == Some(line) {
            debug!("ui_tradeskill: opener {spell_id} closes skill line {line}");
            open.close();
            repeat.clear();
            continue;
        }
        debug!("ui_tradeskill: opener {spell_id} opens skill line {line}");
        if open.line != Some(line) {
            repeat.clear();
        }
        open.line = Some(line);
        open.spell = spell_id;
        open.link = None;
    }
}

/// A skill line's raw `(value, max)` off `PLAYER_SKILL_INFO`, which the rank bar shows as is,
/// where the reference's `GetTradeSkillLine` (`0x4fdd40`) adds the permanent skill bonus to each
/// nonzero half (`0x4fddf7`, `0x4fde6a`).
fn skill_rank(store: &ObjectStore, skill_id: u32) -> (u32, u32) {
    for i in 0..PLAYER_SKILL_SLOTS {
        if let Some(s) = store.0.player_skill(i) {
            if u32::from(s.skill_id) == skill_id {
                return (u32::from(s.value), u32::from(s.max));
            }
        }
    }
    (0, 0)
}

/// The difficulty band (`0x4fcbfc`), with a zero `trivialLow` read as `trivialHigh - 25`; the
/// TradeSkill window passes the raw rank.
pub(crate) fn difficulty(rank: u32, low: u32, high: u32) -> TradeSkillDifficulty {
    let low = if low == 0 {
        high.saturating_sub(25)
    } else {
        low
    };
    if rank >= high {
        TradeSkillDifficulty::Trivial
    } else if rank >= (low + high) / 2 {
        TradeSkillDifficulty::Easy
    } else if rank >= low {
        TradeSkillDifficulty::Medium
    } else {
        TradeSkillDifficulty::Optimal
    }
}

/// The row icon, `GetTradeSkillIcon` (`0x4fdae0`): `EffectItemType[0]` as an item id with no
/// `Effect[0]` check, and nil on any miss (a zero id, a template in flight), never the spell's
/// icon; the reference repaints when the item callback fires `TRADE_SKILL_UPDATE`. The trainer
/// resolves icons by another law; the reference shares no resolver.
fn recipe_icon(
    d: &wrathplus_formats::SpellDisplay,
    icons: Option<&ItemDisplays>,
    items: &Items,
    commands: &NetCommands,
) -> Option<String> {
    let item = d.effect_item_type[0];
    if item == 0 {
        return None;
    }
    let display = items.template(item, 0, commands)?.display_info_id;
    item_icon(icons, display)
}

/// Build one recipe row; names still in flight are `None` and resolve when the template lands.
fn resolve_recipe(
    spell_id: u32,
    rank: u32,
    spells: &Spells,
    skill_lines: &SkillLines,
    tool_names: &tools::ToolNames,
    icons: Option<&ItemDisplays>,
    subclasses: Option<&crate::items::ItemSubClasses>,
    store: &ObjectStore,
    objects: &Objects,
    items: &Items,
    commands: &NetCommands,
    cooldowns: &crate::app_core::cooldowns::Cooldowns,
    now: Instant,
    describe: &dyn Fn(&wrathplus_formats::SpellDisplay) -> Option<String>,
) -> Option<TradeSkillRecipe> {
    let d = spells.catalog.get(spell_id)?;
    let sla = skill_lines.catalog.ability(spell_id)?;

    // Reagents: `(entry, need)` pairs off `Spell.dbc`; have is the carried count.
    let mut reagents = Vec::new();
    let mut num_available = u32::MAX;
    for &(entry, need) in d.reagents.iter().filter(|&&(e, n)| e != 0 && n != 0) {
        let have = count_of(&store.0, objects, entry, InventoryScope::CARRIED);
        let (name, icon) = match items.template(entry, 0, commands) {
            Some(t) => (Some(t.name.clone()), item_icon(icons, t.display_info_id)),
            None => (None, None),
        };
        reagents.push(TradeSkillReagent {
            item: entry,
            name,
            icon,
            need,
            have,
        });
        num_available = num_available.min(have / need);
    }
    if reagents.is_empty() {
        num_available = 0;
    }

    // The product: `CREATE_ITEM`'s slot-0 item, where every probed 5875 recipe has it. Made
    // count (`GetTradeSkillNumMade` `0x4fdc50`): min `BasePoints + BaseDice`, max
    // `BasePoints + DieSides * BaseDice`, at least 1.
    let (product_item, min_made, max_made) = if d.effects[0] == SPELL_EFFECT_CREATE_ITEM {
        let base = d.effect_base_points[0].max(0) as u32;
        let dice = d.effect_base_dice[0].max(0) as u32;
        let die = d.effect_die_sides[0].max(0) as u32;
        let min = (base + dice).max(1);
        (d.effect_item_type[0], min, (base + die * dice).max(min))
    } else {
        (0, 1, 1)
    };
    let icon = recipe_icon(d, icons, items, commands);
    // The header key (`0x4fca20`): the product's `(class, subclass)` named from
    // `ItemSubClass.dbc`, `None` while its template is in flight, as in the client. The same
    // template gives the `InventoryType` the slot filter reads and the `ItemLevel` the sort
    // breaks ties on (`record+0x14`).
    let (group, product_inv_type, product_item_level) = (product_item != 0)
        .then(|| {
            items.template(product_item, 0, commands).map(|t| {
                let name = subclasses
                    .and_then(|sc| sc.0.name(t.class, t.subclass))
                    .unwrap_or_default()
                    .to_string();
                ((t.class, t.subclass, name), t.inventory_type, t.item_level)
            })
        })
        .flatten()
        .map_or((None, 0, 0), |(g, it, il)| (Some(g), it, il));

    let focus = tool_names.focus.as_deref().map(|f| &f.catalog);
    let categories = tool_names.categories.as_deref().map(|c| &c.0);
    let tools = tools::recipe_tools(d, focus, categories, store, objects, items, commands);

    // `GetTradeSkillInfo`'s altVerb (`0x5db2a0`): the open line's `alternateVerb` unless the first
    // effect makes an item (24 `CREATE_ITEM`, 157 `CREATE_ITEM_2`, 59 `CREATE_RANDOM_ITEM`).
    let alt_verb = (!matches!(d.effects[0], 24 | 157 | 59))
        .then(|| {
            let line = skill_lines.catalog.spell_to_line(spell_id)?;
            Some(skill_lines.catalog.line(line)?.alternate_verb.clone())
        })
        .flatten();

    let cd = cooldowns.info(spell_id, 0, Some(d), now);
    Some(TradeSkillRecipe {
        group,
        spell_id,
        name: d.name.clone(),
        difficulty: difficulty(rank, sla.trivial_low, sla.trivial_high),
        num_available,
        icon,
        min_made,
        max_made,
        cooldown_secs: (cd.remaining_ms > 0).then(|| u64::from(cd.remaining_ms).div_ceil(1000)),
        product_item,
        product_inv_type,
        product_item_level,
        reagents,
        tools,
        description: describe(d),
        alt_verb,
    })
}

/// Build the book: the known recipes of the open line, banded against the current rank.
fn feed_trade_skill(
    script: Option<NonSendMut<UiScript>>,
    open: Res<TradeSkillOpen>,
    actions: Res<PlayerActions>,
    spells: Option<Res<Spells>>,
    skill_lines: Option<Res<SkillLines>>,
    tool_names: tools::ToolNames,
    icons: Option<Res<ItemDisplays>>,
    subclasses: Option<Res<crate::items::ItemSubClasses>>,
    repeat: Res<TradeSkillRepeat>,
    self_store: Query<(&ObjectStore, &crate::net::Guid), With<SelfPlayer>>,
    objects: Objects,
    items: Res<Items>,
    commands: Res<NetCommands>,
    cooldowns: Res<crate::app_core::cooldowns::Cooldowns>,
    names: Res<crate::names::NameCache>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<TradeSkillState>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let last = last.get(&script);
    let fresh = (|| -> Option<TradeSkillState> {
        let line = open.line?;
        let spells = spells.as_deref()?;
        let skill_lines = skill_lines.as_deref()?;
        let (store, own_guid) = self_store.single().ok()?;
        // A linked book shows the link's rank (`0xc235e8`/`0xc235ec`), ours the descriptor's.
        let (rank, max_rank) = match &open.link {
            Some(l) => (l.rank, l.max_rank),
            None => skill_rank(store, line),
        };
        let line_name = skill_lines
            .catalog
            .line(line)
            .map(|l| l.name.clone())
            .unwrap_or_else(|| format!("Skill {line}"));
        let now = Instant::now();
        // 12340's `GetTradeSkillDescription` (`0x5dbe10` → `0x57abc0`), no caster modifiers.
        let global = |key: &str| wrathplus_ui::strings::global(script.lua(), key);
        let ctx = wrathplus_formats::TokenContext {
            durations: &spells.durations,
            radii: &spells.radii,
            ranges: Some(&spells.ranges),
            skill: &|id| {
                crate::spell::spell_skill_value(Some(store), Some(&skill_lines.catalog), id)
            },
            lookup: &|id| spells.catalog.get(id),
            mods: None,
            unmodified_points: false,
            home_area: &|| None,
            global: &global,
            printf: &crate::ui_script::token_printf,
            player: Some(&crate::ui_script::SelfTokens::new(Some(store), None)),
        };
        let describe = |d: &wrathplus_formats::SpellDisplay| {
            d.description
                .as_deref()
                .filter(|t| !t.is_empty())
                .map(|t| wrathplus_formats::substitute(t, d, &ctx))
        };
        let is_tradeskill = |s: u32| {
            spells
                .catalog
                .get(s)
                .is_some_and(|d| d.attributes & SPELL_ATTR_IS_TRADESKILL != 0)
        };
        // Ours: the known spells of the line. A link's (`0x5dd5a0`): each spell its mask marks.
        let book: Vec<u32> = match &open.link {
            Some(l) => l
                .spells
                .iter()
                .copied()
                .filter(|&s| is_tradeskill(s))
                .collect(),
            None => actions
                .spells
                .iter()
                .copied()
                .filter(|&s| is_tradeskill(s) && skill_lines.catalog.spell_to_line(s) == Some(line))
                .collect(),
        };
        let recipes: Vec<TradeSkillRecipe> = book
            .iter()
            .filter_map(|&s| {
                resolve_recipe(
                    s,
                    rank,
                    spells,
                    skill_lines,
                    &tool_names,
                    icons.as_deref(),
                    subclasses.as_deref(),
                    store,
                    &objects,
                    &items,
                    &commands,
                    &cooldowns,
                    now,
                    &describe,
                )
            })
            .collect();
        // No sort here: the engine orders the book (`0x4fd180`).
        Some(TradeSkillState {
            line,
            line_name,
            rank,
            max_rank,
            recipes,
            repeat_count: repeat.remaining,
            linked: open.link.as_ref().map(|l| {
                names
                    .resolve(l.guid, &commands)
                    .unwrap_or_default()
                    .to_string()
            }),
            list_link: own_trade_link(
                &skill_lines.catalog,
                spells,
                &actions.spells,
                store,
                own_guid.0,
                open.spell,
            ),
        })
    })();

    let repeat_changed = match (&*last, &fresh) {
        (Some(l), Some(f)) => l.repeat_count != f.repeat_count,
        _ => false,
    };
    if fresh == *last {
        return;
    }
    script.set_trade_skill(fresh.clone());
    // The link verbs never query, as the client has every template cached by the time the list
    // shows, so the feed asks for the missing ones here.
    if let Some(f) = &fresh {
        script.ask_item_templates(f.recipes.iter().flat_map(|r| {
            std::iter::once(r.product_item).chain(r.reagents.iter().map(|re| re.item))
        }));
    }
    match (&*last, &fresh) {
        (None, Some(f)) => {
            debug!(
                "ui_tradeskill: window opens — {} recipe(s), first group {:?}",
                f.recipes.len(),
                f.recipes.first().and_then(|r| r.group.clone())
            );
            script.fire_event("TRADE_SKILL_SHOW", vec![]);
        }
        (Some(_), Some(_)) => {
            script.fire_event("TRADE_SKILL_UPDATE", vec![]);
            if repeat_changed {
                script.fire_event("UPDATE_TRADESKILL_RECAST", vec![]);
            }
        }
        (Some(_), None) => script.fire_event("TRADE_SKILL_CLOSE", vec![]),
        (None, None) => {}
    }
    *last = fresh;
}

/// Drain the Lua intents and run the repeat: `DoTradeSkill` latches and casts through the one
/// cast-send path, our own GO re-casts, and a failure or `CloseTradeSkill` stops it.
fn drain_trade_skill(
    script: Option<NonSendMut<UiScript>>,
    mut open: ResMut<TradeSkillOpen>,
    mut repeat: ResMut<TradeSkillRepeat>,
    mut cast_events: MessageReader<CastEvent>,
    targeting: cast_target::CastTargeting,
    mut ladder: CastLadder,
    skill_lines: Option<Res<SkillLines>>,
) {
    let Some(mut script) = script else {
        return;
    };

    for (spell_id, count) in script.take_trade_skill_dos() {
        if open.line.is_none() {
            debug!("ui_tradeskill: DoTradeSkill({spell_id}) with no open book — ignored");
            continue;
        }
        debug!("ui_tradeskill: DoTradeSkill({spell_id}) ×{count}");
        repeat.spell_id = spell_id;
        repeat.remaining = count.max(1);
        ladder.send(spell_id, &targeting.context(), CastCommit::Spell);
    }

    // The repeat's continuation: our own cast edges for the latched spell.
    let self_entity = ladder.self_player.single().ok().map(|(e, _)| e);
    for ev in cast_events.read() {
        if repeat.remaining == 0 || Some(ev.entity) != self_entity || ev.spell_id != repeat.spell_id
        {
            continue;
        }
        match ev.kind {
            CastEventKind::Go => {
                repeat.remaining -= 1;
                if repeat.remaining > 0 {
                    debug!(
                        "ui_tradeskill: recast {} ({} left)",
                        repeat.spell_id, repeat.remaining
                    );
                    ladder.send(repeat.spell_id, &targeting.context(), CastCommit::Spell);
                }
            }
            CastEventKind::Fail => {
                debug!("ui_tradeskill: cast failed — repeat stops");
                repeat.clear();
            }
            _ => {}
        }
    }

    // A filter or expand/collapse fires `TRADE_SKILL_UPDATE` from inside the reference's C call
    // (`0x4fd710`, `0x4fd750`); ours is the touched flag, answered the same frame.
    let touched = script.take_trade_skill_touched();
    if touched && open.line.is_some() {
        script.fire_event("TRADE_SKILL_UPDATE", vec![]);
    }
    // Build 12340's name filter fires `TRADE_SKILL_FILTER_UPDATE` (event `0x27d`, `0x5dcf60`).
    if script.take_trade_skill_filter_touched() && open.line.is_some() {
        script.fire_event("TRADE_SKILL_FILTER_UPDATE", vec![]);
    }
    // `StopTradeSkillRepeat`: the Create All loop ends after the cast in flight.
    if script.take_trade_skill_stop_repeat() {
        repeat.clear();
    }

    if script.take_trade_skill_close() {
        debug!("ui_tradeskill: client-side close (no packet)");
        open.close();
        repeat.clear();
    }

    // A clicked trade link opens the linker's book over ours (`0x5de120`).
    for raw in script.take_trade_link_clicks() {
        let opened = skill_lines
            .as_deref()
            .is_some_and(|sl| open.open_link(&sl.catalog, &raw));
        if opened {
            debug!("ui_tradeskill: trade link opens skill line {:?}", open.line);
            repeat.clear();
        } else {
            debug!("ui_tradeskill: trade link {raw:?} opens nothing");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{test_template, TestDeps};
    use std::collections::HashMap;
    use wrathplus_formats::{
        ItemDisplay, ItemDisplayCatalog, SpellDisplay, SPELL_EFFECT_ENCHANT_ITEM,
    };

    /// A recipe with its own spell icon, so a wrong arm names itself in the assertion.
    fn recipe(effect: u32, item: u32) -> SpellDisplay {
        SpellDisplay {
            name: "Runed Copper Breastplate".into(),
            icon: Some("SPELL".into()),
            effects: [effect, 0, 0],
            effect_item_type: [item, 0, 0],
            ..Default::default()
        }
    }

    /// Item 777's template and `ItemDisplayInfo` row, landed.
    fn landed_item(deps: &mut TestDeps) -> ItemDisplays {
        let mut t = test_template("Runed Copper Breastplate");
        t.display_info_id = 5;
        deps.items.insert_template(777, Some(t));
        ItemDisplays::icons_for_tests(ItemDisplayCatalog::from_displays(HashMap::from([(
            5,
            ItemDisplay {
                icon: Some("ITEM".into()),
                ..Default::default()
            },
        )])))
    }

    #[test]
    fn law_c_paints_the_created_items_icon() {
        let mut deps = TestDeps::new();
        let icons = landed_item(&mut deps);
        let d = recipe(SPELL_EFFECT_CREATE_ITEM, 777);
        assert_eq!(
            recipe_icon(&d, Some(&icons), &deps.items, &deps.commands),
            Some("ITEM".into()),
        );
    }

    /// `0x4fdae0` never reads `Effect[0]`.
    #[test]
    fn law_c_does_not_gate_on_the_effect_type() {
        let mut deps = TestDeps::new();
        let icons = landed_item(&mut deps);
        let d = recipe(SPELL_EFFECT_ENCHANT_ITEM, 777);
        assert_eq!(
            recipe_icon(&d, Some(&icons), &deps.items, &deps.commands),
            Some("ITEM".into()),
        );
    }

    /// The binding's last return is a bare `lua_pushnil`.
    #[test]
    fn law_c_pushes_nil_on_every_miss_never_the_spells_icon() {
        let mut deps = TestDeps::new();
        let icons = landed_item(&mut deps);

        // EffectItemType[0] == 0: 0x55ba30 short-circuits on a zero id before hashing.
        let none = recipe(SPELL_EFFECT_ENCHANT_ITEM, 0);
        assert_eq!(
            recipe_icon(&none, Some(&icons), &deps.items, &deps.commands),
            None,
        );

        // A template that never lands: nil, asked for once.
        let missing = recipe(SPELL_EFFECT_CREATE_ITEM, 999);
        assert_eq!(
            recipe_icon(&missing, Some(&icons), &deps.items, &deps.commands),
            None,
        );
        assert_eq!(
            recipe_icon(&missing, Some(&icons), &deps.items, &deps.commands),
            None,
        );
        assert_eq!(deps.queried_entries(), vec![999], "ask-once, not ask-often");

        // Template landed, `ItemDisplayInfo` missing: still nil.
        let d = recipe(SPELL_EFFECT_CREATE_ITEM, 777);
        assert_eq!(recipe_icon(&d, None, &deps.items, &deps.commands), None);
    }

    /// Driven through the real handler registration.
    #[test]
    fn the_session_end_closes_the_book_and_drops_the_repeat() {
        let mut app = App::new();
        app.add_plugins(UiTradeSkillPlugin);
        app.world_mut().resource_mut::<TradeSkillOpen>().line = Some(197);
        {
            let mut repeat = app.world_mut().resource_mut::<TradeSkillRepeat>();
            repeat.spell_id = 3915;
            repeat.remaining = 4;
        }

        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![wrathplus_protocol::SessionEvent::Disconnected {
                reason: "logged out".into(),
                end: wrathplus_protocol::SessionEnd::LoggedOut,
            }],
        );

        assert_eq!(app.world().resource::<TradeSkillOpen>().line, None);
        let repeat = app.world().resource::<TradeSkillRepeat>();
        assert_eq!((repeat.spell_id, repeat.remaining), (0, 0));
    }

    /// Alchemy (171) with eight rows, spells 1..=8, opened by 2259; row 7 is Orc-only.
    fn alchemy(can_link: bool) -> wrathplus_formats::SkillLineCatalog {
        let mut cat = wrathplus_formats::SkillLineCatalog::from_spell_lines(
            std::iter::once((2259, 171)).chain((1..=8).map(|s| (s, 171))),
        );
        cat.set_line(
            171,
            wrathplus_formats::SkillLineInfo {
                name: "Alchemy".into(),
                category_id: 11,
                icon: None,
                description: String::new(),
                can_link,
                alternate_verb: String::new(),
            },
        );
        cat.set_line_abilities(
            171,
            (1..=8)
                .map(|s| wrathplus_formats::LineAbility {
                    spell_id: s,
                    race_mask: if s == 7 { 1 << 1 } else { 0 },
                    class_mask: 0,
                })
                .collect(),
        );
        cat
    }

    #[test]
    fn a_trade_link_opens_the_linkers_book_only_on_a_linkable_line_and_a_whole_mask() {
        let mut open = TradeSkillOpen::default();
        assert!(open.open_link(
            &alchemy(true),
            "|cffffd000|Htrade:2259:75:150:2A:DB|h[Alchemy]|h|r"
        ));
        assert_eq!((open.line, open.spell), (Some(171), 2259));
        assert_eq!(
            open.link,
            Some(LinkedBook {
                guid: 0x2a,
                rank: 75,
                max_rank: 150,
                spells: vec![1, 2, 7],
            })
        );
        let mut open = TradeSkillOpen::default();
        assert!(
            !open.open_link(&alchemy(true), "trade:2259:75:150:2A:D"),
            "short mask"
        );
        assert!(
            !open.open_link(&alchemy(false), "trade:2259:75:150:2A:DB"),
            "no canLink"
        );
        assert_eq!(open.line, None);
    }

    #[test]
    fn our_list_link_marks_the_known_rows_our_race_may_take() {
        use wrathplus_protocol::types::{Field, ObjectFields};
        let spells = Spells {
            catalog: wrathplus_formats::SpellCatalog::from_displays(HashMap::from([(
                2259,
                SpellDisplay {
                    name: "Alchemy".into(),
                    effects: [wrathplus_formats::SPELL_EFFECT_TRADE_SKILL, 0, 0],
                    ..Default::default()
                },
            )])),
            ..Spells::empty_for_tests()
        };
        // A human (race 1) warrior with Alchemy 75/150.
        let store = ObjectStore(ObjectFields::from_field_slots(&[
            (Field::PlayerSkillInfo11, 0, 171),
            (Field::PlayerSkillInfo11, 1, 75 | 150 << 16),
            (Field::UnitBytes0, 0, 1 | 1 << 8),
        ]));
        let known = [1, 2, 7].into_iter().collect();
        assert_eq!(
            own_trade_link(&alchemy(true), &spells, &known, &store, 0x2a, 2259).as_deref(),
            Some("|cffffd000|Htrade:2259:75:150:2A:DA|h[Alchemy]|h|r"),
            "row 7 is known but Orc-only"
        );
        assert_eq!(
            own_trade_link(&alchemy(false), &spells, &known, &store, 0x2a, 2259),
            None
        );
    }

    #[test]
    fn our_own_opener_on_our_open_book_closes_it() {
        let mut app = App::new();
        app.init_resource::<TradeSkillOpens>()
            .init_resource::<TradeSkillOpen>()
            .init_resource::<TradeSkillRepeat>()
            .insert_resource(SkillLines {
                catalog: alchemy(true),
            })
            .add_systems(Update, open_trade_skill);
        app.world_mut()
            .resource_mut::<TradeSkillOpens>()
            .0
            .push(2259);
        app.update();
        assert_eq!(app.world().resource::<TradeSkillOpen>().line, Some(171));
        app.world_mut()
            .resource_mut::<TradeSkillOpens>()
            .0
            .push(2259);
        app.update();
        assert_eq!(app.world().resource::<TradeSkillOpen>().line, None);
    }
}
