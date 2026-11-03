//! `wp.quest` against a stubbed quest giver and party: the dialogs, sharing, the party's quests.

use wrathplus_ui::script::UiScript;

/// Remy "Two Times" with two quests on offer through gossip; Pallytest in the party.
const GAME: &str = r#"
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

fn quests() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-quest-{}", std::process::id()));
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
             GetGossipActiveQuests = function() return 'Old', 5, false, true end \
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
        "interact 0xR | turn in 1 | complete | reward 1 | interact 0xR | select 1 | accept | PARTY: Handed in: Quest 1. Picked up: Quest 2."
    );
}
