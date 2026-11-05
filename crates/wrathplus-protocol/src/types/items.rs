//! Item messages. Item templates are not in descriptors: `CMSG_ITEM_QUERY_SINGLE` (opcode 86,
//! entry and guid) asks, and `SMSG_ITEM_QUERY_SINGLE_RESPONSE` (88) answers in the field order of
//! vmangos `ItemHandler.cpp:269-415`, every build conditional included at 5875. A miss is the lone
//! `u32` `entry | 0x8000_0000`.

/// One item template, as `SMSG_ITEM_QUERY_SINGLE_RESPONSE` carries it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemInfo {
    pub class: u32,
    pub subclass: u32,
    pub name: String,
    /// `ItemDisplayInfo.dbc` id, for the icon and model.
    pub display_info_id: u32,
    /// 0 poor to 6 artifact; indexes the quality colour table.
    pub quality: u32,
    /// `ItemPrototypeFlags` bits (conjured, lootable, wrapper, …); tooltip lines key on them.
    pub flags: u32,
    /// Copper a vendor charges per [`crate::types::VendorItem::buy_count`] stack.
    pub buy_price: u32,
    /// Copper a vendor pays per unit; 0 is unsellable, the tooltip's "No sell price" line.
    pub sell_price: u32,
    /// `InventoryType`, the equip-slot family (1 head, 21/22 main/off-hand weapon, …).
    pub inventory_type: u32,
    /// Class bitmask; `-1`, all bits, means no restriction.
    pub allowable_class: i32,
    /// Race bitmask; `-1` means no restriction.
    pub allowable_race: i32,
    /// Item level, the `DurabilityCosts.dbc` row for the repair cost.
    pub item_level: u32,
    /// The "Requires Level N" line; 0 means none.
    pub required_level: u32,
    /// `SkillLine.dbc` id; 0 means none.
    pub required_skill: u32,
    /// The minimum [`Self::required_skill`] value.
    pub required_skill_rank: u32,
    /// A `Spell.dbc` id the player must know to use the item.
    pub required_spell: u32,
    pub required_honor_rank: u32,
    pub required_city_rank: u32,
    /// `Faction.dbc` id; 0 means none.
    pub required_rep_faction: u32,
    /// Sent as 0 whenever [`Self::required_rep_faction`] is 0 (`ItemHandler.cpp:321-322`).
    pub required_rep_rank: u32,
    /// `MaxCount`, 0 uncapped; the tooltip's "Unique" lines derive from it and [`Self::flags`].
    pub max_count: u32,
    /// Maximum stack size; 1 does not stack.
    pub stackable: u32,
    /// Slots a bag grants; 0 for anything else.
    pub container_slots: u32,
    /// The 10-slot `ItemStat` block as `(type, value)`, all-zero slots dropped, in wire order
    /// (`ItemModType`: 0 mana, 1 health, 3 agility, 4 strength, 5 intellect, 6 spirit, 7 stamina).
    pub stats: Vec<(u32, i32)>,
    /// The 5-slot `Damage` block, entries with `max > 0` only, in wire order.
    pub damages: Vec<ItemDamage>,
    /// Damage block 0's minimum, whether or not block 0 is in [`Self::damages`].
    pub dmg_min: f32,
    /// Damage block 0's per-hit maximum.
    pub dmg_max: f32,
    /// Damage block 0's school (0 physical, 1 Holy … 6 Arcane).
    pub dmg_type: u32,
    /// The first slot of the wire's 7-wide resistance run.
    pub armor: u32,
    /// The other six resistances: holy, fire, nature, frost, shadow, arcane (`int32` in vmangos).
    pub resistances: [i32; 6],
    /// Attack delay in milliseconds (the tooltip's "Speed" = delay / 1000).
    pub delay_ms: u32,
    /// The projectile a ranged weapon consumes (0 none, 2 arrow, 3 bullet).
    pub ammo_type: u32,
    /// A ranged weapon's range multiplier.
    pub ranged_mod_range: f32,
    /// The 5-slot `ItemSpell` block, `spell_id != 0` only, in wire order; positions here are not
    /// block ordinals, [`ItemSpellEntry::index`] is.
    pub spells: Vec<ItemSpellEntry>,
    /// Block 0's `SpellCharges`, unfiltered: the reference's `template+0x144`.
    pub spell_charges_0: i32,
    /// The first ON_USE (trigger 0) spell block: what a use casts, and the item's cooldown key
    /// (the reference's `GetItemCooldown` scan, `0x6e2ed0`).
    pub use_spell: Option<ItemUseSpell>,
    /// `ItemBondingType` (0 none … 4 quest-bind).
    pub bonding: u32,
    /// Flavor text, the tooltip's italic line; empty for none.
    pub description: String,
    /// A readable item's `PageText.wdb` id; 0 for none.
    pub page_text: u32,
    /// `Languages.dbc` id a readable's text is written in.
    pub language_id: u32,
    /// `PageTextMaterial.dbc` id, the book frame's background.
    pub page_material: u32,
    /// The quest this item starts; 0 for none.
    pub start_quest: u32,
    /// `Lock.dbc` id; nonzero means the item must be picked or keyed open.
    pub lock_id: u32,
    /// `Material.dbc` id, for the item's sounds.
    pub material: u32,
    /// Sheath style, in the values of the descriptor's virtual-item sheath byte.
    pub sheath: u32,
    /// `ItemRandomProperties.dbc` id for an "of the Whale" suffix, rolled per instance.
    pub random_property: u32,
    /// A shield's block value.
    pub block: u32,
    /// `ItemSet.dbc` id; 0 for none.
    pub item_set: u32,
    /// 0 for items without durability.
    pub max_durability: u32,
    /// `AreaTable.dbc` zone the item is bound to; 0 for anywhere.
    pub area: u32,
    /// `Map.dbc` map the item is bound to; 0 for anywhere.
    pub map: u32,
    /// `BagFamily`, the specialised containers an item fits: a mask, which `GetItemFamily`
    /// (`0x510f20`) answers raw and `HasKey` (`0x51ae80`) tests `& 0x100`, the keys; 9 has no key
    /// bit.
    pub bag_family: u32,
    /// `TotemCategory.dbc` row this item counts as for a recipe's tool (a Blacksmith Hammer),
    /// 0 for none; 3.3.5 only.
    pub totem_category: u32,
}

impl ItemInfo {
    /// The reference's `template+0x144 != 0 && != -1` (`-1` unlimited, `0` none), which gates the
    /// use path's mode-`0x20` search for a copy with charges left.
    pub fn has_finite_charges(&self) -> bool {
        self.spell_charges_0 != 0 && self.spell_charges_0 != -1
    }

    /// The first ON_USE spell's block ordinal, the third byte of `CMSG_USE_ITEM`; usually 0, but
    /// an item with an ON_EQUIP block 0 has its on-use in a later block.
    pub fn use_spell_index(&self) -> Option<u8> {
        self.spells.iter().find(|s| s.trigger == 0).map(|s| s.index)
    }

    /// Consumable as the action bar's count means it (reference `IsConsumableAction`,
    /// `0x4e5250`): ammo or thrown, or an ON_USE spell with negative charges, which use the item
    /// up. Neither positive charges nor `Class` count: a mount, with charges 0, shows no count.
    pub fn is_consumable(&self) -> bool {
        const INVTYPE_AMMO: u32 = 0x18;
        const INVTYPE_THROWN: u32 = 0x19;
        matches!(self.inventory_type, INVTYPE_AMMO | INVTYPE_THROWN)
            || self.spells.iter().any(|s| s.trigger == 0 && s.charges < 0)
    }

    /// The reference `PlaceAction`'s only item filter (`0x4e6571`): an on-use spell or anything
    /// equippable, bags included; anything else is refused silently.
    pub fn placeable_on_action_bar(&self) -> bool {
        self.use_spell.is_some() || self.inventory_type != 0
    }

    /// Whether the tooltip shows the green `<Right Click to Open>` line (`0x52e2f8`), given the
    /// instance's `ITEM_FIELD_FLAGS`: lootable and, if locked, unlocked; or a wrapped gift. An
    /// object-less view (a hyperlink, a merchant row) gets no line at all (`0x52e2e0`). Stricter
    /// than [`Self::opens_loot`] on purpose: the click still sends for a locked box.
    pub fn shows_open_line(&self, instance_flags: u32) -> bool {
        let lootable = self.flags & ITEM_FLAG_LOOTABLE != 0
            && (self.lock_id == 0 || instance_flags & ITEM_DYNFLAG_UNLOCKED != 0);
        lootable || self.unwraps_gift(instance_flags)
    }

    /// Whether a right-click sends `CMSG_OPEN_ITEM` to unwrap a gift (`0x5d8d92`). The reference
    /// tests it before the quest-starter and readable arms; the loot arm comes after them.
    pub fn unwraps_gift(&self, instance_flags: u32) -> bool {
        self.flags & ITEM_FLAG_WRAPPER != 0 && instance_flags & ITEM_DYNFLAG_WRAPPED != 0
    }

    /// Whether a right-click arms the gift-wrap cursor: wrapping paper, not a wrapped present.
    /// Nothing is sent (`0x5edea0` locks the paper and sets cursor mode 2); the next left-click on
    /// a container slot sends [`wrap_item`].
    pub fn begins_gift_wrap(&self, instance_flags: u32) -> bool {
        self.flags & ITEM_FLAG_WRAPPER != 0 && instance_flags & ITEM_DYNFLAG_WRAPPED == 0
    }

    /// Whether a right-click sends `CMSG_OPEN_ITEM` to loot the item (`0x5d8f7c`): the bare
    /// template bit with no lock test, so a locked box still sends and the server answers
    /// `EQUIP_ERR_ITEM_LOCKED`.
    pub fn opens_loot(&self) -> bool {
        self.flags & ITEM_FLAG_LOOTABLE != 0
    }
}

/// Template bit: right-click opens a loot window, as on a clam or lockbox (`ItemPrototype.h:66`).
pub const ITEM_FLAG_LOOTABLE: u32 = 0x0000_0004;
/// Template bit for gift wrapping (`ItemPrototype.h:73`).
pub const ITEM_FLAG_WRAPPER: u32 = 0x0000_0200;
/// Instance (`ITEM_FIELD_FLAGS`) bit a lockbox gains once picked or keyed open.
pub const ITEM_DYNFLAG_UNLOCKED: u32 = 0x0000_0004;
/// Instance bit on a wrapped gift; opening it swaps the entry back from `character_gifts`.
pub const ITEM_DYNFLAG_WRAPPED: u32 = 0x0000_0008;

/// One `Damage` block of [`ItemInfo::damages`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemDamage {
    pub min: f32,
    pub max: f32,
    /// 0 physical, 1 Holy … 6 Arcane (`Resistances.dbc` id).
    pub school: u32,
}

/// One template spell block of [`ItemInfo::spells`]. Negative `charges` use the item up when they
/// run out, positive ones leave it (`ItemPrototype::_ItemSpell::SpellCharges`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemSpellEntry {
    /// The template block ordinal (0..4), which `CMSG_USE_ITEM`'s spell byte carries.
    pub index: u8,
    pub spell_id: u32,
    /// `ItemSpelltriggerType`: 0 ON_USE, 1 ON_EQUIP, 2 CHANCE_ON_HIT.
    pub trigger: u32,
    pub charges: i32,
    /// Use-cooldown ms; negative = the spell's own `RecoveryTime`.
    pub cooldown_ms: i32,
    /// Shared-cooldown category (potions 4, …); the wire's resolved value.
    pub category: u32,
    /// Category cooldown ms; negative = the spell's own `CategoryRecoveryTime`.
    pub category_cooldown_ms: i32,
}

/// An item's first ON_USE spell block. The server substitutes the spell's own cooldowns for unset
/// ones (`ItemHandler.cpp:354-380`), but a `-1` can still arrive; negative means the spell's
/// `Spell.dbc` value, as the reference's `StartCooldown` (`0x6e2c60`) reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemUseSpell {
    pub spell_id: u32,
    /// Use-cooldown ms; negative = the spell's own `RecoveryTime`.
    pub cooldown_ms: i32,
    /// Shared-cooldown category (potions 4, …); the wire's resolved value.
    pub category: u32,
    /// Category cooldown ms; negative = the spell's own `CategoryRecoveryTime`.
    pub category_cooldown_ms: i32,
}

/// Body of `CMSG_ITEM_QUERY_SINGLE`: entry, then a full item guid (0 with no instance in hand).
pub fn item_query(entry: u32, guid: u64) -> Vec<u8> {
    let mut body = Vec::with_capacity(12);
    body.extend_from_slice(&entry.to_le_bytes());
    body.extend_from_slice(&guid.to_le_bytes());
    body
}

/// The player's own bag index (`INVENTORY_SLOT_BAG_0`): `slot` then indexes the player's item
/// array, equipment 0-18, bags 19-22, backpack 23-38 (vmangos `Player.h`).
pub const BAG_PLAYER_INVENTORY: u8 = 255;
/// The backpack's first player-array slot (`INVENTORY_SLOT_ITEM_START`).
pub const SLOT_PACK_FIRST: u8 = 23;
/// The first equipped-bag player-array slot (`INVENTORY_SLOT_BAG_START`; bags occupy 19-22).
pub const SLOT_BAG_FIRST: u8 = 19;

/// `SpellCastTargets` mask bits, vmangos `SpellDefines.h`.
const TARGET_FLAG_GAMEOBJECT: u16 = 0x0800;

const TARGET_FLAG_UNIT: u16 = 0x0002;
const TARGET_FLAG_DEST_LOCATION: u16 = 0x0040;
const TARGET_FLAG_SOURCE_LOCATION: u16 = 0x0020;
const TARGET_FLAG_ITEM: u16 = 0x0010;

/// The `SpellCastTargets` block of a `CMSG_USE_ITEM`, the same block a `CMSG_CAST_SPELL` carries:
/// the reference's `SendCast` (`0x6e54f0`) only picks the opcode (`0x6e57d8`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum UseItemTarget {
    /// Mask 0 (`TARGET_FLAG_SELF`): an ordinary consumable; the server resolves the target.
    #[default]
    SelfImplicit,
    /// `TARGET_FLAG_UNIT` and a packed guid: a bandage, a soulstone, an offensive trinket.
    Unit(u64),
    /// `TARGET_FLAG_GAMEOBJECT` and a packed guid: a key used on a lock, which the server honours
    /// only from an item cast (`Spell.cpp:7892`). The mask is `0x0800` alone; the reference's
    /// `BindTarget` (`0x6e5b40`) never writes `TARGET_FLAG_LOCKED`.
    Object(u64),
    /// `TARGET_FLAG_DEST_LOCATION` and three `f32` coords: a thrown item, dynamite or a grenade.
    Dest([f32; 3]),
    /// `TARGET_FLAG_ITEM` and a packed guid: a poison, oil or stone applied to a clicked item.
    Item(u64),
    /// `TARGET_FLAG_SOURCE_LOCATION` and three `f32` coords (`BindLocation`, `0x6e60f0`); only
    /// items carrying spell 265, such as Martin Fury, use it. vmangos reads it before the dest.
    Source([f32; 3]),
    /// A corpse bit and the corpse's packed guid: Goblin Jumper Cables' Defibrillate
    /// (`Targets 0x8000`) on a released player's corpse.
    Corpse(super::spells::CorpseTarget, u64),
}

/// Body of `CMSG_USE_ITEM` (opcode 171): bag index (a bag's player-array slot 19-22, or
/// [`BAG_PLAYER_INVENTORY`]), slot within it, the spell block ordinal, then the targets block.
pub fn use_item(bag_index: u8, slot: u8, spell_slot: u8, target: UseItemTarget) -> Vec<u8> {
    let mut body = Vec::with_capacity(5);
    body.push(bag_index);
    body.push(slot);
    body.push(spell_slot);
    let guid = match target {
        UseItemTarget::SelfImplicit => {
            body.extend_from_slice(&0u16.to_le_bytes());
            return body;
        }
        UseItemTarget::Unit(guid) => {
            body.extend_from_slice(&TARGET_FLAG_UNIT.to_le_bytes());
            guid
        }
        UseItemTarget::Object(guid) => {
            body.extend_from_slice(&TARGET_FLAG_GAMEOBJECT.to_le_bytes());
            guid
        }
        UseItemTarget::Item(guid) => {
            body.extend_from_slice(&TARGET_FLAG_ITEM.to_le_bytes());
            guid
        }
        UseItemTarget::Corpse(corpse, guid) => {
            body.extend_from_slice(&corpse.target_flag().to_le_bytes());
            guid
        }
        UseItemTarget::Dest(dest) => {
            body.extend_from_slice(&TARGET_FLAG_DEST_LOCATION.to_le_bytes());
            for c in dest {
                body.extend_from_slice(&c.to_le_bytes());
            }
            return body;
        }
        UseItemTarget::Source(src) => {
            body.extend_from_slice(&TARGET_FLAG_SOURCE_LOCATION.to_le_bytes());
            for c in src {
                body.extend_from_slice(&c.to_le_bytes());
            }
            return body;
        }
    };
    crate::wire::write_packed_guid(guid, &mut body).expect("write to Vec cannot fail");
    body
}

/// Body of `CMSG_OPEN_ITEM` (opcode 172, `Server/Packets/Spell.h:36-45`): bag index and slot, as
/// in [`use_item`]. The server loots the item on its own guid, or unwraps a gift in place.
pub fn open_item(bag_index: u8, slot: u8) -> Vec<u8> {
    vec![bag_index, slot]
}

/// Body of `CMSG_WRAP_ITEM` (opcode 467, `Server/Packets/Item.cpp:121-127`): the paper's bag and
/// slot, then the target's. The server refuses with `SMSG_INVENTORY_CHANGE_FAILURE`; success is
/// only field updates, the target taking the paper's gift entry and `ITEM_DYNFLAG_WRAPPED`.
pub fn wrap_item(gift_bag: u8, gift_slot: u8, item_bag: u8, item_slot: u8) -> Vec<u8> {
    vec![gift_bag, gift_slot, item_bag, item_slot]
}

#[cfg(test)]
mod tests {
    use super::*;

    // The query-response parse goldens are in `tests/items.rs`; do not duplicate them here.

    // Item-move body goldens follow vmangos `Server/Packets/Item.cpp`; every field is a `u8`.

    #[test]
    fn wrap_item_body_paper_first() {
        // 467: giftbag, giftslot, itembag, itemslot (Item.cpp:121-127).
        assert_eq!(wrap_item(255, 23, 255, 24), vec![255, 23, 255, 24]);
        // Paper in the first bag's slot 3, the target in backpack slot 24: four distinct bytes.
        assert_eq!(wrap_item(19, 3, 255, 24), vec![19, 3, 255, 24]);
    }

    // Reason 1 is `EQUIP_ERR_CANT_EQUIP_LEVEL_I` (`Objects/ItemDefines.h`, `Item.cpp:198-209`).
}
