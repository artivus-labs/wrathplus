//! The session checklist's headless UI halves (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-SESSION-CHECKLIST`, class H): every
//! major 3.3.5 window opened through its stock toggle on the live-session load, with no Lua raise.

use wrathplus_ui::script::UiScript;

/// One window: what the player's toggle or the server's event runs, and the frame that must show.
struct Window {
    name: &'static str,
    open: &'static str,
    frame: &'static str,
}

/// The toggles as the stock micro buttons and bindings call them (`MainMenuBarMicroButtons.xml`,
/// `Bindings.xml`); the NPC windows open on the event the server's answer fires, as `UIParent_OnEvent`
/// handles it.
const WINDOWS: &[Window] = &[
    // The character window and its tabs (`CharacterFrame.lua:28`, Experience's), each through
    // `ToggleCharacter` as the micro button and the tab buttons call it.
    Window {
        name: "character",
        open: "ToggleCharacter('PaperDollFrame')",
        frame: "PaperDollFrame",
    },
    Window {
        name: "reputation",
        open: "ToggleCharacter('ReputationFrame')",
        frame: "ReputationFrame",
    },
    Window {
        name: "skills",
        open: "ToggleCharacter('SkillFrame')",
        frame: "SkillFrame",
    },
    Window {
        name: "currency",
        open: "ToggleCharacter('TokenFrame')",
        frame: "TokenFrame",
    },
    Window {
        name: "spellbook",
        open: "ToggleSpellBook(BOOKTYPE_SPELL)",
        frame: "SpellBookFrame",
    },
    Window {
        name: "talents",
        open: "ToggleTalentFrame()",
        frame: "PlayerTalentFrame",
    },
    Window {
        name: "quest log",
        open: "ToggleFrame(QuestLogFrame)",
        frame: "QuestLogFrame",
    },
    Window {
        name: "bags",
        open: "OpenAllBags()",
        frame: "ContainerFrame1",
    },
    Window {
        name: "world map",
        open: "ToggleFrame(WorldMapFrame)",
        frame: "WorldMapFrame",
    },
    Window {
        name: "friends",
        open: "ToggleFriendsFrame(1)",
        frame: "FriendsFrame",
    },
    Window {
        name: "who",
        open: "ToggleFriendsFrame(2)",
        frame: "FriendsFrame",
    },
    Window {
        name: "guild",
        open: "ToggleFriendsFrame(3)",
        frame: "FriendsFrame",
    },
    Window {
        name: "dungeon finder",
        open: "ToggleLFDParentFrame()",
        frame: "LFDParentFrame",
    },
    Window {
        name: "pvp",
        open: "TogglePVPFrame()",
        frame: "PVPParentFrame",
    },
    Window {
        name: "achievements",
        open: "ToggleAchievementFrame()",
        frame: "AchievementFrame",
    },
    Window {
        name: "calendar",
        open: "Calendar_LoadUI() Calendar_Toggle()",
        frame: "CalendarFrame",
    },
    Window {
        name: "macros",
        open: "ShowMacroFrame()",
        frame: "MacroFrame",
    },
    Window {
        name: "key bindings",
        open: "KeyBindingFrame_LoadUI() ShowUIPanel(KeyBindingFrame)",
        frame: "KeyBindingFrame",
    },
    Window {
        name: "mail",
        open: "MAIL_SHOW",
        frame: "MailFrame",
    },
    Window {
        name: "merchant",
        open: "MERCHANT_SHOW",
        frame: "MerchantFrame",
    },
    Window {
        name: "trade skill",
        open: "TRADE_SKILL_SHOW",
        frame: "TradeSkillFrame",
    },
    Window {
        name: "auction",
        open: "AUCTION_HOUSE_SHOW",
        frame: "AuctionFrame",
    },
    Window {
        name: "bank",
        open: "BANKFRAME_OPENED",
        frame: "BankFrame",
    },
];

/// Open one window, returning what went wrong: a raise, a recorded script error, or no frame shown.
fn open(script: &mut UiScript, w: &Window) -> Vec<String> {
    let _ = script.take_errors();
    let mut problems = Vec::new();
    let is_event = w.open.chars().all(|c| c.is_ascii_uppercase() || c == '_');
    if is_event {
        script.fire_event(w.open, vec![]);
    } else if let Err(e) = script.run(w.open) {
        problems.push(format!("raise: {e}"));
    }
    script.tick(0.05);
    problems.extend(
        script
            .take_errors()
            .into_iter()
            .map(|e| format!("error: {e}")),
    );
    let shown = script
        .eval::<bool>(&format!("return {f} ~= nil and {f}:IsShown()", f = w.frame))
        .unwrap_or(false);
    if !shown {
        problems.push(format!("{} not shown", w.frame));
    }
    // Close it again through the panel manager, then the game menu that closing the key bindings
    // returns to (`KeyBindingFrame` hides into `GameMenuFrame`, a center panel that refuses others).
    let _ = script.run(&format!(
        "if {f} then HideUIPanel({f}) end HideUIPanel(CharacterFrame) HideUIPanel(GameMenuFrame)",
        f = w.frame
    ));
    script.tick(0.05);
    let _ = script.take_errors();
    let slots = script
        .eval::<String>(
            "local t = {} for _, k in ipairs({'left','center','right','doublewide','fullscreen'}) \
             do local f = GetUIPanel(k) if f then t[#t+1] = k .. '=' .. (f:GetName() or '?') end end return table.concat(t, ' ')",
        )
        .unwrap_or_else(|e| e.to_string());
    if !slots.is_empty() {
        problems.push(format!("panels still held after close: {slots}"));
    }
    problems
}

#[test]
#[ignore = "reads the installed Wrath client; run with WOW_CLIENT_PROFILE=wrath WOW_DATA=…"]
fn installed_wrath_opens_every_major_window() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let (mut script, _failures) = super::stock_session(&data);
    script.clear_instruction_budget();
    // The windows a level-1 character cannot open (talents 10, PvP 10, dungeon finder 15) and the
    // backpack `OpenAllBags` needs: a level-80 player with a 16-slot backpack.
    script.set_unit("player", Some(super::stock_player(80)));
    // `ToggleAchievementFrame` needs the static catalogs (`lifecycle::new_vm`'s seed), the server's
    // achievement answer and one earned achievement (`CanShowAchievementUI`,
    // `HasCompletedAnyAchievement`), as SMSG_ALL_ACHIEVEMENT_DATA fills: Level 10 (6).
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    if let Some(metadata) = crate::ui_achievements::metadata_from(&mut chain) {
        script.set_wrath_achievement_metadata(metadata);
    }
    script.set_wrath_achievement_progress(wrathplus_ui::script::WrathAchievementProgress {
        earned: vec![(6, 0)],
        criteria: Default::default(),
        faction: 1,
    });
    // The guild tab opens only in a guild (`ToggleFriendsFrame`, `FriendsFrame.lua:1435`).
    script.set_guild(wrathplus_ui::script::GuildState {
        in_guild: true,
        name: "Benchers".into(),
        ..Default::default()
    });
    script.set_container(
        0,
        Some(wrathplus_ui::script::ContainerState {
            num_slots: 16,
            ..Default::default()
        }),
    );
    let mut failed = Vec::new();
    for w in WINDOWS {
        let problems = open(&mut script, w);
        eprintln!(
            "WINDOW {:<16} {}",
            w.name,
            if problems.is_empty() {
                "ok".into()
            } else {
                problems.join(" | ")
            }
        );
        if !problems.is_empty() {
            failed.push(w.name);
        }
    }
    for (name, problems) in seed_bars(&mut script) {
        eprintln!(
            "BAR    {name:<16} {}",
            if problems.is_empty() {
                "ok".into()
            } else {
                problems.join(" | ")
            }
        );
        if !problems.is_empty() {
            failed.push(name);
        }
    }
    assert!(
        failed.is_empty(),
        "windows that raised or did not show: {failed:?}"
    );
}

/// The action bars a session swaps in, each seeded as its packets would and announced by the event
/// the stock bar listens for: the pet bar (`UNIT_PET`, `PetActionBarFrame.lua:38`), the possess bar
/// (`UPDATE_BONUS_ACTIONBAR`, `BonusActionBarFrame.lua:208`) and the vehicle art
/// (`UNIT_ENTERED_VEHICLE`, `MainMenuBar.lua:251`), each checked before the next replaces it (a
/// vehicle hides the possess bar).
fn seed_bars(script: &mut UiScript) -> Vec<(&'static str, Vec<String>)> {
    use wrathplus_ui::script::{
        PossessView, ScriptValue, UnitState, UnitVehicleView, WrathVehicles,
    };
    let player = ScriptValue::Str("player".into());
    let mut out = Vec::new();
    let check = |script: &mut UiScript, frame: &str| -> Vec<String> {
        let mut problems: Vec<String> = script
            .take_errors()
            .into_iter()
            .map(|e| format!("error: {e}"))
            .collect();
        let shown = script
            .eval::<bool>(&format!("return {frame} ~= nil and {frame}:IsShown()"))
            .unwrap_or(false);
        if !shown {
            problems.push(format!("{frame} not shown"));
        }
        problems
    };
    let _ = script.take_errors();

    script.set_unit(
        "pet",
        Some(UnitState {
            exists: true,
            has_object: true,
            guid: 0xF140_0000_0000_0002,
            name: Some("Wolf".into()),
            ..Default::default()
        }),
    );
    script.set_pet_actions(true, true, true, Vec::new());
    script.fire_event("UNIT_PET", vec![player.clone()]);
    for _ in 0..20 {
        script.tick(0.1);
    }
    out.push(("pet bar", check(script, "PetActionBarFrame")));

    script.set_wrath_vehicles(WrathVehicles {
        possess_spell: Some(605),
        possess: Some(PossessView {
            name: "Mind Control".into(),
            cancelable: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    script.fire_event("UPDATE_BONUS_ACTIONBAR", vec![]);
    script.tick(0.1);
    out.push(("possess bar", check(script, "PossessBarFrame")));

    // Seat flags: can control (`0x800`) and has the vehicle UI (`0x2000_0000`).
    script.set_wrath_vehicles(WrathVehicles {
        units: std::collections::HashMap::from([(
            0x0100_0000_0000_0001,
            UnitVehicleView {
                seated: true,
                using: true,
                controlling: true,
                seat: Some((0x2000_0800, 0)),
                ..Default::default()
            },
        )]),
        player_seat_flags: Some(0x2000_0800),
        ..Default::default()
    });
    script.fire_event(
        "UNIT_ENTERED_VEHICLE",
        vec![player, ScriptValue::Bool(true), ScriptValue::Bool(true)],
    );
    for _ in 0..20 {
        script.tick(0.1);
    }
    out.push(("vehicle bar", check(script, "VehicleMenuBar")));
    out
}

/// The whole stock session's cost, for profiling the interface load: `AZB_LOAD_REPS` fresh
/// sessions (default 1, so the census run stays cheap), each timed, under the census's seeding; a sampler attached to the run
/// finds the hot spots.
#[test]
#[ignore = "reads the installed Wrath client; run with WOW_CLIENT_PROFILE=wrath WOW_DATA=…"]
fn installed_wrath_framexml_load_timing() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let reps = std::env::var("AZB_LOAD_REPS")
        .ok()
        .and_then(|reps| reps.parse().ok())
        .unwrap_or(1);
    let mut times = Vec::new();
    for _ in 0..reps {
        let started = std::time::Instant::now();
        let (script, failures) = super::stock_session(&data);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
        eprintln!(
            "LOAD-TIMING {:.0} ms, {} failures",
            times[times.len() - 1],
            failures.len()
        );
        drop(script);
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "LOAD-TIMING median {:.0} ms of {reps}",
        times[times.len() / 2]
    );
}

#[path = "wrath_worldmap_poi_tests.rs"]
mod worldmap_poi;

#[path = "wrath_chat_scroll_tests.rs"]
mod chat_scroll;

#[path = "wrath_quest_track_tests.rs"]
mod quest_track;

#[path = "wrath_quest_greeting_tests.rs"]
mod quest_greeting;

#[path = "wrath_exp_bar_login_tests.rs"]
mod exp_bar_login;

#[path = "wrath_request_loop_tests.rs"]
mod request_loop;
