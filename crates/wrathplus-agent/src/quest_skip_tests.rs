//! Quest givers a visit finds nothing at are skipped for a while (`Quests.attempt`/`settle`):
//! through the fast tier's intent, through the goal action, and against a real gossip menu.

use super::goals_tests::{goals, text, ticks};
use super::quest_tests::GAME;
use wrathplus_ui::script::{GossipMenu, GossipOptionView, GossipQuestRow, UiScript};

/// Smith Argus in Goldshire: a "!" for a quest our own module refuses, as `pickup_filter` did.
const ARGUS: &str = r##"
remy.name, remy.guid = "Smith Argus", "0xA"
LOG = { { "Kobold Candles", 9, nil, 0, nil, nil, nil, nil, 60 } }
GetNumQuestLogEntries = function() return #LOG end
GetQuestLogTitle = function(i) return unpack(LOG[i]) end
GetNumQuestLeaderBoards = function() return 0 end
AgentSignal = function(...)
  local parts = {}
  for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
  signals[#signals + 1] = table.concat(parts, " ")
end
function doing() local a = Agent.activity() return a and a.name or "idle" end
"##;

const PICKUP_FILTER: &str =
    "Quests.pick_up = function() return 'skipped: only Smith Argus offers' end\nreturn {}\n";

fn argus() -> (UiScript, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wp-agent-quest-skip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pickup_filter.lua"), PICKUP_FILTER).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir.clone()).unwrap();
    script.run(GAME).unwrap();
    script.run(ARGUS).unwrap();
    (script, dir)
}

fn offered(script: &UiScript) -> bool {
    script
        .eval("for _, i in ipairs(Agent.available()) do if i.name == 'pick_up_quests' then return true end end return false")
        .unwrap()
}

#[test]
fn a_giver_who_gives_nothing_is_skipped_for_longer_each_time_until_the_log_changes() {
    let (mut script, dir) = argus();
    script.run("require('pickup_filter')").unwrap();
    assert!(offered(&script));
    script.run("Agent.run('pick_up_quests')").unwrap();
    script.tick(0.3);
    // Nothing taken: Argus stops counting, said in the log, and signalled with our own modules.
    assert!(!offered(&script));
    assert_eq!(
        text(&script, "return Quests.log[#Quests.log]"),
        "Smith Argus offered nothing we took; skipping them for 2 min"
    );
    assert_eq!(
        text(&script, "return table.concat(Quests.skipped(), ', ')"),
        "Smith Argus (offered nothing we took, 2 min left)"
    );
    let signals = text(&script, "return table.concat(signals, ' | ')");
    assert!(
        signals.contains(
            "NOOP pick_up_quests Smith Argus offered nothing we took; skipping them for 2 min pickup_filter"
        ),
        "{signals}"
    );
    // Back after two minutes; nothing again: twice as long.
    for _ in 0..121 {
        script.tick(1.0);
    }
    assert!(offered(&script));
    script.run("Agent.run('pick_up_quests')").unwrap();
    script.tick(0.3);
    assert_eq!(
        text(&script, "return Quests.log[#Quests.log]"),
        "Smith Argus offered nothing we took; skipping them for 4 min"
    );
    // A log update that changes nothing keeps him skipped; a new quest in the log forgets it.
    script.fire_event("QUEST_LOG_UPDATE", vec![]);
    assert!(!offered(&script));
    script
        .run("LOG[2] = { 'Princess Must Die!', 12, nil, 0, nil, nil, nil, nil, 88 }")
        .unwrap();
    script.fire_event("QUEST_LOG_UPDATE", vec![]);
    assert!(offered(&script));
    // So does a level.
    script.run("Agent.run('pick_up_quests')").unwrap();
    script.tick(0.3);
    assert!(!offered(&script));
    script.fire_event("PLAYER_LEVEL_UP", vec![]);
    assert!(offered(&script));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_visit_that_takes_a_quest_skips_nobody() {
    let (mut script, dir) = argus();
    script.run("Agent.run('pick_up_quests')").unwrap();
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script
        .run("LOG[2] = { 'Quest 1', 10, nil, 0, nil, nil, nil, nil, 70 }")
        .unwrap();
    script.fire_event("QUEST_DETAIL", vec![]);
    script.tick(0.6);
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    script.fire_event("QUEST_DETAIL", vec![]);
    for _ in 0..4 {
        script.tick(0.6);
    }
    assert_eq!(text(&script, "return doing()"), "idle");
    assert_eq!(
        text(&script, "return table.concat(Quests.skipped(), ', ')"),
        ""
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Smith Argus's menu as the server sends it: his quest row beside the "Train me" option.
fn argus_menu(quests: Vec<GossipQuestRow>) -> Option<GossipMenu> {
    Some(GossipMenu {
        greeting: "Need something?".into(),
        quests,
        options: vec![GossipOptionView {
            label: "Train me.".into(),
            icon_type: "trainer".into(),
            ..Default::default()
        }],
    })
}

#[test]
fn the_gossip_path_selects_the_quest_row_beside_the_options() {
    // The real gossip verbs, not the harness's stubs.
    let dir = std::env::temp_dir().join(format!("wp-agent-quest-menu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut script = UiScript::new().unwrap();
    super::install(&script, dir.clone()).unwrap();
    script
        .run("REAL = { GetNumGossipAvailableQuests, SelectGossipAvailableQuest, CloseGossip }")
        .unwrap();
    script.run(GAME).unwrap();
    script
        .run("GetNumGossipAvailableQuests, SelectGossipAvailableQuest, CloseGossip = unpack(REAL)")
        .unwrap();
    let row = |title: &str, active: bool| GossipQuestRow {
        title: title.into(),
        level: 10,
        active,
        ..Default::default()
    };
    // A finished quest's row first, then the one on offer: the visit picks the available one.
    script.set_gossip(argus_menu(vec![
        row("Report to Goldshire", true),
        row("Elmore's Task", false),
    ]));
    script.run("Quests.pick_up()").unwrap();
    script.tick(0.6);
    script.fire_event("GOSSIP_SHOW", vec![]);
    assert_eq!(script.take_gossip_quest_selects(), vec![2]);
    assert!(!script.take_gossip_close());
    // Only the options: nothing to pick up, so the menu is closed, not "Train me" chosen.
    script.set_gossip(argus_menu(vec![]));
    script.fire_event("GOSSIP_SHOW", vec![]);
    assert!(script.take_gossip_quest_selects().is_empty());
    assert!(script.take_gossip_selects().is_empty());
    assert!(script.take_gossip_close());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_goal_action_that_takes_nothing_stops_counting_the_giver_as_near() {
    let mut script = goals();
    script
        .run(
            "units = { { guid = '0xA', name = 'Smith Argus', kind = 'unit', reaction = 'friendly', quest = 'available', \
             distance = 8, x = 8, y = 0, z = 0 } } Goals.on = true",
        )
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "pick_up_quests"
    );
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:pick_up_quests");
    // He is talked to again and again, and no dialog gives anything, until the visit gives up.
    ticks(&mut script, 92.0);
    assert!(text(&script, "return heard('ACTION_FAILED')")
        .contains("ACTION_FAILED progress pick_up_quests took nothing"));
    assert_eq!(
        text(&script, "return heard('NOOP')"),
        "NOOP pick_up_quests Smith Argus offered nothing we took; skipping them for 2 min "
    );
    assert!(!script
        .eval::<bool>("return Goals.facts().quest_giver_near")
        .unwrap());
    assert_ne!(
        text(&script, "return plan_of('progress')"),
        "pick_up_quests"
    );
}
