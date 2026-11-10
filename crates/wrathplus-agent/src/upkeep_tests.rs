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

#[test]
fn a_better_weapon_of_the_same_kind_goes_on_and_a_refused_item_is_left_after_two_tries() {
    let mut script = upkeep();
    // A two-handed sword for a two-handed sword: worn. The hatchet (another kind) stays.
    script
        .run(
            "ITEMS.claymore = { 'Claymore', 1, 13, 1, 'Weapon', 'Two-Handed Swords', 'INVTYPE_2HWEAPON', 900 } \
             bags = { [0] = { 'axe', 'claymore' } }",
        )
        .unwrap();
    for _ in 0..3 {
        script.tick(1.1);
    }
    assert_eq!(calls(&script), "equip claymore 16");
    // A bow the game will not let us wear (no skill): tried twice, then left in the bags.
    script
        .run(
            "calls = {} ITEMS.bow = { 'Cracked Shortbow', 1, 3, 1, 'Weapon', 'Bows', 'INVTYPE_RANGED', 5 } \
             bags = { [0] = { 'bow' } } \
             EquipCursorItem = function(inv) calls[#calls + 1] = 'equip bow ' .. inv end",
        )
        .unwrap();
    script.fire_event("BAG_UPDATE", vec![]);
    for _ in 0..6 {
        script.tick(1.1);
    }
    assert_eq!(calls(&script), "equip bow 18 | equip bow 18");
}

#[test]
fn at_a_merchant_it_buys_food_it_can_eat_within_its_means() {
    let mut script = upkeep();
    script
        .run(
            "ITEMS.jerky = { 'Tough Jerky', 1, 5, 1, 'Consumable', 'Food & Drink', '', 6 } \
             ITEMS.haunch = { 'Haunch of Meat', 1, 15, 5, 'Consumable', 'Food & Drink', '', 31 } \
             ITEMS.water = { 'Refreshing Spring Water', 1, 5, 1, 'Consumable', 'Food & Drink', '', 6 } \
             SHOP = { { 'jerky', 25, 5 }, { 'haunch', 125, 5 }, { 'water', 25, 5 } } \
             GetMerchantNumItems = function() return #SHOP end \
             GetMerchantItemLink = function(i) return SHOP[i][1] end \
             GetMerchantItemInfo = function(i) return ITEMS[SHOP[i][1]][1], '', SHOP[i][2], SHOP[i][3], -1, 1 end \
             GetItemSpell = function(link) if link == 'water' then return 'Drink' end if link == 'jerky' or link == 'haunch' then return 'Food' end end \
             BuyMerchantItem = function(i, n) calls[#calls + 1] = 'buy ' .. SHOP[i][1] money = money - SHOP[i][2] end \
             bags = { [0] = {} } money = 300 Upkeep.sell = false UnitPowerType = function() return 1 end",
        )
        .unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    // Level 4: the jerky (the haunch needs level 5), a third of 300c at 25c a stack of five; a
    // warrior buys no water.
    assert_eq!(
        calls(&script),
        "buy jerky | buy jerky | buy jerky | buy jerky"
    );
    let log: String = script.eval("return Upkeep.log[#Upkeep.log]").unwrap();
    assert_eq!(log, "bought 20 Tough Jerky");
    // Stocked already (the bags hold ten or more): nothing bought.
    script
        .run("calls = {} bags = { [0] = { 'jerky' } } GetContainerItemInfo = function() return nil, 12 end")
        .unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(calls(&script), "");
}

/// Bags as at the live playtest: a backpack, nothing in the bag slots, and bags lying in the pack.
/// Sizes come off the tooltip in the client; here off a table.
const BAGS: &str = r#"
ITEMS.pouch = { "Small Blue Pouch", 1, 5, 1, "Container", "Bag", "INVTYPE_BAG", 25 }
ITEMS.brown = { "Small Brown Pouch", 1, 5, 1, "Container", "Bag", "INVTYPE_BAG", 10 }
ITEMS.big = { "Large Brown Sack", 1, 15, 1, "Container", "Bag", "INVTYPE_BAG", 90 }
ITEMS.soul = { "Small Soul Pouch", 1, 10, 1, "Container", "Soul Bag", "INVTYPE_BAG", 10 }
SIZES = { pouch = 6, brown = 4, big = 8, soul = 6 }
bags = { [0] = {}, {}, {}, {}, {} }
GetContainerNumSlots = function(bag)
  if bag == 0 then return 16 end
  return worn[19 + bag] and SIZES[worn[19 + bag]] or 0
end
GetItemFamily = function(link) return link == "soul" and 256 or 0 end
Upkeep.bag_slots = function(where, a, b)
  if where == "bag" then return SIZES[bags[a][b]] end
  return SIZES[MERCHANT[a].link]
end
"#;

#[test]
fn spare_bags_go_on_largest_first_and_a_larger_one_replaces_the_smallest() {
    let mut script = upkeep();
    script.run(BAGS).unwrap();
    script
        .run("bags[0][3] = 'brown' bags[0][5] = 'pouch' bags[0][7] = 'soul'")
        .unwrap();
    for _ in 0..4 {
        script.tick(1.1);
    }
    // The pouch (6) first, then the brown pouch (4); the soul pouch holds only shards.
    assert_eq!(calls(&script), "equip pouch 20 | equip brown 21");
    // All four slots full: a larger bag takes the smallest one's place.
    script
        .run("calls = {} Upkeep.log = {} worn[22] = 'pouch' worn[23] = 'pouch' bags[0][9] = 'big'")
        .unwrap();
    script.fire_event("BAG_UPDATE", vec![]);
    for _ in 0..3 {
        script.tick(1.1);
    }
    assert_eq!(calls(&script), "equip big 21");
    let log: String = script
        .eval("return table.concat(Upkeep.log, '; ')")
        .unwrap();
    assert_eq!(
        log,
        "equipped Large Brown Sack (8 slots) in place of Small Brown Pouch (4 slots)"
    );
}

/// Full bags at a merchant, as at the live playtest: white trade goods, a quest item, the
/// Hearthstone, a scroll, a white belt it would wear and an axe it would not.
const FULL: &str = r#"
ITEMS.meat = { "Stringy Wolf Meat", 1, 5, 0, "Trade Goods", "Meat", "", 4 }
ITEMS.linen = { "Linen Cloth", 1, 5, 0, "Trade Goods", "Cloth", "", 13 }
ITEMS.malachite = { "Malachite", 1, 7, 0, "Gem", "Simple", "", 5 }
ITEMS.okra = { "Okra", 1, 1, 0, "Quest", "Quest", "", 0 }
ITEMS.hearth = { "Hearthstone", 1, 1, 0, "Miscellaneous", "Junk", "", 5 }
ITEMS.scroll = { "Scroll of Agility", 1, 5, 0, "Consumable", "Scroll", "", 18 }
ITEMS.wbelt = { "Linen Belt", 1, 5, 1, "Armor", "Cloth", "INVTYPE_WAIST", 6 }
IDS = { hearth = 6948 }
bags = { [0] = { "meat", "meat", "linen", "malachite", "okra", "hearth", "scroll", "wbelt", "axe", "claw" } }
GetContainerItemID = function(bag, slot) return IDS[bags[bag][slot]] or 1 end
GetContainerItemQuestInfo = function(bag, slot) return bags[bag][slot] == "okra", nil, nil end
"#;

#[test]
fn full_bags_also_sell_trade_goods_and_gear_it_would_not_wear_keeping_the_rest() {
    let mut script = upkeep();
    script.run(FULL).unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    // The junk first; one freed slot is under two, so the whites it has no use for go too.
    assert_eq!(
        calls(&script),
        "sell claw | sell meat | sell meat | sell linen | sell malachite | sell axe | repair"
    );
    let log: String = script
        .eval("return table.concat(Upkeep.log, '; ')")
        .unwrap();
    assert!(
        log.contains("bags full: sold 5 more for 31c (Stringy Wolf Meat, Linen Cloth, Malachite, Rusty Hatchet)"),
        "{log}"
    );
    let filling: String = script.eval("return Upkeep.filling()").unwrap();
    assert_eq!(
        filling,
        "Hearthstone (hearthstone), Linen Belt (an upgrade), Okra (quest), Scroll of Agility (consumable)"
    );
    // The planner keeps the cloth; with room in the bags only the junk goes.
    script
        .run(&format!(
            "calls = {{}} {FULL} Goals.keep({{ 'Linen Cloth' }})"
        ))
        .unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert!(!calls(&script).contains("sell linen"));
    script
        .run("calls = {} bags = { [0] = { 'meat', 'claw' } } GetContainerNumSlots = function(bag) return bag == 0 and 4 or 0 end")
        .unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(calls(&script), "sell claw | repair");
}

/// A vendor's bags, and the API a merchant window answers with.
const MERCHANT: &str = r#"
MERCHANT = { { link = "brown", price = 400 }, { link = "pouch", price = 1000 }, { link = "axe", price = 50 } }
GetMerchantNumItems = function() return #MERCHANT end
GetMerchantItemLink = function(i) return MERCHANT[i].link end
GetMerchantItemInfo = function(i) local m = MERCHANT[i] return ITEMS[m.link][1], "", m.price, 1, -1, 1 end
BuyMerchantItem = function(i) calls[#calls + 1] = "buy " .. MERCHANT[i].link money = money - MERCHANT[i].price end
"#;

#[test]
fn a_merchant_sells_it_the_largest_bag_when_the_money_left_covers_the_reserve() {
    let mut script = upkeep();
    script.run(BAGS).unwrap();
    script.run(MERCHANT).unwrap();
    // 2000c, 6c of repairs: one 1000c pouch, which leaves more than the 166c reserve (level 4:
    // 160c, and the repairs); a second would not.
    script.run("money = 2000").unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(calls(&script), "repair | buy pouch");
    // Short of money: nothing bought, and the log says why.
    script
        .run("calls = {} Upkeep.log = {} money = 500")
        .unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(calls(&script), "repair");
    let log: String = script
        .eval("return table.concat(Upkeep.log, '; ')")
        .unwrap();
    assert_eq!(
        log,
        "repaired for 6c; did not buy Small Blue Pouch (6 slots) for 1000c: keeping 166c for training and repairs"
    );
    // Every slot holds a bag as large as any it sells: nothing to buy.
    script
        .run("calls = {} money = 5000 for inv = 20, 23 do worn[inv] = 'pouch' end")
        .unwrap();
    script.fire_event("MERCHANT_SHOW", vec![]);
    assert_eq!(calls(&script), "repair");
}
