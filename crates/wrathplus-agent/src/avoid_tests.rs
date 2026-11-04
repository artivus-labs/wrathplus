//! `wp.avoid` against a stubbed walk: circles round hostile mobs, and asking before passing them.

use wrathplus_ui::script::UiScript;

/// A level 7 walking east along y = 0; a wolf pack ahead on the route, a boar far off it.
const GAME: &str = r#"
calls = {}
avoided = {}
me = { x = 0, y = 0, z = 0 }
wolf = { guid = "0xW", name = "Mangy Wolf", kind = "unit", reaction = "hostile", level = 6, x = 60, y = 5 }
boar = { guid = "0xB", name = "Young Boar", kind = "unit", reaction = "hostile", level = 5, x = 0, y = 80 }
sheep = { guid = "0xS", name = "Sheep", kind = "unit", reaction = "neutral", level = 1, x = 30, y = 0 }
AgentNearby = function() return { wolf, boar, sheep } end
AgentPosition = function() return me end
walking = true
AgentWalk = function() return { status = walking and "walking" or "arrived", goal = { 120, 0, 0 }, route = { { 120, 0, 0 } } } end
AgentAvoid = function(zones) avoided = zones return #zones end
AgentMoveTo = function(x, y) calls[#calls + 1] = string.format("move %d,%d", x, y) return {} end
AgentInteract = function(g) calls[#calls + 1] = "interact " .. g end
UnitLevel = function() return 7 end
UnitIsDeadOrGhost = function() return false end
"#;

fn avoiding() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-avoid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn avoided(script: &UiScript) -> String {
    script
        .eval("local out = {} for _, z in ipairs(avoided) do out[#out + 1] = z.guid .. ' ' .. z.r end return table.concat(out, ', ')")
        .unwrap()
}

#[test]
fn hostile_mobs_become_circles_and_one_on_the_route_is_asked_about() {
    let mut script = avoiding();
    script.tick(1.1);
    // Hostiles only, at their aggro radius (20 - 1 level below + 3 margin; 20 - 2 + 3).
    assert_eq!(avoided(&script), "0xW 22, 0xB 21");
    // The wolf is on the walk: the fast tier is asked, once.
    let asked: String = script
        .eval("local q = Agent.questions() return #q .. ' ' .. q[1].name .. ': ' .. q[1].question")
        .unwrap();
    assert_eq!(
        asked,
        "1 pass_hostiles: Your walk to (120, 0) passes 1 hostile mob on the way, not at the destination: Mangy Wolf (level 6). Go round them, or fight through?"
    );
    // Round: the walk is routed again, now round the circle.
    script
        .run("Agent.resolve(Agent.questions()[1].id, 'around')")
        .unwrap();
    assert_eq!(
        script
            .eval::<String>("return table.concat(calls, ' | ')")
            .unwrap(),
        "move 120,0"
    );
    script.tick(1.1);
    assert_eq!(script.eval::<i64>("return #Agent.questions()").unwrap(), 0);
}

#[test]
fn told_to_fight_it_walks_through_and_a_fought_mob_is_never_avoided() {
    let mut script = avoiding();
    script.tick(1.1);
    script
        .run("Agent.resolve(Agent.questions()[1].id, 'fight')")
        .unwrap();
    script.tick(1.1);
    assert_eq!(avoided(&script), "0xB 21");
    assert_eq!(
        script
            .eval::<String>("return table.concat(calls, ' | ')")
            .unwrap(),
        ""
    );
    // Switched off: no circles at all.
    script.run("Avoid.on = false").unwrap();
    script.tick(1.1);
    assert_eq!(avoided(&script), "");
}

#[test]
fn mobs_at_the_walks_goal_are_where_it_is_going_not_in_its_way() {
    let mut script = avoiding();
    // Walking to the wolf's camp: it is the destination, so no circle and no question.
    script
        .run("AgentWalk = function() return { status = 'walking', goal = { 62, 5, 0 }, route = { { 62, 5, 0 } } } end")
        .unwrap();
    script.tick(1.1);
    assert_eq!(avoided(&script), "0xB 21");
    assert_eq!(script.eval::<i64>("return #Agent.questions()").unwrap(), 0);
}
