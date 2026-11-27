//! `wp.taxi` against a stubbed level 5 gnome in Loch Modan: learning the flight master's path,
//! the map's fares, and the goal runner's long walks flying where that beats the walk.

use super::goals_tests::{goals, text, ticks};
use wrathplus_ui::script::UiScript;

/// Thelsamar's node 50 yards east of us and Ironforge's 3000 yards on, both known; Sentinel Hill's
/// unknown. The map, when open, is `MAP` (name, type, fare); moves and taxi orders are recorded.
const FLIGHTS: &str = r#"
money = 1000
GetMoney = function() return money end
GetRepairAllCost = function() return 0 end
NODES = {
  { id = 8, name = "Thelsamar, Loch Modan", map = 0, x = 50, y = 0, z = 0, known = true, alliance = true, horde = false },
  { id = 6, name = "Ironforge, Dun Morogh", map = 0, x = 3000, y = 0, z = 0, known = true, alliance = true, horde = false },
  { id = 4, name = "Sentinel Hill, Westfall", map = 0, x = -3000, y = 0, z = 0, known = false, alliance = true, horde = false },
}
AgentTaxiNodes = function()
  local out = {}
  for _, n in ipairs(NODES) do
    local c = {}
    for k, v in pairs(n) do c[k] = v end
    c.distance = math.sqrt((n.x - pos.x) ^ 2 + (n.y - pos.y) ^ 2)
    out[#out + 1] = c
  end
  table.sort(out, function(a, b) return a.distance < b.distance end)
  return out, true
end
AgentMoveTo = function(x, y) calls[#calls + 1] = string.format("move %d,%d", x, y) walking = true return {} end
MAP = nil
NumTaxiNodes = function() return MAP and #MAP or 0 end
TaxiNodeName = function(i) return MAP[i][1] end
TaxiNodeGetType = function(i) return MAP[i] and MAP[i][2] or "NONE" end
TaxiNodeCost = function(i) return MAP[i][3] end
TakeTaxiNode = function(i) calls[#calls + 1] = "take " .. i MAP = nil end
CloseTaxiMap = function() calls[#calls + 1] = "close map" MAP = nil end
on_taxi = false
UnitOnTaxi = function() return on_taxi and 1 or nil end
GOSSIP = {}
GetNumGossipOptions = function() return #GOSSIP / 2 end
GetGossipOptions = function() return unpack(GOSSIP) end
SelectGossipOption = function(i) calls[#calls + 1] = "gossip " .. GOSSIP[i * 2] GOSSIP = {} end
CloseGossip = function() calls[#calls + 1] = "close gossip" GOSSIP = {} end
master = { guid = "0xF", name = "Thorgrum Borrelson", kind = "unit", npc = { "flightmaster" }, taxi = "known",
  reaction = "friendly", level = 55, x = 45, y = 0, z = 0, distance = 45, dead = false }
LOG = { { id = 1, title = "Ride to Ironforge", level = 10, complete = true, objectives = {} } }
hand_in_x = 3050
AgentQuestPOI = function(id) return { { objective = -1, map = 0, x = hand_in_x, y = 0 } } end
function moves() return table.concat(calls, " ") end
"#;

fn flights() -> UiScript {
    let script = goals();
    script.run(FLIGHTS).unwrap();
    script
}

/// From a planned hand-in flight, walk up to the flight master: the map is open next.
fn at_the_master(script: &mut UiScript) {
    script.run("Goals.on = true").unwrap();
    ticks(script, 0.75);
    assert_eq!(text(script, "return doing()"), "goal:hand_in_quests");
    assert_eq!(text(script, "return moves()"), "move 50,0");
    script
        .run("pos = { x = 30, y = 0, z = 0, map = 0 } master.distance = 15 units = { master }")
        .unwrap();
    ticks(script, 0.5);
    assert_eq!(text(script, "return moves()"), "move 50,0 interact 0xF");
}

#[test]
fn an_unknown_flight_master_near_is_talked_to_and_its_path_learnt() {
    let mut script = flights();
    script
        .run("LOG = {} master.taxi = 'unknown' units = { master }")
        .unwrap();
    assert!(script
        .eval::<bool>("return Goals.facts().flight_path_unknown_near")
        .unwrap());
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "learn_flight_path"
    );
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:learn_flight_path");
    assert!(text(&script, "return moves()").contains("interact 0xF"));
    // The server's first-visit learn: the green "!" goes.
    script.run("master.taxi = 'known'").unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(
        text(&script, "return heard('FLIGHT_PATH_LEARNED')"),
        "FLIGHT_PATH_LEARNED Thorgrum Borrelson"
    );
    assert!(!script
        .eval::<bool>("return Goals.facts().flight_path_unknown_near")
        .unwrap());
}

#[test]
fn a_flight_master_the_realm_named_is_walked_to_only_nearby_and_only_once() {
    let script = flights();
    script.run("LOG = {}").unwrap();
    let near = |script: &UiScript, x: i32| -> bool {
        script
            .run(&format!(
                "Goals.places.flightmaster = {{ {{ name = 'Thor', x = {x}, y = 0, z = 0, map = 0 }} }}"
            ))
            .unwrap();
        script
            .eval("return Goals.facts().flight_path_unknown_near")
            .unwrap()
    };
    // Thelsamar's master stands at a known node; Sentinel Hill's is unknown but 3000 yards off.
    assert!(!near(&script, 45));
    assert!(!near(&script, -2990));
    script
        .run("pos = { x = -2800, y = 0, z = 0, map = 0 }")
        .unwrap();
    assert!(near(&script, -2990));
    // Talked to once, it waits `learn_wait` before another walk there.
    script
        .run("Goals.actions.learn_flight_path.run({})")
        .unwrap();
    assert!(!near(&script, -2990));
}

#[test]
fn an_open_map_records_its_fares_and_the_known_paths_are_listed() {
    let mut script = flights();
    script
        .run(
            "MAP = { { 'Thelsamar, Loch Modan', 'CURRENT', 0 }, { 'Ironforge, Dun Morogh', 'REACHABLE', 45 } }",
        )
        .unwrap();
    script.fire_event("TAXIMAP_OPENED", vec![]);
    assert_eq!(
        script
            .eval::<i64>("return Taxi.fares['Thelsamar, Loch Modan']['Ironforge, Dun Morogh']")
            .unwrap(),
        45
    );
    assert_eq!(
        text(
            &script,
            "local k, read = Taxi.known() local n = {} for _, v in ipairs(k) do n[#n + 1] = v.name end \
             return table.concat(n, '; ') .. ' ' .. tostring(read)"
        ),
        "Thelsamar, Loch Modan; Ironforge, Dun Morogh true"
    );
    // The plan prices the flight at the fare seen.
    assert_eq!(
        script
            .eval::<i64>("return Taxi.plan(3050, 0).fare")
            .unwrap(),
        45
    );
}

#[test]
fn a_long_hand_in_flies_between_known_paths_and_walks_on_after_landing() {
    let mut script = flights();
    assert_eq!(
        text(
            &script,
            "local p = Taxi.plan(3050, 0) return p.from.name .. ' -> ' .. p.to.name .. ' ' .. p.fly .. '/' .. p.walk"
        ),
        "Thelsamar, Loch Modan -> Ironforge, Dun Morogh 191/566"
    );
    at_the_master(&mut script);
    script
        .run(
            "MAP = { { 'Thelsamar, Loch Modan', 'CURRENT', 0 }, { 'Ironforge, Dun Morogh', 'REACHABLE', 45 } }",
        )
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(
        text(&script, "return moves()"),
        "move 50,0 interact 0xF take 2"
    );
    script.run("on_taxi = true units = {}").unwrap();
    ticks(&mut script, 5.0);
    assert_eq!(text(&script, "return doing()"), "goal:hand_in_quests");
    assert_eq!(
        text(&script, "return Move.state"),
        "flying to Ironforge, Dun Morogh"
    );
    // Landed: the walk goes on to the hand-in.
    script
        .run("on_taxi = false pos = { x = 3000, y = 5, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(
        text(&script, "return heard('FLIGHT_DONE')"),
        "FLIGHT_DONE true landed at Ironforge, Dun Morogh"
    );
    assert_eq!(
        text(&script, "return moves()"),
        "move 50,0 interact 0xF take 2 move 3050,0"
    );
    script
        .run("walking = false pos = { x = 3049, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return Move.last.outcome"), "arrived");
}

#[test]
fn a_short_trip_walks_and_so_does_one_we_cannot_pay_for() {
    let mut script = flights();
    script.run("hand_in_x = 300").unwrap();
    assert_eq!(
        text(&script, "return select(2, Taxi.plan(300, 0))"),
        "300 yards is a walk (min_trip 500)"
    );
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return moves()"), "move 300,0");
    // Under Upkeep's reserve (25 x 10 copper at level 5) plus the cheapest fare: on foot.
    let script = flights();
    script.run("money = 280").unwrap();
    assert_eq!(
        text(&script, "return select(2, Taxi.plan(3050, 0))"),
        "30 copper spare over the reserve, under the fare to Ironforge, Dun Morogh (50)"
    );
    script.run("money = 1000 Taxi.reserve = 980").unwrap();
    assert!(script
        .eval::<bool>("return Taxi.plan(3050, 0) == nil")
        .unwrap());
}

#[test]
fn a_fare_over_what_is_spare_at_the_map_walks_on_and_says_why() {
    let mut script = flights();
    at_the_master(&mut script);
    script
        .run(
            "MAP = { { 'Thelsamar, Loch Modan', 'CURRENT', 0 }, { 'Ironforge, Dun Morogh', 'REACHABLE', 900 } }",
        )
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(
        text(&script, "return heard('FLIGHT_DONE')"),
        "FLIGHT_DONE false the fare to Ironforge, Dun Morogh is 900 copper, 750 spare over the reserve"
    );
    assert_eq!(
        text(&script, "return moves()"),
        "move 50,0 interact 0xF close map move 3050,0"
    );
    assert_eq!(text(&script, "return doing()"), "goal:hand_in_quests");
}

#[test]
fn a_flight_master_who_greets_with_gossip_is_asked_for_the_flight() {
    let mut script = flights();
    at_the_master(&mut script);
    script
        .run("GOSSIP = { 'Hello there.', 'gossip', 'Show me where I can fly.', 'taxi' }")
        .unwrap();
    ticks(&mut script, 0.5);
    assert!(text(&script, "return moves()").ends_with("interact 0xF gossip taxi"));
}

#[test]
fn no_flight_master_at_the_node_walks_on_and_leaves_that_node_out() {
    let mut script = flights();
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 0.75);
    // The walk arrives at the node; nobody there.
    script
        .run("walking = false pos = { x = 50, y = 0, z = 0, map = 0 }")
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(
        text(&script, "return heard('FLIGHT_DONE')"),
        "FLIGHT_DONE false no flight master at Thelsamar, Loch Modan"
    );
    assert_eq!(text(&script, "return moves()"), "move 50,0 move 3050,0");
    assert_eq!(text(&script, "return doing()"), "goal:hand_in_quests");
    assert_eq!(
        text(&script, "return select(2, Taxi.plan(3050, 0))"),
        "fewer than two known flight paths on this map to fly between"
    );
}
