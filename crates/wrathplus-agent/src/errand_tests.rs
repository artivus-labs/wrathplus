//! Errands seen going wrong live: a sale that could not empty the bags chosen again and again, the
//! planner's own walk fighting the runner's, a walk stopped short hanging its action, and the
//! nearest of the vendors seen.

use super::super::goals_tests::{goals, text, ticks};
use wrathplus_ui::script::UiScript;

const BUDGET: u64 = 200_000_000;

/// Quartermaster Hicks in sight; the bags full, with nothing a merchant would take besides.
const FULL: &str = r#"
hicks = { guid = "0xH", name = "Quartermaster Hicks", kind = "unit", reaction = "friendly",
  npc = { "vendor", "repair" }, x = 10, y = 0, z = 0, distance = 10 }
free = 0
GetContainerNumFreeSlots = function() return free end
Upkeep.sell = false
Upkeep.sellable = function() return true end
Upkeep.filling = function() return "Okra (quest), Hearthstone (hearthstone)" end
"#;

fn starts(script: &UiScript, action: &str) -> usize {
    text(script, "return table.concat(Goals.log, '\\n')")
        .matches(&format!("start {action} "))
        .count()
}

#[test]
fn bags_a_sale_cannot_empty_back_the_errand_off_and_say_what_fills_them() {
    let mut script = goals();
    script.run(FULL).unwrap();
    script.run("units = { hicks } Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:sell_and_repair");
    assert_eq!(
        text(&script, "return Goals.current.args.going"),
        "Quartermaster Hicks"
    );
    script.fire_event("MERCHANT_SHOW", vec![]);
    ticks(&mut script, 2.0);
    let failed = text(&script, "return heard('ACTION_FAILED')");
    assert!(
        failed.starts_with("ACTION_FAILED progress sell_and_repair the bags are still full after selling: they hold Okra (quest), Hearthstone (hearthstone)."),
        "{failed}"
    );
    // The standing goal does not choose it again while it backs off: it walks off to find mobs.
    ticks(&mut script, 15.0);
    assert_eq!(starts(&script, "sell_and_repair"), 1);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    // Worn-out gear is still a reason to go: repairs are not what backs off.
    script
        .run("GetInventoryItemDurability = function(slot) if slot == 1 then return 1, 40 end end Agent.stop('replaced by test')")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:sell_and_repair");
}

#[test]
fn a_walk_the_planner_starts_holds_the_runner_until_it_ends_and_says_what_it_pre_empted() {
    let mut script = goals();
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    let reply = crate::lua::run(&script, "AgentMoveTo(500, 0)", BUDGET);
    assert_eq!(reply["ok"], true, "{reply}");
    let said = reply["output"][0].as_str().unwrap_or_default();
    assert!(
        said.starts_with("This walk pre-empted goal action seek_mobs (going to (")
            && said.ends_with(
                "for goal progress; the goal runner resumes when the walk ends (at most 122 s)."
            ),
        "{said}"
    );
    // The runner's action ended as an interruption (no backoff) and the runner holds.
    assert_eq!(text(&script, "return doing()"), "idle");
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
    assert_eq!(
        text(&script, "return (Goals.holding())"),
        "your walk to (500, 0)"
    );
    assert_eq!(
        text(&script, "return walking and 'walking' or 'still'"),
        "walking"
    );
    ticks(&mut script, 10.0);
    assert_eq!(text(&script, "return doing()"), "idle");
    // The walk ends: the runner carries on at once, with its own walks not held.
    script.run("walking = false").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    assert_eq!(text(&script, "return tostring(Goals.holding())"), "nil");
    // An activity started through the port is the runner's (the fast tier's choice), and an NPC
    // within reach is no walk.
    script
        .run("units = { { guid = '0xN', name = 'Smith Argus', kind = 'unit', reaction = 'friendly', distance = 3, x = 3, y = 0, z = 0 } }")
        .unwrap();
    let reply = crate::lua::run(
        &script,
        "AgentInteract('0xN') Agent.stop('replaced by the fast tier') return Agent.run('goal:seek_mobs')",
        BUDGET,
    );
    assert_eq!(reply["values"][0], true, "{reply}");
    assert_eq!(reply["output"], serde_json::json!([]));
    assert_eq!(text(&script, "return tostring(Goals.holding())"), "nil");
    // The walk verbs are the client's own again once the call is over.
    assert!(script
        .eval::<bool>("return Agent.port_call ~= nil and Move.to ~= nil")
        .unwrap());
}

#[test]
fn a_walk_stopped_short_ends_the_errand_as_an_interruption_and_it_starts_again() {
    let mut script = goals();
    script.run(FULL).unwrap();
    // Hicks seen before, now out of sight down the road.
    script
        .run("Goals.learn_places({ { kind = 'repair', name = 'Quartermaster Hicks', guid = '0xH', x = 300, y = 0 } }) Goals.on = true")
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:sell_and_repair");
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return Move.mode or 'none'"), "to");
    // The planner stops the walk ("to rest first"): the client's walk reads stopped.
    script
        .run("walking = false AgentWalk = function() return { status = walking and 'walking' or 'stopped' } end")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
    assert!(text(&script, "return table.concat(Goals.log, ' | ')")
        .contains("sell_and_repair interrupted: walk interrupted"));
    assert_eq!(starts(&script, "sell_and_repair"), 2);
}

#[test]
fn several_vendors_are_remembered_and_the_nearest_is_the_one_visited() {
    let mut script = goals();
    script.run(FULL).unwrap();
    let n: i64 = script
        .eval("return Goals.learn_places({ { kind = 'vendor', name = 'Innkeeper Farley', x = 900, y = 0 }, \
               { kind = 'vendor', name = 'Quartermaster Lewis', x = 250, y = 40, map = 0 }, \
               { kind = 'repair', name = 'Andrew Krighton', x = 600, y = 0 } })")
        .unwrap();
    assert_eq!(n, 3);
    // A vendor seen in passing joins them; a place on another continent is never the nearest.
    script
        .run("units = { { guid = '0xT', name = 'Thurman Mullby', kind = 'unit', reaction = 'friendly', npc = { 'vendor' }, \
              x = 60, y = 0, z = 0, distance = 60 } } \
              Goals.learn_places({ { kind = 'vendor', name = 'Far Away', x = 5, y = 0, map = 1 } }) Goals.list() units = {}")
        .unwrap();
    assert_eq!(text(&script, "return tostring(#Goals.places.vendor)"), "5");
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:sell_and_repair");
    assert_eq!(
        text(&script, "return Goals.current.args.going"),
        "Thurman Mullby"
    );
}

#[test]
fn a_stop_or_a_sit_by_hand_holds_the_runner_a_while_and_says_so() {
    let mut script = goals();
    script
        .run("DoEmote = function(token) calls[#calls + 1] = 'emote ' .. token end Goals.on = true")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    let reply = crate::lua::run(&script, "AgentStop()", BUDGET);
    let said = reply["output"][0].as_str().unwrap_or_default();
    assert!(
        said.starts_with("Stopped pre-empted goal action seek_mobs (going to (")
            && said.contains("for goal progress; the goal runner holds for 30 s"),
        "{said}"
    );
    assert_eq!(text(&script, "return (Goals.holding())"), "you stopped");
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
    // Not released by the walk being over: it runs its 30 s, then the usual grace.
    ticks(&mut script, 20.0);
    assert_eq!(text(&script, "return doing()"), "idle");
    ticks(&mut script, 14.0);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    // A sit emote while the runner holds for it already only renews the hold.
    let reply = crate::lua::run(&script, "DoEmote('sit')", BUDGET);
    assert!(reply["output"][0]
        .as_str()
        .unwrap_or_default()
        .starts_with("Sat down pre-empted goal action seek_mobs"));
    assert_eq!(text(&script, "return (Goals.holding())"), "you sat down");
}

#[test]
fn resting_goes_on_to_full_health_then_says_so() {
    let mut script = goals();
    script.run("hp = 40 Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:rest");
    // Past `rest_until` but not full: still resting, nothing said.
    script.run("hp = 95").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:rest");
    assert_eq!(text(&script, "return heard('RESTED')"), "");
    script.run("hp = 100").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return heard('RESTED')"), "RESTED 100 nil");
}
