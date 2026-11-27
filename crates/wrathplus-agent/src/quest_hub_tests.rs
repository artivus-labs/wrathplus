//! Quest hubs in `wp.goals`: with nothing to do in reach, the runner walks to a quest hub it knows
//! (the realm's `questgiver-hub` places, the givers it saw) in a zone that suits its level before
//! it grinds; a hub with nothing for it backs off with its town; travel to a zone ends at
//! something in the zone, not at the zone line; and the steps further afield keep out of a zone
//! below us. Seen live: a level 12 gnome with an empty quest log shuttling across the Dun Morogh /
//! Loch Modan border for an hour, 600 yards from Thelsamar's quest givers.

use super::{goals, text, ticks};
use wrathplus_ui::script::{ScriptValue, UiScript};

/// A level 12 gnome at the Dun Morogh / Loch Modan border (x = 0), with an empty quest log. The
/// realm's quest hubs: Thelsamar (two givers, 600 and 640 yards east, Loch Modan), one nearer in
/// Dun Morogh (below us) and one nearer in the Wetlands (above us).
const BORDER: &str = r#"
lvl = 12
zone = "Loch Modan"
GetRealZoneText = function() return zone end
AgentZones = function() return {
  { id = 1, name = "Dun Morogh", map = 0, faction = "Alliance", city = false, min = 1, max = 10, x = -500, y = 0 },
  { id = 38, name = "Loch Modan", map = 0, faction = "Alliance", city = false, min = 10, max = 20, x = 1000, y = 0 },
  { id = 11, name = "Wetlands", map = 0, faction = "Alliance", city = false, min = 20, max = 30, x = 300, y = 2000 },
} end
moves = {}
AgentMoveTo = function(x, y)
  calls[#calls + 1] = "move"
  moves[#moves + 1] = string.format("%d,%d", math.floor(x + 0.5), math.floor(y + 0.5))
  walking = true
  return {}
end
hubs = {
  { kind = "questgiver-hub", name = "Mountaineer Kadrell", x = 600, y = 0, z = 0, map = 0, zone = "Loch Modan", members = 8 },
  { kind = "questgiver-hub", name = "Vidra Hearthstove", x = 640, y = 20, z = 0, map = 0, zone = "Loch Modan", members = 8 },
  { kind = "questgiver-hub", name = "Kharanos Giver", x = -200, y = 0, z = 0, map = 0, zone = "Dun Morogh" },
  { kind = "questgiver-hub", name = "Wetlands Giver", x = 300, y = 200, z = 0, map = 0, zone = "Wetlands" },
}
function giver(name, x, distance)
  return { guid = "0x" .. name, name = name, kind = "unit", reaction = "friendly", npc = { "questgiver" },
    quest = "available", x = x, y = 0, z = 0, distance = distance, dead = false }
end
"#;

fn border() -> UiScript {
    let script = goals();
    script.run(BORDER).unwrap();
    script
}

#[test]
fn an_empty_log_walks_to_the_nearest_hub_that_suits_our_level_before_grinding() {
    let mut script = border();
    // Nothing known: no hub to walk to, and a level 12 mob in sight is ground.
    script.run("wolf.level = 12 units = { wolf }").unwrap();
    assert_eq!(text(&script, "return plan_of('progress')"), "grind");
    // The realm names its hubs: questing comes first, mob in sight or not.
    script.run("assert(Goals.learn_places(hubs) == 4)").unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "visit_quest_hub pick_up_quests"
    );
    script.run("units = {}").unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "visit_quest_hub pick_up_quests"
    );
    assert_eq!(
        text(
            &script,
            "return tostring(Goals.places['questgiver-hub'][4].members)"
        ),
        "8"
    );
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:visit_quest_hub");
    // Thelsamar, not the nearer hubs in a zone below or above us.
    assert_eq!(
        text(&script, "return Goals.current.args.going"),
        "Mountaineer Kadrell in Loch Modan"
    );
    assert_eq!(text(&script, "return table.concat(moves, ' ')"), "600,0");
    // There: the giver stands 50 yards off, beyond reach, so the visit walks up to them.
    script
        .run("walking = false pos = { x = 600, y = 0, z = 0, map = 0 } units = { giver('Kadrell', 650, 50) }")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:visit_quest_hub");
    assert_eq!(text(&script, "return moves[#moves]"), "650,0");
    // In reach: the visit is over and the quests are picked up.
    script
        .run("walking = false pos = { x = 645, y = 0, z = 0, map = 0 } units = { giver('Kadrell', 650, 5) }")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
    assert_eq!(text(&script, "return doing()"), "goal:pick_up_quests");
}

#[test]
fn a_hub_with_nothing_for_us_backs_off_with_its_town_until_the_next_level() {
    let mut script = border();
    script
        .run("Goals.learn_places(hubs) Goals.on = true")
        .unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:visit_quest_hub");
    // There, and no giver in sight has a quest for us.
    script
        .run("walking = false pos = { x = 600, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 1.0);
    assert!(text(&script, "return heard('ACTION_FAILED')").starts_with(
        "ACTION_FAILED progress visit_quest_hub nothing to pick up at Mountaineer Kadrell"
    ));
    let wait: f64 = script
        .eval("return Goals.failures['visit_quest_hub:Mountaineer Kadrell'].until_ - GetTime()")
        .unwrap();
    assert!(wait > 800.0, "{wait}");
    // Vidra Hearthstove, 40 yards on in the same town, backs off with it; the hubs left are below
    // or above us, so progress does something else.
    assert!(!script
        .eval::<bool>("return Goals.facts().quest_hub_known")
        .unwrap());
    assert!(!text(&script, "return plan_of('progress')").contains("visit_quest_hub"));
    // Back where we started, still nothing: the town is not walked to again.
    script
        .run("pos = { x = 0, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 2.0);
    assert_ne!(text(&script, "return doing()"), "goal:visit_quest_hub");
    // A new level opens new quests: the town is worth a visit again.
    script.run("lvl = 13").unwrap();
    script.fire_event("PLAYER_LEVEL_UP", vec![ScriptValue::Int(13)]);
    assert!(script
        .eval::<bool>("return Goals.facts().quest_hub_known")
        .unwrap());
}

#[test]
fn travel_to_a_zone_ends_at_something_in_it_not_at_the_zone_line() {
    let mut script = border();
    // In Dun Morogh, below us, and no hub known: travel to Loch Modan's middle.
    script
        .run("zone = 'Dun Morogh' pos = { x = -10, y = 0, z = 0, map = 0 } Goals.on = true")
        .unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:travel_to_zone");
    assert_eq!(text(&script, "return table.concat(moves, ' ')"), "1000,0");
    // Over the line, still walking: not there yet.
    script
        .run("zone = 'Loch Modan' pos = { x = 10, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 2.0);
    assert_eq!(text(&script, "return doing()"), "goal:travel_to_zone");
    assert!(!text(&script, "return table.concat(Goals.log, ' | ')").contains("did travel_to_zone"));
    // The realm names Loch Modan's hubs as we enter: the walk ends there and the hub visit takes
    // over.
    script.run("Goals.learn_places(hubs)").unwrap();
    ticks(&mut script, 1.0);
    assert!(text(&script, "return table.concat(Goals.log, ' | ')").contains("did travel_to_zone"));
    assert_eq!(text(&script, "return doing()"), "goal:visit_quest_hub");
}

#[test]
fn travel_heads_for_a_known_hub_in_the_zone_and_ends_when_the_walk_does() {
    let mut script = border();
    // The planner's own travel goal, with Thelsamar known: the walk goes to its hub.
    script
        .run(
            "zone = 'Dun Morogh' pos = { x = -10, y = 0, z = 0, map = 0 } Goals.learn_places(hubs) \
             Goals.push({ name = 'to_loch', priority = 60, data = { zone = 'Loch Modan' }, \
               want = { zone_fit = 'ok', mobs_for_xp = true } }) Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:travel_to_zone");
    assert_eq!(
        text(&script, "return Goals.current.args.going"),
        "Mountaineer Kadrell in Loch Modan"
    );
    script
        .run("zone = 'Loch Modan' pos = { x = 10, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:travel_to_zone");
    script
        .run("walking = false pos = { x = 600, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 1.0);
    assert!(text(&script, "return table.concat(Goals.log, ' | ')").contains("did travel_to_zone"));
}

#[test]
fn steps_further_afield_keep_out_of_a_zone_below_us_when_another_way_goes_on() {
    let mut script = border();
    // At the border in Loch Modan, nothing in sight and no hub: seek_mobs walks on. The first
    // direction tried is west (back toward Dun Morogh's middle), the next east.
    script
        .run(
            "local r = { 0.5, 0.0 } local i = 0 \
             math.random = function() i = i + 1 return r[i] or 0.0 end Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    assert_eq!(text(&script, "return table.concat(moves, ' ')"), "100,0");
    // Every way west: it still goes on, back over the line, rather than fail.
    script
        .run("Agent.stop('replaced by the test') walking = false moves = {} math.random = function() return 0.5 end")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    assert_eq!(text(&script, "return table.concat(moves, ' ')"), "-100,0");
}

/// Seen live: a level 13 dwarf at (-5500, -750) in Dun Morogh (levels 5-10), its quest log empty,
/// its hubs all in Dun Morogh and Thelsamar 2200 yards off, beyond the realm's services radius.
/// The level 8-10 troggs still give XP, so zone_fit read "ok" and it ground there for good.
const OUTLEVELLED: &str = r#"
lvl = 13
zone = "Dun Morogh"
pos = { x = -500, y = 0, z = 0, map = 0 }
AgentZones = function() return {
  { id = 1, name = "Dun Morogh", map = 0, faction = "Alliance", city = false, min = 5, max = 10, x = -500, y = 0 },
  { id = 38, name = "Loch Modan", map = 0, faction = "Alliance", city = false, min = 10, max = 20, x = 1000, y = 0 },
  { id = 40, name = "Westfall", map = 0, faction = "Alliance", city = false, min = 10, max = 20, x = -500, y = -4000 },
} end
function trogg(n, level, distance)
  return { guid = "0xT" .. n, name = "Rockjaw Trogg", kind = "unit", reaction = "hostile", level = level,
    x = -500 + distance, y = 0, z = 0, distance = distance, dead = false }
end
units = { trogg(1, 8, 25), trogg(2, 9, 40), trogg(3, 10, 55) }
assert(Goals.learn_places({
  { kind = "questgiver-hub", name = "Kharanos Giver", x = -450, y = 0, z = 0, map = 0, zone = "Dun Morogh" },
}) == 1)
"#;

fn outlevelled() -> UiScript {
    let script = border();
    script.run(OUTLEVELLED).unwrap();
    script
}

#[test]
fn an_outlevelled_zone_with_no_quest_work_is_left_for_one_that_fits() {
    let mut script = outlevelled();
    // The troggs give XP (grey is 7 at level 13), so the zone still fits by them, but its band is
    // below us and their median (9) is within near_grey (3) of grey: outlevelled.
    assert_eq!(text(&script, "return Goals.facts().zone_fit"), "ok");
    assert!(script
        .eval::<bool>("return Goals.facts().zone_outlevelled")
        .unwrap());
    assert!(script
        .eval::<bool>("return Goals.facts().mobs_for_xp")
        .unwrap());
    assert!(!script
        .eval::<bool>("return Goals.facts().quest_hub_known")
        .unwrap());
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "travel_to_zone grind"
    );
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:travel_to_zone");
    // The nearest zone that fits, to its middle (no hub known there yet).
    assert_eq!(
        text(&script, "return Goals.current.args.going"),
        "Loch Modan"
    );
    assert_eq!(text(&script, "return table.concat(moves, ' ')"), "1000,0");
}

#[test]
fn quest_work_here_comes_before_leaving_an_outlevelled_zone() {
    let script = outlevelled();
    // An open objective on the troggs in sight: worked first, though the zone is outlevelled.
    script
        .run(
            "LOG = { { id = 9, title = 'The Troll Cave', level = 10, \
               objectives = { { 'Rockjaw Trogg slain: 2/6', 'monster', false } } } }",
        )
        .unwrap();
    assert!(script
        .eval::<bool>("return Goals.facts().zone_outlevelled")
        .unwrap());
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "work_objective"
    );
    // Done with it but not handed in: the hand-in still comes first.
    script
        .run(
            "LOG[1].complete = true LOG[1].objectives[1] = { 'Rockjaw Trogg slain: 6/6', 'monster', true } \
             units[#units + 1] = { guid = '0xG', name = 'Senir', kind = 'unit', reaction = 'friendly', \
               npc = { 'questgiver' }, quest = 'reward', x = -490, y = 0, z = 0, distance = 10, dead = false }",
        )
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "hand_in_quests"
    );
}

#[test]
fn a_zone_that_fits_or_a_pocket_of_higher_mobs_is_not_left() {
    let script = outlevelled();
    // Loch Modan suits level 13: grinding there, as before.
    script
        .run("zone = 'Loch Modan' pos = { x = 1000, y = 0, z = 0, map = 0 }")
        .unwrap();
    assert!(!script
        .eval::<bool>("return Goals.facts().zone_outlevelled")
        .unwrap());
    assert_eq!(text(&script, "return plan_of('progress')"), "grind");
    // In Dun Morogh, but among level 11-12 mobs (a cave of higher ones, spread out
    // so none is a crowd): not outlevelled.
    script
        .run(
            "zone = 'Dun Morogh' pos = { x = -500, y = 0, z = 0, map = 0 } \
             units = { trogg(1, 11, 25), trogg(2, 11, 55), trogg(3, 12, 85) }",
        )
        .unwrap();
    assert!(!script
        .eval::<bool>("return Goals.facts().zone_outlevelled")
        .unwrap());
    assert_eq!(text(&script, "return plan_of('progress')"), "grind");
}

#[test]
fn an_outlevelled_zone_with_no_route_out_is_still_ground_in() {
    let mut script = outlevelled();
    // No route out: travel fails and backs off for each zone that fits, and the troggs are ground
    // meanwhile rather than nothing done.
    script
        .run(
            "AgentMoveTo = function() calls[#calls + 1] = 'move' return nil, 'no route' end \
             Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 3.0);
    assert!(text(&script, "return heard('ACTION_FAILED')")
        .contains("ACTION_FAILED progress travel_to_zone"));
    assert!(script
        .eval::<bool>("return Goals.cooling('travel_to_zone:Loch Modan') and Goals.cooling('travel_to_zone:Westfall')")
        .unwrap());
    assert_eq!(text(&script, "return plan_of('progress')"), "grind");
    assert_eq!(text(&script, "return doing()"), "goal:grind");
}
