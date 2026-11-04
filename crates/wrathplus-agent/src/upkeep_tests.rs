//! `wp.upkeep` against a stubbed character: what it wears, sells, repairs and trains.

use wrathplus_ui::script::{ScriptValue, UiScript};

/// A level 4 warrior with a two-hander, and bags from the playtest. Items are links to a table.
const GAME: &str = r#"
calls = {}
ITEMS = {
  belt = { "Outfitter Belt", 1, 5, 1, "Armor", "Leather", "INVTYPE_WAIST", 4 },
  junkbelt = { "Ragged Leather Belt", 0, 2, 1, "Armor", "Leather", "INVTYPE_WAIST", 4 },
  plate = { "Old Plate Helm", 2, 30, 1, "Armor", "Plate", "INVTYPE_HEAD", 90 },
  shield = { "Pikeman Shield", 1, 5, 1, "Armor", "Shields", "INVTYPE_SHIELD", 10 },
  claw = { "Chipped Claw", 0, 1, 0, "Miscellaneous", "Junk", "", 3 },
  sword = { "Worn Greatsword", 1, 2, 1, "Weapon", "Two-Handed Swords", "INVTYPE_2HWEAPON", 5 },
  axe = { "Rusty Hatchet", 1, 3, 1, "Weapon", "One-Handed Axes", "INVTYPE_WEAPON", 5 },
}
bags = { [0] = { "junkbelt", "belt", "plate", "shield", "claw", "axe" } }
worn = { [16] = "sword" }
money, combat = 0, false
GetItemInfo = function(link)
  local i = ITEMS[link]
  if not i then return nil end
  return i[1], link, i[2], i[3], i[4], i[5], i[6], 1, i[7], "", i[8]
end
GetContainerNumSlots = function(bag) return bags[bag] and #bags[bag] or 0 end
GetContainerItemLink = function(bag, slot) return bags[bag] and bags[bag][slot] or nil end
GetContainerItemInfo = function() return nil, 1 end
GetInventoryItemLink = function(_, inv) return worn[inv] end
IsEquippableItem = function(link) return ITEMS[link] and ITEMS[link][7] ~= "" end
UnitClass = function() return "Warrior", "WARRIOR" end
UnitLevel = function() return 4 end
UnitAffectingCombat = function() return combat end
UnitIsDeadOrGhost = function() return false end
picked = nil
PickupContainerItem = function(bag, slot) picked = { bag, slot } end
EquipCursorItem = function(inv)
  local link = bags[picked[1]][picked[2]]
  calls[#calls + 1] = "equip " .. link .. " " .. inv
  bags[picked[1]][picked[2]] = worn[inv]
  worn[inv] = link
end
UseContainerItem = function(bag, slot)
  calls[#calls + 1] = "sell " .. bags[bag][slot]
  money = money + ITEMS[bags[bag][slot]][8]
  bags[bag][slot] = nil
end
GetMoney = function() return money end
CanMerchantRepair = function() return true end
GetRepairAllCost = function() return 6 end
RepairAllItems = function() calls[#calls + 1] = "repair" end
TRAINER = { { "Charge", "", "available", 10 }, { "Rend", "Rank 1", "available", 10 }, { "Thunder Clap", "", "unavailable", 100 } }
IsTradeskillTrainer = function() return false end
GetNumTrainerServices = function() return #TRAINER end
GetTrainerServiceInfo = function(i) return TRAINER[i][1], TRAINER[i][2], TRAINER[i][3] end
GetTrainerServiceCost = function(i) return TRAINER[i][4] end
BuyTrainerService = function(i) calls[#calls + 1] = "train " .. TRAINER[i][1] money = money - TRAINER[i][4] end
"#;

fn upkeep() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-upkeep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn calls(script: &UiScript) -> String {
    script.eval("return table.concat(calls, ' | ')").unwrap()
}

#[test]
fn it_wears_upgrades_sells_junk_repairs_and_trains() {
    let mut script = upkeep();
    // Out of combat it fills the empty waist with the better belt; the plate helm is too heavy at
    // level 4, the shield cannot go beside a two-hander, and the axe never replaces a weapon.
    for _ in 0..4 {
        script.tick(1.1);
    }
    assert_eq!(calls(&script), "equip belt 6");
    // At a merchant: the junk goes (the ragged belt, now in the bag, and the claw), then a repair.
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(
        calls(&script),
        "equip belt 6 | sell junkbelt | sell claw | repair"
    );
    // At a class trainer: what is available and paid for (7c of junk money covers nothing, so give
    // it some first).
    script.run("money = 25 calls = {}").unwrap();
    script.fire_event("TRAINER_SHOW", vec![]);
    assert_eq!(calls(&script), "train Charge | train Rend");
    let log: String = script
        .eval("return table.concat(Upkeep.log, '; ')")
        .unwrap();
    assert_eq!(
        log,
        "equipped Outfitter Belt; sold 2 junk for 7c; repaired for 6c; trained Charge, Rend (Rank 1)"
    );
}

#[test]
fn nothing_is_equipped_in_combat_and_switches_turn_it_off() {
    let mut script = upkeep();
    script.run("combat = true").unwrap();
    for _ in 0..3 {
        script.tick(1.1);
    }
    assert_eq!(calls(&script), "");
    script.run("combat = false Upkeep.sell = false").unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(calls(&script), "");
}

/// Group loot: rolls offered on the Journeyman's Pants (cloth, an upgrade over the recruit's) and
/// a sword it would not wear over its greatsword.
const ROLLS: &str = r#"
ITEMS.pants = { "Journeyman's Pants", 1, 5, 1, "Armor", "Cloth", "INVTYPE_LEGS", 8 }
ITEMS.rpants = { "Recruit's Pants", 1, 1, 1, "Armor", "Cloth", "INVTYPE_LEGS", 1 }
ITEMS.blade = { "Rusty Blade", 1, 4, 1, "Weapon", "One-Handed Swords", "INVTYPE_WEAPON", 9 }
worn[7] = "rpants"
bags = { [0] = {} }
ROLL = { [11] = "pants", [12] = "blade" }
GetLootRollItemInfo = function(id)
  local i = ITEMS[ROLL[id]]
  return "icon", i[1], 1, i[2], id == 12, true, true
end
GetLootRollItemLink = function(id) return ROLL[id] end
RollOnLoot = function(id, kind) calls[#calls + 1] = "roll " .. id .. " " .. kind end
ConfirmLootRoll = function(id, kind) calls[#calls + 1] = "confirm " .. id .. " " .. kind end
"#;

#[test]
fn it_needs_on_upgrades_greeds_on_the_rest_and_confirms_only_its_own_rolls() {
    let mut script = upkeep();
    script.run(ROLLS).unwrap();
    script.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(11), ScriptValue::Int(60000)],
    );
    script.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(12), ScriptValue::Int(60000)],
    );
    assert_eq!(calls(&script), "roll 11 1 | roll 12 2");
    // The blade binds on pickup: its confirmation is for the greed it rolled, not a need.
    script.fire_event(
        "CONFIRM_LOOT_ROLL",
        vec![ScriptValue::Int(12), ScriptValue::Int(1)],
    );
    script.fire_event(
        "CONFIRM_LOOT_ROLL",
        vec![ScriptValue::Int(12), ScriptValue::Int(2)],
    );
    assert_eq!(calls(&script), "roll 11 1 | roll 12 2 | confirm 12 2");
    let log: String = script
        .eval("return table.concat(Upkeep.log, '; ')")
        .unwrap();
    assert_eq!(
        log,
        "needed Journeyman's Pants (an upgrade); greeded Rusty Blade"
    );
    // Left to the agent: no roll at all.
    script.run("Upkeep.loot = false calls = {}").unwrap();
    script.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(11), ScriptValue::Int(60000)],
    );
    assert_eq!(calls(&script), "");
}

#[test]
fn its_own_class_trainer_is_found_not_just_any() {
    let script = upkeep();
    script
        .run(
            "AgentNearby = function() return { \
               { name = 'Priestess Josetta', title = 'Priest Trainer', npc = { 'trainer', 'classtrainer' }, distance = 4 }, \
               { name = 'Lyria Du Lac', title = 'Warrior Trainer', npc = { 'trainer', 'classtrainer' }, distance = 12 } } end",
        )
        .unwrap();
    let name: String = script.eval("return Upkeep.trainer().name").unwrap();
    assert_eq!(name, "Lyria Du Lac");
}

#[test]
fn a_visit_with_nothing_bought_says_why() {
    let mut script = upkeep();
    // 5c in hand: Charge and Rend (10c each) are too dear.
    script.run("money = 5").unwrap();
    script.fire_event("TRAINER_SHOW", vec![]);
    // All bought already: nothing new, until Thunder Clap's level.
    script
        .run("TRAINER = { { 'Thunder Clap', '', 'unavailable', 100 } } GetTrainerServiceLevelReq = function() return 6 end")
        .unwrap();
    script.fire_event("TRAINER_SHOW", vec![]);
    let log: String = script
        .eval("return table.concat(Upkeep.log, '; ')")
        .unwrap();
    assert_eq!(
        log,
        "nothing affordable at the trainer: 20c of skills, 5c in hand; nothing new at the trainer until level 6"
    );
}
