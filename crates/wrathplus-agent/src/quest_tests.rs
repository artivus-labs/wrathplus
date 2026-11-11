//! `wp.quest` against a stubbed quest giver and party: the dialogs, sharing, the party's quests.

use wrathplus_ui::script::UiScript;

/// Remy "Two Times" with two quests on offer through gossip; Pallytest in the party.
pub(super) const GAME: &str = r#"
calls = {}
remy = { guid = "0xR", name = "Remy", kind = "unit", quest = "available", distance = 6, x = 6, y = 0, z = 0 }
pally = { guid = "0xP", name = "Pallytest", kind = "player", distance = 3, quests = { { id = 47, complete = false } } }
offers = 2
AgentNearby = function() return { remy, pally } end
AgentInteract = function(g) calls[#calls + 1] = "interact " .. g end
GetNumPartyMembers = function() return 1 end
SendChatMessage = function(text, channel) calls[#calls + 1] = channel .. ": " .. text end
UnitAffectingCombat = function() return false end
GossipFrame = { IsShown = function() return false end }
CloseGossip = function() calls[#calls + 1] = "close" end
GetNumGossipAvailableQuests = function() return offers end
SelectGossipAvailableQuest = function(i) calls[#calls + 1] = "select " .. i end
AcceptQuest = function() calls[#calls + 1] = "accept" offers = offers - 1 if offers == 0 then remy.quest = nil end end
GetTitleText = function() return "Quest " .. (2 - offers + 1) end
UnitIsPlayer = function(u) return questnpc_player == true end
UnitInParty = function() return true end
UnitName = function(u) if u == "questnpc" then return "Pallytest" end end
signals = {}
AgentSignal = function(name, who, what, id) signals[#signals + 1] = name .. " " .. who .. " " .. what .. " " .. id end
Move = { companion_now = function() return "Pallytest" end }
"#;

pub(super) fn quests() -> UiScript {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "wp-agent-quest-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
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
fn it_takes_every_quest_a_giver_offers_then_stops() {
    let mut script = quests();
    assert!(script.eval::<bool>("return Quests.pick_up()").unwrap());
    script.tick(0.6);
    assert_eq!(calls(&script), "interact 0xR");
    // Gossip opens: the first quest, its details, accept; then ask again for the second.
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    assert_eq!(
        calls(&script),
        "interact 0xR | select 1 | accept | interact 0xR | select 1 | accept"
    );
    // Done: one line to the party.
    script.tick(0.6);
    assert!(
        calls(&script).ends_with("PARTY: Picked up: Quest 1, Quest 2."),
        "{}",
        calls(&script)
    );
    assert!(!script.eval::<bool>("return Quests.busy()").unwrap());
    let log: String = script
        .eval("return table.concat(Quests.log, '; ')")
        .unwrap();
    assert_eq!(
        log,
        "accepted Quest 1; accepted Quest 2; picked up everything on offer"
    );
}

#[test]
fn a_shared_quest_is_accepted_and_the_companions_quests_are_watched() {
    let mut script = quests();
    // Pallytest shares: the details open with them as the giver.
    script.run("questnpc_player = true").unwrap();
    script.fire_event("QUEST_DETAIL", vec![]);
    assert_eq!(calls(&script), "accept");
    // They pick up quest 52 and hand in 47: both are signalled, once each.
    script.tick(1.1);
    script
        .run("pally.quests = { { id = 52, complete = false } }")
        .unwrap();
    script.tick(1.1);
    script.tick(1.1);
    let signals: String = script
        .eval("table.sort(signals) return table.concat(signals, ' | ')")
        .unwrap();
    assert_eq!(
        signals,
        "PARTY_QUEST Pallytest accepted 52 | PARTY_QUEST Pallytest completed 47"
    );
    // The fast tier is offered the pick-up while Remy has something.
    let offered: String = script
        .eval("local o = {} for _, i in ipairs(Agent.available()) do o[#o + 1] = i.name end return table.concat(o, ',')")
        .unwrap();
    assert!(offered.contains("pick_up_quests"), "{offered}");
}

#[test]
fn a_visit_hands_in_then_takes_what_is_on_offer() {
    let mut script = quests();
    // Remy takes a finished quest back and offers one more.
    script
        .run(
            "remy.quest = 'reward' \
             GetGossipActiveQuests = function() return 'A', 5, nil, nil, 'B', 5, nil, nil, 'Old', 5, nil, 1, 'D', 5, nil, nil end \
             SelectGossipActiveQuest = function(i) calls[#calls + 1] = 'turn in ' .. i end \
             IsQuestCompletable = function() return true end \
             CompleteQuest = function() calls[#calls + 1] = 'complete' end \
             GetNumQuestChoices = function() return 0 end \
             GetQuestReward = function(i) calls[#calls + 1] = 'reward ' .. i remy.quest = 'available' offers = 1 end",
        )
        .unwrap();
    assert!(script.eval::<bool>("return Quests.hand_in()").unwrap());
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_PROGRESS", vec![]);
    script.fire_event("QUEST_COMPLETE", vec![]);
    // Nothing left to hand in, but Remy now has an offer: same visit, take it.
    script.tick(0.6);
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    script.tick(0.6);
    assert!(!script.eval::<bool>("return Quests.busy()").unwrap());
    assert_eq!(
        calls(&script),
        "interact 0xR | turn in 3 | complete | reward 1 | interact 0xR | select 1 | accept | PARTY: Handed in: Quest 1. Picked up: Quest 2."
    );
    // One signal, for the hand-in asked for, with both counts (the beta reported "pick_up", the
    // phase it ended in).
    let signals: String = script.eval("return table.concat(signals, ' | ')").unwrap();
    assert_eq!(signals, "QUESTS_DONE hand_in 1 1");
}

/// Every call ends in its own AGENT_QUESTS_DONE: a second call while a visit runs ends that one
/// first, with what it did so far, then starts afresh.
#[test]
fn a_call_while_a_visit_runs_ends_it_with_its_own_signal() {
    let mut script = quests();
    assert!(script.eval::<bool>("return Quests.pick_up()").unwrap());
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    assert!(script.eval::<bool>("return Quests.pick_up()").unwrap());
    let signals: String = script.eval("return table.concat(signals, ' | ')").unwrap();
    assert_eq!(signals, "QUESTS_DONE pick_up 1 0");
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    script.tick(0.6);
    assert!(!script.eval::<bool>("return Quests.busy()").unwrap());
    let signals: String = script.eval("return table.concat(signals, ' | ')").unwrap();
    assert_eq!(signals, "QUESTS_DONE pick_up 1 0 | QUESTS_DONE pick_up 1 0");
}

#[test]
fn the_quest_log_reads_in_3_3_5_order_and_a_visit_ends_with_a_signal() {
    let mut script = quests();
    script
        .run(
            "LOG = { { 'Elwynn Forest', 0, nil, 0, 1, nil, nil, nil, 0 }, \
                     { 'The Jasperlode Mine', 10, nil, 0, nil, nil, nil, nil, 76 }, \
                     { 'Wolves Across the Border', 6, nil, 0, nil, nil, 1, nil, 33 } } \
             GetNumQuestLogEntries = function() return #LOG, 2 end \
             GetQuestLogTitle = function(i) return unpack(LOG[i]) end \
             GetNumQuestLeaderBoards = function(i) return i == 3 and 1 or 0 end \
             GetQuestLogLeaderBoard = function(j, i) return 'Tough Wolf Meat: 8/8', 'item', 1 end",
        )
        .unwrap();
    let listed: String = script
        .eval(
            "local out = {} for _, q in ipairs(Quests.list()) do \
               out[#out + 1] = q.id .. ' ' .. q.title .. (q.complete and ' (complete)' or '') .. ' ' .. #q.objectives \
             end return table.concat(out, ' | ')",
        )
        .unwrap();
    assert_eq!(
        listed,
        "76 The Jasperlode Mine 0 | 33 Wolves Across the Border (complete) 1"
    );
    // A pick-up visit: AGENT_QUESTS_DONE with what it took.
    script.run("signals = {} offers = 1").unwrap();
    visit(&mut script);
    let signals: String = script.eval("return table.concat(signals, ' | ')").unwrap();
    assert!(signals.contains("QUESTS_DONE pick_up 1 0"), "{signals}");
}

/// Pick up the one quest on offer, through gossip, and let the visit end.
fn visit(script: &mut UiScript) {
    script.run("Quests.pick_up()").unwrap();
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    for _ in 0..10 {
        script.tick(0.6);
    }
}

#[test]
fn where_gives_the_hand_in_with_its_map_and_text_reads_the_log_without_moving_its_selection() {
    let script = quests();
    script
        .run(
            r#"
            AgentQuestPOI = function(id)
              if id == 37 then
                return { { objective = -1, map = 0, x = -9336, y = -986 }, { objective = 0, map = 0, x = -9400, y = -900 } }
              end
              if id == 5 then return {} end
            end
            LOG = { { "Elwynn Forest", true }, { "Find the Lost Guards", false, 37 }, { "Elmore's Task", false, 1097 } }
            GetNumQuestLogEntries = function() return #LOG end
            GetQuestLogTitle = function(i) return LOG[i][1], 10, nil, 0, LOG[i][2] or nil, nil, 1, nil, LOG[i][3] end
            selected, selections = 2, {}
            GetQuestLogSelection = function() return selected end
            SelectQuestLogEntry = function(i) selected = i selections[#selections + 1] = i end
            GetQuestLogQuestText = function()
              if selected == 3 then return "A story.", "Speak with Grimand Elmore in Stormwind." end
              return "A story.", ""
            end
            "#,
        )
        .unwrap();
    let found: String = script
        .eval(
            "local w = Quests.where(37) return w.hand_in.x .. ' ' .. w.hand_in.y .. ' ' .. w.hand_in.map \
             .. ' ' .. #w.objectives .. ' ' .. w.objectives[1].map",
        )
        .unwrap();
    assert_eq!(found, "-9336 -986 0 1 0");
    // Not answered yet is nil; answered with no areas is a quest with no known hand-in.
    assert!(script
        .eval::<bool>("return Quests.where(1097) == nil and Quests.where(5).hand_in == nil")
        .unwrap());
    let said: String = script
        .eval("return tostring(Quests.text(1097)) .. ' / ' .. tostring(Quests.text(37)) .. ' / ' .. tostring(Quests.text(999))")
        .unwrap();
    assert_eq!(said, "Speak with Grimand Elmore in Stormwind. / nil / nil");
    assert_eq!(
        script
            .eval::<String>("return selected .. ' ' .. table.concat(selections, ' ')")
            .unwrap(),
        "2 3 2 2"
    );
}

/// A cinematic hides the quest givers: a visit waits, rather than conclude nobody is left.
#[test]
fn a_cinematic_holds_a_visit_open() {
    let mut script = quests();
    script
        .run(
            "cinematic = 'a cinematic is playing' AgentCinematic = function() return cinematic end \
             AgentNearby = function() return cinematic and {} or { remy, pally } end \
             Quests.pick_up()",
        )
        .unwrap();
    for _ in 0..5 {
        script.tick(0.6);
    }
    assert!(script.eval::<bool>("return Quests.busy()").unwrap());
    assert_eq!(calls(&script), "");
    let state: String = script.eval("return Quests.state").unwrap();
    assert_eq!(state, "waiting: a cinematic is playing");
    script.run("cinematic = nil").unwrap();
    script.tick(0.6);
    assert_eq!(calls(&script), "interact 0xR");
}
