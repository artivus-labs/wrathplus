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
AgentMoveTo = function(x, y) calls[#calls + 1] = string.format("move %d,%d", x, y) walking = true return {} end
AgentStop = function() calls[#calls + 1] = "stop" walking = false end
UnitExists = function(u) return u == "party1" or u == "player" end
UnitGUID = function(u) if u == "party1" then return "0xP" elseif u == "player" then return "0xME" end end
UnitName = function(u) if u == "party1" then return "Pallytest" end end
UnitIsDeadOrGhost = function() return false end
UnitIsConnected = function() return true end
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
    let offered = |script: &UiScript| -> String {
        script
            .eval(
                "local out = {} for _, i in ipairs(Agent.available()) do out[#out + 1] = i.name .. ': ' .. i.when end \
                 return table.concat(out, ' / ')",
            )
            .unwrap()
    };
    assert_eq!(
        offered(&script),
        // stay_here is only run on request, never offered.
        "follow_companion: In a party: follow Pallytest, staying close and fighting beside them."
    );
    // Leading the party itself, it follows the first other member; a companion named wins.
    script
        .run(
            "GetPartyLeaderIndex = function() return 0 end GetNumPartyMembers = function() return 2 end UnitExists = function() return true end \
             UnitName = function(u) return ({ party1 = 'Pallytest', party2 = 'Agentone' })[u] end",
        )
        .unwrap();
    assert!(offered(&script).starts_with("follow_companion: In a party: follow Pallytest,"));
    script.run("Move.companion = 'agentone'").unwrap();
    assert!(offered(&script).starts_with("follow_companion: In a party: follow Agentone,"));
}

#[test]
fn going_to_a_place_or_a_name_not_in_sight() {
    let mut script = moving();
    let (ok, why): (bool, Option<String>) =
        script.eval("return Move.to('Marshal Dughan')").unwrap();
    assert!(!ok);
    assert_eq!(
        why.as_deref(),
        Some("Marshal Dughan is not in sight: walk closer, or give coordinates")
    );
    assert!(script.eval::<bool>("return Move.mode == nil").unwrap());
    // A place: the route, then arrival when the walk does.
    assert!(script.eval::<bool>("return Move.to(-9460, 64)").unwrap());
    assert!(
        calls(&script).ends_with("move -9460,64"),
        "{}",
        calls(&script)
    );
    script.tick(0.3);
    assert_eq!(state(&script), "going to (-9460, 64) (9460 yd)");
    script.run("walking = false").unwrap();
    script.tick(0.3);
    let (over, arrived): (bool, bool) = script.eval("return Move.reached()").unwrap();
    assert!(over && arrived);
}

#[test]
fn following_leaves_someone_elses_walk_and_an_open_window_alone() {
    let mut script = moving();
    script.run("Move.follow('Pallytest') pally.x = 20").unwrap();
    // Another walk is running (the agent sent itself to an innkeeper): follow neither stops nor
    // reroutes it.
    script.run("walking = true").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "");
    assert_eq!(state(&script), "letting another walk finish");
    // At the innkeeper with his gossip open: it waits for the window to close.
    script
        .run("walking = false GossipFrame = { IsShown = function() return true end }")
        .unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "");
    assert_eq!(state(&script), "talking to someone; following again after");
    script.run("GossipFrame = nil").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "move 18,0");
}

#[test]
fn a_window_left_open_is_closed_so_following_carries_on() {
    let mut script = moving();
    script
        .run(
            "ClassTrainerFrame = { IsShown = function() return trainer_open end } trainer_open = true \
             CloseTrainer = function() calls[#calls + 1] = 'close trainer' trainer_open = false end",
        )
        .unwrap();
    // Asked to follow while at a trainer: the window closes at once.
    script.run("Move.follow('Pallytest')").unwrap();
    assert_eq!(calls(&script), "close trainer");
    // A window opened while following (not quest work) is waited on briefly, then closed.
    script
        .run("calls = {} trainer_open = true pally.x = 20")
        .unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "");
    for _ in 0..20 {
        script.tick(0.3);
    }
    assert_eq!(calls(&script), "close trainer | move 18,0");
}

#[test]
fn dead_nothing_is_offered_and_a_companion_who_logs_off_is_not_followed() {
    let mut script = moving();
    let offered = |script: &UiScript| -> String {
        script
            .eval("local out = {} for _, i in ipairs(Agent.available()) do out[#out + 1] = i.name end return table.concat(out, ' ')")
            .unwrap()
    };
    assert_eq!(offered(&script), "follow_companion");
    script
        .run("UnitIsDeadOrGhost = function() return true end")
        .unwrap();
    assert_eq!(offered(&script), "");
    script
        .run("UnitIsDeadOrGhost = function() return false end connected = true UnitIsConnected = function() return connected end")
        .unwrap();
    // Following; then Pallytest logs off (gone from sight, and offline): following ends.
    script.run("Move.follow('Pallytest') pally.x = 20").unwrap();
    script.tick(0.3);
    assert_eq!(state(&script), "following Pallytest (20 yd)");
    script.run("connected = false pally = nil").unwrap();
    script.tick(0.3);
    assert!(script
        .eval::<bool>("return Move.mode == nil and Move.last.outcome == 'offline'")
        .unwrap());
    assert_eq!(offered(&script), "");
}

#[test]
fn it_turns_to_whoever_talks_to_it_from_close_by_when_idle() {
    let mut script = moving();
    script
        .run(
            "faced = {} AgentFace = function(g) faced[#faced + 1] = g end \
             UnitName = function(u) return ({ player = 'Agenttwo', party1 = 'Pallytest' })[u] end \
             UnitAffectingCombat = function() return false end UnitInParty = function() return false end",
        )
        .unwrap();
    let say = |script: &mut UiScript, event: &str, text: &str, who: &str| {
        script.fire_event(
            event,
            vec![
                wrathplus_ui::script::ScriptValue::Str(text.into()),
                wrathplus_ui::script::ScriptValue::Str(who.into()),
            ],
        );
    };
    // Party chat from Pallytest, 2 yards away: turn to them.
    say(&mut script, "CHAT_MSG_PARTY", "over here", "Pallytest");
    // Someone's /say that is not for us, and our own line: no turn.
    say(
        &mut script,
        "CHAT_MSG_SAY",
        "anyone selling bread?",
        "Pallytest",
    );
    say(&mut script, "CHAT_MSG_PARTY", "on my way", "Agenttwo");
    // An emote at us, but we are walking: leave it.
    script.run("walking = true").unwrap();
    say(
        &mut script,
        "CHAT_MSG_TEXT_EMOTE",
        "Pallytest waves at you.",
        "Pallytest",
    );
    let faced: String = script.eval("return table.concat(faced, ' | ')").unwrap();
    assert_eq!(faced, "0xP");
}

#[test]
fn a_walk_someone_stops_short_ends_interrupted_and_one_that_gains_no_ground_ends_stalled() {
    let mut script = moving();
    script
        .run(
            "said = {} AgentSignal = function(name, ...) said[#said + 1] = name .. ' ' .. table.concat({ ... }, ' ') end \
             UnitAffectingCombat = function() return false end UnitBuff = function() return nil end \
             AgentWalk = function() return { status = walking and 'walking' or 'stopped' } end",
        )
        .unwrap();
    assert!(script.eval::<bool>("return Move.to(50, 0)").unwrap());
    script.tick(0.3);
    // AgentStop from someone else: no end the walker announces, so Move ends it.
    script.run("walking = false").unwrap();
    script.tick(0.3);
    assert!(script
        .eval::<bool>("return Move.mode == nil and Move.last.outcome == 'interrupted'")
        .unwrap());
    // A walk that stands still: sent again once after 20 s, then over as stalled, and said.
    script.run("calls = {} said = {}").unwrap();
    assert!(script.eval::<bool>("return Move.to(50, 0)").unwrap());
    for _ in 0..70 {
        script.tick(0.3);
    }
    assert_eq!(calls(&script), "move 50,0 | move 50,0");
    assert_eq!(
        script.eval::<String>("return Move.mode or 'none'").unwrap(),
        "to"
    );
    for _ in 0..70 {
        script.tick(0.3);
    }
    assert!(script
        .eval::<bool>("return Move.mode == nil and Move.last.outcome == 'stalled'")
        .unwrap());
    let said: String = script.eval("return table.concat(said, ' | ')").unwrap();
    assert_eq!(
        said,
        "MOVE_DONE (50, 0) stalled | WALK_DONE stalled (50, 0)"
    );
    // Standing still to eat is no stall.
    script
        .run("calls = {} UnitBuff = function(_, name) if name == 'Food' then return 'Food' end end")
        .unwrap();
    assert!(script.eval::<bool>("return Move.to(50, 0)").unwrap());
    for _ in 0..140 {
        script.tick(0.3);
    }
    assert_eq!(calls(&script), "move 50,0");
    assert_eq!(
        script.eval::<String>("return Move.mode or 'none'").unwrap(),
        "to"
    );
}

/// A cinematic hides everyone: going to someone waits for it to end instead of ending "failed"
/// or "lost", and asks for no walk meanwhile.
#[test]
fn a_cinematic_holds_a_walk_to_someone_without_losing_them() {
    let mut script = moving();
    script
        .run(
            "signals = {} AgentSignal = function(name, who, how) signals[#signals + 1] = name .. ' ' .. tostring(how) end \
             cinematic = nil AgentCinematic = function() return cinematic end \
             local seen = AgentNearby AgentNearby = function() return cinematic and {} or seen() end \
             Move.lose_after = 1 pally.x = 30 Move.to('party1')",
        )
        .unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "move 27,0");
    script
        .run("cinematic = 'a cinematic is playing' walking = false")
        .unwrap();
    for _ in 0..10 {
        script.tick(0.3);
    }
    assert_eq!(calls(&script), "move 27,0");
    assert_eq!(state(&script), "waiting: a cinematic is playing");
    assert!(script
        .eval::<bool>("return Move.mode == 'to' and #signals == 0")
        .unwrap());
    // A place asked for meanwhile is refused at once, with the reason (AgentMoveTo's).
    script.run("cinematic = nil").unwrap();
    script.tick(0.3);
    assert_eq!(calls(&script), "move 27,0 | move 27,0");
}
