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
COST = { ["Battle Shout"] = 10, ["Rend"] = 10, ["Heroic Strike"] = 15 }
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
-- A client without the game's own answers, until a test gives it some.
IsUsableSpell, IsSpellInRange, IsCurrentSpell, IsAutoRepeatSpell = nil, nil, nil, nil
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
    // Rage: the shout (on itself), then Rend; Heroic Strike waits for 25 rage.
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
        .run("BOOK[#BOOK + 1] = 'Execute' COST['Execute'] = 15")
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

#[test]
fn costs_are_in_the_powers_own_units_and_the_games_own_answer_wins() {
    let script = rotation();
    script
        .run("Rotation.list = { { spell = 'Heroic Strike' } } rage = 5")
        .unwrap();
    // GetSpellInfo gives 15 for Heroic Strike's 15 rage (not tenths): at 5 rage it is not tried.
    // (Read as tenths it was cast at 2 rage, refused "Not enough rage", and backed off for
    // seconds: the playtest's "Rend 11 cast, 8 failed".)
    assert_eq!(tick(&script), "");
    script.run("rage = 15").unwrap();
    assert_eq!(tick(&script), "Heroic Strike");
    // Where the client has IsUsableSpell, that is the answer: a stance, a proc, the power.
    script
        .run("rage = 80 IsUsableSpell = function() return nil, 1 end")
        .unwrap();
    assert_eq!(tick(&script), "");
    script
        .run("IsUsableSpell = function() return 1 end IsCurrentSpell = function() return queued end queued = true")
        .unwrap();
    // Queued for the next swing already: not queued again.
    assert_eq!(tick(&script), "");
    script.run("queued = false").unwrap();
    assert_eq!(tick(&script), "Heroic Strike");
}

#[test]
fn melee_spells_wait_for_reach_and_an_opener_for_its_band() {
    let mut script = rotation();
    script
        .run(
            "BOOK[#BOOK + 1] = 'Charge' COST.Charge = 0 rage = 50 \
             local info = GetSpellInfo \
             GetSpellInfo = function(s) if s == 'Charge' then return s, '', '', 0, false, 1, 0, 8, 25 end return info(s) end \
             UnitAffectingCombat = function() return fighting end \
             buffs = { 'Battle Shout' } \
             function at(d) now = now + 1.1 return Rotation.tick({ distance = d }, 90) or '' end",
        )
        .unwrap();
    script.fire_event("SPELLS_CHANGED", vec![]);
    // 40 yards off: nothing reaches. 20: Charge, in its 8-25 band, out of combat. Rend and Heroic
    // Strike are not thrown from there.
    assert_eq!(script.eval::<String>("return at(40)").unwrap(), "");
    assert_eq!(script.eval::<String>("return at(20)").unwrap(), "Charge");
    script.run("fighting = true").unwrap();
    assert_eq!(script.eval::<String>("return at(20)").unwrap(), "");
    assert_eq!(script.eval::<String>("return at(4)").unwrap(), "Rend");
    // The game's range answer, where it has one, is the one used.
    script
        .run("debuffs = { 'Rend' } UnitGUID = function() return '0xT' end IsSpellInRange = function() return 0 end")
        .unwrap();
    assert_eq!(script.eval::<String>("return at(2)").unwrap(), "");
    script
        .run("IsSpellInRange = function() return 1 end")
        .unwrap();
    assert_eq!(
        script.eval::<String>("return at(2)").unwrap(),
        "Heroic Strike"
    );
    // For a spell with no band of its own the client says "in range" from anywhere (seen live:
    // 1 for Heroic Strike at 40 yards), so those go by distance alone.
    script
        .run("debuffs = {} Rotation.list = { { spell = 'Rend' } }")
        .unwrap();
    assert_eq!(script.eval::<String>("return at(20)").unwrap(), "");
    assert_eq!(script.eval::<String>("return at(3)").unwrap(), "Rend");
}

fn error(script: &mut UiScript, message: &str) {
    script.fire_event("UI_ERROR_MESSAGE", vec![ScriptValue::Str(message.into())]);
}

#[test]
fn only_the_casts_own_refusal_counts_and_range_or_facing_is_tried_again_at_once() {
    let mut script = rotation();
    script
        .run("rage = 50 Rotation.list = { { spell = 'Rend' } }")
        .unwrap();
    assert_eq!(tick(&script), "Rend");
    // A second later the auto-attack's own error: not Rend's.
    script.run("now = now + 1").unwrap();
    error(&mut script, "You are too far away!");
    let failed: i64 = script.eval("return Rotation.stats.Rend.failed").unwrap();
    assert_eq!(failed, 0);
    // Refused for facing, said twice by the game: one failure, and tried again a second on
    // (Combat turns meanwhile), not after the 4 s a stance error earns.
    assert_eq!(tick(&script), "Rend");
    refuse(&mut script, "Rend");
    error(&mut script, "You are facing the wrong way!");
    let failed: i64 = script.eval("return Rotation.stats.Rend.failed").unwrap();
    assert_eq!(failed, 1);
    assert_eq!(ticks_until(&script, "Rend"), 1);
    // A stance it cannot be cast in: backed off 4 s.
    error(&mut script, "Must be in Battle Stance");
    assert_eq!(ticks_until(&script, "Rend"), 4);
}

/// Another class at the stub: its book, and the default list rebuilt.
fn play_as(script: &mut UiScript, class: &str, book: &str) {
    script
        .run(&format!(
            "UnitClass = function() return '{class}', '{class}' end BOOK = {{ {book} }} \
             COST = {{}} GetSpellInfo = function(s) return s, '', '', 0, false, 0, 0, 0, 30 end \
             UnitPowerType = function() return 0 end mana = 100 \
             UnitPower = function() return mana end UnitPowerMax = function() return 100 end \
             IsUsableSpell = function(s) return (s == 'Shoot' or s == 'Auto Shot' or mana >= 20) and 1 or nil end \
             IsAutoRepeatSpell = function(s) return repeating == s end \
             function at(d) now = now + 1.6 return Rotation.tick({{ distance = d }}, 90) or '' end"
        ))
        .unwrap();
    script.fire_event("SPELLS_CHANGED", vec![]);
}

#[test]
fn a_warrior_claps_with_two_on_it_and_takes_its_first_rage_from_bloodrage() {
    let mut script = rotation();
    script
        .run(
            "BOOK = { 'Battle Shout', 'Rend', 'Heroic Strike', 'Thunder Clap', 'Bloodrage' } \
             COST['Thunder Clap'] = 20 COST.Bloodrage = 0 buffs = { 'Battle Shout' } debuffs = { 'Rend' } \
             UnitGUID = function() return '0xME' end mobs = {} \
             AgentNearby = function() return mobs end",
        )
        .unwrap();
    script.fire_event("SPELLS_CHANGED", vec![]);
    // No rage in a fight at good health: Bloodrage.
    assert_eq!(tick(&script), "Bloodrage");
    // One mob: no clap; Heroic Strike once there is rage to spare.
    script.run("rage = 30 cooldown.Bloodrage = now").unwrap();
    assert_eq!(tick(&script), "Heroic Strike");
    // A second mob on us: Thunder Clap first.
    script
        .run(
            "local function on_us(g) return { guid = g, kind = 'unit', in_combat = true, target = '0xME', reaction = 'hostile' } end \
             mobs = { on_us('0xA'), on_us('0xB') }",
        )
        .unwrap();
    assert_eq!(tick(&script), "Thunder Clap");
}

#[test]
fn casters_wand_when_dry_hunters_shoot_once_and_healers_heal_themselves() {
    let mut script = rotation();
    play_as(&mut script, "MAGE", "'Frostbolt', 'Fire Blast', 'Shoot'");
    assert_eq!(script.eval::<String>("return at(25)").unwrap(), "Frostbolt");
    // Out of mana: the wand, once; it repeats by itself.
    script.run("mana = 10").unwrap();
    assert_eq!(script.eval::<String>("return at(25)").unwrap(), "Shoot");
    script.run("repeating = 'Shoot'").unwrap();
    assert_eq!(script.eval::<String>("return at(25)").unwrap(), "");
    // A hunter: Auto Shot from range once, Raptor Strike with the mob on top of it.
    play_as(&mut script, "HUNTER", "'Auto Shot', 'Raptor Strike'");
    script.run("repeating = nil").unwrap();
    assert_eq!(script.eval::<String>("return at(25)").unwrap(), "Auto Shot");
    script.run("repeating = 'Auto Shot'").unwrap();
    assert_eq!(script.eval::<String>("return at(25)").unwrap(), "");
    assert_eq!(
        script.eval::<String>("return at(3)").unwrap(),
        "Raptor Strike"
    );
    // A priest below half health heals itself before it smites.
    play_as(&mut script, "PRIEST", "'Lesser Heal', 'Smite', 'Shoot'");
    assert_eq!(script.eval::<String>("return at(25)").unwrap(), "Smite");
    script
        .run("UnitHealth = function(u) return u == 'target' and 80 or 45 end")
        .unwrap();
    assert_eq!(
        script.eval::<String>("return at(25)").unwrap(),
        "Lesser Heal"
    );
    let casts: String = script.eval("return casts[#casts]").unwrap();
    assert_eq!(casts, "Lesser Heal @player");
}
