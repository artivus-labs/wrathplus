//! Wrath character-sheet bindings (`PaperDollFrame.lua`, `PetPaperDollFrame.lua`): the equipment
//! flyout's `GetInventoryItemsForSlot` (`0x5e95c0`), `GameTooltip:AddTexture` (`0x61d810`) and the
//! pet stat getters `GetPetSpellBonusDamage` (`0x60e410`), `GetUnitHealthModifier` (`0x6127f0`)
//! and `GetUnitPowerModifier` (`0x612900`).

use mlua::{Lua, ObjectLike, Table, Value, Variadic};

use super::item_stats::{item_usable, ItemTemplateView, PlayerReqState};
use super::object::frame_handle_of;
use super::tooltip::{tip_mut, REG_TOOLTIP_METHODS};
use super::Model;

/// `0xa2d288`: each InventoryType's 0-based equipment-slot bits (index 0..=28, read from the
/// pinned `Wow.exe`). Two-handers, main-hand and off-hand weapons all name both hands.
const SLOT_BITS: [u32; 29] = [
    0, 0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x200, 0xc00, 0x3000, 0x18000, 0x10000,
    0x20000, 0x4000, 0x18000, 0x78_0000, 0x40000, 0x10, 0x18000, 0x18000, 0x10000, 0, 0x20000,
    0x20000, 0, 0x20000,
];
/// 0-based equipment slots the fit test special-cases.
const MAIN_HAND: u32 = 15;
const OFF_HAND: u32 = 16;
const RANGED: u32 = 17;
/// InventoryTypes: one-hand, two-hand, main-hand, off-hand weapon, relic.
const INV_WEAPON: u32 = 13;
const INV_2HWEAPON: u32 = 17;
const INV_WEAPONMAINHAND: u32 = 21;
const INV_WEAPONOFFHAND: u32 = 22;
const INV_RELIC: u32 = 28;
/// Location words (`Constants.lua:183-186`): `ITEM_INVENTORY_LOCATION_PLAYER/BAGS/BANK`.
const LOC_PLAYER: u32 = 0x10_0000;
const LOC_BAGS: u32 = 0x20_0000;
const LOC_BANK: u32 = 0x40_0000;
/// `AddTexture`'s gaps (`0xa246fc`, `0xa24bcc`): ±0.005 in the engine's normalized units, which
/// are 1/1024 of the UI's (the line gap's `-2/(scale·1024)` beside it); read at UI scale 1.
const TEXTURE_GAP: f32 = 0.005 * 1024.0;
/// The tooltip template's `$parentTexture1..10` (`GameTooltipTemplate.xml:161-210`).
const TOOLTIP_TEXTURES: usize = 10;

/// `0x7082b0`: whether a template may go in 0-based `slot`. The ranged slot takes relics exactly
/// when the class has a relic slot; a two-hander or main-hand weapon in the off hand, or an
/// off-hand weapon in the main hand, needs a spell-granted slot (`0x70839d`, Titan's Grip),
/// which is not modelled, so those never list.
fn fits_slot(t: &ItemTemplateView, slot: u32, relic_class: bool) -> bool {
    if slot == RANGED && relic_class != (t.inventory_type == INV_RELIC) {
        return false;
    }
    if t.class == 2
        && ((slot == OFF_HAND && matches!(t.inventory_type, INV_2HWEAPON | INV_WEAPONMAINHAND))
            || (slot == MAIN_HAND && t.inventory_type == INV_WEAPONOFFHAND))
    {
        return false;
    }
    SLOT_BITS
        .get(t.inventory_type as usize)
        .is_some_and(|bits| bits & (1 << slot) != 0)
}

/// `0x5e9250`: the flyout's usable test. Its own legs: a one-hand or off-hand weapon in the off
/// hand needs dual wield, and a zero class or race mask admits everyone. The legs past the skill
/// check are untraced; the shared `0x5ea930` gate supplies them.
fn flyout_usable(
    t: &ItemTemplateView,
    req: &PlayerReqState,
    slot: u32,
    knows_spell: impl Fn(u32) -> bool,
) -> bool {
    if req.level != 0
        && slot == OFF_HAND
        && matches!(t.inventory_type, INV_WEAPON | INV_WEAPONOFFHAND)
        && !req.can_dual_wield
    {
        return false;
    }
    let mut masks = t.clone();
    for mask in [&mut masks.allowable_class, &mut masks.allowable_race] {
        if *mask == 0 {
            *mask = -1;
        }
    }
    item_usable(&masks, req, knows_spell)
}

/// Every `(location, item id)` that fits 0-based `slot`, in the reference's walk order: equipped,
/// backpack, bags 1-4, and with the bank open its slots and bags 5-11. An uncached template is
/// skipped and asked for, as `0x67ca30` queries a miss.
fn items_for_slot(model: &mut Model, slot: u32) -> Vec<(u32, u32)> {
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for s in 1..=19u32 {
        if let Some(v) = model.inv_slot("player", s as usize) {
            candidates.push((LOC_PLAYER | s, v.item_id));
        }
    }
    let bag_slots = |model: &Model, bag: i64| -> Vec<(u32, u32)> {
        let mut slots: Vec<(u32, u32)> = model
            .containers
            .get(&bag)
            .map(|b| b.slots.iter().map(|(&k, v)| (k, v.item_id)).collect())
            .unwrap_or_default();
        slots.sort_unstable();
        slots
    };
    for bag in 0..=4i64 {
        for (k, id) in bag_slots(model, bag) {
            candidates.push((LOC_PLAYER | LOC_BAGS | ((bag as u32) << 8) | k, id));
        }
    }
    if model.bank.is_some() {
        // Bank slot k is inventory index 38 + k (0-based), written as that index plus one.
        for (k, id) in bag_slots(model, -1) {
            candidates.push((LOC_BANK | (39 + k), id));
        }
        for bag in 5..=11i64 {
            for (k, id) in bag_slots(model, bag) {
                candidates.push((LOC_BANK | LOC_BAGS | (((bag - 4) as u32) << 8) | k, id));
            }
        }
    }
    let relic = model.player_req.has_relic_slot;
    let mut out = Vec::new();
    for (location, id) in candidates.into_iter().filter(|&(_, id)| id != 0) {
        let Some(t) = model.item_templates.get(&id) else {
            model.item_stat_asks.insert(id);
            continue;
        };
        let knows = |spell| model.spellbook.slots.iter().any(|s| s.spell_id == spell);
        if fits_slot(t, slot, relic) && flyout_usable(t, &model.player_req, slot, knows) {
            out.push((location, id));
        }
    }
    out
}

/// `AddTexture(file [, minx, maxx, miny, maxy])` on the last line (`0x61c8b0`): up to ten per
/// tooltip, never on line 1. The line indents by the texture's width plus the gap unless the
/// line above already carries one; the next pooled line takes the matching outdent.
fn add_texture(lua: &Lua, this: &Table, file: Option<String>, rect: [f32; 4]) -> mlua::Result<()> {
    let h = frame_handle_of(lua, this)?;
    let (name, index, last, inherit, lefts) = {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let name = model.arena.frame(h).and_then(|f| f.name.clone());
        let t = tip_mut(&mut model, h)?;
        let index = t.texture_lines.len();
        if index >= TOOLTIP_TEXTURES || t.num_lines < 2 {
            return Ok(());
        }
        let last = t.num_lines - 1;
        let inherit = t.texture_lines.last() == Some(&(last - 1));
        (name, index, last, inherit, t.left_lines.clone())
    };
    let Some(name) = name else { return Ok(()) };
    let Ok(Value::Table(texture)) = lua.globals().get(format!("{name}Texture{}", index + 1)) else {
        return Ok(());
    };
    let Ok(Value::Table(line)) = lua.globals().get(format!("{name}TextLeft{}", last + 1)) else {
        return Ok(());
    };
    let width: f32 = texture.call_method("GetWidth", ())?;
    let indent = width + TEXTURE_GAP;
    {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let next = lefts.get(last + 1).map(|&rh| (rh, -indent));
        for (rh, x) in
            std::iter::once((lefts[last], if inherit { 0.0 } else { indent })).chain(next)
        {
            // Each line hangs at the previous one's BOTTOMLEFT; only the x offset moves.
            if let Some(a) = model
                .region_data
                .get_mut(&rh)
                .and_then(|d| d.anchors.first_mut())
            {
                a.x_off = x;
                model.touch_layout_region(rh);
            }
        }
        tip_mut(&mut model, h)?.texture_lines.push(last);
    }
    texture.call_method::<()>("SetTexture", file)?;
    texture.call_method::<()>("SetTexCoord", (rect[0], rect[1], rect[2], rect[3]))?;
    texture.call_method::<()>("ClearAllPoints", ())?;
    texture.call_method::<()>("SetPoint", ("TOPRIGHT", line, "TOPLEFT", -TEXTURE_GAP, 0.0))?;
    texture.call_method::<()>("Show", ())
}

/// `0x6127f0`/`0x612900`: `isstring` or the Usage; the player (`0x4cee50`, 0) or a pet-numbered
/// unit (`0x60c1f0`, `[desc+0x114]`) reads `0x71b8d0`/`0x71b960`. Labelled: only our pet's pair.
fn pet_modifier(lua: &Lua, unit: Value, usage: &'static str, pick: PetPick) -> mlua::Result<f64> {
    let unit = super::binding_abi::string_arg(lua, unit, usage)?;
    let model = lua.app_data_ref::<Model>().expect("model");
    let pet = model.stat_unit(&unit) == super::stat_unit::StatUnit::Pet && model.pet_bar.has_ui;
    let stats = pet.then_some(&model.pet_bar.stats);
    Ok(stats.map_or(0.0, |s| f64::from(pick(s))))
}

type PetPick = fn(&super::PetStats) -> f32;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "GetInventoryItemsForSlot",
        lua.create_function(|lua, (slot, table): (Value, Option<Table>)| {
            let slot = match &slot {
                Value::Integer(n) => u32::try_from(*n - 1).ok(),
                Value::Number(n) => u32::try_from(n.round() as i64 - 1).ok(),
                // The name table lookup; its 0-or-1 base is untraced, so names map as
                // `GetInventorySlotInfo`'s 1-based ids.
                Value::String(s) => super::char_stats::slot_id_by_name(&s.to_str()?)
                    .and_then(|id| u32::try_from(id - 1).ok()),
                _ => None,
            };
            let Some(slot) = slot else {
                return Err(mlua::Error::runtime(
                    "Usage: GetInventoryItemsForSlot(slot [, returnTable])",
                ));
            };
            if slot > 18 {
                return Ok(Value::Nil);
            }
            let found = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                items_for_slot(&mut model, slot)
            };
            let table = match table {
                Some(t) => t,
                None => lua.create_table()?,
            };
            for (location, id) in found {
                table.raw_set(location, id)?;
            }
            Ok(Value::Table(table))
        })?,
    )?;
    g.set(
        "GetPetSpellBonusDamage",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(f64::from(model.wrath_paperdoll_stats.pet_spell_power))
        })?,
    )?;
    g.set(
        "GetUnitHealthModifier",
        lua.create_function(|lua, unit: Value| {
            let usage = r#"Usage: GetUnitHealthModifier("unit")"#;
            pet_modifier(lua, unit, usage, |s| s.health_modifier)
        })?,
    )?;
    g.set(
        "GetUnitPowerModifier",
        lua.create_function(|lua, unit: Value| {
            let usage = r#"Usage: GetUnitPowerModifier("unit")"#;
            pet_modifier(lua, unit, usage, |s| s.power_modifier)
        })?,
    )?;
    let tooltips: Table = lua.named_registry_value(REG_TOOLTIP_METHODS)?;
    tooltips.set(
        "AddTexture",
        lua.create_function(
            |lua, (this, file, rect): (Table, Option<String>, Variadic<Value>)| {
                let rect = match rect.as_slice() {
                    [] => [0.0, 1.0, 0.0, 1.0],
                    [a, b, c, d, ..] => {
                        let n = |v: &Value| match v {
                            Value::Integer(i) => Some(*i as f32),
                            Value::Number(f) => Some(*f as f32),
                            _ => None,
                        };
                        match (n(a), n(b), n(c), n(d)) {
                            (Some(a), Some(b), Some(c), Some(d)) => [a, b, c, d],
                            _ => return Err(add_texture_usage()),
                        }
                    }
                    _ => return Err(add_texture_usage()),
                };
                add_texture(lua, &this, file, rect)
            },
        )?,
    )?;
    Ok(())
}

fn add_texture_usage() -> mlua::Error {
    mlua::Error::runtime("Usage: GameTooltip:AddTexture(\"filename\" [, minx, maxx, miny, maxy])")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(class: u32, inventory_type: u32) -> ItemTemplateView {
        ItemTemplateView {
            class,
            inventory_type,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        }
    }

    #[test]
    fn slot_bits_follow_the_native_table_and_hand_rules() {
        let ring = template(4, 11);
        assert!(fits_slot(&ring, 10, false) && fits_slot(&ring, 11, false));
        assert!(!fits_slot(&ring, 12, false));
        let robe = template(4, 20);
        assert!(fits_slot(&robe, 4, false));
        let one_hand = template(2, INV_WEAPON);
        assert!(fits_slot(&one_hand, MAIN_HAND, false) && fits_slot(&one_hand, OFF_HAND, false));
        let two_hand = template(2, INV_2HWEAPON);
        assert!(fits_slot(&two_hand, MAIN_HAND, false));
        assert!(
            !fits_slot(&two_hand, OFF_HAND, false),
            "no Titan's Grip leg"
        );
        let off_weapon = template(2, INV_WEAPONOFFHAND);
        assert!(
            !fits_slot(&off_weapon, MAIN_HAND, false) && fits_slot(&off_weapon, OFF_HAND, false)
        );
        let relic = template(4, INV_RELIC);
        let bow = template(2, 15);
        assert!(fits_slot(&relic, RANGED, true) && !fits_slot(&relic, RANGED, false));
        assert!(fits_slot(&bow, RANGED, false) && !fits_slot(&bow, RANGED, true));
        assert!(!fits_slot(&template(4, 99), 0, false));
    }

    #[test]
    fn usable_adds_dual_wield_and_open_zero_masks() {
        let req = PlayerReqState {
            level: 80,
            class_id: 1,
            race_id: 1,
            ..Default::default()
        };
        let one_hand = template(2, INV_WEAPON);
        assert!(flyout_usable(&one_hand, &req, MAIN_HAND, |_| false));
        assert!(!flyout_usable(&one_hand, &req, OFF_HAND, |_| false));
        let wielder = PlayerReqState {
            can_dual_wield: true,
            ..req.clone()
        };
        assert!(flyout_usable(&one_hand, &wielder, OFF_HAND, |_| false));
        let open = ItemTemplateView {
            allowable_class: 0,
            allowable_race: 0,
            ..template(4, 1)
        };
        assert!(flyout_usable(&open, &req, 0, |_| false));
        let priest_only = ItemTemplateView {
            allowable_class: 1 << 4,
            ..template(4, 1)
        };
        assert!(!flyout_usable(&priest_only, &req, 0, |_| false));
    }

    use crate::script::{
        BankState, ContainerSlot, ContainerState, InvSlotView, PetStats, UiScript,
    };

    fn wrath() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_player_req_state(PlayerReqState {
            level: 80,
            class_id: 1,
            race_id: 1,
            ..Default::default()
        });
        s
    }

    fn bag(items: &[(u32, u32)]) -> Option<ContainerState> {
        Some(ContainerState {
            num_slots: 16,
            slots: items
                .iter()
                .map(|&(k, id)| {
                    (
                        k,
                        ContainerSlot {
                            item_id: id,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        })
    }

    #[test]
    fn the_flyout_walk_packs_native_locations_and_gates_the_bank() {
        let mut s = wrath();
        s.set_item_template(100, template(4, 1)); // helm
        s.set_item_template(101, template(4, 1));
        s.set_item_template(102, template(4, 7)); // legs: never a head candidate
        s.set_item_template(103, template(4, 1));
        let mut slots: crate::script::InventorySlots = Default::default();
        slots[1] = Some(InvSlotView {
            item_id: 100,
            ..Default::default()
        });
        s.set_inventory_slots(slots);
        s.set_container(0, bag(&[(3, 101), (4, 102)]));
        s.set_container(2, bag(&[(5, 103), (6, 999)]));
        s.set_container(-1, bag(&[(2, 103)]));
        s.set_container(6, bag(&[(1, 101)]));
        let read = |s: &UiScript| {
            s.eval::<String>(
                "local t, out = GetInventoryItemsForSlot(1), {} \
                 for loc, id in pairs(t) do out[#out+1] = string.format('%x=%d', loc, id) end \
                 table.sort(out) return table.concat(out, ' ')",
            )
            .unwrap()
        };
        assert_eq!(read(&s), "100001=100 300003=101 300205=103");
        s.set_bank(Some(BankState::default()));
        assert_eq!(
            read(&s),
            "100001=100 300003=101 300205=103 400029=103 600201=101"
        );
        // The uncached id was asked for; a caller table is filled and returned.
        assert!(s.take_item_stat_asks().contains(&999));
        s.run(
            "local mine = {} assert(GetInventoryItemsForSlot(1, mine) == mine and mine[0x100001] == 100) \
             assert(GetInventoryItemsForSlot(20) == nil) \
             assert(not pcall(GetInventoryItemsForSlot)) \
             assert(GetInventoryItemsForSlot('HeadSlot')[0x100001] == 100)",
        )
        .unwrap();
    }

    #[test]
    fn add_texture_indents_its_line_and_clear_restores_it() {
        let s = wrath();
        s.run(
            r#"
            local tt = CreateFrame("GameTooltip", "TT")
            for i = 1, 10 do local t = tt:CreateTexture("TTTexture" .. i) t:SetWidth(12) t:Hide() end
            tt:SetOwner(CreateFrame("Frame"), "ANCHOR_NONE")
            tt:AddLine("one") tt:AddTexture("first.blp")
            assert(not TTTexture1:IsShown(), "line 1 never takes a texture")
            tt:AddLine("two") tt:AddTexture("a.blp")
            tt:AddLine("three") tt:AddTexture("b.blp", 0, .5, 0, .5)
            assert(TTTexture1:IsShown() and TTTexture2:IsShown())
            assert(TTTexture1:GetTexture():lower():find("a"))
            local p, rel, rp, x = TTTextLeft2:GetPoint(1)
            assert(rel == TTTextLeft1 and rp == "BOTTOMLEFT" and math.abs(x - (12 + 5.12)) < 1e-4)
            p, rel, rp, x = TTTextLeft3:GetPoint(1)
            assert(x == 0, "the line under a textured line inherits its indent")
            p, rel, rp, x = TTTexture2:GetPoint(1)
            assert(p == "TOPRIGHT" and rel == TTTextLeft3 and rp == "TOPLEFT")
            assert(math.abs(x + 5.12) < 1e-4)
            assert(not pcall(tt.AddTexture, tt, "x.blp", 1, 2))
            tt:ClearLines()
            assert(not TTTexture1:IsShown() and not TTTexture2:IsShown())
            p, rel, rp, x = TTTextLeft2:GetPoint(1)
            assert(x == 0)
            "#,
        )
        .unwrap();
    }

    #[test]
    fn pet_getters_read_the_pushed_pet_and_player_words() {
        let mut s = wrath();
        s.run("assert(GetPetSpellBonusDamage() == 0 and GetUnitHealthModifier('pet') == 0)")
            .unwrap();
        let stats = crate::script::WrathPaperDollStats {
            pet_spell_power: 321,
            ..Default::default()
        };
        s.set_wrath_paperdoll_stats(stats);
        s.set_unit_guids(&crate::script::UnitGuids {
            player: 1,
            pet: 2,
            ..Default::default()
        });
        s.set_pet_stats(
            true,
            PetStats {
                health_modifier: 1.5,
                power_modifier: 0.75,
                ..Default::default()
            },
        );
        s.run(
            "assert(GetPetSpellBonusDamage() == 321) \
             assert(GetUnitHealthModifier('pet') == 1.5 and GetUnitPowerModifier('PET') == 0.75) \
             assert(GetUnitHealthModifier('player') == 0 and GetUnitHealthModifier('playerpet') == 1.5)",
        )
        .unwrap();
        // A non-string raises the Usage (`0xa237f4`); a number reads as its text, naming nobody.
        let err = s.run("GetUnitPowerModifier()").unwrap_err().to_string();
        assert!(
            err.contains(r#"Usage: GetUnitPowerModifier("unit")"#),
            "{err}"
        );
        s.run("assert(GetUnitHealthModifier(12) == 0)").unwrap();
        s.set_pet_stats(false, PetStats::default());
        s.run("assert(GetUnitHealthModifier('pet') == 0)").unwrap();
    }
}
