//! `wp.services` against a stubbed `AgentServices`: asked on entering the world and on a new zone,
//! the answer handed to `Goals.learn_places`, and a realm without the module asked no more.

use wrathplus_ui::script::UiScript;

const GAME: &str = r#"
asks, learned = 0, {}
zone = "Northshire Valley"
answers = {}
AgentPosition = function() return { x = 0, y = 0, z = 0 } end
GetRealZoneText = function() return zone end
AgentServices = function(kinds)
  asks = asks + 1
  local a = table.remove(answers, 1)
  if a == nil then return { pending = true } end
  if a == "absent" then return nil, "this realm does not say where services are (it has no mod-agent-nav)" end
  return a
end
Goals = Goals or {}
Goals.learn_places = function(places) learned[#learned + 1] = places end
"#;

fn services() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-services-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
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
