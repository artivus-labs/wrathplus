//! `wp.move` against a stubbed party: following with a slack band, assisting, going to a unit.

use wrathplus_ui::script::UiScript;

/// The agent at the origin; Pallytest, its party leader, somewhere along the x axis.
const GAME: &str = r#"
calls = {}
me = { x = 0, y = 0, z = 0 }
pally = { guid = "0xP", name = "Pallytest", kind = "player", x = 2, y = 0, z = 0, distance = 2 }
wolf = nil
walking = false
local function dist(u) return math.sqrt((u.x - me.x) ^ 2 + (u.y - me.y) ^ 2) end
AgentNearby = function()
  local out = {}
  for _, u in ipairs({ pally, wolf }) do if u then u.distance = dist(u) out[#out + 1] = u end end
  return out
end
AgentPosition = function() return me end
AgentWalk = function() return { status = walking and "walking" or "arrived" } end
AgentMoveTo = function(x, y) calls[#calls + 1] = string.format("move %d,%d", x, y) walking = true end
AgentStop = function() calls[#calls + 1] = "stop" walking = false end
UnitExists = function(u) return u == "party1" or u == "player" end
UnitGUID = function(u) if u == "party1" then return "0xP" elseif u == "player" then return "0xME" end end
UnitName = function(u) if u == "party1" then return "Pallytest" end end
UnitIsDeadOrGhost = function() return false end
GetNumPartyMembers = function() return 1 end
GetPartyLeaderIndex = function() return 1 end
Combat = { active = nil, engage = function(g) calls[#calls + 1] = "engage " .. g Combat.active = g return true end,
  stop = function() Combat.active = nil end }
"#;

fn moving() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-move-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    // The game's functions replace the stock ones; the module reads them when it runs.
    script.run(GAME).unwrap();
    script
}

fn calls(script: &UiScript) -> String {
    script.eval("return table.concat(calls, ' | ')").unwrap()
}

fn state(script: &UiScript) -> String {
    script.eval("return Move.state or ''").unwrap()
}

#[test]
fn following_waits_out_the_slack_then_closes_and_assists_in_a_fight() {
    let mut script = moving();
    assert!(script
        .eval::<bool>("return Move.follow('Pallytest')")
        .unwrap());
    script.tick(0.3);
    assert_eq!(state(&script), "with Pallytest");
    // A few yards off is inside the band: no walk yet.
    script.run("pally.x = 5").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "");
    // Past follow distance plus slack: close in, aiming 3 * 0.6 yd short on our side.
    script.run("pally.x = 20").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "move 18,0");
    assert_eq!(state(&script), "following Pallytest (20 yd)");
    // Arrived beside them: stop.
    script.run("me.x = 18 walking = false").unwrap();
    script.tick(0.3);
    assert_eq!(state(&script), "with Pallytest");
    // They fight a wolf: engage their target, and follow again once it is over.
    script
        .run(
            "wolf = { guid = '0xW', name = 'Wolf', kind = 'unit', reaction = 'hostile', x = 25, y = 0, z = 0, in_combat = true } \
             pally.in_combat, pally.target = true, '0xW'",
        )
        .unwrap();
    script.tick(0.3);
    assert!(calls(&script).ends_with("engage 0xW"), "{}", calls(&script));
    script
        .run("Combat.active = nil wolf = nil pally.in_combat = false pally.x = 40")
        .unwrap();
    script.tick(0.3);
    assert!(calls(&script).ends_with("move 38,0"), "{}", calls(&script));
}

#[test]
fn going_to_a_moving_unit_arrives_within_reach() {
    let mut script = moving();
    script.run("pally.x = 30 Move.to('party1')").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "move 27,0");
    // It walked on: aim again.
    script.run("me.x = 20 pally.x = 40").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "move 27,0 | move 37,0");
    script.run("me.x = 37").unwrap();
    script.tick(0.3);
    let (over, ok): (bool, bool) = script.eval("return Move.reached()").unwrap();
    assert!(over && ok);
    assert!(script.eval::<bool>("return Move.mode == nil").unwrap());
}

#[test]
fn grouped_the_fast_tier_is_offered_follow_and_stay() {
    let script = moving();
    let offered: String = script
        .eval(
            "local out = {} for _, i in ipairs(Agent.available()) do out[#out + 1] = i.name .. ': ' .. i.when end \
             return table.concat(out, ' / ')",
        )
        .unwrap();
    assert_eq!(
        offered,
        "follow_party_leader: In a party: follow the leader (Pallytest), staying close and fighting beside them. / \
         stay_here: Wait at this spot: fight what attacks you, but don't wander."
    );
}
