//! `GetItemCount`'s walk (`0x51c2e0` → `0x754d00`) over the VM's bags, worn slots, bank and
//! currency, and the charge form `includeCharges` asks for.

use super::wrath_spell_use::SpellUse;
use super::Model;

/// `0x7546f0`'s skip, which the `0x80` flag lifts: a copy that is broken (a max durability, 0
/// left, not wrapped `0x8`) or carries `ITEM_FIELD_FLAGS` `0x10`.
fn skipped(flags: u32, durability: Option<(u32, u32)>) -> bool {
    let broken = flags & 8 == 0 && durability.is_some_and(|(cur, max)| max != 0 && cur == 0);
    broken || flags & 0x10 != 0
}

/// Every carried copy of `item`, as `0x754d00` sums `ITEM_FIELD_STACK_COUNT` (`0x754560`) over
/// the slots its flags pass (`0x753a50`): by default (`0x247`, `0x7546f0`) the worn equipment and
/// bag slots, the backpack, the bags' contents, the keyring and the currency slots; `bank` adds
/// the bank and its bags (`8`); `all` (`0x80`) keeps the copies [`skipped`] drops.
pub(crate) fn item_count_with(model: &Model, item: u32, bank: bool, all: bool) -> u32 {
    if item == 0 {
        return 0;
    }
    let keep = |flags, dur| all || !skipped(flags, dur);
    let bags: &[i64] = if bank {
        &[0, 1, 2, 3, 4, -2, -1, 5, 6, 7, 8, 9, 10, 11]
    } else {
        &[0, 1, 2, 3, 4, -2]
    };
    let in_bags: u32 = bags
        .iter()
        .filter_map(|b| model.containers.get(b))
        .flat_map(|c| c.slots.values())
        .filter(|s| s.item_id == item && keep(s.flags, s.durability))
        .map(|s| s.count.max(1))
        .sum();
    let slots = |v: &[Option<super::InvSlotView>]| -> u32 {
        v.iter()
            .flatten()
            .filter(|s| s.item_id == item && keep(s.flags, s.durability))
            .map(|s| s.count.max(1))
            .sum()
    };
    // Slot 0 is the ammo display, a bag sum, not an item.
    let worn = slots(&model.inventory_slots[1..]);
    let bank_bags = if bank {
        slots(&model.bank_bag_slots)
    } else {
        0
    };
    let currency: u32 = model
        .wrath_currency
        .entries
        .iter()
        .filter(|e| !e.is_header && e.item_id == item)
        .map(|e| e.count)
        .sum();
    in_bags + worn + bank_bags + currency
}

/// [`item_count_with`] at the default flags, as `GetSpellCount`'s reagent walk asks (`0x7ffa30`).
pub(crate) fn item_count(model: &Model, item: u32, bank: bool) -> u32 {
    item_count_with(model, item, bank, false)
}

/// `GetItemCount` (`0x51c2e0`): the count, and with `includeCharges` for a template of stack
/// size below 2 (`+0x5c`) whose use spell (or enchant) spends charges, the copies' charges, in
/// turn replaced by the use spell's reagent casts when it has reagents (`0x801770`).
pub(super) fn get_item_count(
    model: &Model,
    state: &SpellUse,
    item: u32,
    bank: bool,
    charges: bool,
) -> u32 {
    let n = item_count_with(model, item, bank, charges);
    if n == 0 || !charges {
        return n;
    }
    if !model
        .item_templates
        .get(&item)
        .is_some_and(|t| t.stackable < 2)
    {
        return n;
    }
    let live = state.items.get(&item).copied().unwrap_or_default();
    let template = state
        .item_facts
        .get(&item)
        .and_then(|f| f.use_spells.first())
        .map(|s| (s.spell, s.charges));
    let (spell, spell_charges) = match template.or(live.enchant_spell) {
        Some(s) => s,
        None => (0, 0),
    };
    let mut n = n;
    if spell_charges < 0 {
        n = live.charges[usize::from(bank)];
    }
    let casts = model
        .spell_catalog
        .get(&spell)
        .map_or(0, |r| super::wrath_spell_use::reagent_casts(model, r));
    if casts != 0 {
        n = casts;
    }
    n
}
