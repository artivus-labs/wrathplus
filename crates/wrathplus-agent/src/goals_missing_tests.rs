//! An NPC a visit finds absent (the realm's event spawns, as at Anvilmar) stays out of
//! `Goals.places` for `Goals.missing_for` seconds, listed in `Goals.missing`, however often the
//! realm's list names it; seeing it again, or the planner forcing it, clears the mark.

use super::{goals, text, ticks};
use wrathplus_ui::script::UiScript;

/// Bags full; the realm's list: a holiday vendor whose event is off, nearest, and a standing one.
const SETUP: &str = r#"
free = 0
GetContainerNumFreeSlots = function() return free end
Upkeep.sell = false
Upkeep.sellable = function() return true end
realm = {
  { kind = 'vendor', name = 'Brewfest Vendor', x = 100, y = 0, z = 0, map = 0, entry = 23710,
    event = true, event_active = false },
  { kind = 'vendor', name = 'Thurman Mullby', x = 300, y = 0, z = 0, map = 0, entry = 1285,
    event = false },
}
"#;

fn vendors(script: &UiScript) -> String {
    text(
        script,
        "local out = {} for _, p in ipairs(Goals.places.vendor or {}) do out[#out + 1] = p.name end \
         return table.concat(out, ', ')",
    )
}

fn learn(script: &UiScript, call: &str) -> (i64, i64) {
    script.eval(call).unwrap()
}

/// The runner walks to the event vendor, finds nobody, and it is marked missing.
fn walk_to_the_absent_vendor() -> UiScript {
    let mut script = goals();
    script.run(SETUP).unwrap();
    assert_eq!(learn(&script, "return Goals.learn_places(realm)"), (2, 0));
    // The realm's event fields stay on the place, for the planner to read.
    assert_eq!(
        text(
            &script,
            "local p = Goals.places.vendor[2] \
             return tostring(p.event) .. ' ' .. tostring(p.event_active) .. ' ' .. p.entry"
        ),
        "true false 23710"
    );
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 1.5);
    assert_eq!(text(&script, "return doing()"), "goal:sell_and_repair");
    assert_eq!(
        text(&script, "return Goals.current.args.going"),
        "Brewfest Vendor"
    );
    // There, and nobody in sight.
    script.run("walking = false").unwrap();
    ticks(&mut script, 1.0);
    let failed = text(&script, "return heard('ACTION_FAILED')");
    assert!(
        failed.contains("Brewfest Vendor is not where they stood"),
        "{failed}"
    );
    script
}

#[test]
fn a_vendor_found_absent_is_not_learned_back_from_the_realm_until_its_mark_runs_out() {
    let script = walk_to_the_absent_vendor();
    assert_eq!(
        text(
            &script,
            "local m = Goals.missing[1] return #Goals.missing .. ' ' .. m.name .. ' ' .. m.x .. ' ' .. m.map"
        ),
        "1 Brewfest Vendor 100 0"
    );
    assert_eq!(vendors(&script), "Thurman Mullby");
    // The realm's next list names it again: left out, and said so.
    assert_eq!(learn(&script, "return Goals.learn_places(realm)"), (1, 1));
    assert_eq!(vendors(&script), "Thurman Mullby");
    assert_eq!(text(&script, "return tostring(Goals.missing_for)"), "1800");
    // Once the mark is older than missing_for, it is learned again and the mark is gone.
    script.run("Goals.missing_for = 0").unwrap();
    assert_eq!(learn(&script, "return Goals.learn_places(realm)"), (2, 0));
    assert_eq!(vendors(&script), "Thurman Mullby, Brewfest Vendor");
    assert_eq!(text(&script, "return tostring(#Goals.missing)"), "0");
}

#[test]
fn seeing_the_npc_again_or_the_planner_forcing_it_clears_the_mark() {
    let mut script = walk_to_the_absent_vendor();
    // The planner wants it anyway: forced, it is taken and the mark cleared.
    assert_eq!(
        learn(
            &script,
            "return Goals.learn_places({ realm[1] }, { force = true })"
        ),
        (1, 0)
    );
    assert_eq!(text(&script, "return tostring(#Goals.missing)"), "0");
    // Marked again by hand, then seen standing there (its event began): the mark goes.
    script
        .run("Goals.missing[1] = { name = 'Brewfest Vendor', map = 0, x = 100, y = 0, z = 0, at = GetTime() }")
        .unwrap();
    assert_eq!(learn(&script, "return Goals.learn_places(realm)"), (1, 1));
    script
        .run("Goals.on = false pos = { x = 95, y = 0, z = 0, map = 0 } \
              units = { { guid = '0xB', name = 'Brewfest Vendor', kind = 'unit', reaction = 'friendly', \
              npc = { 'vendor' }, x = 100, y = 0, z = 0, distance = 5 } } Goals.list()")
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return tostring(#Goals.missing)"), "0");
    assert_eq!(learn(&script, "return Goals.learn_places(realm)"), (2, 0));
}
