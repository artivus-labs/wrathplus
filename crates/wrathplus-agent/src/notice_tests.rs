//! `wp.notice` against a stubbed world: what is new in sight is signalled once (AGENT_NOTICED),
//! mutes hold the signal back but not the record, subzones are noticed on entry, and the scan
//! Goals' facts make is shared rather than read again.

use wrathplus_ui::script::UiScript;

const GAME: &str = r#"
signals, scans = {}, 0
near = {}
sub = "Northshire Abbey"
AgentPosition = function() return { x = 10, y = 20, z = 0, map = 0 } end
AgentNearby = function(yards)
  -- wp.notice's own reads (its radius); the other modules read the world on their own terms.
  if yards == Notice.radius then scans = scans + 1 end
  local out = {}
  for _, u in ipairs(near) do
    if u.distance <= (yards or 1e9) then out[#out + 1] = u end
  end
  return out
end
AgentWalk = function() return { status = "idle" } end
AgentAvoid = function() return {} end
GetRealZoneText = function() return "Elwynn Forest" end
GetSubZoneText = function() return sub end
AgentSignal = function(name, kind, who, guid, distance, x, y, extra)
  if name == "NOTICED" then
    signals[#signals + 1] = kind .. " " .. tostring(who) .. " " .. tostring(extra)
  end
end
function giver(guid, name, quest, distance, extra)
  local u = { guid = guid, kind = "unit", name = name, quest = quest, distance = distance,
    x = distance, y = 0, z = 0, level = 5, reaction = "friendly" }
  for k, v in pairs(extra or {}) do u[k] = v end
  return u
end
function said() local s = table.concat(signals, "; ") signals = {} return s end
"#;

fn notice() -> UiScript {
    let dir = std::env::temp_dir().join(format!(
        "wp-agent-notice-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn said(script: &UiScript) -> String {
    script.eval("return said()").unwrap()
}

fn int(script: &UiScript, code: &str) -> i64 {
    script.eval(code).unwrap()
}

#[test]
fn something_new_in_sight_is_signalled_once_per_kind() {
    let mut script = notice();
    script
        .run(
            r#"near = {
              giver("0x1", "Deputy Willem", "available", 12),
              giver("0x2", "Smith Argus", nil, 25, { npc = { "vendor", "repair" }, title = "Weaponsmith" }),
              giver("0x3", "Far Away", "available", 90),
            }"#,
        )
        .unwrap();
    script.tick(1.1);
    // The subzone we stand in first, then what is in sight, nearest first; one notice per service.
    assert_eq!(
        said(&script),
        "subzone Northshire Abbey Elwynn Forest; quest_available Deputy Willem nil; \
         vendor Smith Argus Weaponsmith; repair Smith Argus Weaponsmith"
    );
    // Seen again: nothing new.
    script.tick(1.1);
    assert_eq!(said(&script), "");
    // Walking closer brings the far one inside the radius.
    script.run("near[3].distance = 50").unwrap();
    script.tick(1.1);
    assert_eq!(said(&script), "quest_available Far Away nil");
    assert_eq!(int(&script, "return #Notice.recent"), 5);
    let guid: String = script.eval("return Notice.recent[2].guid").unwrap();
    assert_eq!(guid, "0x1");
}

#[test]
fn a_muted_kind_is_recorded_but_not_signalled() {
    let mut script = notice();
    script
        .run(
            r#"Notice.mute("player")
            near = { { guid = "0x9", kind = "player", name = "Bob", level = 7, distance = 10, x = 1, y = 1 },
                     giver("0x1", "Deputy Willem", "available", 12) }"#,
        )
        .unwrap();
    script.tick(1.1);
    assert_eq!(
        said(&script),
        "subzone Northshire Abbey Elwynn Forest; quest_available Deputy Willem nil"
    );
    let muted: bool = script
        .eval("for _, n in ipairs(Notice.recent) do if n.kind == 'player' then return n.muted end end")
        .unwrap();
    assert!(muted);
    // The digest counts it, muted.
    let digest: String = script
        .eval(
            "local p = Notice.passed(0) local out = {} \
             for _, k in ipairs(p.kinds) do out[#out + 1] = k.kind .. '=' .. k.count .. '/' .. k.muted \
             .. ':' .. table.concat(k.names, ',') end table.sort(out) return table.concat(out, ' ')",
        )
        .unwrap();
    assert_eq!(
        digest,
        "player=1/1:Bob quest_available=1/0:Deputy Willem subzone=1/0:Northshire Abbey"
    );
    // Unmuted, a new player is signalled again.
    script
        .run(r#"Notice.unmute("player") near[#near + 1] = { guid = "0xA", kind = "player", name = "Sue", distance = 20, x = 2, y = 2 }"#)
        .unwrap();
    script.tick(1.1);
    assert_eq!(said(&script), "player Sue nil");
}

#[test]
fn a_filter_mutes_grey_quests_only() {
    let mut script = notice();
    script
        .run(
            r#"Notice.mute("quest_available", "n.trivial")
            near = { giver("0x1", "Old Timer", "available_low", 10), giver("0x2", "Marshal", "available", 15),
                     giver("0x3", "Hand-in", "reward", 18) }"#,
        )
        .unwrap();
    script.tick(1.1);
    assert_eq!(
        said(&script),
        "subzone Northshire Abbey Elwynn Forest; quest_available Marshal nil; quest_turnin Hand-in nil"
    );
    let trivial: bool = script
        .eval("return Notice.recent[2].trivial and Notice.recent[2].muted")
        .unwrap();
    assert!(trivial);
    assert_eq!(int(&script, "return Notice.mutes().quest_available"), 1);
    // A bad mute is refused with the kinds.
    let err = script
        .run(r#"Notice.mute("dragons")"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("quest_available"), "{err}");
}

#[test]
fn a_subzone_is_noticed_the_first_time_it_is_entered() {
    let mut script = notice();
    script.tick(1.1);
    assert_eq!(said(&script), "subzone Northshire Abbey Elwynn Forest");
    script.run(r#"sub = "Northshire Vineyards""#).unwrap();
    script.tick(1.1);
    assert_eq!(said(&script), "subzone Northshire Vineyards Elwynn Forest");
    script.run(r#"sub = "Northshire Abbey""#).unwrap();
    script.tick(1.1);
    script.run(r#"sub = """#).unwrap();
    script.tick(1.1);
    assert_eq!(said(&script), "");
    // Forgotten, it is new again.
    script
        .run(r#"Notice.forget("subzone") sub = "Northshire Abbey""#)
        .unwrap();
    script.tick(1.1);
    assert_eq!(said(&script), "subzone Northshire Abbey Elwynn Forest");
}

#[test]
fn nodes_chests_mailboxes_and_unfriendly_rares_are_noticed() {
    let mut script = notice();
    script
        .run(
            r#"said_sub = nil
            near = {
              { guid = "0xG1", kind = "gameobject", name = "Peacebloom", entry = 1617, go_type = 3, lock = "GatherHerbs", distance = 5, x = 100, y = 100 },
              { guid = "0xG2", kind = "gameobject", name = "Copper Vein", entry = 1731, go_type = 3, lock = "Mine", distance = 6, x = 110, y = 100 },
              { guid = "0xG3", kind = "gameobject", name = "Battered Chest", entry = 2843, go_type = 3, distance = 7, x = 120, y = 100 },
              { guid = "0xG4", kind = "gameobject", name = "Crate of Elunite", entry = 9999, go_type = 3, distance = 8, x = 130, y = 100 },
              { guid = "0xG5", kind = "gameobject", name = "Mailbox", entry = 32349, go_type = 19, distance = 9, x = 140, y = 100 },
              giver("0x7", "Guard Thomas", nil, 10, { classification = "elite" }),
              giver("0x8", "Mother Fang", nil, 30, { classification = "rare", reaction = "hostile", level = 10 }),
            }"#,
        )
        .unwrap();
    script.tick(1.1);
    assert_eq!(
        said(&script),
        "subzone Northshire Abbey Elwynn Forest; node Peacebloom herb; node Copper Vein ore; \
         chest Battered Chest nil; mailbox Mailbox nil; rare Mother Fang level 10 rare"
    );
    // The same node by its place, though its guid changed (a respawn).
    script.run(r#"near = { { guid = "0xG9", kind = "gameobject", name = "Peacebloom", entry = 1617, go_type = 3, lock = "GatherHerbs", distance = 5, x = 101, y = 101 } }"#).unwrap();
    script.tick(1.1);
    assert_eq!(said(&script), "");
}

#[test]
fn a_listener_can_take_a_notice_in_hand() {
    let mut script = notice();
    script
        .run(
            r#"heard = {}
            Notice.listen("detour", function(n)
              heard[#heard + 1] = n.name
              return n.kind == "quest_available"
            end)
            near = { giver("0x1", "Deputy Willem", "available", 12), giver("0x2", "Innkeeper Farley", nil, 20, { npc = { "innkeeper" } }) }"#,
        )
        .unwrap();
    script.tick(1.1);
    assert_eq!(
        said(&script),
        "subzone Northshire Abbey Elwynn Forest; innkeeper Innkeeper Farley nil"
    );
    assert_eq!(int(&script, "return #heard"), 3);
    let handled: bool = script.eval("return Notice.recent[2].handled").unwrap();
    assert!(handled);
}

#[test]
fn the_digest_reads_from_a_cursor() {
    let mut script = notice();
    script
        .run(r#"near = { giver("0x1", "Deputy Willem", "available", 12) }"#)
        .unwrap();
    script.tick(1.1);
    let next = int(&script, "return Notice.passed().next");
    assert_eq!(next, 2);
    assert_eq!(
        int(&script, &format!("return #Notice.passed({next}).kinds")),
        0
    );
    script
        .run(r#"near[2] = giver("0x2", "Marshal McBride", "available", 14)"#)
        .unwrap();
    script.tick(1.1);
    assert_eq!(
        int(
            &script,
            &format!("return Notice.passed({next}).kinds[1].count")
        ),
        1
    );
    // A cursor from before a client restart (past the end) reads everything kept.
    assert_eq!(int(&script, "return Notice.passed(999).kinds[1].count"), 2);
}

#[test]
fn the_goals_scan_is_shared() {
    let mut script = notice();
    // A radius no other module reads with, so the stub counts wp.notice's own reads only.
    script.run("Notice.radius = 55").unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return scans"), 1);
    // Goals' facts read the world: wp.notice looks at that list, and its next pass does not read
    // the world again; the one after does.
    script
        .run(r#"near = { giver("0x1", "Deputy Willem", "available", 12) } Goals.facts()"#)
        .unwrap();
    let after_facts = int(&script, "return scans");
    assert!(said(&script).contains("quest_available Deputy Willem"));
    script.tick(1.1);
    assert_eq!(int(&script, "return scans"), after_facts);
    script.tick(1.1);
    assert_eq!(int(&script, "return scans"), after_facts + 1);
}
