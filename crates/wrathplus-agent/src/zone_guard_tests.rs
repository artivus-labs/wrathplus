//! Travel respects the zones' levels: a goal's `data.zone` above us is refused at `Goals.push`,
//! and an action the planner taught that walks into such a zone is stopped there.

use super::goals_tests::{goals, text, ticks};

/// A level 11 warrior in Elwynn, with the client's zone table for Stormwind's surroundings.
const ZONES: &str = r#"
lvl = 11
zone = "Elwynn Forest"
GetRealZoneText = function() return zone end
AgentZones = function() return {
  { id = 12, name = "Elwynn Forest", map = 0, faction = "Alliance", city = false, min = 1, max = 9, x = 0, y = 0 },
  { id = 40, name = "Westfall", map = 0, faction = "Alliance", city = false, min = 12, max = 17, x = 0, y = 1500 },
  { id = 10, name = "Duskwood", map = 0, faction = "Alliance", city = false, min = 19, max = 28, x = -1500, y = 0 },
  { id = 38, name = "Loch Modan", map = 0, faction = "Alliance", city = false, min = 11, max = 15, x = 3000, y = 0 },
  { id = 1519, name = "Stormwind City", map = 0, faction = "Alliance", city = true, min = 1, max = 80, x = 0, y = 800 },
} end
function refusal(spec)
  local ok, err = pcall(Goals.push, spec)
  return ok and "accepted" or err
end
"#;

#[test]
fn a_goal_that_travels_to_a_zone_above_us_is_refused_with_the_zones_that_fit() {
    let script = goals();
    script.run(ZONES).unwrap();
    assert_eq!(
        text(
            &script,
            "return refusal({ name = 'level_in_duskwood', why = 'Duskwood is 10-18', data = { zone = 'Duskwood' } })"
        ),
        "goal level_in_duskwood: Duskwood is levels 19-28, above you at level 11: a goal that travels \
         there is refused. Zones that fit level 11: Westfall (12-17, 1500 yd), Loch Modan (11-15, 3000 yd). \
         Zone levels come from Goals.zones(), not from memory."
    );
    assert!(script
        .eval::<bool>("return Goals.goals.level_in_duskwood == nil")
        .unwrap());
    // A name that is no zone to level in (a city, a typo) is refused the same way.
    assert!(text(
        &script,
        "return refusal({ name = 'to_town', data = { zone = 'Stormwind City' } })"
    )
    .starts_with("goal to_town: no zone named Stormwind City to level in on this continent. Zones that fit level 11: Westfall"));
    // A zone that fits is taken, and travel_to_zone heads there.
    assert_eq!(
        text(
            &script,
            "return refusal({ name = 'to_westfall', priority = 60, data = { zone = 'Westfall' }, want = { zone_fit = 'ok', mobs_for_xp = true } })"
        ),
        "accepted"
    );
    assert_eq!(
        text(&script, "return plan_of('to_westfall')"),
        "travel_to_zone"
    );
}

#[test]
fn a_taught_action_that_walks_into_a_zone_above_us_is_stopped_there() {
    let mut script = goals();
    script.run(ZONES).unwrap();
    script
        .run(
            r#"
            Goals.fact("in_duskwood", function() return zone == "Duskwood" end)
            Goals.action({ name = "travel_duskwood", why = "walk to Duskwood", effects = { in_duskwood = true },
              run = function() AgentMoveTo(-10500, -800) end, done = function() return false end })
            Goals.push({ name = "reach_duskwood", priority = 80, want = { in_duskwood = true } })
            Goals.on = true
            "#,
        )
        .unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:travel_duskwood");
    assert!(script.eval::<bool>("return walking").unwrap());
    // Still in Elwynn: it walks on.
    ticks(&mut script, 2.0);
    assert_eq!(text(&script, "return doing()"), "goal:travel_duskwood");
    // Over the border: stopped, the walk too, and the planner told why and where to go instead.
    script
        .run("zone = 'Duskwood' moves = 0 AgentStop = function() walking = false moves = moves + 1 end")
        .unwrap();
    ticks(&mut script, 0.5);
    assert!(script.eval::<i64>("return moves").unwrap() >= 1);
    assert_eq!(
        text(&script, "return heard('ACTION_FAILED')"),
        "ACTION_FAILED reach_duskwood travel_duskwood walked into Duskwood, levels 19-28, above you at \
         level 11: stopped there. Zones that fit: Westfall (12-17), Loch Modan (11-15). Travel with a \
         goal's data.zone (travel_to_zone), not an action of your own."
    );
    // The zone is above us, so progress heads back to one that fits.
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:travel_to_zone");
}

#[test]
fn built_in_actions_are_not_stopped_and_a_taught_one_that_stays_put_carries_on() {
    let mut script = goals();
    script.run(ZONES).unwrap();
    assert!(script
        .eval::<bool>("return Goals.actions.travel_to_zone.built_in == true")
        .unwrap());
    script
        .run(
            r#"
            Goals.action({ name = "camp", effects = { camped = true }, run = function() end,
              done = function() return false end })
            Goals.push({ name = "camped", priority = 80 })
            Goals.on = true
            "#,
        )
        .unwrap();
    assert!(script
        .eval::<bool>("return Goals.actions.camp.built_in == nil")
        .unwrap());
    ticks(&mut script, 3.0);
    assert_eq!(text(&script, "return doing()"), "goal:camp");
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
}
