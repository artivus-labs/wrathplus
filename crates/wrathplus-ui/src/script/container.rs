//! The 1.12 bag verbs, all bare globals. The app pushes each bag's resolved snapshot and drains
//! the intents the verbs queue; the engine holds no item knowledge. Bag ids are the 1.12 client's
//! (0 the backpack, 1-4 the worn bags, -1 the bank's own slots, 5-10 the bank bags, -2 the
//! keyring); slots are 1-based.

use mlua::{Lua, Table, Value};

use super::cursor::{self, CursorItem, CursorPayload};
use super::Model;

/// The petition lines an item tooltip prints under the name (`0x854d7c..0x854dd0`): the title and
/// the creator, keyed `GUILD_CHARTER_*` for a charter, else `PETITION_*`. The signature-count line
/// is not built: its source is untraced, and vmangos's write of the on-item count is commented out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PetitionSlotView {
    /// The record's charter bit, also `GetPetitionInfo`'s first return.
    pub is_charter: bool,
    /// The proposed guild's (or petition's) name.
    pub title: String,
    /// The owner's name from the name cache; `None`, and no line, while the query is in flight.
    pub owner: Option<String>,
}

/// One occupied bag slot, resolved by the app from the item object and its template.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerSlot {
    /// Icon texture path (`Interface\Icons\…`); `None` while the template answer is in flight.
    pub texture: Option<String>,
    pub count: u32,
    /// Item quality 0..6; `None` while unresolved (the API reports `nil`).
    pub quality: Option<u32>,
    /// The item's template entry; 0 while unresolved.
    pub item_id: u32,
    /// An `|Hitem:…|h[Name]|h` link once the name is known.
    pub link: Option<String>,
    pub locked: bool,
    /// The 1-based inventory slots the item equips into, from `inventoryType`; empty if none.
    pub equip_slots: Vec<u8>,
    /// Whether the item may go on an action bar: `PlaceAction`'s only item filter (`0x4e62e0`), an
    /// on-use spell or equippable.
    pub bar_placeable: bool,
    /// Current and max durability; `None` when the max is 0 or the item object has not arrived.
    pub durability: Option<(u32, u32)>,
    /// The use cooldown, `(start_ms on the GetTime clock, duration_ms, enabled)`; `None` if cold.
    pub cooldown: Option<(i64, u32, bool)>,
    /// `GetContainerItemInfo`'s `readable` (`0x5d7bc3`-`0x5d7be6`): the template's `PageText`
    /// (`CGItem` vfunc `0xdc`, `[rec+0x180]`) or `ITEM_FIELD_FLAGS & 0x200` (a mail copy); it
    /// shows the Inspect cursor (`ContainerFrame.lua:306`, `:787-788`).
    pub readable: bool,
    /// `GetContainerItemInfo`'s `lootable` (`0x5d7c04`): the template's `Flags & 0x4`, read
    /// through the item's flag accessor (`0x707390`, `[rec+0x18]`).
    pub lootable: bool,
    /// The template's `InventoryType`: 0 (a Hearthstone, food) answers quality -1 (`0x5d7ba7`).
    pub inventory_type: u32,
    /// The `ITEM_FIELD_CREATOR` name for the "Written by" or "<Made by>" line, once resolved.
    pub creator: Option<String>,
    /// `ITEM_FIELD_FLAGS`; the tooltip reads UNLOCKED `0x4` and WRAPPED `0x8`.
    pub flags: u32,
    /// Bound at runtime (`0x5da2c0`): `ITEM_FIELD_FLAGS & 1`, or an enchant whose
    /// `SpellItemEnchantment` row binds. The tooltip's bind line then says Soulbound.
    pub already_bound: bool,
    /// The enchant slots in slot order, named from `SpellItemEnchantment.dbc`; empty when none.
    pub enchants: Vec<super::EnchantView>,
    /// A timed item's lifetime left in ms (`SMSG_ITEM_TIME_UPDATE`), floored to the second.
    pub duration_ms: Option<u64>,
    /// A signable charter's petition, printed between the name and the green `ITEM_SIGNABLE`
    /// line; `None` until the record the hover queries arrives.
    pub petition: Option<PetitionSlotView>,
}

/// One enchant slot as the tooltip renders it (`0x52c9f9`..`0x52ca23`); the app resolves the row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnchantView {
    /// `ITEM_FIELD_ENCHANTMENT` slot: 0 permanent, 1 temporary, 2-6 the random suffix; only 0 and
    /// 1 are coloured.
    pub slot: u8,
    /// The id was negative: `abs(id)` names the row, and slots 0 and 1 paint red, not green
    /// (`0x52ca29`..`0x52ca49`).
    pub negative: bool,
    /// The `SpellItemEnchantment` row's name, copied unformatted (`0x52ca8b`..`0x52caa1`).
    pub name: String,
    /// The slot's charges; nonzero appends " (N Charges)".
    pub charges: u32,
    /// Ms left on a temporary enchant, from `SMSG_ITEM_ENCHANT_TIME_UPDATE`, never the item's own
    /// duration field; `Some` prints the countdown form.
    pub remaining_ms: Option<u64>,
}

/// One `ItemRandomProperties` row, an item's random-suffix roll. The app pushes the whole table at
/// load, and a tooltip source supplies only the id, as in the reference.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RandomPropertyView {
    /// The suffix (`"of the Monkey"`), joined on by `ITEM_SUFFIX_TEMPLATE` (`0x5d8b00`), for the
    /// names the app does not compose itself.
    pub suffix: String,
    /// The roll's five enchants, named, in slots 2-6 (`0x52b7e0`..`0x52b7fb`); always white.
    pub enchants: Vec<EnchantView>,
}

/// One bag's snapshot: its name, capacity, and occupied slots (1-based).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerState {
    /// `GetBagName` (the bag item's own name; the backpack is the client-local "Backpack").
    pub name: Option<String>,
    pub num_slots: u32,
    pub slots: std::collections::HashMap<u32, ContainerSlot>,
}

/// One queued move from `(src_bag, src_slot)` to `(dst_bag, dst_slot)`. `count` is `None` for a
/// whole stack (a move, a swap, or a merge the server tops up), `Some(n)` for a split placement
/// (`CMSG_SPLIT_ITEM`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMove {
    pub src_bag: i64,
    pub src_slot: u32,
    pub dst_bag: i64,
    pub dst_slot: u32,
    pub count: Option<u32>,
}

/// One queued auto-store (`PutItemInBag`, `PutItemInBackpack`): the item at `(src_bag, src_slot)`
/// into container `dst_bag`, with no slot: `CMSG_AUTOSTORE_BAG_ITEM` (0x10B) carries none and
/// `CMSG_SPLIT_ITEM` (0x10E) sends `0xFF`, so the server picks. `Some(n)` in `count` is a split,
/// the fork the reference makes on `[0xb4b40c]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BagAutoStore {
    pub src_bag: i64,
    pub src_slot: u32,
    /// The container id: 0 the backpack, 1-4 a worn bag, 5-10 a bank bag.
    pub dst_bag: i64,
    pub count: Option<u32>,
}

impl super::UiScript {
    /// Drain the auto-stores `PutItemInBag`/`PutItemInBackpack` queued since the last call.
    pub fn take_bag_autostores(&mut self) -> Vec<BagAutoStore> {
        std::mem::take(&mut self.model_mut().bag_autostores)
    }

    /// Push one bag's snapshot, or remove it with `None`; cooldowns arrive in ms on the `GetTime`
    /// clock and are stored in seconds.
    pub fn set_container(&mut self, bag: i64, state: Option<ContainerState>) {
        let mut model = self.model_mut();
        model.container_cooldowns.retain(|&(b, _), _| b != bag);
        match state {
            Some(s) => {
                for (&slot, cs) in &s.slots {
                    let Some((start_ms, duration_ms, enabled)) = cs.cooldown else {
                        continue;
                    };
                    let abs = (
                        start_ms as f64 / 1000.0,
                        f64::from(duration_ms) / 1000.0,
                        enabled,
                    );
                    model.container_cooldowns.insert((bag, slot), abs);
                }
                model.containers.insert(bag, s);
            }
            None => {
                model.containers.remove(&bag);
            }
        }
    }

    /// Push `HasKey()`'s answer: whether the player owns a `BagFamily` key item anywhere.
    pub fn set_has_key(&mut self, has_key: bool) {
        self.model_mut().has_key = has_key;
    }

    /// Take the `UseContainerItem` calls out of the call stream, as `(bag, slot)`.
    pub fn take_container_uses(&mut self) -> Vec<(i64, u32)> {
        self.take_calls_where(|c| match c {
            super::ScriptCall::UseContainerItem { bag, slot } => Some((*bag, *slot)),
            _ => None,
        })
    }

    /// Drain the pick/place/swap moves queued by `PickupContainerItem` since the last call.
    pub fn take_container_moves(&mut self) -> Vec<ContainerMove> {
        std::mem::take(&mut self.model_mut().container_moves)
    }

    /// Drain the `(bag, slot)` sources `AutoEquipCursorItem` queued, for `CMSG_AUTOEQUIP_ITEM`.
    pub fn take_container_autoequips(&mut self) -> Vec<(i64, u32)> {
        std::mem::take(&mut self.model_mut().container_autoequips)
    }

    /// Drain the `(bag, slot)` clicks repair mode queued, for `CMSG_REPAIR_ITEM`.
    pub fn take_container_repairs(&mut self) -> Vec<(i64, u32)> {
        std::mem::take(&mut self.model_mut().container_repairs)
    }

    /// Drain the `(bag, slot, count)` destroys `DeleteCursorItem` queued, a 0 count being the whole
    /// stack, for `CMSG_DESTROYITEM`.
    pub fn take_container_destroys(&mut self) -> Vec<(i64, u32, u32)> {
        std::mem::take(&mut self.model_mut().container_destroys)
    }

    /// Arm a gift wrap with the paper at `(bag, slot)`, as `0x5edea0` does: the paper locks, the
    /// cursor takes mode 2 (the cast art, table `0x853b88`), and nothing is sent. The next
    /// left-click on a container slot completes it.
    pub fn arm_gift_wrap(&mut self, bag: i64, slot: u32) {
        let mut model = self.model_mut();
        model.pending_wrap = Some(PendingWrap { bag, slot });
        model.ui_cursor = Some(UiCursorMode::Cast);
        model.ui_cursor_dirty = true;
        cursor::queue_lock_changed(&mut model, bag, slot);
    }

    /// The armed gift wrap, if any; the app reads it to hold the cursor's base at Cast.
    pub fn gift_wrap_armed(&self) -> Option<PendingWrap> {
        self.model_ref().pending_wrap
    }

    /// Drain the `(giftBag, giftSlot, itemBag, itemSlot)` wraps queued, for `CMSG_WRAP_ITEM`.
    pub fn take_container_wraps(&mut self) -> Vec<(i64, u32, i64, u32)> {
        std::mem::take(&mut self.model_mut().container_wraps)
    }

    /// The mode the last FrameXML cursor call set, `None` after a `ResetCursor`; the app acts on
    /// [`Self::take_cursor_write`] instead.
    pub fn ui_cursor(&self) -> Option<UiCursorMode> {
        self.model_ref().ui_cursor
    }

    /// Drain the pending cursor write: `Some(mode)` a set, `Some(None)` a reset, `None` no call,
    /// which must leave the cursor as it was, so an armed spell keeps its cast cursor over a
    /// spellbook button.
    #[allow(clippy::option_option)]
    pub fn take_cursor_write(&mut self) -> Option<Option<UiCursorMode>> {
        let mut model = self.model_mut();
        std::mem::take(&mut model.ui_cursor_dirty).then_some(model.ui_cursor)
    }
}

/// The armed gift wrap's paper. The reference keeps it beside the cursor mode, not as a cursor
/// payload, so `CursorHasItem()` stays false while a wrap is armed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PendingWrap {
    /// The paper's container id.
    pub bag: i64,
    /// The paper's 1-based slot within that container.
    pub slot: u32,
}

/// A displayed-cursor override from the FrameXML cursor verbs, the reference's one displayed mode
/// (`0xbe2c2c`), until `ResetCursor`; the app maps each to its `Interface\Cursor` art.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiCursorMode {
    /// Buy (3): selling a bag item, or a merchant or buyback item the player can afford.
    Buy,
    /// UnableBuy (23): a merchant or buyback item the player cannot afford.
    UnableBuy,
    /// Inspect (7): the magnifier, `ShowInspectCursor`.
    Inspect,
    /// Point (1), `SetCursor("POINT_CURSOR")`.
    Point,
    /// Cast (2), `SetCursor("CAST_CURSOR")`: a unit frame the armed spell can target.
    Cast,
    /// UnableCast (22), `SetCursor("CAST_ERROR_CURSOR")`: a unit frame the armed spell cannot
    /// target, and leaving one while still targeting.
    CastError,
}

/// `PickupContainerItem`'s gesture (`0x4f9b30`):
/// - an empty cursor picks up a resolved item whose slot is not locked;
/// - a click on the held item's own slot cancels;
/// - a whole stack placed anywhere else queues the move and clears the cursor; a swap lands the
///   displaced item in the source slot, never on the cursor (no `SetCursorItem`, `0x5e0c40`);
/// - a split carry places onto an empty or same-item slot, and stays held over another item;
/// - a spell or action payload stays held.
fn pickup_container_item(model: &mut super::Model, bag: i64, slot: u32) {
    // An armed gift wrap completes here, before the cursor is read. `PickupContainerItem` is the
    // only caller of the wrap sender `0x5edfc0`, so an equipped item cannot be wrapped.
    if let Some(wrap) = model.pending_wrap {
        // An empty or unresolved target leaves the wrap armed, as in the reference.
        if !model
            .containers
            .get(&bag)
            .and_then(|c| c.slots.get(&slot))
            .is_some_and(|s| s.item_id != 0)
        {
            return;
        }
        // A vanished paper also bails, the wrap armed and the paper locked (`0x5edfef`).
        if !model
            .containers
            .get(&wrap.bag)
            .and_then(|c| c.slots.get(&wrap.slot))
            .is_some_and(|s| s.item_id != 0)
        {
            return;
        }
        // No eligibility test: the server refuses with an `ERR_CANT_WRAP_*` reason
        // (`SMSG_INVENTORY_CHANGE_FAILURE` 43-48), and a local gate would eat that error line.
        model.pending_wrap = None;
        model.container_wraps.push((wrap.bag, wrap.slot, bag, slot));
        // The paper unlocks and the cursor resets right after the send (`0x5eded0` at
        // `0x5ee0aa`), not on the answer; the target never locks.
        cursor::queue_lock_changed(model, wrap.bag, wrap.slot);
        model.ui_cursor = None;
        model.ui_cursor_dirty = true;
        return;
    }
    // Only a held item (`0x4f9c38`) or vendor row (`0x4f9c43`) takes the click first. Past them
    // an empty slot does nothing (`0x4f9c4e`), an armed item-targeting spell binds the item
    // (`0x4f9c54`) and repair mode repairs it (`0x4f9c7b`), any other payload staying held; the
    // app's repair drain runs the arm's `ITEM_REPAIR` and send.
    let placing = matches!(
        model.cursor,
        Some(CursorPayload::Item(_) | CursorPayload::Merchant(_) | CursorPayload::GuildBankItem(_))
    );
    if !placing && (model.item_pick_armed || model.repair_mode) {
        let occupied = model
            .containers
            .get(&bag)
            .and_then(|c| c.slots.get(&slot))
            .is_some_and(|s| s.item_id != 0);
        if occupied && model.item_pick_armed {
            model.item_picks.push((bag, slot));
        } else if occupied {
            model.container_repairs.push((bag, slot));
        }
        return;
    }
    match model.cursor.take() {
        None => {
            let picked = model
                .containers
                .get(&bag)
                .and_then(|c| c.slots.get(&slot))
                .filter(|s| s.item_id != 0 && !s.locked)
                .map(|s| CursorItem {
                    bag,
                    slot,
                    item_id: s.item_id,
                    texture: s.texture.clone(),
                    link: s.link.clone(),
                    quality: s.quality,
                    count: None,
                    bar_placeable: s.bar_placeable,
                    equip_slots: s.equip_slots.clone(),
                });
            if let Some(item) = picked {
                model.cursor = Some(CursorPayload::Item(item));
                cursor::queue_cursor_update(model);
                cursor::queue_lock_changed(model, bag, slot);
            }
        }
        Some(CursorPayload::Item(held)) if held.bag == bag && held.slot == slot => {
            cursor::queue_cursor_update(model);
            cursor::queue_lock_changed(model, held.bag, held.slot);
        }
        Some(CursorPayload::Item(held)) => {
            // Cloned so the moves below can borrow `model`; an unresolved slot reads as empty.
            let dest = model
                .containers
                .get(&bag)
                .and_then(|c| c.slots.get(&slot))
                .cloned()
                .filter(|d| d.item_id != 0);
            match (held.count, dest) {
                // A split carry cannot swap with a different item: it stays held.
                (Some(_), Some(d)) if d.item_id != held.item_id => {
                    model.cursor = Some(CursorPayload::Item(held));
                }
                // Anything else queues the move and clears: a split, a merge the server tops up,
                // or a swap that lands the displaced item in the source slot.
                (count, _) => {
                    queue_move(model, &held, bag, slot, count);
                    cursor::queue_cursor_update(model);
                    cursor::queue_lock_changed(model, held.bag, held.slot);
                }
            }
        }
        // Mode 5, a held vendor row, buys into this slot whatever it holds (`0x4f9efa` tests
        // `[0xb4d900] == 5`): `CMSG_BUY_ITEM_IN_SLOT` (0x1a3) with count 1, and the cursor clears
        // either way. Deviation: a row that no longer resolves is refused, as `0x4c7300` does,
        // because this path dereferences it unchecked (`0x4f9f35`), a latent null deref.
        Some(CursorPayload::Merchant(held)) => {
            let entry = model
                .merchant
                .as_ref()
                .and_then(|m| m.items.get(held.row as usize))
                .filter(|it| it.item_id == held.item_id)
                .map(|it| it.item_id);
            if let Some(entry) = entry {
                model.merchant_slot_buys.push((bag, slot, entry));
            }
            cursor::queue_cursor_update(model);
        }
        // Mode 11, a guild bank item (`0x5d7ff0` tests `[0xbd0748] == 0xb`): with the banker still
        // open (`0xc1dc10`) the withdraw into this slot (`0x6db880`), then the cursor clears.
        Some(CursorPayload::GuildBankItem(held)) => {
            if model.wrath_friends.bank.tabs.is_empty() {
                model.cursor = Some(CursorPayload::GuildBankItem(held));
                return;
            }
            model
                .wrath_friends
                .requests
                .push(crate::script::WrathGuildBankRequest::DropOnBag {
                    tab: held.tab,
                    slot: held.slot,
                    bag,
                    bag_slot: slot,
                    count: held.count,
                });
            cursor::queue_cursor_update(model);
        }
        Some(
            other @ (CursorPayload::Spell(_)
            | CursorPayload::Action(_)
            | CursorPayload::Macro(_)
            | CursorPayload::PetAction(_)
            // Mode 10, a stabled pet, stays held for the stable window.
            | CursorPayload::StablePet(_)
            // Mode 2, money, stays held for a money frame's DropFunc.
            | CursorPayload::Money(_)
            // Modes 7 and 0xd, an item by id and an equipment set, have no bag slot to take.
            | CursorPayload::ItemId(_)
            | CursorPayload::EquipmentSet(_)),
        ) => {
            model.cursor = Some(other);
        }
    }
}

/// Queue one move, the shared tail of every clearing branch above.
fn queue_move(
    model: &mut super::Model,
    held: &CursorItem,
    dst_bag: i64,
    dst_slot: u32,
    count: Option<u32>,
) {
    model.container_moves.push(ContainerMove {
        src_bag: held.bag,
        src_slot: held.slot,
        dst_bag,
        dst_slot,
        count,
    });
}

/// `UseContainerItem` with the trade window open: the bag item goes into our first free traded
/// slot (1 to 6, never the enchant slot), queued as `ClickTradeButton` would queue it. Build
/// 12340's arm of `0x5d8650` (inferred from its shape: a per-item check `0x59d0b0`, a free-slot
/// search `0x59a2b0`, the lock and the set `0x59a280`). An empty, locked or soulbound item stays
/// put (AzerothCore's `HandleSetTradeItemOpcode` cancels the whole trade on an untradeable one),
/// as does any item when the six are full.
fn trade_into_first_free(model: &mut Model, bag: i64, slot: u32) {
    const SOULBOUND: u32 = 0x1;
    const BOP_TRADEABLE: u32 = 0x100;
    let tradeable = model
        .containers
        .get(&bag)
        .and_then(|c| c.slots.get(&slot))
        .is_some_and(|s| {
            s.item_id != 0
                && !s.locked
                && (s.flags & SOULBOUND == 0 || s.flags & BOP_TRADEABLE != 0)
        });
    let queued: Vec<u32> = model.trade_set_items.iter().map(|&(id, _, _)| id).collect();
    let free = model.trade.as_ref().and_then(|t| {
        (1..super::trade::TRADE_SLOTS as u32)
            .find(|id| t.player.slots[*id as usize - 1].is_none() && !queued.contains(id))
    });
    if let (true, Some(id)) = (tradeable, free) {
        model.trade_set_items.push((id, bag, slot));
        cursor::queue_lock_changed(model, bag, slot);
    }
}

/// Register the container verbs as bare globals, as build 12340 has them; `C_Container` is a later
/// client's namespace, and an addon that finds it assumes that client.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // ContainerIDToInventoryID(containerID) (`0x4f94e0`): `id - 1`, then +20 below 4 else +60, so
    // the keyring -2 → 17, the backpack 0 → 19, bags 1-4 → 20-23 and bank bags 5-10 → 64-69, with
    // no special case and no range check.
    lua.globals().set(
        "ContainerIDToInventoryID",
        lua.create_function(|lua, id: Value| {
            let id = super::binding_abi::number_arg(
                lua,
                id,
                "Usage: ContainerIDToInventoryID(containerID)",
            )?;
            // The reference's steps and signed compare (`0x4f9524 jl`), wrapping at the `i32` edge.
            Ok(i64::from(
                lua.app_data_ref::<Model>()
                    .expect("model")
                    .container_inventory_slot(id),
            ))
        })?,
    )?;

    // SetBagPortaitTexture(texture, containerID) (`0x4fa4f0`, the reference's spelling): 0 and
    // below return before the clear (`0x4fa5b9`), 1-10 clear and then set the bag's icon, 11 and
    // up raise. The reference also needs the bank open for 5-10, which is not checked here.
    lua.globals().set(
        "SetBagPortaitTexture",
        lua.create_function(|lua, (region, container): (Table, i64)| {
            if container >= 12 {
                return Err(mlua::Error::runtime("Invalid slot in SetBagPortaitTexture"));
            }
            if container <= 0 {
                return Ok(());
            }
            // `ContainerIDToInventoryID`'s arithmetic, inlined: an addon may replace that global,
            // and the reference's portrait read does not go through Lua.
            let inv = lua
                .app_data_ref::<Model>()
                .expect("model")
                .container_inventory_slot(container as i32);
            let path = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                usize::try_from(inv)
                    .ok()
                    .and_then(|slot| model.inv_slot("player", slot))
                    .and_then(|v| v.icon.clone())
            };
            let rh = crate::script::region::region_handle_of(lua, &region)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let data = model.region_data.entry(rh).or_default();
            // An empty slot's `None` is the answer: the reference clears first, then sets.
            data.texture = path;
            data.circular = true;
            data.portrait_unit = None;
            Ok(())
        })?,
    )?;

    lua.globals().set(
        "GetContainerNumSlots",
        lua.create_function(|lua, bag: i64| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.containers.get(&bag).map_or(0, |c| c.num_slots))
        })?,
    )?;

    lua.globals().set(
        "GetBagName",
        lua.create_function(|lua, bag: i64| {
            let name = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                model.containers.get(&bag).and_then(|c| c.name.clone())
            };
            match name {
                Some(n) => Ok(Value::String(lua.create_string(&n)?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;

    // GetContainerItemCooldown(bag, slot) (`0x4f99b0`) → start, duration, enable on the `GetTime`
    // clock, as `GetActionCooldown`; enable 0 is a held cooldown. An expired one reads cold, so a
    // re-feed cannot replay the finish flash.
    lua.globals().set(
        "GetContainerItemCooldown",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let now = crate::script::clock::now(lua);
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match model.container_cooldowns.get(&(bag, slot)) {
                Some(&(start, duration, enabled)) if start + duration > now || !enabled => {
                    (start, duration, i32::from(enabled))
                }
                _ => (0.0, 0.0, 1),
            })
        })?,
    )?;

    lua.globals().set(
        "GetContainerItemLink",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let link = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                model
                    .containers
                    .get(&bag)
                    .and_then(|c| c.slots.get(&slot))
                    .and_then(|s| s.link.clone())
            };
            match link {
                Some(l) => Ok(Value::String(lua.create_string(&l)?)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;

    // GetContainerItemInfo(bag, slot) (`0x5d7a90`) → texture, itemCount, locked, quality,
    // readable, lootable, link; no values for an empty or unknown slot.
    lua.globals().set(
        "GetContainerItemInfo",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let (info, held_here) = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let held = matches!(
                    &model.cursor,
                    Some(CursorPayload::Item(c)) if c.bag == bag && c.slot == slot
                ) ||
                // An armed wrap's paper reads locked until the send or a cancel (`0x4953e0` at
                // `0x5edeb5`).
                    matches!(
                        &model.pending_wrap,
                        Some(w) if w.bag == bag && w.slot == slot
                    );
                (
                    model
                        .containers
                        .get(&bag)
                        .and_then(|c| c.slots.get(&slot))
                        .cloned(),
                    held,
                )
            };
            let Some(s) = info else {
                return Ok(mlua::MultiValue::new());
            };
            // The flags are 1 or nil; an uncached template, or one with `InventoryType` 0, answers
            // quality -1 (`0x5d7ba5`-`0x5d7bb1`).
            let text = |v: &Option<String>| -> mlua::Result<Value> {
                Ok(match v {
                    Some(p) => Value::String(lua.create_string(p.as_str())?),
                    None => Value::Nil,
                })
            };
            Ok(mlua::MultiValue::from_vec(vec![
                text(&s.texture)?,
                Value::Integer(i64::from(s.count)),
                // A held item's source slot reads locked, derived from the cursor so the app's
                // re-push cannot wipe it.
                super::binding_abi::flag(s.locked || held_here),
                Value::Integer(
                    s.quality
                        .filter(|_| s.inventory_type != 0)
                        .map_or(-1, i64::from),
                ),
                super::binding_abi::flag(s.readable),
                super::binding_abi::flag(s.lootable),
                text(&s.link)?,
            ]))
        })?,
    )?;

    lua.globals().set(
        "UseContainerItem",
        lua.create_function(|lua, (bag, slot, _rest): (i64, u32, mlua::MultiValue)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // Build 12340 (`0x5d8650`): a locked item does nothing, then a targeting spell that
            // wants an item binds it (`0x7fd620`, `0x5210d0`) before any use, repair or sale, as
            // stock `ContainerFrameItemButton_OnClick` relies on (`ContainerFrame.lua:698-700`).
            if model.item_pick_armed {
                if model
                    .containers
                    .get(&bag)
                    .and_then(|c| c.slots.get(&slot))
                    .is_some_and(|s| s.item_id != 0 && !s.locked)
                {
                    model.item_picks.push((bag, slot));
                }
                return Ok(());
            }
            // Every right-click clears the cursor before it reads the slot (`0x4fa198`): a held
            // payload goes back, never placed, and an armed gift wrap is cancelled.
            cursor::clear_cursor(&mut model);
            if model.repair_mode {
                // Then the item in the slot is repaired (`0x4fa1a6`, `0x4fa1da`).
                let occupied = model
                    .containers
                    .get(&bag)
                    .and_then(|c| c.slots.get(&slot))
                    .is_some_and(|s| s.item_id != 0);
                if occupied {
                    model.container_repairs.push((bag, slot));
                }
            } else if model.trade.is_some() {
                trade_into_first_free(&mut model, bag, slot);
            } else {
                model
                    .script_calls
                    .push(super::ScriptCall::UseContainerItem { bag, slot });
            }
            Ok(())
        })?,
    )?;

    // PickupContainerItem(bag, slot) (`0x4f9b30`): pick up, or place and swap; no return value on
    // any path.
    lua.globals().set(
        "PickupContainerItem",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            pickup_container_item(&mut model, bag, slot);
            Ok(())
        })?,
    )?;

    // ShowContainerSellCursor(bag, slot) (`0x4fa460`): Buy (3) over an occupied slot, with no
    // price check, so selling never greys; an empty slot leaves the cursor alone.
    lua.globals().set(
        "ShowContainerSellCursor",
        lua.create_function(|lua, (bag, slot): (i64, u32)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let occupied = model
                .containers
                .get(&bag)
                .and_then(|c| c.slots.get(&slot))
                .is_some();
            // An armed spell or repair cursor suppresses it first; each is sticky, so a Buy here
            // would stamp out the active base cursor.
            if occupied && !model.spell_targeting && !model.repair_mode {
                model.ui_cursor = Some(UiCursorMode::Buy);
                model.ui_cursor_dirty = true;
            }
            Ok(())
        })?,
    )?;
    // ShowInspectCursor() (`0x48ac60`): Inspect (7), unconditionally.
    lua.globals().set(
        "ShowInspectCursor",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.ui_cursor = Some(UiCursorMode::Inspect);
            model.ui_cursor_dirty = true;
            Ok(())
        })?,
    )?;
    // HasKey() (`0x51ae80`) → the number 1 if the player owns a key (`BagFamily & 0x100`)
    // anywhere, else nil, never a boolean; the app does the search.
    lua.globals().set(
        "HasKey",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(if model.has_key {
                Value::Integer(1)
            } else {
                Value::Nil
            })
        })?,
    )?;
    // SetCursor(name) (`0x489490`): POINT, CAST, BUY and ATTACK are modes 1-4, each `*_ERROR` 20
    // higher, and no argument is a `ResetCursor`. The stock UI calls it only from the unit-frame
    // hover (`UnitFrame.lua:50`).
    lua.globals().set(
        "SetCursor",
        lua.create_function(|lua, name: Option<String>| {
            let mode = match name.as_deref() {
                None => None, // no-arg == ResetCursor
                Some("POINT_CURSOR") => Some(UiCursorMode::Point),
                Some("INSPECT_CURSOR") => Some(UiCursorMode::Inspect), // `UIParent.lua:2917-2919`
                Some("CAST_CURSOR") => Some(UiCursorMode::Cast),
                Some("CAST_ERROR_CURSOR") => Some(UiCursorMode::CastError),
                Some("BUY_CURSOR") => Some(UiCursorMode::Buy),
                Some("BUY_ERROR_CURSOR") => Some(UiCursorMode::UnableBuy),
                // Ignored: an unknown name is a custom cursor bitmap, which is not built.
                Some(_) => return Ok(()),
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.ui_cursor = mode;
            model.ui_cursor_dirty = true;
            Ok(())
        })?,
    )?;
    // ResetCursor() (`0x48ac70` → `0x523d30`): back to the base mode, whatever set the override.
    lua.globals().set(
        "ResetCursor",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.ui_cursor = None;
            model.ui_cursor_dirty = true;
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
