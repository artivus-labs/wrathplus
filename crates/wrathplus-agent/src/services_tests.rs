//! `wp.services` against a stubbed `AgentServices`: asked on entering the world and on a new zone,
//! the answer handed to `Goals.learn_places`, and a realm without the module asked no more.

use wrathplus_ui::script::UiScript;

const GAME: &str = r#"
asks, learned = 0, {}
zone = "Northshire Valley"
answers = {}
AgentPosition = function() return { x = 0, y = 0, z = 0 } end
AgentNearby = function() return {} end
AgentWalk = function() return { status = "idle" } end
AgentAvoid = function() return {} end
GetRealZoneText = function() return zone end
AgentServices = function(kinds)
  asks = asks + 1
  local a = table.remove(answers, 1)
  if a == nil then return { pending = true } end
  if a == "absent" then return nil, "this realm does not say where services are (it has no mod-agent-nav)" end
  return a
end
"#;

fn services_with_real_goals() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-services-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn services() -> UiScript {
    let script = services_with_real_goals();
    script
        .run("Goals.learn_places = function(places) learned[#learned + 1] = places end")
        .unwrap();
    script
}

fn int(script: &UiScript, code: &str) -> i64 {
    script.eval(code).unwrap()
}

#[test]
fn the_realms_places_are_learned_on_arrival_and_in_each_new_zone() {
    let mut script = services();
    // Asked at once: pending, then asked again each second until the answer is in.
    script
        .run(
            "queued = { { kind = 'vendor', name = 'Brother Danil', x = 1, y = 2, z = 3, map = 0, \
             zone = 'Elwynn Forest' } }",
        )
        .unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 1);
    assert_eq!(int(&script, "return #learned"), 0);
    script.run("answers = { queued }").unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 2);
    assert_eq!(int(&script, "return #learned"), 1);
    let name: String = script.eval("return learned[1][1].name").unwrap();
    assert_eq!(name, "Brother Danil");
    assert_eq!(int(&script, "return #Services.places"), 1);
    // Nothing more until a new zone (or `every` seconds).
    script.tick(1.1);
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 2);
    script
        .run("zone = 'Elwynn Forest' answers = { {} }")
        .unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 3);
    assert_eq!(int(&script, "return #learned"), 2);
    // Services.ask() asks now.
    script.run("answers = { {} } Services.ask()").unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 4);
}

#[test]
fn a_realm_without_the_module_is_asked_again_only_in_the_next_zone() {
    let mut script = services();
    script.run("answers = { 'absent' }").unwrap();
    script.tick(1.1);
    let why: String = script.eval("return Services.why").unwrap();
    assert!(why.contains("mod-agent-nav"), "{why}");
    for _ in 0..5 {
        script.tick(1.1);
    }
    assert_eq!(int(&script, "return asks"), 1);
    script
        .run("zone = 'Goldshire' answers = { 'absent' }")
        .unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 2);
    assert_eq!(int(&script, "return #learned"), 0);
}

#[test]
fn services_learn_all_protocol_kinds_with_real_goals_while_goals_are_off() {
    let mut script = services_with_real_goals();
    script
        .run(
            r#"
            Goals.on = false
            kinds = { "vendor", "questgiver-hub", "trainer_first_aid", "repair", "trainer",
              "flightmaster", "innkeeper", "banker", "trainer_cooking", "trainer_profession" }
            queued = {}
            for i, kind in ipairs(kinds) do
              queued[i] = { kind = kind, name = "Service " .. i, x = i, y = 2, z = 3,
                map = 0, zone = zone }
            end
            "#,
        )
        .unwrap();
    // Exercise the actual frame poll: pending first, then a mixed answer whose hub used to
    // abort learning after vendor and before first aid (and leave Services.learned at zero).
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 1);
    assert_eq!(int(&script, "return Services.learned"), 0);
    script.run("answers = { queued }").unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 2);
    assert_eq!(int(&script, "return Services.learned"), 1);
    assert_eq!(int(&script, "return #Services.places"), 10);
    script
        .run(
            r#"
            assert(Goals.on == false)
            assert(Services.why == nil)
            assert(#Services.kinds == #kinds)
            for _, kind in ipairs(Services.kinds) do
              assert(Goals.places[kind] and #Goals.places[kind] >= 1, kind)
            end
            for i, kind in ipairs(kinds) do
              local found = false
              for _, p in ipairs(Goals.places[kind]) do
                if p.name == queued[i].name then
                  assert(p.x == i and p.y == 2 and p.z == 3 and p.map == 0 and p.zone == zone)
                  found = true
                end
              end
              assert(found, kind)
            end
            -- Only this exact hyphenated kind is accepted; shared name validation stays strict.
            for _, kind in ipairs({ "questgiver-hubs", "questgiver--hub", "Questgiver-hub" }) do
              assert(not pcall(Goals.learn_places, {{ kind = kind, name = "Bad", x = 1, y = 2 }}))
            end
            assert(not pcall(Goals.fact, "questgiver-hub", function() return true end))
            "#,
        )
        .unwrap();
    script.tick(1.1);
    assert_eq!(int(&script, "return asks"), 2);
    assert!(!script.eval::<bool>("return Goals.on").unwrap());
    let errors = script.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}
