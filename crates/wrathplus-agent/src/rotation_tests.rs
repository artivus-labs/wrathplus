//! `wp.rotation` against a stubbed warrior: priorities, conditions, cooldowns, power, failures.

use wrathplus_ui::script::{ScriptValue, UiScript};

/// A level 5 warrior who knows Battle Shout, Rend and Heroic Strike (not Charge or Execute).
const GAME: &str = r#"
casts = {}
now = 100
rage = 0
buffs, debuffs = {}, {}
target_hp = 80
cooldown = {}
BOOK = { "Battle Shout", "Rend", "Heroic Strike", "Attack" }
COST = { ["Battle Shout"] = 100, ["Rend"] = 100, ["Heroic Strike"] = 150 }
GetTime = function() return now end
GetSpellName = function(i) return BOOK[i] end
GetSpellInfo = function(s) if COST[s] or s == "Attack" then return s, "", "", COST[s] or 0, false, 1, 0, 0, s == "Heroic Strike" and 5 or 0 end end
GetSpellCooldown = function(i, book) assert(type(i) == "number" and book == "spell", "Invalid spell slot") return cooldown[BOOK[i]] or 0, 10, 1 end
UnitPower = function() return rage end
UnitPowerType = function() return 1 end
UnitClass = function() return "Warrior", "WARRIOR" end
UnitHealth = function(u) return u == "target" and target_hp or 100 end
UnitHealthMax = function() return 100 end
UnitExists = function() return true end
UnitAffectingCombat = function() return true end
UnitCastingInfo = function() return nil end
UnitChannelInfo = function() return nil end
UnitAura = function(unit, i, filter)
  local list = unit == "player" and buffs or debuffs
  return list[i]
end
AgentNearby = function() return {} end
CastSpellByName = function(s, on) casts[#casts + 1] = s .. (on and (" @" .. on) or "") end
"#;

fn rotation() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-rotation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn tick(script: &UiScript) -> String {
    script
        .eval("now = now + 1.1 local s = Rotation.tick({ distance = 2 }, 90) return s or ''")
        .unwrap()
}

#[test]
fn the_default_casts_by_priority_as_power_cooldowns_and_auras_allow() {
    let script = rotation();
    // The default is the class's, of known spells only.
    let described: String = script.eval("return Rotation.describe()").unwrap();
    assert_eq!(described, "Battle Shout > Rend > Heroic Strike");
    // No rage: nothing affordable.
    assert_eq!(tick(&script), "");
    // Rage: the shout (on itself), then Rend; Heroic Strike waits for 30 rage.
    script.run("rage = 20").unwrap();
    assert_eq!(tick(&script), "Battle Shout");
    script.run("buffs = { 'Battle Shout' }").unwrap();
    assert_eq!(tick(&script), "Rend");
    script.run("debuffs = { 'Rend' }").unwrap();
    assert_eq!(tick(&script), "");
    script.run("rage = 40").unwrap();
    assert_eq!(tick(&script), "Heroic Strike");
    // On cooldown: skipped.
    script.run("cooldown['Heroic Strike'] = now").unwrap();
    assert_eq!(tick(&script), "");
    let casts: String = script.eval("return table.concat(casts, ' | ')").unwrap();
    assert_eq!(casts, "Battle Shout @player | Rend | Heroic Strike");
}

#[test]
fn a_list_of_its_own_and_a_failing_spell_backing_off() {
    let mut script = rotation();
    script
        .run(
            "rage = 50 Rotation.list = { { spell = 'Heroic Strike' }, { spell = 'Rend', when = function(c) return c.target_hp > 50 end } }",
        )
        .unwrap();
    assert_eq!(tick(&script), "Heroic Strike");
    // The server refuses it: left alone for a few seconds, and counted.
    script.fire_event(
        "UNIT_SPELLCAST_FAILED",
        vec![
            ScriptValue::Str("player".into()),
            ScriptValue::Str("Heroic Strike".into()),
        ],
    );
    assert_eq!(tick(&script), "Rend");
    script.run("target_hp = 40").unwrap();
    assert_eq!(tick(&script), "");
    let failed: i64 = script
        .eval("return Rotation.stats['Heroic Strike'].failed")
        .unwrap();
    assert_eq!(failed, 1);
    // Learning a spell is noticed (the default list is rebuilt), and a list of its own is kept.
    script
        .run("BOOK[#BOOK + 1] = 'Execute' COST['Execute'] = 150")
        .unwrap();
    script.fire_event("LEARNED_SPELL_IN_TAB", vec![]);
    let own: String = script.eval("return Rotation.describe()").unwrap();
    assert_eq!(own, "Heroic Strike > Rend");
    script.run("Rotation.list = nil").unwrap();
    let default: String = script.eval("return Rotation.describe()").unwrap();
    assert_eq!(default, "Execute > Battle Shout > Rend > Heroic Strike");
}

fn refuse(script: &mut UiScript, spell: &str) {
    script.fire_event(
        "UNIT_SPELLCAST_FAILED",
        vec![
            ScriptValue::Str("player".into()),
            ScriptValue::Str(spell.into()),
        ],
    );
}

/// Ticks (1.1 s apart) until the spell is cast again.
fn ticks_until(script: &UiScript, spell: &str) -> usize {
    (1..100).find(|_| tick(script) == spell).unwrap()
}

#[test]
fn victory_rush_waits_for_a_kill_and_a_refused_spell_waits_longer_each_time() {
    let mut script = rotation();
    script
        .run("BOOK[#BOOK + 1] = 'Victory Rush' COST['Victory Rush'] = 0 Rotation.list = { { spell = 'Victory Rush' } }")
        .unwrap();
    // Usable only after a kill: no Victorious, no cast, and no refusal to back off from.
    assert_eq!(tick(&script), "");
    script.run("buffs = { 'Victorious' }").unwrap();
    assert_eq!(tick(&script), "Victory Rush");
    // Refused anyway: 4 s, then 8 s; a cast that goes through starts over.
    refuse(&mut script, "Victory Rush");
    assert_eq!(ticks_until(&script, "Victory Rush"), 4);
    refuse(&mut script, "Victory Rush");
    assert_eq!(ticks_until(&script, "Victory Rush"), 8);
    script.fire_event(
        "UNIT_SPELLCAST_SUCCEEDED",
        vec![
            ScriptValue::Str("player".into()),
            ScriptValue::Str("Victory Rush".into()),
        ],
    );
    refuse(&mut script, "Victory Rush");
    assert_eq!(ticks_until(&script, "Victory Rush"), 4);
}
