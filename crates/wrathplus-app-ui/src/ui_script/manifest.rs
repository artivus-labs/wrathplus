//! Th in-game interface: the core, the player's own stock FrameXML as their chain's
//! `FrameXML.toc` lists it ([`reference_ui::core`]) and then its `Bindings.xml`
//! ([`load_core_bindings`]), then WrathPlus's layer, [`LAYER_MANIFEST`], every entry of which is
//! ours and loads after the whole core.

use bevy::prelude::*;

use wrathplus_ui::script::UiScript;

#[cfg(test)]
use super::addons::Addon;
use super::reference_ui;

/// Every entry in load order: the core's rows, none without an install, then the Wrath root's.
#[cfg(test)]
pub(super) fn manifest_files() -> Vec<String> {
    let mut files = reference_ui::core()
        .map(|core| core.toc.files)
        .unwrap_or_default();
    files.extend(shipped_manifest_files());
    files
}

/// The entries that name a file we ship rather than one off the player's install: the Wrath
/// root's.
#[cfg(test)]
pub(super) fn shipped_manifest_files() -> Vec<String> {
    Addon::wrath_bootstrap().toc.files
}

/// What a screen-size change re-runs, from `extract::tick_script`'s resize arm: the Wrath root's
/// `UIParent` follows the screen, as its OnLoad sized it.
pub(super) fn on_screen_resized(script: &UiScript) {
    if let Err(error) = script.run("if UIParent then UIParent:SetWidth(GetScreenWidth()); UIParent:SetHeight(GetScreenHeight()) end") {
        error!("ui_script: Wrath root resize: {error}");
    }
}

/// Runs a UI load in the counted sound-suppression scope, as `CGGameUI::Initialize` (`0x48fbf0`,
/// called at login `0x48f681` and `/reloadui` `0x495669`) runs its own: enter (`0x458f50`) at
/// `0x48fbfa`, leave (`0x458f60`) at `0x49016d`, and `PlaySoundByName` (`0x458030`) drops every
/// call in between. `PLAYER_LOGIN` fires inside it only on `/reloadui` (`0x490168`); a fresh login
/// fires it from the player's create (`0x5deb60` to `0x4908c0`), a cascade with its own scope
/// (`0x4908d5` to `0x490a56`). The stock `TargetFrame_OnHide` at load is one sound it swallows.
pub(super) fn silenced_ui_load<S: std::borrow::Borrow<UiScript>, R>(
    script: &mut S,
    body: impl FnOnce(&mut S) -> R,
) -> R {
    script.borrow().push_sound_suppression();
    let out = body(script);
    script.borrow().pop_sound_suppression();
    out
}

/// The in-game load without the third-party addons, for the tests and [`crate::addon_harness`]:
/// [`load_wrath_ui`]'s core and AzerothBench root on a VM seeded as a live session seeds it
/// (`lifecycle::load_ingame_ui_on_world_entry`). The CVar table first, since stock OnLoads read
/// CVars; the installed `ChrClasses` names (`Constants.lua` fills its class tables at file
/// scope); a live Wrath account on enUS; the screen and its one resolution
/// (`UpdateMenuBarTop` divides by it, `UIParent.lua:1171`); and, unless the caller seated one, a
/// player with a race and class (`DressUpFrame.lua:16` uppercases the race file). Returns every
/// failure; a loader error is tagged `"<Addon>/<file>: <error>"`.
pub fn load_default_ui(script: &mut UiScript) -> Vec<String> {
    load_session_ui(script, false)
}

/// [`load_default_ui`], with the AddOns root's third-party addons after the root when `addons`, as
/// [`load_wrath_ui`] loads them (no character identity, version check on).
pub(super) fn load_session_ui(script: &mut UiScript, addons: bool) -> Vec<String> {
    use wrathplus_ui::script::{PlayerRecord, ScreenResolution, UnitState};
    script.register_cvars(crate::shipped_cvars::pairs());
    if let Some(names) = wrathplus_formats::wow_data().and_then(|data| {
        let mut chain = wrathplus_formats::open_chain(&data).ok()?;
        wrathplus_formats::load_chr_classes(&mut chain).ok()
    }) {
        script.set_localized_class_names(names.localized_names());
    }
    script.set_wrath_session(Some(2), vec!["enUS".into()]);
    // The caller's screen stands (a VM starts at 1024x768), and is its one display mode.
    let (width, height) = script
        .eval::<(f64, f64)>("return GetScreenWidth(), GetScreenHeight()")
        .unwrap_or((1024.0, 768.0));
    script.set_screen_resolutions(
        vec![],
        Some(ScreenResolution {
            width: width.round() as u32,
            height: height.round() as u32,
        }),
    );
    let seated = script
        .lua()
        .load("return UnitExists('player')")
        .eval::<Option<i64>>()
        .ok()
        .flatten()
        .is_some();
    if !seated {
        script.set_player_record(PlayerRecord {
            name: "Probefour".into(),
            race: Some(("Orc".into(), "Orc".into())),
            class: Some(("Hunter".into(), "HUNTER".into())),
            sex: 2,
        });
        script.set_unit(
            "player",
            Some(UnitState {
                exists: true,
                name: Some("Probefour".into()),
                level: 60,
                race: Some("Orc".into()),
                race_file: Some("Orc".into()),
                class: Some("Hunter".into()),
                class_file: Some("HUNTER".into()),
                // `BattlefieldFrame_Update` concatenates it (`BattlefieldFrame.lua:387`).
                faction_group: Some("Horde".into()),
                faction_group_localized: Some("Horde".into()),
                ..Default::default()
            }),
        );
    } else {
        // A caller's own player still has the Enter World record a session always holds:
        // `UnitRace("player")` and `UnitClass("player")` read only that record (`0x518269`),
        // never the unit, so a seat without one leaves `DressUpTexturePath` a nil race file.
        let (name, race_file) = script
            .lua()
            .load("local _, f = UnitRace('player') return UnitName('player'), f")
            .eval::<(Option<String>, Option<String>)>()
            .unwrap_or_default();
        if race_file.is_none() {
            script.set_player_record(PlayerRecord {
                name: name.unwrap_or_default(),
                race: Some(("Orc".into(), "Orc".into())),
                class: Some(("Hunter".into(), "HUNTER".into())),
                sex: 2,
            });
        }
    }
    script.set_instruction_budget(super::addons::LOAD_INSTRUCTION_BUDGET);
    // No sound during the load, as on the live edge (`silenced_ui_load`).
    script.push_sound_suppression();
    let mut failures = load_core(script);
    // As [`load_wrath_ui`]: the archive's startup Blizzard addon with the core.
    failures.extend(super::addons::load_wrath_startup_blizzard(script));
    failures.extend(super::wrath_addons::load_bootstrap(script));
    if addons {
        failures.extend(super::addons::load_wrath_third_party(
            script,
            None,
            &[],
            true,
        ));
    }
    script.pop_sound_suppression();
    script.clear_instruction_budget();
    // A raise one call below the loader's own dispatch (an OnLoad that shows a frame whose OnShow
    // raises) lands in the VM's error list, not the loader's report; it is a load failure too.
    failures.extend(
        script
            .errors()
            .into_iter()
            .map(|e| format!("a handler raised during the load walk: {e}")),
    );
    failures
}

/// The core: every row of the chain's `FrameXML.toc`, in its order, each file in document order,
/// then the chain's `Bindings.xml`. A chain without the toc builds no stock interface: the runner
/// reports `"Couldn't open %s"` (`0x846ff4`) at severity 2 (`0x6edc3f`) and returns 0, which
/// `UI_Init` never reads (`0x48fff2`), going on to `Bindings.xml` and the addons. WrathPlus goes on
/// the same way and reports it as a load failure, an `ERROR` line and a row in `/errors`.
pub(super) fn load_core(script: &UiScript) -> Vec<String> {
    let Some(core) = reference_ui::core() else {
        let e = format!(
            "{}: not found — the stock interface is not built",
            reference_ui::TOC
        );
        error!("ui_script: {e}");
        script.report_load_failure(&e);
        // Into the UI load's own record, before any banner (`0x6edc3f`).
        let mut log = wrathplus_ui::status::Status::default();
        log.report(
            wrathplus_ui::status::FAILURE,
            wrathplus_ui::status::missing(reference_ui::TOC, false),
        );
        script.report_load_status(log);
        let mut failures = vec![e];
        failures.extend(load_core_bindings(script));
        return failures;
    };
    let mut toc = wrathplus_ui::status::Status::default();
    let mut failures = core.load_files_into(script, &core.toc.files, &mut toc);
    let mut log = wrathplus_ui::status::Status::default();
    let banner = wrathplus_ui::status::toc_banner(reference_ui::TOC);
    toc.close_into(&mut log, script.framexml_debug(), banner);
    script.report_load_status(log);
    failures.extend(load_core_bindings(script));
    failures
}

/// The stock key-binding commands, `UI_Init`'s next step after the toc walk: the chain's
/// `Interface\FrameXML\Bindings.xml` (`0x842f9c`), when the chain has it (`0x48fff9`), through
/// the loader addons' files go through (`0x4b6f70`, called at `0x490018` here and `0x51f443` for
/// an addon). Every command WrathPlus has is a row of this file, its body the file's own Lua.
pub(super) fn load_core_bindings(script: &UiScript) -> Vec<String> {
    let Some(bytes) = reference_ui::read(reference_ui::BINDINGS) else {
        warn!(
            "ui_script: {} is not in the patch chain — no key binding has a command",
            reference_ui::BINDINGS
        );
        return Vec::new();
    };
    let text = wrathplus_ui::source::decode(&bytes);
    // Build 12340's `<ModifiedClick>` rows, the `IsModifiedClick` actions.
    if let Ok(clicks) = wrathplus_ui::bindings_xml::parse_modified_clicks(&text) {
        script.register_modified_clicks(&clicks);
    }
    match wrathplus_ui::bindings_xml::parse(&text) {
        Ok(bindings) => {
            script.register_bindings(&bindings);
            info!(
                "bindings: {} commands registered from {}",
                bindings.len(),
                reference_ui::BINDINGS
            );
            Vec::new()
        }
        // `"Couldn't parse XML in %s"` (`0x846fd8`) at severity 2 (`0x4b700c`).
        Err(e) => {
            let e = format!("{}: {e}", reference_ui::BINDINGS);
            error!("ui_script: {e}");
            script.report_load_failure(&e);
            vec![e]
        }
    }
}

/// The defaults, set 0, off the player's chain: `WTF\DefaultBindings.wtf` (`0x846e44`), which
/// `0x4b62b0` reads line by line (`0x4b6140`); empty when the chain lacks it.
pub(crate) fn default_bindings() -> Vec<wrathplus_ui::script::keybind::KeyBinding> {
    match reference_ui::read(reference_ui::DEFAULT_BINDINGS) {
        Some(bytes) => {
            wrathplus_ui::script::keybind::parse_bindings_wtf(&String::from_utf8_lossy(&bytes))
        }
        None => {
            warn!(
                "ui_script: {} is not in the patch chain — every command ships unbound",
                reference_ui::DEFAULT_BINDINGS
            );
            Vec::new()
        }
    }
}

/// The chain's `Bindings.xml` text, for the tests that hold WrathPlus against it.
#[cfg(any(test, feature = "test-support"))]
pub fn stock_bindings_file() -> Option<String> {
    reference_ui::read(reference_ui::BINDINGS)
        .map(|bytes| wrathplus_ui::source::decode(&bytes).into_owned())
}

/// The stock bindings on a VM a test built by hand: the defaults loaded live, as a first login's
/// account set is, and the core's commands.
#[cfg(any(test, feature = "test-support"))]
pub fn load_stock_bindings(script: &mut UiScript) {
    script.set_default_bindings(default_bindings());
    script.load_binding_set(1);
    assert!(
        load_core_bindings(script).is_empty(),
        "the stock Bindings.xml loads"
    );
}

/// The Wrath host: the installed stock FrameXML, the authored AzerothBench root, then local-state
/// addons. Every chunk runs trusted; see `wrathplus_ui`'s Wrath surface.
pub(crate) fn load_wrath_ui(
    script: &mut UiScript,
    identity: Option<&(String, String)>,
    roster: &[String],
    version_check: bool,
) -> Vec<String> {
    script.set_instruction_budget(super::addons::LOAD_INSTRUCTION_BUDGET);
    let mut failures = load_core(script);
    // The archive's startup Blizzard addon with the core, as a clean login has it: stock
    // `CharacterFrame_ShowSubFrame` walks its `TokenFrame` unguarded.
    failures.extend(super::addons::load_wrath_startup_blizzard(script));
    failures.extend(super::wrath_addons::load_bootstrap(script));
    failures.extend(super::addons::load_wrath_third_party(
        script,
        identity,
        roster,
        version_check,
    ));
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_script::reference_ui::fixture;

    /// A chain whose `FrameXML.toc` lists `rows`, each a `.lua` that appends its own letter to
    /// `LOAD_LOG`, and one `.xml` whose `<Script file=>` does the same.
    fn lay_letters(toc: &str) -> fixture::Guard {
        const B: &[u8] = b"LOAD_LOG = (LOAD_LOG or '') .. 'b'";
        const A: &[u8] = b"LOAD_LOG = (LOAD_LOG or '') .. 'a'";
        const X: &[u8] = br#"<Ui><Script file="x.lua"/></Ui>"#;
        const XL: &[u8] = b"LOAD_LOG = (LOAD_LOG or '') .. 'x'";
        fixture::lay(&[
            (reference_ui::TOC, Some(toc.as_bytes())),
            (r"Interface\FrameXML\b.lua", Some(B)),
            (r"Interface\FrameXML\a.lua", Some(A)),
            (r"Interface\FrameXML\x.xml", Some(X)),
            (r"Interface\FrameXML\x.lua", Some(XL)),
        ])
    }

    /// The core's load list is the chain's `FrameXML.toc`: every file line, in its order, under the
    /// toc's own directory; directives, comments and blank lines load nothing.
    #[test]
    fn the_cores_load_list_is_the_chain_tocs_rows_in_order() {
        let _chain =
            lay_letters("## Interface: 11200\r\n# a comment\r\nb.lua\r\n\r\nx.xml\r\na.lua\r\n");
        let core = reference_ui::core().expect("the laid toc");
        assert_eq!(
            core.toc.files,
            [
                r"Interface\FrameXML\b.lua",
                r"Interface\FrameXML\x.xml",
                r"Interface\FrameXML\a.lua"
            ]
        );
    }

    /// The load follows the toc: the same files under a reordered toc load in the new order, each
    /// `.xml` running its own `<Script file=>` in place.
    #[test]
    fn a_reordered_toc_reorders_the_load() {
        for (toc, want) in [
            ("b.lua\nx.xml\na.lua\n", "bxa"),
            ("a.lua\nb.lua\nx.xml\n", "abx"),
        ] {
            let _chain = lay_letters(toc);
            let s = UiScript::new().unwrap();
            let failures = load_core(&s);
            assert!(failures.is_empty(), "{toc:?}: {failures:#?}");
            assert_eq!(
                s.eval::<String>("return LOAD_LOG").unwrap(),
                want,
                "{toc:?}"
            );
        }
    }

    /// A chain without `FrameXML.toc` builds no stock interface and says so, once, as a load
    /// failure: the reference's runner reports `"Couldn't open %s"` at severity 2 (`0x6edc3f`).
    #[test]
    fn a_chain_without_the_toc_builds_no_core_and_says_so() {
        let _chain = fixture::lay(&[(reference_ui::TOC, None)]);
        assert!(reference_ui::core().is_none());
        let s = UiScript::new().unwrap();
        let failures = load_core(&s);
        assert_eq!(failures.len(), 1, "{failures:#?}");
        assert!(
            failures[0].contains(reference_ui::TOC) && failures[0].contains("not found"),
            "{failures:?}"
        );
        assert!(
            s.diagnostics().iter().any(|d| {
                d.kind == wrathplus_ui::script::diagnostics::DiagnosticKind::Load
                    && d.message.contains(reference_ui::TOC)
            }),
            "the player's /errors carries it"
        );
    }

    /// The core never reads our tree: a toc row that names one of the Wrath root's files loads the
    /// chain's file of that name or reports it missing, never ours.
    #[test]
    fn the_core_never_loads_a_file_of_ours() {
        let ours = Addon::wrath_bootstrap().toc.files;
        let row = ours
            .iter()
            .find(|f| f.as_str() == "wrath/UIParent.xml")
            .expect("the Wrath root ships wrath/UIParent.xml");
        let toc = format!("{row}\n");
        let _chain = fixture::lay(&[
            (reference_ui::TOC, Some(toc.as_bytes())),
            (&format!(r"Interface\FrameXML\{row}"), None),
        ]);
        let s = UiScript::new().unwrap();
        let failures = load_core(&s);
        assert!(
            failures.len() == 1 && failures[0].contains("not found"),
            "{failures:#?}"
        );
    }

    /// Against the player's own install: the core's load list is their `FrameXML.toc`'s file
    /// lines, read here byte by byte, in order, and every one resolves off their chain.
    #[test]
    fn the_cores_load_list_is_the_players_own_toc() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        let bytes = reference_ui::read(reference_ui::TOC).expect("the player's FrameXML.toc");
        let text = String::from_utf8_lossy(&bytes);
        let want: Vec<String> = text
            .trim_start_matches('\u{feff}')
            .split("\r\n")
            .flat_map(|l| l.split('\n'))
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| format!(r"Interface\FrameXML\{l}"))
            .collect();
        let core = reference_ui::core().expect("the player's FrameXML.toc");
        assert_eq!(core.toc.files, want);
        assert!(want.len() >= 90, "only {} rows parsed", want.len());
        for row in &core.toc.files {
            assert!(
                reference_ui::read(row).is_some(),
                "FrameXML.toc lists {row}, which the chain does not hold"
            );
        }
    }

    /// Order shows: the frames registered for one event run in registration order (`SignalEvent`,
    /// `0x703e50`, walks its tail-appended list), so the toc's row order is their handlers' order.
    /// `ExhaustionTick` (`MainMenuBar.lua:13`) and `PlayerFrame` (`PlayerFrame.lua:17`) both take
    /// `PLAYER_UPDATE_RESTING`, and the toc lists `MainMenuBar.xml` above `PlayerFrame.xml`.
    #[test]
    fn two_stock_frames_on_one_event_run_in_the_tocs_order() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        let _l = crate::local_state::test_env::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let toc = reference_ui::read(reference_ui::TOC).expect("the player's FrameXML.toc");
        let toc = String::from_utf8_lossy(&toc);
        let at = |leaf: &str| {
            toc.lines()
                .position(|l| l.trim() == leaf)
                .unwrap_or_else(|| panic!("FrameXML.toc lists {leaf}"))
        };
        assert!(
            at("MainMenuBar.xml") < at("PlayerFrame.xml"),
            "the stock toc's order"
        );
        let (mut s, failures) =
            super::super::production_load::production_load_with("event-order", "", |_| {});
        assert!(failures.is_empty(), "load failures: {failures:#?}");
        s.run(
            r#"EVENT_ORDER = {}
            for _, name in ipairs({ "PlayerFrame", "ExhaustionTick" }) do
                local who = name
                getglobal(who):SetScript("OnEvent", function() table.insert(EVENT_ORDER, who) end)
            end"#,
        )
        .unwrap();
        s.fire_event("PLAYER_UPDATE_RESTING", vec![]);
        let order: Vec<String> = s.eval("return EVENT_ORDER").unwrap();
        assert_eq!(order, ["ExhaustionTick", "PlayerFrame"]);
    }

    /// A shipped `.xml` neither the Wrath root nor the glue VM's `glue/glue.toc` lists never loads.
    #[test]
    fn the_manifest_lists_every_shipped_file_and_nothing_else() {
        let mut listed = [Addon::glue_additions().toc.files, shipped_manifest_files()].concat();
        let mut shipped: Vec<String> = super::super::content::shipped_tree_files()
            .into_iter()
            .filter(|f| f.ends_with(".xml"))
            .map(str::to_owned)
            .collect();
        listed.sort();
        shipped.sort();
        assert_eq!(listed, shipped);
    }

    /// [`reference_ui::is_chain_entry`] reads a path separator as a chain entry, so a shipped file
    /// in a subdirectory would be looked for on the player's install instead.
    #[test]
    fn every_file_we_ship_is_a_bare_name_so_a_path_can_only_mean_the_chain() {
        for name in super::super::content::shipped_files() {
            assert!(
                !reference_ui::is_chain_entry(name),
                "assets/ui is flat, but ships {name}: a separator makes it a file off the install"
            );
        }
    }

    /// A patched chain's `Bindings.xml`, rows reordered, a body edited, and the loader's skips.
    const PATCHED_BINDINGS: &[u8] = br#"<Bindings>
    <Binding name="TOGGLEBACKPACK" header="INTERFACE">Probe("bag")</Binding>
    <Binding name="JUMP" header="MOVEMENT">Probe("edited jump")</Binding>
    <Binding name="TOGGLESTATS" hidden="true" debug="true">Probe("debug")</Binding>
    <Binding name="EMPTYROW"></Binding>
    <Bindings><Binding name="NESTEDROW">Probe("nested")</Binding></Bindings>
    <Binding name="ITUNES_PLAYPAUSE" header="ITUNES_REMOTE" platform="mac">Probe("mac")</Binding>
</Bindings>"#;

    /// The core's commands are the chain's `Bindings.xml`, loaded after the toc's files (which
    /// see none) as `UI_Init` does (`0x48ffed`, then `0x490018`): its order, its headers and its
    /// bodies are what registers and runs, the defaults are the chain's `DefaultBindings.wtf`, and
    /// a debug, an empty, a nested and a Mac row are no commands.
    #[test]
    fn the_cores_commands_are_the_chain_bindings_xml() {
        const TOC_LUA: &[u8] =
            b"TOC_SAW = GetNumBindings() function Probe(n) PROBED = (PROBED or '') .. n end";
        let _chain = fixture::lay(&[
            (reference_ui::TOC, Some(b"a.lua\r\n")),
            (r"Interface\FrameXML\a.lua", Some(TOC_LUA)),
            (reference_ui::BINDINGS, Some(PATCHED_BINDINGS)),
            (
                reference_ui::DEFAULT_BINDINGS,
                Some(b"bind J JUMP\r\nbind B TOGGLEBACKPACK\r\nbind CTRL-Y TOGGLESTATS\r\n"),
            ),
        ]);
        let mut s = UiScript::new().unwrap();
        s.set_default_bindings(default_bindings());
        s.load_binding_set(1);
        let failures = load_core(&s);
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(s.eval::<i64>("return TOC_SAW").unwrap(), 0);
        let rows: Vec<Vec<String>> = s
            .eval(
                "local out = {} \
                 for i = 1, GetNumBindings() do out[i] = { GetBinding(i) } end \
                 return out",
            )
            .unwrap();
        let row = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                row(&["HEADER_INTERFACE"]),
                row(&["TOGGLEBACKPACK", "B"]),
                row(&["HEADER_MOVEMENT"]),
                row(&["JUMP", "J"]),
            ]
        );
        assert!(s.execute_binding("JUMP", true).unwrap());
        assert_eq!(s.eval::<String>("return PROBED").unwrap(), "edited jump");
        for skipped in ["TOGGLESTATS", "EMPTYROW", "NESTEDROW", "ITUNES_PLAYPAUSE"] {
            assert!(
                !s.execute_binding(skipped, true).unwrap(),
                "{skipped} is no command"
            );
        }
        assert_eq!(
            s.eval::<String>(r#"return GetBindingAction("CTRL-Y")"#)
                .unwrap(),
            "TOGGLESTATS",
            "a default key on a command the file never declared is still the key table's"
        );
    }

    /// A chain without `Bindings.xml` loads no command and fails nothing (`0x48fff9` skips it).
    #[test]
    fn a_chain_without_bindings_xml_declares_no_command() {
        let _chain = fixture::lay(&[
            (reference_ui::TOC, Some(b"")),
            (reference_ui::BINDINGS, None),
        ]);
        let s = UiScript::new().unwrap();
        assert!(load_core(&s).is_empty());
        assert_eq!(s.eval::<i64>("return GetNumBindings()").unwrap(), 0);
    }

    /// The install's own file read as the loader reads it: 219 commands once the nine debug rows
    /// are skipped, the Mac rows among them for the Mac build alone; the six `MOVEVIEW*` rows sit
    /// in an XML comment.
    #[test]
    fn the_installs_bindings_xml_reads_as_the_loader_reads_it() {
        wrathplus_formats::wow_data_or_skip!();
        let binds = wrathplus_ui::bindings_xml::parse(&stock_bindings_file().unwrap()).unwrap();
        assert_eq!(binds.len(), 262);
        assert_eq!(binds.iter().filter(|b| b.run_on_up).count(), 101);
        assert_eq!(binds.iter().filter(|b| b.header.is_some()).count(), 19);
        assert_eq!(binds.iter().filter(|b| b.hidden).count(), 2);
        assert_eq!(binds.iter().filter(|b| b.platform.is_some()).count(), 9);
        let names: Vec<&str> = binds.iter().map(|b| b.name.as_str()).collect();
        for gone in ["TOGGLESTATS", "RESETPERFORMANCEVALUES", "MOVEVIEWIN"] {
            assert!(!names.contains(&gone), "{gone}");
        }
        for kept in [
            "MOVEANDSTEER",
            "PITCHUP",
            "TURNORACTION",
            "TOGGLEUI",
            "RAIDTARGETNONE",
        ] {
            assert!(names.contains(&kept), "{kept}");
        }
        assert_eq!(binds[0].header.as_deref(), Some("MOVEMENT"));

        // Registered on this client, the PC build: every row but the nine Mac ones, so the list
        // walks 251 visible commands and 17 headers, two headers riding on Mac rows.
        let mut s = UiScript::new().unwrap();
        load_stock_bindings(&mut s);
        assert_eq!(s.eval::<i64>("return GetNumBindings()").unwrap(), 268);
        assert!(!s.execute_binding("ITUNES_PLAYPAUSE", true).unwrap());
    }
}
