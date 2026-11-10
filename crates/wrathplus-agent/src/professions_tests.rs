//! `wp.professions` against a stubbed level 11 warrior: the skill lines, Cooking and First Aid
//! trainers seen and visited, the tradeskill window, crafting at a fire, bandages, food, keeping
//! reagents.

use super::goals_tests::{goals, text, ticks};
use wrathplus_ui::script::{ScriptValue, UiScript};

/// The stock API around professions: skill lines (Secondary Skills folded), bags of items by link,
/// a trainer window, a tradeskill window opened by casting the profession, auras.
const PROFESSIONS: &str = r##"
lvl, money = 11, 2000
SKILLS = { { "Weapon Skills", true }, { "Swords", false, 30, 55 }, { "Secondary Skills", true } }
folded = false
IsUsableItem = function() return true end
GetNumSkillLines = function()
  local n = 0
  for _, s in ipairs(SKILLS) do if not folded or s[2] or s[1] == "Swords" then n = n + 1 end end
  return n
end
GetSkillLineInfo = function(i)
  local rows = {}
  for _, s in ipairs(SKILLS) do if not folded or s[2] or s[1] == "Swords" then rows[#rows + 1] = s end end
  local s = rows[i]
  if not s then return nil end
  if s[2] then return s[1], 1, (not folded or s[1] ~= "Secondary Skills") and 1 or nil, 0, 0, 0, 0 end
  return s[1], nil, nil, s[3], 0, 0, s[4]
end
ExpandSkillHeader = function() calls[#calls + 1] = "expand skills" folded = false end
function learn(name, rank, max) SKILLS[#SKILLS + 1] = { name, false, rank, max } end
-- name, quality, ilevel, minlevel, class, subclass, use spell
ITEMS = {
  wolf = { "Stringy Wolf Meat", 1, 5, 0, "Trade Goods", "Meat" },
  linen = { "Linen Cloth", 1, 5, 0, "Trade Goods", "Cloth" },
  charred = { "Charred Wolf Meat", 1, 5, 1, "Consumable", "Food & Drink", "Food" },
  jerky = { "Tough Jerky", 1, 5, 5, "Consumable", "Food & Drink", "Food" },
  bread = { "Haunch of Meat", 1, 15, 15, "Consumable", "Food & Drink", "Food" },
  bandage = { "Linen Bandage", 1, 5, 0, "Consumable", "Bandage", "First Aid" },
  claw = { "Chipped Claw", 0, 1, 0, "Miscellaneous", "Junk" },
}
bags = { [0] = {} }
counts = {}
function carry(link, n) local b = bags[0] b[#b + 1] = link counts[#b] = n end
GetContainerNumSlots = function(bag) return bags[bag] and #bags[bag] or 0 end
GetContainerItemLink = function(bag, slot) return bags[bag] and bags[bag][slot] end
GetContainerItemInfo = function(bag, slot) return nil, counts[slot] or 1 end
GetItemInfo = function(link)
  local i = ITEMS[link]
  if not i then
    for k, v in pairs(ITEMS) do if v[1] == link then i, link = v, k end end
  end
  if not i then return nil end
  return i[1], link, i[2], i[3], i[4], i[5], i[6], 20, "", "", 1
end
GetItemSpell = function(link) local i = ITEMS[link] return i and i[7] end
UseContainerItem = function(bag, slot, unit)
  calls[#calls + 1] = "use " .. bags[bag][slot] .. (unit and (" on " .. unit) or "")
end
GetMoney = function() return money end
AURAS = {}
UnitAura = function(_, i) return AURAS[i] end
SPELLS = { "Attack", "Battle Stance" }
GetSpellName = function(i) return SPELLS[i] end
CastSpellByName = function(name)
  calls[#calls + 1] = "cast " .. name
  if name == "Cooking" or name == "First Aid" then line = name end
end
-- The tradeskill window: recipes as name, difficulty, reagent link, need, tool.
line = nil
RECIPES = {
  { "Charred Wolf Meat", "optimal", "wolf", 1, "Cooking Fire" },
  { "Spice Bread", "trivial", "flour", 1, "Cooking Fire" },
}
PRODUCT = { ["Charred Wolf Meat"] = "charred", ["Linen Bandage"] = "bandage" }
local function have(link) local n = 0 for s, l in ipairs(bags[0]) do if l == link then n = n + (counts[s] or 1) end end return n end
GetTradeSkillLine = function() if line then return line, 1, 75 end return "UNKNOWN", 0, 0 end
GetNumTradeSkills = function() return line and #RECIPES or 0 end
GetTradeSkillInfo = function(i)
  local r = RECIPES[i]
  return r[1], r[2], math.floor(have(r[3]) / r[4])
end
GetTradeSkillNumReagents = function() return 1 end
GetTradeSkillReagentInfo = function(i) local r = RECIPES[i] return ITEMS[r[3]] and ITEMS[r[3]][1] or r[3], nil, r[4], have(r[3]) end
GetTradeSkillTools = function(i) if RECIPES[i][5] then return RECIPES[i][5], 1 end end
GetTradeSkillItemLink = function(i) return PRODUCT[RECIPES[i][1]] end
DoTradeSkill = function(i, n) calls[#calls + 1] = "make " .. RECIPES[i][1] .. " x" .. n end
CloseTradeSkill = function() calls[#calls + 1] = "close book" line = nil end
-- A tradeskill trainer: name, rank text, category, cost, level.
SERVICES = {}
IsTradeskillTrainer = function() return true end
GetNumTrainerServices = function() return #SERVICES end
GetTrainerServiceInfo = function(i) local s = SERVICES[i] return s[1], s[2], s[3] end
GetTrainerServiceCost = function(i) return SERVICES[i][4] end
GetTrainerServiceLevelReq = function(i) return SERVICES[i][5] or 0 end
GetTrainerServiceSkillReq = function(i) return SERVICES[i][6] end
BuyTrainerService = function(i) calls[#calls + 1] = "train " .. SERVICES[i][1] money = money - SERVICES[i][4] end
CloseTrainer = function() calls[#calls + 1] = "close trainer" end
UnitName = function(u) if u == "npc" then return npc end return "Agentfour" end
tomas = { guid = "0xT", name = "Tomas", title = "Cook", kind = "unit", reaction = "friendly",
  npc = { "gossip", "trainer", "professiontrainer" }, x = 30, y = 0, z = 0, distance = 30, dead = false }
michelle = { guid = "0xM", name = "Michelle Belle", title = "Physician", kind = "unit", reaction = "friendly",
  npc = { "gossip", "trainer", "professiontrainer" }, x = 40, y = 0, z = 0, distance = 40, dead = false }
function said_calls() return table.concat(calls, " | ") end
"##;

fn professions() -> UiScript {
    let script = goals();
    script.run(PROFESSIONS).unwrap();
    script
}

fn calls(script: &UiScript) -> String {
    text(script, "return said_calls()")
}

#[test]
fn it_reads_the_professions_and_remembers_their_trainers_from_sight() {
    let script = professions();
    // Secondary Skills is folded: it is opened to read, and nothing is known yet.
    script
        .run("folded = true learn('Cooking', 12, 75)")
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local k = Professions.known().Cooking return k.rank .. '/' .. k.max"
        ),
        "12/75"
    );
    assert_eq!(calls(&script), "expand skills");
    script.run("SKILLS[#SKILLS] = nil calls = {}").unwrap();
    // Tomas the Cook and Michelle Belle the Physician (Goldshire's trainers, by their titles).
    script.run("units = { tomas, michelle }").unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "learn_professions"
    );
    assert_eq!(
        text(
            &script,
            "return Goals.places.trainer_cooking.name .. ', ' .. Goals.places.trainer_first_aid.name"
        ),
        "Tomas, Michelle Belle"
    );
    // A class trainer's title or another trade's is no cook.
    assert_eq!(
        text(&script, "return tostring((Professions.title_of('Cooking Trainer'))) .. ' ' .. tostring((Professions.title_of('Blacksmithing Trainer'))) .. ' ' .. tostring((Professions.title_of('First Aid Trainer')))"),
        "Cooking nil First Aid"
    );
    // Both known and below their caps: nothing to learn.
    script
        .run("learn('Cooking', 12, 75) learn('First Aid', 1, 75)")
        .unwrap();
    assert_eq!(
        text(
            &script,
            "return tostring(Goals.facts().profession_to_learn)"
        ),
        "false"
    );
}

#[test]
fn at_a_cooking_trainer_it_learns_the_rank_then_recipes_and_notes_the_cap() {
    let mut script = professions();
    script
        .run(
            "units = { tomas } npc = 'Tomas' \
             SERVICES = { { 'Cooking', 'Apprentice', 'available', 100 }, { 'Spice Bread', '', 'unavailable', 10, 0, 'Cooking' }, \
                          { 'Kaldorei Spider Kabob', '', 'unavailable', 1000, 0, 'Cooking' } }",
        )
        .unwrap();
    script.fire_event("TRAINER_SHOW", vec![]);
    assert_eq!(calls(&script), "train Cooking");
    // The server confirms: the recipes open up; the bread fits a quarter of the money, the kabob not.
    script
        .run("learn('Cooking', 1, 75) SERVICES[1][3] = 'used' SERVICES[2][3] = 'available' SERVICES[3][3] = 'available'")
        .unwrap();
    script.fire_event("TRAINER_UPDATE", vec![]);
    assert_eq!(calls(&script), "train Cooking | train Spice Bread");
    script.fire_event("TRAINER_UPDATE", vec![]);
    assert_eq!(
        calls(&script),
        "train Cooking | train Spice Bread | close trainer"
    );
    assert_eq!(
        text(&script, "return table.concat(Professions.log, '; ')"),
        "learned Cooking (Apprentice), Spice Bread"
    );
    // Later, at the cap of 75 with no Journeyman rank here: Tomas is noted as unable to help.
    script
        .run("SKILLS[#SKILLS] = nil learn('Cooking', 75, 75) calls = {} SERVICES = { { 'Cooking', 'Apprentice', 'used', 100 } }")
        .unwrap();
    assert_eq!(
        text(
            &script,
            "return tostring(Goals.facts().profession_to_learn)"
        ),
        "true"
    );
    script.run("npc = 'Tomas'").unwrap();
    script.fire_event("TRAINER_SHOW", vec![]);
    assert_eq!(calls(&script), "close trainer");
    assert_eq!(
        text(&script, "return tostring(Professions.notes.Tomas.cap)"),
        "75"
    );
    // First Aid is still wanted, but no trainer for it is known: nothing to learn here now.
    assert_eq!(
        text(
            &script,
            "return tostring(Goals.facts().profession_to_learn)"
        ),
        "false"
    );
}

#[test]
fn it_cooks_a_batch_at_a_campfire_it_lights_and_stops_at_the_stock() {
    let mut script = professions();
    script
        .run("learn('Cooking', 1, 75) carry('wolf', 6) carry('claw', 1) SPELLS[#SPELLS + 1] = 'Basic Campfire'")
        .unwrap();
    // Six meat and no food: a batch to cook, first in the plan.
    assert_eq!(
        text(
            &script,
            "local f = Goals.facts() return tostring(f.can_cook) .. ' ' .. tostring(f.food_low)"
        ),
        "true true"
    );
    assert_eq!(text(&script, "return plan_of('progress')"), "craft");
    // Meat to cook is food enough: no errand to buy any.
    assert_eq!(
        text(&script, "return tostring(Goals.facts().needs_food)"),
        "false"
    );
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:craft");
    // The window opens on the cast; the recipe wants a fire and none is in sight: it lights one.
    assert_eq!(calls(&script), "cast Cooking | cast Basic Campfire");
    script.fire_event(
        "UNIT_SPELLCAST_SUCCEEDED",
        vec![
            ScriptValue::Str("player".into()),
            ScriptValue::Str("Basic Campfire".into()),
        ],
    );
    ticks(&mut script, 0.75);
    assert_eq!(
        calls(&script),
        "cast Cooking | cast Basic Campfire | make Charred Wolf Meat x6"
    );
    for _ in 0..6 {
        script.fire_event(
            "UNIT_SPELLCAST_SUCCEEDED",
            vec![
                ScriptValue::Str("player".into()),
                ScriptValue::Str("Charred Wolf Meat".into()),
            ],
        );
    }
    script.run("bags[0][1] = 'charred' counts[1] = 6").unwrap();
    ticks(&mut script, 0.75);
    // The book closes and the runner goes on (here, to look for mobs).
    assert!(
        calls(&script).contains("make Charred Wolf Meat x6 | close book"),
        "{}",
        calls(&script)
    );
    assert_eq!(
        text(&script, "return table.concat(Professions.log, '; ')"),
        "made 6 Charred Wolf Meat (Cooking 1/75)"
    );
    // The meat is gone: nothing more to cook, and craft is off the plan.
    assert_eq!(
        text(&script, "return tostring(Goals.facts().can_cook)"),
        "false"
    );
    // Two meat for a stock 14 short is no batch; within one of the stock, one is.
    script.run("carry('wolf', 2)").unwrap();
    assert_eq!(
        text(&script, "return tostring(Goals.facts().can_cook)"),
        "false"
    );
    script.run("counts[1] = 19").unwrap();
    assert_eq!(
        text(&script, "return tostring(Goals.facts().can_cook)"),
        "true"
    );
}

#[test]
fn a_refused_cast_at_a_forge_lights_a_fire() {
    let mut script = professions();
    script
        .run(
            "learn('Cooking', 1, 75) carry('wolf', 3) SPELLS[#SPELLS + 1] = 'Basic Campfire' \
             units = { { guid = '0xF', name = 'Forge', kind = 'gameobject', go_type = 8, distance = 5, x = 5, y = 0, z = 0 } } \
             Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 1.5);
    // A focus object close by: it tries there first, and the server says no.
    assert_eq!(calls(&script), "cast Cooking | make Charred Wolf Meat x3");
    script.fire_event(
        "UI_ERROR_MESSAGE",
        vec![ScriptValue::Str("Requires Cooking Fire".into())],
    );
    ticks(&mut script, 0.5);
    assert_eq!(
        calls(&script),
        "cast Cooking | make Charred Wolf Meat x3 | cast Basic Campfire"
    );
}

#[test]
fn resting_eats_the_best_food_and_bandages_are_there_for_a_fight() {
    let mut script = professions();
    script
        .run("carry('charred', 3) carry('jerky', 2) carry('bread', 1) carry('bandage', 4) hp = 30")
        .unwrap();
    // The Haunch is for level 15: the jerky (level 5) is the best food we can eat.
    assert_eq!(
        text(
            &script,
            "local _, _, name = Professions.best_food() return name"
        ),
        "Tough Jerky"
    );
    // The runner's rest (Survive.eat) eats Professions' choice.
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:rest");
    assert_eq!(calls(&script), "use jerky");
    // A bandage on ourselves, for fight code; Recently Bandaged says no, and why.
    script.run("calls = {}").unwrap();
    assert_eq!(
        text(&script, "return tostring(Professions.bandage())"),
        "true"
    );
    assert_eq!(calls(&script), "use bandage on player");
    script.run("AURAS = { 'Recently Bandaged' }").unwrap();
    assert_eq!(
        text(
            &script,
            "local ok, why = Professions.bandage() return tostring(ok) .. ' ' .. why"
        ),
        "false recently bandaged"
    );
    // Once it has cooked the wolf meat itself, its own cooking comes first.
    script
        .run("Professions.products['Charred Wolf Meat'] = 'Cooking'")
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local _, _, name = Professions.best_food() return name"
        ),
        "Charred Wolf Meat"
    );
}

#[test]
fn it_keeps_reagents_up_to_the_cap_and_nothing_else() {
    let script = professions();
    script
        .run("carry('linen', 20) carry('linen', 10) carry('claw', 1)")
        .unwrap();
    // First Aid is not known yet but will be learnt: the cloth stays.
    assert_eq!(
        text(
            &script,
            "local k, cap = Professions.keep('linen') return tostring(k) .. ' ' .. cap"
        ),
        "true 40"
    );
    // Selling a stack that would leave 30 of 40 is a no; leaving 45 is fine.
    assert_eq!(
        text(&script, "return tostring(Professions.keep('Linen Cloth', 30)) .. ' ' .. tostring(Professions.keep('linen', 45))"),
        "true false"
    );
    assert_eq!(
        text(&script, "return tostring(Professions.keep('claw'))"),
        "false"
    );
    // Neither learnt nor wanted: no reason to keep it.
    script.run("Professions.learn = {}").unwrap();
    assert_eq!(
        text(&script, "return tostring(Professions.keep('linen'))"),
        "false"
    );
}

#[test]
fn a_trainer_the_realm_names_is_one_to_visit() {
    let script = professions();
    // wp.services hands the realm's places on as they come (entry, members and profession too).
    script
        .run(
            "Goals.learn_places({ { kind = 'trainer_first_aid', name = 'Michelle Belle', x = 40, y = 0, z = 0, \
             map = 0, entry = 2329, members = 1, profession = 'First Aid' } })",
        )
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local w = Professions.wanted() return w.profession .. ' from ' .. w.place.name"
        ),
        "First Aid from Michelle Belle"
    );
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "learn_professions"
    );
}
