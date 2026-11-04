//! The merchant window's app side: [`MerchantOpen`] holds the `SMSG_LIST_INVENTORY` rows,
//! [`feed_merchant`] pushes them with the buyback and repair rows, the purse and the refusals,
//! [`feed_repair_costs`] the repair costs, and [`drain_merchant`] sends the Lua intents. A
//! bag click sells through [`crate::ui_items`].

use bevy::prelude::*;
use wrathplus_protocol::types::{buy_result, sell_result, VendorItem};
use wrathplus_world::interact::WorldRightPress;

use wrathplus_ui::script::{
    ItemStatsHead, MerchantItem, MerchantState, RepairCosts, ScriptValue, UiScript,
};

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::entities::ItemDisplays;
use crate::inventory::slot_guid;
use crate::items::Items;
use crate::names::NameCache;
use crate::net::{ClientCommand, Guid, NetCommands, Objects};
use crate::ui_items::{item_link, wire_pos_for};
use crate::ui_script::gate;
use crate::ui_session::{close_npc_session_out_of_range, npc_switched, NpcSession};
use wrathplus_protocol::types::BAG_PLAYER_INVENTORY;

/// A vendor row's unlimited `current_count` (vmangos `ItemHandler.cpp:795`), `-1` to Lua.
const STOCK_UNLIMITED: u32 = 0xFFFF_FFFF;

#[cfg(test)]
const NPC_FLAG_REPAIR: u32 = 0x1000;

/// The first absolute buyback slot (AzerothCore `BUYBACK_SLOT_START`, slots 74-85).
const BUYBACK_SLOT_FIRST: u32 = 74;

pub use crate::app_core::catalogs::RepairTables;
pub use crate::app_core::interaction::MerchantOpen;

/// Buy and sell refusals waiting for the feed's message line.
#[derive(Resource, Default)]
pub struct MerchantErrors(pub Vec<MerchantRefusal>);

/// A refused buy or sell and its wire reason code.
pub enum MerchantRefusal {
    Buy(u8),
    Sell(u8),
}

mod net;

pub struct UiMerchantPlugin;

impl Plugin for UiMerchantPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        app.add_systems(
            Update,
            end_repair_mode_on_right_press.in_set(crate::target::TargetSlot::RightPress),
        );
        app.init_resource::<MerchantOpen>()
            .init_resource::<MerchantErrors>()
            .add_systems(
                Startup,
                load_extended_costs.after(wrathplus_assets::AssetSet::Open),
            )
            .add_systems(
                Update,
                (
                    // Range-close first so the clear fires `MERCHANT_CLOSED` the same frame. The
                    // feed follows `UnitFeed`: `MERCHANT_SHOW`'s paint reads the item and
                    // player-requirement stores for `GetMerchantItemInfo`'s `isUsable`.
                    close_npc_session_out_of_range::<MerchantOpen>.before(feed_merchant),
                    feed_merchant
                        .after(crate::app_core::schedule::UnitFeed)
                        .in_set(UiFeed),
                    // Before `feed_char`: the `UNIT_INVENTORY_CHANGED` a repair fires refreshes the
                    // Repair All button from this total (`PaperDollFrame.lua:717-724`).
                    feed_repair_costs
                        .in_set(UiFeed)
                        .before(crate::ui_char::feed_char),
                    drain_merchant.after(UiInput),
                ),
            );
    }
}

/// `ItemExtendedCost.dbc`, loaded once under the Wrath profile.
#[derive(Resource)]
pub struct ExtendedCostTable(pub wrathplus_formats::ExtendedCosts);

fn load_extended_costs(
    mut commands: Commands,
    assets: Option<Res<wrathplus_assets::WorldAssets>>,
    profile: Option<Res<wrathplus_assets::ClientDataProfile>>,
) {
    use wrathplus_assets::LockRecover;
    let (Some(assets), Some(profile)) = (assets, profile) else {
        return;
    };
    if !matches!(profile.0, wrathplus_formats::DataProfile::Wrath { .. }) {
        return;
    }
    let loaded = wrathplus_formats::load_item_extended_costs(&mut assets.chain.lock_recover());
    match loaded {
        Ok(table) => commands.insert_resource(ExtendedCostTable(table)),
        Err(e) => warn!("merchant: ItemExtendedCost.dbc failed to load — no extended costs: {e:#}"),
    }
}

/// `0x5a37b0`, the descriptor leg: one of the three `PLAYER_FIELD_ARENA_TEAM_INFO` slots (7 words
/// each) has a team type at least the bracket's size (`0xa15abc`: 2, 3, 5) and a personal rating
/// (word 6) at least the record's. Unresolved, labelled: the reference also requires its client
/// arena-team table (`0xc0f860`) to agree, which is not modelled.
fn personal_rating_met(
    cost: &wrathplus_formats::ExtendedCost,
    player: Option<&wrathplus_protocol::ObjectFields>,
) -> bool {
    if cost.personal_rating == 0 {
        return true;
    }
    let Some(min_size) = [2u32, 3, 5].get(cost.arena_bracket as usize).copied() else {
        return false;
    };
    let word = |slot: u16, i: u16| {
        player
            .and_then(|f| {
                f.get_at(
                    wrathplus_protocol::types::Field::PlayerArenaTeamInfo11,
                    slot * 7 + i,
                )
            })
            .unwrap_or(0)
    };
    (0..3).any(|s| word(s, 1) >= min_size && word(s, 6) >= cost.personal_rating)
}

/// A row's extended cost, its items resolved through the template cache (asked on a miss).
fn resolve_cost(
    id: u32,
    table: Option<&wrathplus_formats::ExtendedCosts>,
    player: Option<&wrathplus_protocol::ObjectFields>,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
) -> Option<wrathplus_ui::script::ExtendedCostView> {
    let cost = table?.get(id)?;
    let cost_items = cost
        .items
        .iter()
        .zip(cost.counts)
        .filter(|(&item, _)| item != 0)
        .map(|(&item_id, count)| {
            let template = items.template(item_id, 0, commands);
            wrathplus_ui::script::CostItemView {
                item_id,
                count,
                texture: template
                    .and_then(|t| icons?.catalog.get(t.display_info_id))
                    .and_then(|d| d.icon.clone()),
                link: template.map(|t| item_link(item_id, &t.name, t.quality)),
            }
        })
        .collect();
    Some(wrathplus_ui::script::ExtendedCostView {
        honor: cost.honor,
        arena: cost.arena,
        items: cost_items,
        rating_ok: personal_rating_met(cost, player),
    })
}

/// The one `BuyResult` code the reference shows nothing for: `0x5dcde7`'s jump table sends 6 to
/// the handler's `ret`. vmangos never sends it.
const BUY_SILENT: u8 = 6;

/// The message a `SMSG_BUY_FAILED` code shows, from the reference's switch at `0x5dcdd8`; its
/// default arm (`0x5dce81`) shows `ERR_ITEM_NOT_FOUND` for 0, 3, 9, 10 and 13 up.
fn buy_error_key(reason: u8) -> Option<&'static str> {
    Some(match reason {
        // One arm (`0x5dcdee`) for both; only code 1 also zeroes the row.
        buy_result::ITEM_ALREADY_SOLD | buy_result::ITEM_SOLD_OUT => "ERR_VENDOR_SOLD_OUT", // 0x23
        buy_result::NOT_ENOUGH_MONEY => "ERR_NOT_ENOUGH_MONEY", // 0x25, speaks line 0x28
        buy_result::SELLER_DONT_LIKE_YOU => "ERR_VENDOR_HATES_YOU", // 0x22
        buy_result::DISTANCE_TOO_FAR => "ERR_VENDOR_TOO_FAR",   // 0x24
        BUY_SILENT => return None,
        buy_result::CANT_CARRY_MORE => "ERR_ITEM_MAX_COUNT", // 0x12, speaks line 0x1e
        buy_result::RANK_REQUIRE => "ERR_CANT_EQUIP_RANK",   // 0x05
        buy_result::REPUTATION_REQUIRE => "ERR_CANT_EQUIP_REPUTATION", // 0x06
        _ => "ERR_ITEM_NOT_FOUND",                           // 0x17, the switch's default
    })
}

/// The message a `SMSG_SELL_ITEM` error code shows, from the reference's switch at `0x5dd22c`.
/// Its default is silence: 0 returns before the switch (`0x5dd21e`), and 5 and 7 up skip the
/// `DisplayError`.
fn sell_error_key(reason: u8) -> Option<&'static str> {
    Some(match reason {
        sell_result::CANT_FIND_ITEM => "ERR_ITEM_NOT_FOUND", // 0x17
        sell_result::CANT_SELL_ITEM => "ERR_VENDOR_NOT_INTERESTED", // 0x21
        sell_result::CANT_FIND_VENDOR => "ERR_VENDOR_HATES_YOU", // 0x22
        sell_result::YOU_DONT_OWN_THAT_ITEM => "ERR_NOT_OWNER", // 0x1b
        sell_result::ONLY_EMPTY_BAG => "ERR_DESTROY_NONEMPTY_BAG", // 0x0b
        _ => return None,
    })
}

/// One wire row as a Lua row: the icon from the wire `display_id` at once, the name and stats once
/// the item template answers.
fn resolve_item(
    item: &VendorItem,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
    costs: (
        Option<&wrathplus_formats::ExtendedCosts>,
        Option<&wrathplus_protocol::ObjectFields>,
    ),
) -> MerchantItem {
    let template = items.template(item.entry, 0, commands);
    let name = template.map(|t| t.name.clone());
    let stats = template.map(|t| ItemStatsHead {
        quality: t.quality,
        inventory_type: t.inventory_type,
        class: t.class,
        subclass: t.subclass,
        dmg_min: t.dmg_min,
        dmg_max: t.dmg_max,
        dmg_type: t.dmg_type,
        delay_ms: t.delay_ms,
        armor: t.armor,
        block: t.block,
        sell_price: t.sell_price,
    });
    let texture = icons
        .and_then(|i| i.catalog.get(item.display_id))
        .and_then(|d| d.icon.clone());
    let num_available = if item.current_count == STOCK_UNLIMITED {
        -1
    } else {
        item.current_count as i32
    };
    // `GetMerchantItemLink`'s link, built as the reference's `0x52adb0` builds it; a vendor row
    // has no enchant or random property.
    let link = template.map(|t| item_link(item.entry, &t.name, t.quality));
    MerchantItem {
        name,
        texture,
        price: item.price,
        quantity: item.buy_count,
        num_available,
        item_id: item.entry,
        stats,
        link,
        max_stack: template.map(|t| t.stackable),
        extended_cost: resolve_cost(item.extended_cost, costs.0, costs.1, items, icons, commands),
    }
}

/// The occupied buyback slots (0-11) in the reference's display order, oldest first: `0x4fafd0`
/// takes the slots with a guid and a price and sorts them by timestamp. Entry `i` is
/// `GetBuybackItemInfo(i + 1)`.
fn buyback_order(store: &wrathplus_protocol::ObjectFields) -> Vec<u8> {
    let mut v: Vec<(u8, u32)> = (0..12u8)
        .filter_map(|i| {
            let guid = store.player_buyback_slot(i).unwrap_or(0);
            let price = store.player_buyback_price(i).unwrap_or(0);
            (guid != 0 && price != 0).then(|| (i, store.player_buyback_timestamp(i).unwrap_or(0)))
        })
        .collect();
    v.sort_by_key(|&(_, ts)| ts);
    v.into_iter().map(|(i, _)| i).collect()
}

/// One buyback slot as a Lua row: the item from its object, which stays streamed while it sits in
/// the slot, and the price from the player's buyback price field.
fn resolve_buyback(
    idx: u8,
    store: &wrathplus_protocol::ObjectFields,
    objects: &Objects,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
) -> MerchantItem {
    let guid = store.player_buyback_slot(idx).unwrap_or(0);
    let obj = objects.object(guid);
    let entry = obj.and_then(|o| o.object_entry()).unwrap_or(0);
    let quantity = obj.and_then(|o| o.item_stack_count()).unwrap_or(1).max(1);
    let template = items.template(entry, guid, commands);
    let name = template.map(|t| t.name.clone());
    let stats = template.map(|t| ItemStatsHead {
        quality: t.quality,
        inventory_type: t.inventory_type,
        class: t.class,
        subclass: t.subclass,
        dmg_min: t.dmg_min,
        dmg_max: t.dmg_max,
        dmg_type: t.dmg_type,
        delay_ms: t.delay_ms,
        armor: t.armor,
        block: t.block,
        sell_price: t.sell_price,
    });
    let display_id = template.map(|t| t.display_info_id).unwrap_or(0);
    let texture = icons
        .and_then(|i| i.catalog.get(display_id))
        .and_then(|d| d.icon.clone());
    MerchantItem {
        name,
        texture,
        price: store.player_buyback_price(idx).unwrap_or(0),
        quantity,
        num_available: 0,
        item_id: entry,
        stats,
        // Build 12340's `GetBuybackItemLink` (`0x5852c0`); the template's plain link, not the
        // instance's enchants (labelled).
        link: template.map(|t| item_link(entry, &t.name, t.quality)),
        // No max stack: `GetMerchantItemMaxStack` indexes the merchant list, and a buyback row
        // is bought whole.
        max_stack: None,
        extended_cost: None,
    }
}

/// One item's repair cost, by the reference's `0x4faf30` arithmetic
/// ([`wrathplus_formats::DurabilityTables`]), after the price `discount` (`0x4faf8a`).
fn item_repair_cost(
    guid: u64,
    objects: &Objects,
    items: &Items,
    tables: &RepairTables,
    commands: &NetCommands,
    discount: f64,
) -> u32 {
    let Some(obj) = objects.object(guid) else {
        return 0;
    };
    let cur = obj.item_durability().unwrap_or(0);
    let max = obj.item_max_durability().unwrap_or(0);
    let entry = obj.object_entry().unwrap_or(0);
    if max == 0 || cur >= max || entry == 0 {
        return 0;
    }
    let points = max - cur;
    let Some(t) = items.template(entry, guid, commands) else {
        return 0;
    };
    let (level, quality, class, subclass) = (t.item_level, t.quality, t.class, t.subclass);
    tables
        .0
        .repair_cost(points, level, quality, class, subclass, discount)
}

/// Each damaged carried or banked item's cost (`0x4faf30`), keyed as the bindings take it.
fn repair_costs(
    store: &wrathplus_protocol::ObjectFields,
    objects: &Objects,
    items: &Items,
    tables: &RepairTables,
    commands: &NetCommands,
    discount: f64,
) -> RepairCosts {
    let cost = |guid: u64| {
        (guid != 0)
            .then(|| item_repair_cost(guid, objects, items, tables, commands, discount))
            .filter(|&c| c > 0)
    };
    let mut costs = RepairCosts::default();
    for i in 0..19u8 {
        if let Some(c) = cost(store.player_inv_slot(i).unwrap_or(0)) {
            costs.equipped.insert(u32::from(i) + 1, c);
        }
    }
    for i in 0..16u8 {
        if let Some(c) = cost(store.player_pack_slot(i).unwrap_or(0)) {
            costs.bags.insert((0, u32::from(i) + 1), c);
        }
    }
    for i in 0..crate::inventory::BANK_SLOTS {
        if let Some(c) = cost(store.player_bank_slot(i).unwrap_or(0)) {
            costs
                .bags
                .insert((crate::inventory::BANK_CONTAINER, u32::from(i) + 1), c);
        }
    }
    // Bags 1-4 are inventory slots 19-22, bank bags 5-10 the bank bag slots.
    let bags = (1..=4u8)
        .map(|b| (i64::from(b), store.player_inv_slot(18 + b)))
        .chain((0..crate::inventory::BANK_BAGS).map(|i| {
            (
                crate::inventory::BANK_BAG_ID_FIRST + i64::from(i),
                store.player_bank_bag_slot(i),
            )
        }));
    for (bag, bag_guid) in bags {
        let bag_guid = bag_guid.unwrap_or(0);
        if bag_guid == 0 {
            continue;
        }
        let Some(b) = objects.object(bag_guid) else {
            continue;
        };
        let n = b.container_num_slots().unwrap_or(0).min(36) as u8;
        for j in 0..n {
            if let Some(c) = b.container_slot(j).and_then(cost) {
                costs.bags.insert((bag, u32::from(j) + 1), c);
            }
        }
    }
    costs
}

/// `GetRepairAllCost`'s sum of the per-item costs (`0x4fbd60`): equipped and bags, never the bank.
fn repair_all_cost(costs: &RepairCosts) -> u32 {
    let carried = costs
        .bags
        .iter()
        .filter(|((bag, _), _)| (0..=4).contains(bag))
        .map(|(_, c)| c);
    let total: u64 = costs
        .equipped
        .values()
        .chain(carried)
        .map(|&c| u64::from(c))
        .sum();
    total.min(u64::from(u32::MAX)) as u32
}

/// The open vendor's unit, `None` with none open or its unit not streamed.
fn vendor_store<'a>(
    open: &MerchantOpen,
    units: &'a Query<(&Guid, &ObjectStore), Without<SelfPlayer>>,
) -> Option<&'a ObjectStore> {
    open.vendor
        .and_then(|g| units.iter().find(|(guid, _)| guid.0 == g))
        .map(|(_, store)| store)
}

fn can_repair_store(store: &ObjectStore) -> bool {
    store.0.unit_npc_services().can_repair()
}

/// The open vendor's repair service, false with none open or its unit not streamed.
fn vendor_can_repair(
    open: &MerchantOpen,
    units: &Query<(&Guid, &ObjectStore), Without<SelfPlayer>>,
) -> bool {
    vendor_store(open, units).is_some_and(can_repair_store)
}

/// What the purse test before a repair's send reads: the items' costs, the open vendor for its
/// price discount, and the message path a refusal takes.
#[derive(bevy::ecs::system::SystemParam)]
pub struct RepairPurse<'w, 's> {
    objects: Objects<'w, 's>,
    items: Res<'w, Items>,
    commands: Res<'w, NetCommands>,
    tables: Option<Res<'w, RepairTables>>,
    units: Query<'w, 's, (&'static Guid, &'static ObjectStore), Without<SelfPlayer>>,
    relations: crate::target::RelationInputs<'w, 's>,
    sink: crate::ui_action::MessageSink<'w>,
}

impl RepairPurse<'_, '_> {
    /// The purse test of the four item-click repair arms (`0x4f9cc8`–`0x4f9ccf`) and of
    /// `RepairAllItems` (`0x4fc08e`): the `0x4faf30` costs of `guids`, summed in a u32, above
    /// `PLAYER_FIELD_COINAGE` (unsigned, so an equal cost passes) raise `ERR_NOT_ENOUGH_MONEY`
    /// (`0x496720(0x25)`, speech line 0x28) and answer `true`, and the caller sends nothing.
    pub fn refuses(
        &mut self,
        script: &mut UiScript,
        open: &MerchantOpen,
        player: &ObjectStore,
        guids: impl IntoIterator<Item = u64>,
    ) -> bool {
        // `0x4faf30` keeps the full price unless it resolves the vendor's unit.
        let discount = vendor_store(open, &self.units).map_or(0.0, |vendor| {
            crate::target::vendor_price_discount(&self.relations, vendor, player)
        });
        let cost = guids.into_iter().filter(|&g| g != 0).fold(0u32, |sum, g| {
            let item = self.tables.as_deref().map_or(0, |t| {
                item_repair_cost(g, &self.objects, &self.items, t, &self.commands, discount)
            });
            sum.wrapping_add(item)
        });
        let money = player.0.player_money().unwrap_or(0);
        if cost <= money {
            return false;
        }
        debug!("ui_merchant: repair of {cost} copper refused with {money} in the purse");
        if let Some(line) = crate::ui_action::keyed_line(script, "ERR_NOT_ENOUGH_MONEY") {
            crate::ui_action::show_messages(script, &mut self.sink, "ui_merchant", [line]);
        }
        true
    }
}

/// What [`feed_repair_costs`] watches and last pushed.
#[derive(Default)]
pub struct RepairFeedMemo {
    templates: gate::Watch,
    discount: gate::Watch,
    repairer: gate::Watch,
    pushed: Option<(u32, RepairCosts)>,
}

/// Push each item's repair cost, and `GetRepairAllCost`'s total while a repairer is open.
pub fn feed_repair_costs(
    script: Option<NonSendMut<UiScript>>,
    open: Res<MerchantOpen>,
    mut inv: crate::items::Inventory,
    items: Res<Items>,
    commands: Res<NetCommands>,
    units: Query<(&Guid, &ObjectStore), Without<SelfPlayer>>,
    relations: crate::target::RelationInputs,
    tables: Option<Res<RepairTables>>,
    mut memo: Local<crate::app_core::vm_memo::VmMemo<RepairFeedMemo>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let (memo, vm_reset) = memo.get_reset(&script);
    // No self store at logout: keep the last push, as `feed_char` does.
    let Some(player) = inv.self_store.iter().next() else {
        return;
    };
    let vendor = vendor_store(&open, &units);
    // The open vendor's price discount (`0x4faf8a`), 0 with none open.
    let discount = vendor.map_or(0.0, |vendor| {
        crate::target::vendor_price_discount(&relations, vendor, player)
    });
    let repairer = vendor.is_some_and(can_repair_store);
    let objects_moved = inv.changes.moved();
    let self_changed = !inv.self_changed.is_empty();
    let templates_moved = memo.templates.moved(items.template_epoch());
    let discount_moved = memo.discount.moved(discount.to_bits());
    let repairer_moved = memo.repairer.moved(u64::from(repairer));
    let tables_added = tables.as_ref().is_some_and(|t| t.is_added());
    gate::trace(
        "feed_repair_costs",
        &[
            ("vm_reset", vm_reset),
            ("objects", objects_moved),
            ("self", self_changed),
            ("templates", templates_moved),
            ("discount", discount_moved),
            ("repairer", repairer_moved),
            ("tables", tables_added),
        ],
    );
    let gate = gate::Gate::new(
        vm_reset
            || objects_moved
            || self_changed
            || templates_moved
            || discount_moved
            || repairer_moved
            || tables_added,
    );
    if gate.skip() {
        return;
    }
    let costs = tables.as_deref().map_or_else(RepairCosts::default, |t| {
        repair_costs(&player.0, &inv.objects, &items, t, &commands, discount)
    });
    let total = if repairer { repair_all_cost(&costs) } else { 0 };
    if memo
        .pushed
        .as_ref()
        .is_none_or(|(t, c)| *t != total || *c != costs)
    {
        gate.audit("feed_repair_costs", "the repair costs");
        script.set_repair_all_cost(total);
        script.set_repair_costs(costs.clone());
        memo.pushed = Some((total, costs));
    }
}

fn snapshot(
    open: &MerchantOpen,
    objects: &Objects,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
    player: Option<&wrathplus_protocol::ObjectFields>,
    can_repair: bool,
    extended_costs: Option<&wrathplus_formats::ExtendedCosts>,
) -> Option<MerchantState> {
    open.vendor?;
    let buyback = player
        .map(|store| {
            buyback_order(store)
                .into_iter()
                .map(|idx| resolve_buyback(idx, store, objects, items, icons, commands))
                .collect()
        })
        .unwrap_or_default();
    Some(MerchantState {
        items: open
            .items
            .iter()
            .map(|it| resolve_item(it, items, icons, commands, (extended_costs, player)))
            .collect(),
        buyback,
        can_repair,
    })
}

/// Push the merchant into the VM and fire its events on a change; show the refusals and push the
/// purse.
fn feed_merchant(
    script: Option<NonSendMut<UiScript>>,
    open: Res<MerchantOpen>,
    objects: Objects,
    items: Res<Items>,
    icons: Option<Res<ItemDisplays>>,
    commands: Res<NetCommands>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    units: Query<(&Guid, &ObjectStore), Without<SelfPlayer>>,
    names: Res<NameCache>,
    mut errors: ResMut<MerchantErrors>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<MerchantState>>>,
    mut last_money: Local<crate::app_core::vm_memo::VmMemo<Option<u64>>>,
    mut last_name: Local<crate::app_core::vm_memo::VmMemo<Option<String>>>,
    mut last_vendor: Local<crate::app_core::vm_memo::VmMemo<Option<u64>>>,
    mut sink: crate::ui_action::MessageSink,
    extended_costs: Option<Res<ExtendedCostTable>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let last = last.get(&script);
    let last_money = last_money.get(&script);
    let last_name = last_name.get(&script);
    let last_vendor = last_vendor.get(&script);
    // Refusals show, and speak, as their message rows say; a silent code has no key.
    let refusals: Vec<_> = errors
        .0
        .drain(..)
        .filter_map(|refusal| {
            let key = match refusal {
                MerchantRefusal::Buy(r) => buy_error_key(r)?,
                MerchantRefusal::Sell(r) => sell_error_key(r)?,
            };
            crate::ui_action::keyed_line(&script, key)
        })
        .collect();
    crate::ui_action::show_messages(&mut script, &mut sink, "ui_merchant", refusals);
    // The purse (`PLAYER_FIELD_COINAGE`): on a change, push it and fire `PLAYER_MONEY`, which the
    // money frames repaint on.
    if let Some(store) = self_q.iter().next() {
        if let Some(copper) = store.0.player_money() {
            let copper = u64::from(copper);
            if *last_money != Some(copper) {
                script.set_money(copper);
                script.fire_event("PLAYER_MONEY", vec![]);
                *last_money = Some(copper);
            }
        }
    }

    // The vendor's service bits gate the repair pair; the player's descriptor carries buyback.
    let player = self_q.iter().next().map(|s| &s.0);
    let fresh = snapshot(
        &open,
        &objects,
        &items,
        icons.as_deref(),
        &commands,
        player,
        vendor_can_repair(&open, &units),
        extended_costs.as_deref().map(|t| &t.0),
    );
    // The vendor's name rides `MERCHANT_SHOW` and `MERCHANT_UPDATE` as arg1, and its landing alone
    // re-fires `MERCHANT_UPDATE`; the reference fires both bare (`0x4fad92`, `0x4facaa`), and
    // stock `MerchantFrame.lua:67` titles with `UnitName("NPC")`.
    let vendor_name = open
        .vendor
        .and_then(|g| names.resolve(g, &commands).map(str::to_string));
    let name_changed = *last_name != vendor_name;
    // A different vendor while open is a close then an open: `ShowUIPanel` returns early on a
    // visible frame, so only a hide replays the open sound.
    let switched = npc_switched(*last_vendor, open.vendor);
    if fresh == *last && !name_changed && !switched {
        return;
    }
    script.set_merchant(fresh.clone());
    let name_arg = || vec![ScriptValue::Str(vendor_name.clone().unwrap_or_default())];
    if switched {
        // `MERCHANT_CLOSED` queues a close intent through OnHide; consume it so the drain keeps
        // the new vendor.
        script.fire_event("MERCHANT_CLOSED", vec![]);
        script.fire_event("MERCHANT_SHOW", name_arg());
        let _ = script.take_merchant_close();
    } else {
        match (&*last, &fresh) {
            (None, Some(_)) => script.fire_event("MERCHANT_SHOW", name_arg()),
            // A change while open: an item or the name landed, or the stock moved.
            (Some(_), Some(_)) => script.fire_event("MERCHANT_UPDATE", name_arg()),
            (Some(_), None) => script.fire_event("MERCHANT_CLOSED", vec![]),
            (None, None) => {}
        }
    }
    *last = fresh;
    *last_name = vendor_name;
    *last_vendor = open.vendor;
}

/// The range guard closes the window, as `CloseMerchant` does, when the vendor is out of range or
/// gone.
impl NpcSession for MerchantOpen {
    fn npc(&self) -> Option<u64> {
        self.vendor
    }

    fn close(&mut self) {
        self.clear();
    }
}

/// Drain the Lua intents: a buy sends `CMSG_BUY_ITEM` by item entry, a buyback
/// `CMSG_BUYBACK_ITEM` with the absolute slot (69-80), a repair-all `CMSG_REPAIR_ITEM` with guid
/// 0 when the purse covers the worn gear, and a close clears locally, as there is no close packet.
fn drain_merchant(
    script: Option<NonSendMut<UiScript>>,
    mut open: ResMut<MerchantOpen>,
    commands: Res<NetCommands>,
    self_q: Query<(&ObjectStore, &Guid), With<SelfPlayer>>,
    objects: Objects,
    mut purse: RepairPurse,
) {
    let Some(mut script) = script else {
        return;
    };
    let self_pair = self_q.iter().next();
    let self_store = self_pair.map(|(store, _)| store);
    for index in script.take_merchant_buybacks() {
        let Some(vendor) = open.vendor else { continue };
        let Some(store) = self_store else {
            continue;
        };
        let order = buyback_order(&store.0);
        match usize::try_from(index)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| order.get(i))
        {
            Some(&idx) => {
                let slot = BUYBACK_SLOT_FIRST + u32::from(idx);
                debug!("ui_merchant: buyback list index {index} → absolute slot {slot}");
                let _ = commands.0.send(ClientCommand::BuybackItem { vendor, slot });
            }
            None => debug!("ui_merchant: BuybackItem({index}) out of range — ignored"),
        }
    }
    if script.take_repair_all() {
        if let (Some(vendor), Some(me)) = (open.vendor, self_store) {
            // `RepairAllItems` (`0x4fbfe0`) prices only the equipped slots 0-18 for its purse test
            // (`0x4fc026`–`0x4fc063`), not the bags `GetRepairAllCost` also sweeps.
            let worn = (0..19u8).filter_map(|i| me.0.player_inv_slot(i));
            if !purse.refuses(&mut script, &open, me, worn) {
                debug!("ui_merchant: repair all");
                let _ = commands.0.send(ClientCommand::RepairItem {
                    vendor,
                    item_guid: 0,
                });
            }
        }
    }
    for (index, quantity) in script.take_merchant_buys() {
        let Some(vendor) = open.vendor else { continue };
        match open.entry_at(index) {
            Some(entry) => {
                // Stack count is a u8 on the wire (`CMSG_BUY_ITEM`); clamp the Lua quantity.
                let count = quantity.clamp(1, u32::from(u8::MAX)) as u8;
                debug!("ui_merchant: buy row {index} (entry {entry} ×{count})");
                let _ = commands.0.send(ClientCommand::BuyItem {
                    vendor,
                    entry,
                    count,
                });
            }
            None => debug!("ui_merchant: BuyMerchantItem({index}) out of range — ignored"),
        }
    }
    // `PickupMerchantItem`'s sell arm, a bag item dropped on the window, sold by its item guid so
    // an emptied slot is a no-op. It runs before the close below, as `0x4fb760`'s sell arm runs
    // before its vendor check (`0x4fb7f7`), so a drop on a closing window still sells.
    for (bag, slot) in script.take_merchant_cursor_sells() {
        let Some(vendor) = open.vendor else { continue };
        let slot0 = u8::try_from(slot.saturating_sub(1)).unwrap_or(0);
        match self_store.and_then(|s| slot_guid(&s.0, bag, slot0, &objects)) {
            Some(item_guid) => {
                debug!("ui_merchant: cursor sell bag {bag} slot {slot} (item {item_guid:#x})");
                let _ = commands.0.send(ClientCommand::SellItem {
                    vendor,
                    item_guid,
                    count: 0,
                });
            }
            None => debug!("ui_merchant: cursor sell on an empty slot ({bag}, {slot}) — ignored"),
        }
    }
    // A held vendor row dropped into a slot: `CMSG_BUY_ITEM_IN_SLOT` with count 1, the
    // reference's fixed count on all three drop paths.
    for (bag, slot, entry) in script.take_merchant_slot_buys() {
        let Some(vendor) = open.vendor else { continue };
        let (Some(store), Some((bag_index, bag_slot))) = (self_store, wire_pos_for(bag, slot))
        else {
            debug!("ui_merchant: slot buy to an unaddressable slot ({bag}, {slot}) — ignored");
            continue;
        };
        // The wire wants the container's guid: the player's for a slot in the player's own
        // array (`BAG_PLAYER_INVENTORY`), the bag's for a real bag.
        let bag_guid = if bag_index == BAG_PLAYER_INVENTORY {
            self_pair.map(|(_, g)| g.0)
        } else if bag >= crate::inventory::BANK_BAG_ID_FIRST {
            store
                .0
                .player_bank_bag_slot((bag - crate::inventory::BANK_BAG_ID_FIRST) as u8)
                .filter(|g| *g != 0)
        } else {
            store.0.player_inv_slot(bag_index).filter(|g| *g != 0)
        };
        match bag_guid {
            Some(bag_guid) => {
                debug!(
                    "ui_merchant: slot buy entry {entry} → bag {bag_guid:#x} slot {bag_slot} \
                     (lua {bag}, {slot})"
                );
                let _ = commands.0.send(ClientCommand::BuyItemInSlot {
                    vendor,
                    entry,
                    bag_guid,
                    bag_slot,
                    count: 1,
                });
            }
            None => debug!("ui_merchant: slot buy into an absent bag {bag} — ignored"),
        }
    }
    if script.take_merchant_close() {
        debug!("ui_merchant: client-side close (no packet)");
        open.clear();
    }
}

/// A right mouse-down in the world ends repair mode: the WorldFrame hook `0x492c20` resets the
/// Repair base mode to Point (`0x492c68`) and consumes nothing. A held payload pre-empts the hook
/// (`0x492b50`), and a press over a UI frame never reaches it.
pub fn end_repair_mode_on_right_press(
    mut presses: MessageReader<WorldRightPress>,
    payload_held: Res<crate::app_core::state::CursorPayloadHeld>,
    script: Option<NonSendMut<UiScript>>,
) {
    if presses.read().last().is_none() || payload_held.0 {
        return;
    }
    if let Some(mut script) = script {
        script.end_repair_mode();
    }
}

/// A level-20 sword 40 points down at a repair vendor flagged `UNIT_FLAG_PVP`, for the purse tests
/// here and in [`crate::ui_items`].
#[cfg(test)]
pub mod purse_fixture {
    use super::*;
    const FIELD_PLAYER_FIELD_COINAGE: u16 = wrathplus_protocol::types::Field::PlayerCoinage.index();
    const FIELD_UNIT_NPC_FLAGS: u16 = wrathplus_protocol::types::Field::UnitNpcFlags.index();
    use wrathplus_protocol::ObjectFields;

    pub const VENDOR: u64 = 0xF130_0000_0000_0043;
    pub const SWORD: u64 = 0x4000_0000_0000_0707;
    pub const SWORD_ENTRY: u32 = 2488;
    /// Absolute descriptor indices.
    const OBJECT_ENTRY: u16 = wrathplus_protocol::types::Field::ObjectEntry.index();
    const ITEM_DURABILITY: u16 = wrathplus_protocol::types::Field::ItemDurability.index();
    const ITEM_MAXDURABILITY: u16 = wrathplus_protocol::types::Field::ItemMaxdurability.index();
    const FIELD_UNIT_FACTIONTEMPLATE: u16 =
        wrathplus_protocol::types::Field::UnitFactionTemplate.index();
    /// The string `ERR_NOT_ENOUGH_MONEY` holds in 1.12's `GlobalStrings.lua`.
    pub const NOT_ENOUGH_MONEY: &str = "You don't have enough money.";

    /// One level-20 row at 1 copper a point for a sword (weapon subclass 7), and quality 1's
    /// multiplier (row `2·1 + 1`) at 1.
    fn tables() -> wrathplus_formats::DurabilityTables {
        let mut row = vec![0u32; 29];
        row[7] = 1;
        wrathplus_formats::DurabilityTables::from_rows([(20, row)], [(3, 1.0)])
    }

    /// The sword's repair in full, and after the 5% a Friendly vendor gives; a test at the
    /// discounted price discriminates the discount only while the two differ.
    pub fn sword_prices() -> (u32, u32) {
        let t = tables();
        let steps = 0.05;
        let full = t.repair_cost(40, 20, 1, 2, 7, 0.0);
        let discounted = t.repair_cost(40, 20, 1, 2, 7, steps);
        assert!(discounted < full, "{discounted} of {full}");
        (full, discounted)
    }

    /// The merchant open on the vendor, whose unit is streamed, the tables, and the sword's
    /// template and object.
    pub fn seat(world: &mut World) {
        world.insert_resource(RepairTables(tables()));
        // Vendor and player share template 1, so the vendor is Friendly toward the player.
        world.insert_resource(factions());
        world
            .resource_mut::<MerchantOpen>()
            .open(VENDOR, Vec::new());
        world.spawn((
            Guid(VENDOR),
            ObjectStore(ObjectFields::from_pairs(&[
                (FIELD_UNIT_NPC_FLAGS, NPC_FLAG_REPAIR),
                (FIELD_UNIT_FACTIONTEMPLATE, 1),
            ])),
        ));
        let mut sword = crate::items::test_template("Sword");
        (sword.class, sword.subclass, sword.quality, sword.item_level) = (2, 7, 1, 20);
        world
            .resource_mut::<Items>()
            .insert_template(SWORD_ENTRY, Some(sword));
        crate::items::test_spawn_item(
            world,
            SWORD,
            ObjectFields::from_pairs(&[
                (OBJECT_ENTRY, SWORD_ENTRY),
                (ITEM_DURABILITY, 35),
                (ITEM_MAXDURABILITY, 75),
            ]),
            false,
        );
    }

    /// The player's fields: the sword in the guid pair at `slot_field`, `money` copper, and
    /// faction template 1.
    pub fn player(slot_field: u16, money: u32) -> ObjectStore {
        ObjectStore(ObjectFields::from_pairs(&[
            (slot_field, SWORD as u32),
            (slot_field + 1, (SWORD >> 32) as u32),
            (FIELD_PLAYER_FIELD_COINAGE, money),
            (FIELD_UNIT_FACTIONTEMPLATE, 1),
        ]))
    }

    /// Template 1 alone, friend to its own group (masks 3/2/12), so the vendor is Friendly.
    fn factions() -> crate::app_core::state::Factions {
        crate::app_core::state::Factions::from_catalog(
            wrathplus_formats::FactionCatalog::from_rows(
                std::collections::HashMap::from([(
                    1,
                    wrathplus_formats::FactionTemplate {
                        faction: 1,
                        group_mask: 3,
                        friend_group_mask: 2,
                        enemy_group_mask: 12,
                        ..Default::default()
                    },
                )]),
                std::collections::HashMap::new(),
            ),
        )
    }

    /// [`seat`] and the player with the sword at `slot_field`; returns the player's entity too.
    pub fn repair_app(slot_field: u16) -> (App, Entity) {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.init_resource::<Items>()
            .init_resource::<crate::net::GuidIndex>()
            .init_resource::<MerchantOpen>()
            .init_resource::<crate::net::Reputations>()
            .insert_resource(NetCommands(tx));
        seat(app.world_mut());
        let me = app
            .world_mut()
            .spawn((SelfPlayer, Guid(0x5e1f), player(slot_field, 0)))
            .id();
        (app, me)
    }

    /// A VM with the sword at live id 16, backpack slot 1 and bank slot 1, and a tooltip `TT`.
    pub fn sword_vm() -> UiScript {
        let mut script = UiScript::new().unwrap();
        let mut inv: wrathplus_ui::script::InventorySlots = Default::default();
        inv[16] = Some(wrathplus_ui::script::InvSlotView {
            item_id: SWORD_ENTRY,
            ..Default::default()
        });
        script.set_inventory_slots(inv);
        let mut bag = wrathplus_ui::script::ContainerState {
            num_slots: 16,
            ..Default::default()
        };
        bag.slots.insert(
            1,
            wrathplus_ui::script::ContainerSlot {
                item_id: SWORD_ENTRY,
                count: 1,
                ..Default::default()
            },
        );
        script.set_container(0, Some(bag.clone()));
        script.set_container(crate::inventory::BANK_CONTAINER, Some(bag));
        script
            .run(
                r#"CreateFrame("GameTooltip", "TT"):SetOwner(CreateFrame("Frame"), "ANCHOR_NONE")"#,
            )
            .unwrap();
        script
    }

    /// The message's string, and a recorder of the `UI_ERROR_MESSAGE` lines [`shown`] reads.
    pub fn record_errors(script: &UiScript) {
        script
            .run(&format!(
                r#"
                ERR_NOT_ENOUGH_MONEY = "{NOT_ENOUGH_MONEY}"
                WRATHPLUS_TEST_LINES = {{}}
                local f = CreateFrame("Frame")
                f:RegisterEvent("UI_ERROR_MESSAGE")
                f:SetScript("OnEvent", function(self, event, arg1, arg2, arg3, arg4, arg5, arg6, arg7, arg8, arg9) table.insert(WRATHPLUS_TEST_LINES, arg1) end)
                "#
            ))
            .unwrap();
    }

    /// The `UI_ERROR_MESSAGE` lines since [`record_errors`].
    pub fn shown(script: &UiScript) -> Vec<String> {
        script
            .eval::<Vec<String>>("return WRATHPLUS_TEST_LINES")
            .unwrap()
    }

    /// The message rows queued for their sound.
    pub fn sounded(world: &World) -> Vec<&'static str> {
        world
            .resource::<crate::sound::MessageSounds>()
            .queued()
            .iter()
            .map(|r| r.key)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::Field;

    /// `GetRepairAllCost` sums each item's cost after the open merchant's discount (`0x4faf8a`):
    /// at Honored with a PvP-flagged vendor and honor rank byte 8, 0.2 comes off each item before
    /// its rounding, and the undiscounted total is not what the VM reads.
    #[test]
    fn the_repair_all_total_takes_the_merchants_discount() {
        const FIELD_PLAYER_INV_SLOT_HEAD: u16 =
            wrathplus_protocol::types::Field::PlayerInvSlotHead.index();
        const FIELD_UNIT_FACTIONTEMPLATE: u16 =
            wrathplus_protocol::types::Field::UnitFactionTemplate.index();
        const FIELD_UNIT_NPC_FLAGS: u16 = wrathplus_protocol::types::Field::UnitNpcFlags.index();
        use bevy::ecs::system::RunSystemOnce;
        use wrathplus_protocol::ObjectFields;

        /// Absolute descriptor indices.
        const OBJECT_ENTRY: u16 = Field::ObjectEntry.index();
        const ITEM_DURABILITY: u16 = Field::ItemDurability.index();
        const ITEM_MAXDURABILITY: u16 = Field::ItemMaxdurability.index();
        const VENDOR: u64 = 0xF130_0000_0000_0042;
        const SWORD: u64 = 0x4000_0000_0000_0007;
        const SWORD_ENTRY: u32 = 2488;

        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let tables = wrathplus_formats::load_durability_tables(&mut chain).expect("durability");
        let (factions, template, reps) = crate::target::stormwind_fixture(&mut chain, 9000);
        // A level 20 green sword (class 2, subclass 7) 40 points down.
        // Honored: 10%, the 12340 reputation ladder.
        let disc = 0.10;
        let want = tables.repair_cost(40, 20, 2, 2, 7, disc);
        let full = tables.repair_cost(40, 20, 2, 2, 7, 0.0);
        assert!(
            want > 0 && want < full,
            "the case discriminates: {want} of {full}"
        );

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.init_resource::<Items>()
            .init_resource::<crate::net::GuidIndex>()
            .insert_resource(NetCommands(tx))
            .insert_resource(RepairTables(tables))
            .insert_resource(factions)
            .insert_resource(reps);
        let mut open = MerchantOpen::default();
        open.open(VENDOR, Vec::new());
        app.insert_resource(open);
        let mut sword = crate::items::test_template("Sword");
        (sword.class, sword.subclass, sword.quality, sword.item_level) = (2, 7, 2, 20);
        app.world_mut()
            .resource_mut::<Items>()
            .insert_template(SWORD_ENTRY, Some(sword));
        crate::items::test_spawn_item(
            app.world_mut(),
            SWORD,
            ObjectFields::from_pairs(&[
                (OBJECT_ENTRY, SWORD_ENTRY),
                (ITEM_DURABILITY, 35),
                (ITEM_MAXDURABILITY, 75),
            ]),
            false,
        );
        // The sword in the main hand (slot 15).
        let main_hand = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 15;
        app.world_mut().spawn((
            SelfPlayer,
            ObjectStore(crate::target::human_warrior_player(&[
                (main_hand, SWORD as u32),
                (main_hand + 1, (SWORD >> 32) as u32),
            ])),
        ));
        app.world_mut().spawn((
            Guid(VENDOR),
            ObjectStore(ObjectFields::from_pairs(&[
                (FIELD_UNIT_NPC_FLAGS, NPC_FLAG_REPAIR),
                (FIELD_UNIT_FACTIONTEMPLATE, template),
            ])),
        ));
        let mut script = UiScript::new().unwrap();
        script.set_merchant(Some(MerchantState {
            can_repair: true,
            ..Default::default()
        }));
        app.insert_non_send_resource(script);

        app.world_mut().run_system_once(feed_repair_costs).unwrap();
        let script = app.world_mut().non_send_resource_mut::<UiScript>();
        let got = script.eval::<i64>("return (GetRepairAllCost())").unwrap();
        assert_eq!(got, i64::from(want), "the undiscounted total is {full}");
    }

    /// `RepairAllItems()` through the feed and the drain, at [`purse_fixture`]'s vendor carrying
    /// `npc_flags`, with the sword at `slot_field` and `money` copper: the packets, the
    /// `UI_ERROR_MESSAGE` lines and the message rows queued for their sound.
    fn repair_all(
        npc_flags: u32,
        slot_field: u16,
        money: u32,
    ) -> (Vec<ClientCommand>, Vec<String>, Vec<&'static str>) {
        const FIELD_UNIT_NPC_FLAGS: u16 = wrathplus_protocol::types::Field::UnitNpcFlags.index();
        use bevy::ecs::system::RunSystemOnce;
        use purse_fixture::*;
        use wrathplus_protocol::ObjectFields;

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.init_resource::<Items>()
            .init_resource::<crate::net::GuidIndex>()
            .init_resource::<MerchantOpen>()
            .init_resource::<MerchantErrors>()
            .init_resource::<NameCache>()
            .init_resource::<crate::net::Reputations>()
            .init_resource::<crate::app_core::chat::ChatLog>()
            .init_resource::<crate::sound::MessageSounds>()
            .insert_resource(NetCommands(tx));
        seat(app.world_mut());
        let mut units = app.world_mut().query::<(&Guid, &mut ObjectStore)>();
        for (guid, mut store) in units.iter_mut(app.world_mut()) {
            if guid.0 == VENDOR {
                let flags = ObjectFields::from_pairs(&[(FIELD_UNIT_NPC_FLAGS, npc_flags)]);
                store.0.merge(flags);
            }
        }
        app.world_mut()
            .spawn((SelfPlayer, Guid(0x5e1f), player(slot_field, money)));
        let script = UiScript::new().unwrap();
        record_errors(&script);
        app.insert_non_send_resource(script);
        app.world_mut().run_system_once(feed_merchant).unwrap();
        while rx.try_recv().is_ok() {} // the feed's name query for the vendor
        app.world()
            .non_send_resource::<UiScript>()
            .run("RepairAllItems()")
            .unwrap();
        app.world_mut().run_system_once(drain_merchant).unwrap();
        let sent = rx.try_iter().collect();
        let lines = shown(app.world().non_send_resource::<UiScript>());
        (sent, lines, sounded(app.world()))
    }

    /// At a vendor that does not repair, `RepairAllItems` sends nothing and says nothing: it
    /// returns (`0x4fbff0`) unless `0x4fadb0` finds the repair flag on the open merchant, before
    /// its purse test. The same worn sword at a repairer goes out.
    #[test]
    fn repair_all_does_nothing_at_a_vendor_that_does_not_repair() {
        /// `UNIT_NPC_FLAG_VENDOR` in build 12340.
        const NPC_FLAG_VENDOR: u32 = 0x80;
        const FIELD_PLAYER_INV_SLOT_HEAD: u16 =
            wrathplus_protocol::types::Field::PlayerInvSlotHead.index();
        use purse_fixture::*;

        let (_, discounted) = sword_prices();
        let main_hand = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 15;
        for money in [discounted, discounted - 1] {
            let (sent, lines, rows) = repair_all(NPC_FLAG_VENDOR, main_hand, money);
            assert!(sent.is_empty(), "{money} copper: no repair goes out");
            assert!(
                lines.is_empty() && rows.is_empty(),
                "{money} copper: no purse refusal"
            );
        }
        let (sent, ..) = repair_all(NPC_FLAG_VENDOR | NPC_FLAG_REPAIR, main_hand, discounted);
        assert!(matches!(
            sent.as_slice(),
            [ClientCommand::RepairItem {
                vendor: VENDOR,
                item_guid: 0
            }]
        ));
    }

    /// `RepairAllItems` prices only the equipped slots for its purse test (`0x4fc026`–`0x4fc063`,
    /// `0x4fc08e`): the sword worn and over the purse is refused with `ERR_NOT_ENOUGH_MONEY` and
    /// no send; the same sword in the backpack, which `GetRepairAllCost` would count, leaves a worn
    /// total of 0 and the repair-all goes out.
    #[test]
    fn repair_all_tests_the_purse_against_the_worn_gear_alone() {
        const FIELD_PLAYER_INV_SLOT_HEAD: u16 =
            wrathplus_protocol::types::Field::PlayerInvSlotHead.index();
        use purse_fixture::*;

        let (_, discounted) = sword_prices();

        // The main hand (slot 15).
        let main_hand = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 15;
        let (sent, lines, rows) = repair_all(NPC_FLAG_REPAIR, main_hand, discounted - 1);
        assert!(sent.is_empty(), "worn, one copper short: nothing goes out");
        assert_eq!(lines, [NOT_ENOUGH_MONEY]);
        assert_eq!(
            rows,
            ["ERR_NOT_ENOUGH_MONEY"],
            "the row that speaks line 0x28"
        );

        // Backpack slot 1, inventory slot 23.
        let backpack_1 = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 23;
        let (sent, lines, rows) = repair_all(NPC_FLAG_REPAIR, backpack_1, discounted - 1);
        assert!(
            matches!(
                sent.as_slice(),
                [ClientCommand::RepairItem {
                    vendor: VENDOR,
                    item_guid: 0
                }]
            ),
            "the bag's {discounted} is not in the purse test"
        );
        assert!(lines.is_empty() && rows.is_empty());
    }

    /// An item's cost is discounted at the vendor and full once it closes (`0x4faf30`).
    #[test]
    fn each_items_cost_takes_the_open_vendors_discount() {
        const FIELD_PLAYER_INV_SLOT_HEAD: u16 =
            wrathplus_protocol::types::Field::PlayerInvSlotHead.index();
        use bevy::ecs::system::RunSystemOnce;
        use purse_fixture::*;

        let (full, discounted) = sword_prices();
        let priced = |slot_field: u16, read: &str| {
            let (mut app, _) = repair_app(slot_field);
            app.insert_non_send_resource(sword_vm());
            let read_cost = |app: &mut App| {
                app.world_mut().run_system_once(feed_repair_costs).unwrap();
                let script = app.world().non_send_resource::<UiScript>();
                script.eval::<i64>(read).unwrap()
            };
            let at_vendor = read_cost(&mut app);
            app.world_mut().resource_mut::<MerchantOpen>().clear();
            (at_vendor, read_cost(&mut app))
        };
        let want = (i64::from(discounted), i64::from(full));

        // Main hand, inventory slot 15.
        let worn = priced(
            FIELD_PLAYER_INV_SLOT_HEAD + 2 * 15,
            r#"local _, _, c = TT:SetInventoryItem("player", 16) return c"#,
        );
        assert_eq!(worn, want, "worn: at the vendor, then with none open");
        // Backpack slot 1, inventory slot 23.
        let carried = priced(
            FIELD_PLAYER_INV_SLOT_HEAD + 2 * 23,
            "local _, c = TT:SetBagItem(0, 1) return c",
        );
        assert_eq!(
            carried, want,
            "in the backpack: at the vendor, then with none open"
        );
    }

    /// Each gate input alone reopens the feed, through `App::update` so the memo persists; a bank
    /// item is priced but left out of the total (`0x4fbd60`).
    #[test]
    fn each_input_alone_reopens_the_repair_cost_gate() {
        const FIELD_PLAYER_INV_SLOT_HEAD: u16 =
            wrathplus_protocol::types::Field::PlayerInvSlotHead.index();
        const FIELD_UNIT_FACTIONTEMPLATE: u16 =
            wrathplus_protocol::types::Field::UnitFactionTemplate.index();
        use purse_fixture::*;
        use wrathplus_protocol::ObjectFields;

        /// `ITEM_FIELD_DURABILITY`, absolute.
        const ITEM_DURABILITY: u16 = Field::ItemDurability.index();
        let main_hand = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 15;
        let backpack_1 = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 23;
        // `PLAYER_FIELD_BANK_SLOT_1`, inventory slot 39.
        let bank_1 = FIELD_PLAYER_INV_SLOT_HEAD + 2 * 39;

        let (mut app, me) = repair_app(main_hand);
        app.add_systems(Update, feed_repair_costs);
        let tables = app.world_mut().remove_resource::<RepairTables>().unwrap();
        let mut script = sword_vm();
        script.set_merchant(Some(MerchantState {
            can_repair: true,
            ..Default::default()
        }));
        app.insert_non_send_resource(script);

        let entity =
            |app: &App, guid: u64| app.world().resource::<crate::net::GuidIndex>().0[&guid];
        let write = |app: &mut App, e: Entity, pairs: &[(u16, u32)]| {
            let mut store = app.world_mut().get_mut::<ObjectStore>(e).unwrap();
            store.0.merge(ObjectFields::from_pairs(pairs));
        };
        // (worn, backpack, bank, `GetRepairAllCost`) after one frame.
        let frame = |app: &mut App| {
            app.update();
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<(i64, i64, i64, i64)>(
                    r#"local _, _, worn = TT:SetInventoryItem("player", 16)
                       local _, carried = TT:SetBagItem(0, 1)
                       local _, banked = TT:SetBagItem(-1, 1)
                       return worn, carried, banked, (GetRepairAllCost())"#,
                )
                .unwrap()
        };
        let vendor = app
            .world_mut()
            .query::<(Entity, &Guid)>()
            .iter(app.world())
            .find(|(_, g)| g.0 == VENDOR)
            .unwrap()
            .0;
        let hi = (SWORD >> 32) as u32;
        let moved =
            |from: u16, to: u16| [(from, 0), (from + 1, 0), (to, SWORD as u32), (to + 1, hi)];

        assert_eq!(frame(&mut app), (0, 0, 0, 0), "no tables, no prices");
        // 1 copper a point, less 0.05: 40 points cost 38.
        app.insert_resource(tables);
        assert_eq!(frame(&mut app), (38, 0, 0, 38), "the tables arrive");
        let sword = entity(&app, SWORD);
        write(&mut app, sword, &[(ITEM_DURABILITY, 55)]);
        assert_eq!(
            frame(&mut app),
            (19, 0, 0, 19),
            "a repair's write: 20 points"
        );
        write(&mut app, me, &moved(main_hand, bank_1));
        assert_eq!(
            frame(&mut app),
            (0, 0, 19, 0),
            "banked: priced, not in the total"
        );
        write(&mut app, me, &moved(bank_1, backpack_1));
        assert_eq!(frame(&mut app), (0, 19, 0, 19), "the sword in the backpack");
        write(&mut app, vendor, &[(FIELD_UNIT_FACTIONTEMPLATE, 0)]);
        assert_eq!(
            frame(&mut app),
            (0, 20, 0, 20),
            "no friendly reaction, no discount"
        );
        app.world_mut().resource_mut::<MerchantOpen>().clear();
        assert_eq!(
            frame(&mut app),
            (0, 20, 0, 0),
            "the repairer gone: no total"
        );
        app.world_mut()
            .resource_mut::<Items>()
            .insert_template(SWORD_ENTRY, None);
        assert_eq!(
            frame(&mut app),
            (0, 0, 0, 0),
            "an unknown template prices nothing"
        );
    }

    fn row(entry: u32, slot: u32, count: u32) -> VendorItem {
        VendorItem {
            slot,
            entry,
            display_id: 100 + entry,
            current_count: count,
            price: 500,
            max_durability: 0,
            buy_count: 1,
            extended_cost: 0,
        }
    }

    /// The ids are asserted, not just the keys: each is its arm's `push <id>; call 0x496720`.
    #[test]
    fn the_refusal_tables_are_the_references_own() {
        let id = |key: &str| {
            wrathplus_ui::messages::by_key(key)
                .unwrap_or_else(|| panic!("{key} is not a catalog row"))
                .id
        };
        // Build 12340's ids (`0xac8af0`) for the rows the 1.12 switches showed; the 12340 arms are
        // untraced. `SMSG_BUY_FAILED` (1.12 `0x5dcdd8`): 1 and 7 share `0x25`, 6 is silent, and
        // codes outside the table fall to `0x19`.
        for (code, want) in [
            (0u8, Some(0x19u16)),
            (1, Some(0x25)),
            (2, Some(0x28)),
            (3, Some(0x19)),
            (4, Some(0x24)),
            (5, Some(0x26)),
            (6, None),
            (7, Some(0x25)),
            (8, Some(0x14)),
            (9, Some(0x19)),
            (10, Some(0x19)),
            (11, Some(0x05)),
            (12, Some(0x07)),
            (13, Some(0x19)),
            (255, Some(0x19)),
        ] {
            assert_eq!(buy_error_key(code).map(id), want, "buy code {code}");
        }
        // `SMSG_SELL_ITEM`, the switch at `0x5dd22c`, where the default is silence.
        for (code, want) in [
            (0u8, None),
            (1, Some(0x19u16)),
            (2, Some(0x23)),
            (3, Some(0x24)),
            (4, Some(0x1d)),
            (5, None),
            (6, Some(0x0d)),
            (7, None),
            (255, None),
        ] {
            assert_eq!(sell_error_key(code).map(id), want, "sell code {code}");
        }
    }

    /// Only these two vendor refusals speak.
    #[test]
    fn the_purse_refusals_carry_their_voice_lines() {
        let tag = |key: &str| wrathplus_ui::messages::by_key(key).expect("row").type_tag;
        assert_eq!(tag("ERR_NOT_ENOUGH_MONEY"), 0x28);
        assert_eq!(tag("ERR_ITEM_MAX_COUNT"), 0x1e);
        for key in [
            "ERR_VENDOR_SOLD_OUT",
            "ERR_VENDOR_HATES_YOU",
            "ERR_VENDOR_TOO_FAR",
            "ERR_ITEM_NOT_FOUND",
            "ERR_VENDOR_NOT_INTERESTED",
            "ERR_NOT_OWNER",
            "ERR_DESTROY_NONEMPTY_BAG",
            "ERR_CANT_EQUIP_RANK",
            "ERR_CANT_EQUIP_REPUTATION",
        ] {
            assert_eq!(tag(key), 0x44, "{key}");
        }
    }

    /// The reference's cache write (`0x5dcdbf`), keyed by entry as vmangos fills the field.
    #[test]
    fn a_sold_out_refusal_zeroes_only_that_rows_count() {
        let mut open = MerchantOpen::default();
        open.open(7, vec![row(11, 1, 3), row(22, 2, 5), row(11, 3, 2)]);
        open.sold_out(11);
        assert_eq!(open.items[0].current_count, 0);
        assert_eq!(
            open.items[1].current_count, 5,
            "a different entry is untouched"
        );
        assert_eq!(
            open.items[2].current_count, 0,
            "the reference does not stop at the first match"
        );
        open.sold_out(999);
        assert_eq!(
            open.items[1].current_count, 5,
            "an entry we do not stock is a no-op"
        );
    }

    #[test]
    fn entry_at_maps_the_one_based_row() {
        let mut open = MerchantOpen::default();
        assert!(!open.is_open());
        open.open(0x42, vec![row(159, 1, STOCK_UNLIMITED), row(4540, 2, 5)]);
        assert!(open.is_open());
        assert_eq!(open.entry_at(1), Some(159));
        assert_eq!(open.entry_at(2), Some(4540));
        assert_eq!(open.entry_at(3), None); // out of range
        assert_eq!(open.entry_at(0), None); // 0 has no row (1-based)
    }

    #[test]
    fn resolve_maps_unlimited_stock_to_minus_one() {
        let items = Items::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);
        // Unlimited stock is `numAvailable` -1; a finite count passes through.
        let unlimited = resolve_item(
            &row(159, 1, STOCK_UNLIMITED),
            &items,
            None,
            &commands,
            (None, None),
        );
        assert_eq!(unlimited.num_available, -1);
        assert_eq!(unlimited.item_id, 159);
        let finite = resolve_item(&row(4540, 2, 5), &items, None, &commands, (None, None));
        assert_eq!(finite.num_available, 5);
        // No template answer yet: no name or stats, the rest present.
        assert!(finite.name.is_none());
        assert!(finite.stats.is_none());
        assert_eq!(finite.price, 500);
    }

    #[test]
    fn resolve_fills_the_tooltip_stats_from_the_template() {
        let mut items = Items::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);
        items.insert_template(
            2129,
            Some(wrathplus_protocol::types::ItemInfo {
                class: 4,
                subclass: 6,
                name: "Chipped Buckler".into(),
                display_info_id: 18730,
                quality: 1,
                flags: 0,
                buy_price: 0,
                sell_price: 0,
                inventory_type: 14,
                allowable_class: -1,
                allowable_race: -1,
                item_level: 0,
                required_level: 0,
                required_skill: 0,
                required_skill_rank: 0,
                required_spell: 0,
                required_honor_rank: 0,
                required_city_rank: 0,
                required_rep_faction: 0,
                required_rep_rank: 0,
                max_count: 0,
                stackable: 1,
                container_slots: 0,
                stats: Vec::new(),
                damages: Vec::new(),
                dmg_min: 0.0,
                dmg_max: 0.0,
                dmg_type: 0,
                armor: 85,
                resistances: [0; 6],
                delay_ms: 0,
                ammo_type: 0,
                ranged_mod_range: 0.0,
                spells: Vec::new(),
                spell_charges_0: 0,
                use_spell: None,
                bonding: 0,
                description: String::new(),
                page_text: 0,
                language_id: 0,
                page_material: 0,
                start_quest: 0,
                lock_id: 0,
                material: 0,
                sheath: 4,
                random_property: 0,
                block: 1,
                item_set: 0,
                max_durability: 0,
                area: 0,
                map: 0,
                bag_family: 0,
                totem_category: 0,
            }),
        );
        let resolved = resolve_item(&row(2129, 1, 3), &items, None, &commands, (None, None));
        let stats = resolved.stats.expect("template answered → stats present");
        assert_eq!(
            (
                stats.quality,
                stats.inventory_type,
                stats.class,
                stats.subclass
            ),
            (1, 14, 4, 6)
        );
        assert_eq!((stats.armor, stats.block), (85, 1));
    }

    #[test]
    fn update_stock_moves_the_matching_row() {
        let mut open = MerchantOpen::default();
        open.open(0x42, vec![row(159, 1, STOCK_UNLIMITED), row(4540, 2, 5)]);
        open.update_stock(2, 4);
        assert_eq!(open.items[1].current_count, 4);
        open.update_stock(99, 0); // no such slot: a no-op
        assert_eq!(open.items[0].current_count, STOCK_UNLIMITED);
    }

    #[test]
    fn clear_closes_the_window() {
        let mut open = MerchantOpen::default();
        open.open(0x42, vec![row(159, 1, 3)]);
        open.clear();
        assert!(!open.is_open());
        assert!(open.items.is_empty());
    }
}

#[cfg(test)]
#[test]
fn repair_service_uses_native_flags_in_each_layout() {
    use wrathplus_protocol::types::Field;
    let (repair, unrelated) = (0x1000, 0x4000);
    let span = Field::UnitNpcFlags
        .span()
        .expect("NPC flags exist in both layouts");
    for (bits, expected) in [
        (0, false),
        (repair, true),
        (unrelated, false),
        (repair | unrelated, true),
    ] {
        let store = ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(&[(
            span.index, bits,
        )]));
        assert_eq!(can_repair_store(&store), expected);
    }
}
