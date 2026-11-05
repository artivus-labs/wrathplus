//! The installed build-12340 FrameXML, loaded whole into a trusted Wrath VM.

use wrathplus_ui::script::{UiScript, UnitState};

#[path = "wrath_census_lexer.rs"]
mod census_lexer;

#[path = "wrath_census_arity.rs"]
mod census_arity;

#[path = "wrath_census_fixtures.rs"]
mod census_fixtures;

#[path = "wrath_windows_tests.rs"]
mod windows;

/// One chunk's `Name(` sites: `function Name(` definitions, `:Name(` methods, and bare global
/// calls, less the names the chunk binds with `local` ([`census_lexer::local_names`]).
fn scan_calls(
    src: &str,
    defined: &mut std::collections::HashSet<String>,
    methods: &mut std::collections::BTreeMap<String, usize>,
    calls: &mut std::collections::BTreeMap<String, usize>,
) {
    let locals = census_lexer::local_names(src);
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = &src[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let before = if start > 0 { bytes[start - 1] } else { b' ' };
            // `function Name(`, and `function <expr>:Name(` / `function <expr>.Name(`: a FrameXML
            // definition, the expression walked back over identifiers, dots and colons.
            let head = if before == b':' || before == b'.' {
                src[..start]
                    .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || "_.:".contains(c))
            } else {
                &src[..start]
            };
            let preceded_by_function = head.trim_end().ends_with("function");
            // `<expr>.Name = <value>`: a method FrameXML defines on an instance, by a function
            // (PaperDollFrame.lua:2353 `self.SetSelection = function`) or another value
            // (InterfaceOptionsPanels.xml:3009 `self.SetDisplayValue = self.SetValue`). A name that
            // is also a Wow.exe string stays counted through the shadow guard.
            if (before == b'.' || before == b':')
                && name.as_bytes()[0].is_ascii_uppercase()
                && src[i..]
                    .trim_start()
                    .strip_prefix('=')
                    .is_some_and(|r| !r.starts_with('='))
            {
                defined.insert(name.to_string());
            }
            if j < bytes.len() && bytes[j] == b'(' && name.as_bytes()[0].is_ascii_uppercase() {
                if preceded_by_function {
                    defined.insert(name.to_string());
                } else if before == b':' {
                    *methods.entry(name.to_string()).or_default() += 1;
                } else if before != b'.'
                    && before != b'"'
                    && before != b'\''
                    && !locals.contains(name)
                {
                    *calls.entry(name.to_string()).or_default() += 1;
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Methods FrameXML defines itself, by assignment or `function <expr>:Name(`, are definitions:
/// their calls are neither missing methods nor missing globals.
#[test]
fn census_treats_frame_xml_defined_methods_as_defined() {
    let mut defined = std::collections::HashSet::new();
    let mut methods = std::collections::BTreeMap::new();
    let mut calls = std::collections::BTreeMap::new();
    let src = census_lexer::executable_text(
        "self.SetSelection = function(self, t) end\nfunction Foo.Bar:Baz(a) end\n\
         function Foo.Qux(a) end\nself:SetSelection(1)\nself:GetName()\n\
         self.SetDisplayValue = self.SetValue\nif self.IsCompared == x then end",
    );
    scan_calls(&src, &mut defined, &mut methods, &mut calls);
    for n in ["SetSelection", "Baz", "Qux", "SetDisplayValue"] {
        assert!(defined.contains(n), "{n}");
    }
    assert!(
        !defined.contains("IsCompared"),
        "a comparison is no assignment"
    );
    assert_eq!(
        methods.get("GetName"),
        Some(&1),
        "an engine method call still counts"
    );
}

/// A `local` binding hides only its own chunk's calls: the same name called as a global in
/// another chunk still counts.
#[test]
fn census_skips_a_chunks_locals_but_counts_the_same_name_elsewhere() {
    let mut defined = std::collections::HashSet::new();
    let mut methods = std::collections::BTreeMap::new();
    let mut calls = std::collections::BTreeMap::new();
    let popup = census_lexer::executable_text(
        "local OnCancel = info.OnCancel\nif OnCancel then OnCancel(self) end\nSendMail(a)",
    );
    let other = census_lexer::executable_text("OnCancel(frame)");
    scan_calls(&popup, &mut defined, &mut methods, &mut calls);
    assert!(!calls.contains_key("OnCancel"), "the chunk's local");
    assert_eq!(calls.get("SendMail"), Some(&1));
    scan_calls(&other, &mut defined, &mut methods, &mut calls);
    assert_eq!(
        calls.get("OnCancel"),
        Some(&1),
        "a global call in another chunk"
    );
}

/// Every distinct failure, error and warning, with a count, printed for triage.
fn census(script: &mut UiScript, failures: &[String]) -> usize {
    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut note = |line: String| match seen.iter_mut().find(|(l, _)| *l == line) {
        Some((_, n)) => *n += 1,
        None => seen.push((line, 1)),
    };
    failures.iter().for_each(|f| note(format!("FAIL  {f}")));
    script
        .take_errors()
        .into_iter()
        .for_each(|e| note(format!("ERROR {e}")));
    script
        .warnings()
        .into_iter()
        .for_each(|w| note(format!("WARN  {w}")));
    for (line, n) in &seen {
        eprintln!("[{n:>3}] {line}");
    }
    seen.iter().filter(|(l, _)| !l.starts_with("WARN")).count()
}

/// The session's player unit, a Human Warrior at `level`, as the fixtures' guid reads it.
fn stock_player(level: u32) -> UnitState {
    UnitState {
        exists: true,
        // The fixtures' player guid, so `UnitGUID("player")` answers as in a session.
        guid: 0x0100_0000_0000_0001,
        name: Some("Benchone".into()),
        level,
        race: Some("Human".into()),
        race_file: Some("Human".into()),
        class: Some("Warrior".into()),
        class_file: Some("WARRIOR".into()),
        health: 60,
        max_health: 60,
        ..Default::default()
    }
}

/// The live session's UI as `lifecycle::new_vm` and world entry build it from the installed client:
/// class names, account, resolution, player record and unit, the addon reply, the CVar seed, the
/// whole FrameXML load, the login events and the first tick. Returns the VM and its load failures.
fn stock_session(data: &std::path::Path) -> (UiScript, Vec<String>) {
    let mut script = UiScript::new().unwrap();
    // A live VM receives its display modes before FrameXML; UIParent uses their aspect ratio.
    script.set_screen_size(1600.0, 1000.0);
    // As `lifecycle::new_vm` seeds it: stock `Constants.lua` fills the class names at file scope.
    let mut chain = wrathplus_formats::open_chain(data).expect("open chain");
    script.set_localized_class_names(
        wrathplus_formats::load_chr_classes(&mut chain)
            .expect("ChrClasses.dbc")
            .localized_names(),
    );
    // A live Wrath account (expansion 2) on the installed locale.
    script.set_wrath_session(Some(2), vec!["enUS".into()]);
    script.set_screen_resolutions(
        vec![],
        Some(wrathplus_ui::script::ScreenResolution {
            width: 1600,
            height: 1000,
        }),
    );
    script.set_player_record(wrathplus_ui::script::PlayerRecord {
        name: "Benchone".into(),
        race: Some(("Human".into(), "Human".into())),
        class: Some(("Warrior".into(), "WARRIOR".into())),
        sex: 2,
        faction_group: Some(("Alliance".into(), "Alliance".into())),
    });
    script.set_unit("player", Some(stock_player(1)));
    eprintln!(
        "default bindings: {} rows",
        super::manifest::default_bindings().len()
    );
    // As the game loads it: the server's addon reply (nothing hidden), the load budget armed and
    // the archive's Blizzard addons registered.
    script.note_addon_info_reply(&[]);
    // The client's CVar table, as `lifecycle::new_vm` seeds it from the registry: stock
    // `VideoOptionsPanels.xml` reads `gamma` at load, a `ui_gamma` row.
    script.seed_cvars(crate::cvars::Cvars::with_rows(crate::shipped_cvars::FEATURE_ROWS).vm_seed());
    let failures = super::manifest::load_wrath_ui(&mut script, None, &[], true);
    let labels = script.eval::<String>("local bad = {} for _, name in ipairs({'CHAT_CONFIG_CHAT_LEFT','CHAT_CONFIG_OTHER_COMBAT','CHAT_CONFIG_OTHER_PVP','CHAT_CONFIG_OTHER_SYSTEM','CHAT_CONFIG_CHAT_CREATURE_LEFT'}) do for _, v in ipairs(_G[name] or {}) do local label = v.text or _G[v.type] if type(label) ~= 'string' then table.insert(bad, tostring(v.type)..'='..type(label)) end end end return table.concat(bad, ', ')").unwrap();
    eprintln!("non-string chat config labels: {labels}");
    for event in ["VARIABLES_LOADED", "PLAYER_LOGIN", "PLAYER_ENTERING_WORLD"] {
        script.fire_event(event, vec![]);
    }
    script.set_screen_size(1600.0, 1000.0);
    let _ = script.run("LoadAddOn('Blizzard_TimeManager')");
    let started = std::time::Instant::now();
    script.tick(0.1);
    eprintln!(
        "first tick: {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    (script, failures)
}

#[test]
#[ignore = "reads the installed Wrath client; run with WOW_CLIENT_PROFILE=wrath WOW_DATA=…"]
fn installed_wrath_framexml_builds_the_stock_hud() {
    let data = wrathplus_formats::wow_data_or_skip!();
    // Every attribute name the loader reads, for the unhandled-attribute audit.
    wrathplus_ui::framexml::record_queried_attributes(true);
    let (mut script, failures) = stock_session(&data);
    for w in script.warnings().iter().filter(|w| w.contains("converge")) {
        eprintln!("CYCLE {w}");
    }
    let hard = census(&mut script, &failures);
    let toc =
        super::reference_ui::read(r"Interface\AddOns\Blizzard_CombatLog\Blizzard_CombatLog.toc");
    eprintln!(
        "Blizzard_CombatLog toc: {:?}",
        toc.map(|b| String::from_utf8_lossy(&b)
            .chars()
            .take(300)
            .collect::<String>())
    );
    let combat_log = script
        .eval::<String>("local ok, why = LoadAddOn('Blizzard_CombatLog') return tostring(ok) .. ' ' .. tostring(why) .. ' ' .. tostring(GetNumAddOns())")
        .unwrap_or_else(|e| e.to_string());
    eprintln!("Blizzard_CombatLog loads: {combat_log}");
    script.set_screen_size(1600.0, 1000.0);
    // Enumerate the installed registry, not a guessed upper bound that raises API errors.
    let addon_count = script.eval::<usize>("return GetNumAddOns()").unwrap();
    let mut not_loaded = Vec::new();
    for i in 1..=addon_count {
        let started = std::time::Instant::now();
        let row = script
            .eval::<String>(&format!(
                "local name = GetAddOnInfo({i}) local ok, why = LoadAddOn({i}) \
                 return tostring(name) .. ' ' .. tostring(ok) .. ' ' .. tostring(why)"
            ))
            .unwrap_or_else(|e| panic!("installed addon {i}/{addon_count} load raised: {e}"));
        eprintln!(
            "LOAD {:>8.1} ms  {row}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        if row.split(' ').nth(1) != Some("1") {
            not_loaded.push(row);
        }
    }
    let (wants, defined, missing, shadow_guarded) = missing_surface(&script);
    let shown = script
        .eval::<bool>("return PlayerFrame ~= nil and PlayerFrame:IsShown() and TargetFrame ~= nil")
        .unwrap_or(false);
    // Last: the probes call bindings with made-up arguments and may change the VM's state.
    script.clear_instruction_budget();
    census_fixtures::seed(&mut script);
    census_arity::audit(&script, &wants, &defined);
    eprintln!("hard failures: {hard}; PlayerFrame shown with TargetFrame built: {shown}");
    assert!(shown);
    assert!(!failures.iter().any(|f| f.contains("instruction budget")));
    // The finish line, asserted rather than read: every count the census prints above.
    assert_eq!(hard, 0, "hard failures (FAIL/ERROR lines above)");
    assert_eq!(missing, 0, "missing surface names (listed above)");
    assert_eq!(shadow_guarded, 0, "shadow-guarded names (listed above)");
    assert!(
        not_loaded.is_empty(),
        "installed addons that did not load: {not_loaded:?}"
    );
}

/// FrameXML-called names build 12340's `Wow.exe` itself lacks, so the native client would raise on
/// the call too: reported as their own count and excluded from "missing". A name enters only
/// with its evidence (a string scan of the pinned `Wow.exe` plus the caller that shows the call is
/// dead natively) in the same commit, and a listed name this VM answers fails the census.
/// The Mac client's movie recorder: the Windows build has none of its bindings.
const MOVIE_RECORDING_ABSENT: &str =
    "no \"MovieRecording\" string anywhere in the Windows Wow.exe 12340 (raw byte scan, \
     /tmp/azb-exp-movie-recording-scan.log); its callers are MacOptionsFrame (shown only by \
     GameMenuButtonMacOptions, hidden unless IsMacClient()), MovieRecordingProgress's OnUpdate \
     (shown by MOVIE_COMPRESSING_PROGRESS) and Minimap.xml's MOVIE_RECORDING_PROGRESS handler, \
     events the Windows client never fires";

const NATIVE_ABSENT: &[(&str, &str)] = &[
    (
        "AchievementMicroButton_Update",
        "not in Wow.exe 12340 (raw byte search) and defined by no installed FrameXML file; its only \
         caller, Blizzard_AchievementUI.lua:671 (AchievementFrame_OnEvent), runs only while \
         AchievementMicroButton is hidden, a stock 3.3.5 dangling reference",
    ),
    ("MovieRecording_Cancel", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_DeleteMovie", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_GetMovieFullPath", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_GetProgress", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_IsCompressing", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_IsRecording", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_IsSupported", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_QueueMovieToCompress", MOVIE_RECORDING_ABSENT),
    ("MovieRecording_SearchUncompressedMovie", MOVIE_RECORDING_ABSENT),
    (
        "MultiCastSummonSpellButtonFlyoutButton_OnClick",
        "not in Wow.exe 12340 (raw byte search) and defined by no installed FrameXML file; its only \
         caller is MultiCastActionBarFrame.xml's hidden $parentFlyoutButton OnClick, a stock 3.3.5 \
         dangling reference",
    ),
    (
        "MultiCastSummonSpellButtonFlyoutButton_OnLeave",
        "not in Wow.exe 12340 (raw byte search) and defined by no installed FrameXML file; its only \
         caller is MultiCastActionBarFrame.xml's hidden $parentFlyoutButton OnLeave, a stock 3.3.5 \
         dangling reference",
    ),
    (
        "MutedList_Update",
        "not in Wow.exe 12340 (raw byte search) and defined by no installed FrameXML file; its only \
         caller, FriendsFrame.lua:1341, is FriendsFrameMuteButton_OnClick of 3.3.5's retired mute list",
    ),
    (
        "Result",
        "not in Wow.exe 12340 (raw byte search, 0 NUL-bounded hits); Dump.lua:335 and :345 call \
         context:Result() only when the caller's own context table defines it (`if \
         (context.Result)`), a Lua-side optional hook, never a widget method",
    ),
    (
        "VoiceChatButton_OnEvent",
        "not in Wow.exe 12340 (raw byte search, 0 NUL-bounded hits) and defined by no installed \
         FrameXML file; its only caller is VoiceChat.xml:60's OnEvent, a dangling reference",
    ),
    (
        "AddSendMailMoney",
        "Wow.exe 12340 has only Get/SetSendMailMoney (string scan, /tmp/azb-ui-mail-helpers.log); \
         MoneyFrame.lua:83 calls it from the SEND_MAIL MoneyTypeInfo drop callback only",
    ),
    (
        "PickupSendMailMoney",
        "Wow.exe 12340 has only Get/SetSendMailMoney (string scan, /tmp/azb-ui-mail-helpers.log); \
         MoneyFrame.lua:79 calls it from the SEND_MAIL MoneyTypeInfo pickup callback only",
    ),
    (
        "AddSendMailCOD",
        "Wow.exe 12340 has only Get/SetSendMailCOD (string scan, /tmp/azb-ui-mail-helpers.log); \
         MoneyFrame.lua:100 calls it from the SEND_MAIL_COD MoneyTypeInfo drop callback only",
    ),
    (
        "PickupSendMailCOD",
        "Wow.exe 12340 has only Get/SetSendMailCOD (string scan, /tmp/azb-ui-mail-helpers.log); \
         MoneyFrame.lua:96 calls it from the SEND_MAIL_COD MoneyTypeInfo pickup callback only",
    ),
    (
        "MovieRecording_DataRate",
        "Wow.exe 12340 has no MovieRecording_ string (raw byte scan, \
         /tmp/azb-exp-movie-recording-scan.log); MacOptionsFrame.lua:359 (MacOptionsFrame_UpdateTime) \
         runs only inside MacOptionsFrame, which opens only from GameMenuButtonMacOptions, hidden \
         when not IsMacClient() (GameMenuFrame.xml:107-111); its dropdown OnLoads return off the Mac",
    ),
    (
        "MovieRecording_MaxLength",
        "Wow.exe 12340 has no MovieRecording_ string (raw byte scan, \
         /tmp/azb-exp-movie-recording-scan.log); MacOptionsFrame.lua:355 (MacOptionsFrame_UpdateTime) \
         runs only inside MacOptionsFrame, which opens only from GameMenuButtonMacOptions, hidden \
         when not IsMacClient() (GameMenuFrame.xml:107-111); its dropdown OnLoads return off the Mac",
    ),
    (
        "MovieRecording_GetAspectRatio",
        "Wow.exe 12340 has no MovieRecording_ string (raw byte scan, \
         /tmp/azb-exp-movie-recording-scan.log); MacOptionsFrame.lua:137/149/316 (resolution dropdown, defaults) \
         runs only inside MacOptionsFrame, which opens only from GameMenuButtonMacOptions, hidden \
         when not IsMacClient() (GameMenuFrame.xml:107-111); its dropdown OnLoads return off the Mac",
    ),
    (
        "MovieRecording_GetViewportWidth",
        "Wow.exe 12340 has no MovieRecording_ string (raw byte scan, \
         /tmp/azb-exp-movie-recording-scan.log); MacOptionsFrame.lua:138/150 (resolution dropdown) \
         runs only inside MacOptionsFrame, which opens only from GameMenuButtonMacOptions, hidden \
         when not IsMacClient() (GameMenuFrame.xml:107-111); its dropdown OnLoads return off the Mac",
    ),
    (
        "MovieRecording_IsCodecSupported",
        "Wow.exe 12340 has no MovieRecording_ string (raw byte scan, \
         /tmp/azb-exp-movie-recording-scan.log); MacOptionsFrame.lua:280 (codec dropdown initializer) \
         runs only inside MacOptionsFrame, which opens only from GameMenuButtonMacOptions, hidden \
         when not IsMacClient() (GameMenuFrame.xml:107-111); its dropdown OnLoads return off the Mac",
    ),
    (
        "MovieRecording_IsCursorRecordingSupported",
        "Wow.exe 12340 has no MovieRecording_ string (raw byte scan, \
         /tmp/azb-exp-movie-recording-scan.log); MacOptionsFrame.lua:86/302 (MacOptionsFrame_Load, _SetDefaults) \
         runs only inside MacOptionsFrame, which opens only from GameMenuButtonMacOptions, hidden \
         when not IsMacClient() (GameMenuFrame.xml:107-111); its dropdown OnLoads return off the Mac",
    ),
];

/// FrameXML-defined names (see [`scan_calls`]) that are also exact strings in `Wow.exe` 12340
/// and that this VM does not answer, reviewed as not being shadowed engine APIs. Any other such
/// name stays counted as missing, so a FrameXML definition cannot hide an engine method or global.
/// A row needs its reason; the list only shrinks.
const SHADOW_REVIEWED: &[(&str, &str)] = &[
    (
        "Desaturate",
        "Blizzard_AchievementUI assigns it per instance (self.Desaturate = AchievementIcon_Desaturate, \
         :787/:806); Wow.exe's string (0x9ea16c) is referenced from a name/value table entry \
         (0xac0efc, value 1) and FUN_00483060, not a script-method table, so no engine method is \
         shadowed (the Texture method is SetDesaturated)",
    ),
    (
        "OnFinished",
        "an animation script-handler name FrameXML defines per instance; Wow.exe's string is the \
         handler name, not an engine method (reviewed by the manager, 2026-10-19)",
    ),
];

/// The census lines a FrameXML definition must not suppress: each `(kind, name, count)` that was
/// suppressed only by a FrameXML definition, is an exact NUL-bounded string in `exe`, is not
/// answered by the VM, and is not in `reviewed`.
fn shadow_guard(
    suppressed: &[(&str, String, usize)],
    exe: &[u8],
    answered: impl Fn(&str) -> bool,
    reviewed: &[(&str, &str)],
) -> Vec<String> {
    suppressed
        .iter()
        .filter(|(_, name, _)| {
            let needle = [b"\0".as_slice(), name.as_bytes(), b"\0".as_slice()].concat();
            exe.windows(needle.len()).any(|w| w == needle.as_slice())
                && !answered(name)
                && !reviewed.iter().any(|(r, _)| r == name)
        })
        .map(|(kind, name, n)| format!("{kind} {name} {n}"))
        .collect()
}

#[test]
fn shadow_guard_keeps_an_unanswered_engine_name_counted() {
    let exe = b"junk\0SetBindingClick\0OnUpdate\0Run\0other";
    let suppressed = vec![
        ("METHOD", "SetBindingClick".to_string(), 2),
        ("METHOD", "OnUpdate".to_string(), 1),
        ("METHOD", "SetSelection".to_string(), 3),
        ("GLOBAL", "Run".to_string(), 1),
    ];
    let reviewed = [("OnUpdate", "a script-handler name, reviewed in this test")];
    let lines = shadow_guard(&suppressed, exe, |n| n == "Run", &reviewed);
    assert_eq!(
        lines,
        vec!["METHOD SetBindingClick 2".to_string()],
        "reviewed OnUpdate, answered Run and a name absent from Wow.exe are not re-counted"
    );
}

/// Stock XML attributes the loader deliberately does not read, each with its reason. Any other
/// attribute the loader never asks for fails the census; a row the loader now reads is stale.
const ACCEPTED_ATTRS: &[(&str, &str)] = &[
    (
        "xmlns",
        "the XML namespace declaration, not a widget attribute",
    ),
    (
        "xsi:schemaLocation",
        "the schema hint, not a widget attribute",
    ),
    ("xmlns:xsi", "the schema namespace, not a widget attribute"),
    (
        "protected",
        "the secure-frame flag; the UI is trusted, nothing is protected",
    ),
    (
        "dontSavePosition",
        "the layout-cache opt-out; no layout cache is kept",
    ),
    (
        "nonBlocking",
        "an asynchronous texture-load hint with no visible effect",
    ),
    (
        "bytes",
        "a FontString's text buffer size; text is not truncated to it (labelled)",
    ),
    (
        "monochrome",
        "a Font's monochrome raster flag; glyphs are not drawn monochrome (labelled)",
    ),
];

/// Every attribute the stock XML uses that the loader never asked for, less [`ACCEPTED_ATTRS`]:
/// printed, then the census fails on any.
fn unhandled_attributes(xml_texts: &[String]) {
    use wrathplus_ui::framexml::{Element, TopLevel};
    let queried = wrathplus_ui::framexml::queried_attributes();
    let mut found: std::collections::BTreeMap<String, (usize, std::collections::BTreeSet<String>)> =
        Default::default();
    fn walk(
        e: &Element,
        queried: &std::collections::HashSet<String>,
        found: &mut std::collections::BTreeMap<String, (usize, std::collections::BTreeSet<String>)>,
    ) {
        for (k, _) in e.attrs() {
            if !queried.contains(&k.to_ascii_lowercase()) {
                let slot = found.entry(k.clone()).or_default();
                slot.0 += 1;
                slot.1.insert(e.tag.clone());
            }
        }
        for c in &e.children {
            walk(c, queried, found);
        }
    }
    for text in xml_texts {
        let Ok(doc) = wrathplus_ui::framexml::parse(text) else {
            continue;
        };
        for item in &doc.items {
            if let TopLevel::Font(e) | TopLevel::Template(e) | TopLevel::Instance(e) = item {
                walk(e, &queried, &mut found);
            }
        }
    }
    let accepted = |n: &str| {
        ACCEPTED_ATTRS
            .iter()
            .any(|(a, _)| a.eq_ignore_ascii_case(n))
    };
    let mut failing = Vec::new();
    for (name, (n, tags)) in &found {
        let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
        if accepted(name) {
            eprintln!("ACCEPTED-ATTR {name} {n} ({})", tags.join(" "));
        } else {
            eprintln!("UNHANDLED-ATTR {name} {n} ({})", tags.join(" "));
            failing.push(name.clone());
        }
    }
    let stale: Vec<&str> = ACCEPTED_ATTRS
        .iter()
        .map(|(a, _)| *a)
        .filter(|a| !a.contains(':') && *a != "xmlns" && queried.contains(&a.to_ascii_lowercase()))
        .collect();
    assert!(
        stale.is_empty(),
        "ACCEPTED_ATTRS rows the loader now reads: {stale:?}"
    );
    assert!(
        failing.is_empty(),
        "stock XML attributes the loader never reads: {failing:?}"
    );
}

/// The `Blizzard_*` addons whose sources the census scans beside FrameXML.toc's files: every
/// stock 3.3.5 addon, so "0 missing" covers the whole stock UI.
const CENSUS_ADDONS: [&str; 24] = super::addons::WRATH_BLIZZARD_ADDONS;

/// Every `:Method(` and capitalised global `Name(` the installed FrameXML source calls that this
/// VM lacks after the load: the remaining API surface, in one list, less [`NATIVE_ABSENT`].
/// Returns the arity audit's inputs and the missing and shadow-guarded counts.
fn missing_surface(
    script: &UiScript,
) -> (
    census_arity::Wants,
    std::collections::HashSet<String>,
    usize,
    usize,
) {
    let core = super::reference_ui::core().expect("FrameXML.toc");
    let mut sources = Vec::new();
    // Each scanned file's origin, FrameXML or its addon, for the per-addon breakdown.
    let mut origins: Vec<&str> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    let mut queue: Vec<(String, &str)> = core
        .toc
        .files
        .iter()
        .map(|f| (f.clone(), "FrameXML"))
        .collect();
    if let Some(dir) = std::env::var_os("AZB_FRAMEXML_DUMP") {
        for table in [
            "Languages",
            "ItemBagFamily",
            "TaxiNodes",
            "GMTicketCategory",
            "EmotesTextData",
            "GroundEffectDoodad",
            "GroundEffectTexture",
            "SpellVisualKit",
            "SpellVisual",
        ] {
            if let Some(bytes) = super::reference_ui::read(&format!(r"DBFilesClient\{table}.dbc")) {
                let _ = std::fs::write(
                    std::path::Path::new(&dir).join(format!("{table}.dbc")),
                    bytes,
                );
            }
        }
        for (path, file) in [
            (r"Creature\Kobold\Kobold.m2", "Kobold.m2"),
            (
                r"World\Generic\Orc\Passive Doodads\Braziers\SmallBrazierPurple01.m2",
                "Brazier.m2",
            ),
            (
                r"World\Azeroth\Elwynn\PassiveDoodads\Campfire\ElwynnCampfire.m2",
                "Campfire.m2",
            ),
        ] {
            if let Some(bytes) = super::reference_ui::read(path) {
                let _ = std::fs::write(std::path::Path::new(&dir).join(file), bytes);
            }
        }
    }
    // The stock addons whose sources are counted too, whatever the dump setting.
    for name in CENSUS_ADDONS {
        let dir = format!(r"Interface\AddOns\{name}");
        if let Some(toc) = super::reference_ui::read(&format!(r"{dir}\{name}.toc")) {
            let toc = wrathplus_ui::toc::Toc::parse(&wrathplus_ui::source::decode(&toc));
            queue.extend(toc.files.iter().map(|f| (format!(r"{dir}\{f}"), name)));
        }
    }
    let mut xml_texts: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    while let Some((path, origin)) = queue.pop() {
        if !seen.insert(path.to_ascii_lowercase()) {
            continue;
        }
        let Some(bytes) = super::reference_ui::read(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(dir) = std::env::var_os("AZB_FRAMEXML_DUMP") {
            let leaf = path.rsplit('\\').next().unwrap_or(&path);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(std::path::Path::new(&dir).join(leaf), &bytes);
        }
        if path.to_ascii_lowercase().ends_with(".xml") {
            let dir = path.rsplit_once('\\').map_or("", |(d, _)| d).to_string();
            for part in text.split("file=\"").skip(1) {
                if let Some(file) = part.split('"').next() {
                    queue.push((format!("{dir}\\{file}"), origin));
                }
            }
        }
        if path.to_ascii_lowercase().ends_with(".xml") {
            xml_texts.push(text.clone());
        }
        sources.push(census_lexer::executable_text(&text));
        origins.push(origin);
        paths.push(path);
    }
    script.clear_instruction_budget();
    let has: Vec<String> = script
        .lua()
        .load(
            r##"
            local kinds = { "Frame", "Button", "CheckButton", "EditBox", "StatusBar", "Slider",
                "ScrollFrame", "MessageFrame", "ScrollingMessageFrame", "GameTooltip", "Model",
                "PlayerModel", "DressUpModel", "TabardModel", "Minimap", "ColorSelect", "SimpleHTML",
                "Cooldown", "LootButton", "MovieFrame", "QuestPOIFrame" }
            local has, out = {}, {}
            for _, kind in ipairs(kinds) do
                local ok, frame = pcall(CreateFrame, kind)
                if ok and frame then
                    for _, object in ipairs({ frame, frame:CreateTexture(), frame:CreateFontString() }) do
                        local mt = getmetatable(object)
                        local index = mt and mt.__index
                        while type(index) == "table" do
                            for name in pairs(index) do has[name] = true end
                            local up = getmetatable(index)
                            index = up and up.__index
                        end
                    end
                else
                    out[#out + 1] = "KIND:" .. kind
                end
            end
            -- Animation groups and animations keep their methods on the object itself.
            local frame = CreateFrame("Frame")
            if frame.CreateAnimationGroup then
                local group = frame:CreateAnimationGroup()
                local objects = { group }
                for _, kind in ipairs({ "Animation", "Alpha", "Translation", "Scale", "Rotation" }) do
                    objects[#objects + 1] = group:CreateAnimation(kind)
                end
                for _, object in ipairs(objects) do
                    for name, value in pairs(object) do
                        if type(name) == "string" and type(value) == "function" then has[name] = true end
                    end
                end
            end
            for name in pairs(has) do out[#out + 1] = name end
            return out
            "##,
        )
        .call(())
        .unwrap();
    let has: std::collections::HashSet<String> = has.into_iter().collect();
    for kind in has.iter().filter(|n| n.starts_with("KIND:")) {
        eprintln!("MISSING {kind}");
    }
    let mut defined = std::collections::HashSet::new();
    let mut methods = std::collections::BTreeMap::<String, usize>::new();
    let mut calls = std::collections::BTreeMap::<String, usize>::new();
    for src in &sources {
        scan_calls(src, &mut defined, &mut methods, &mut calls);
    }
    // ALL_CAPS globals read but never assigned in the source: engine-provided constants.
    let mut constants = std::collections::BTreeMap::<String, usize>::new();
    let mut assigned = std::collections::HashSet::new();
    for src in &sources {
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_uppercase()
                && (i == 0
                    || !(bytes[i - 1].is_ascii_alphanumeric() || b"_.:\"'".contains(&bytes[i - 1])))
            {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_uppercase()
                        || bytes[i].is_ascii_digit()
                        || bytes[i] == b'_')
                {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i].is_ascii_lowercase() || bytes[i] == b'(') {
                    continue;
                }
                let name = &src[start..i];
                if name.len() < 4 || !name.contains('_') {
                    continue;
                }
                let rest = src[i..].trim_start();
                if rest.starts_with('=') && !rest.starts_with("==") {
                    assigned.insert(name.to_string());
                } else {
                    *constants.entry(name.to_string()).or_default() += 1;
                }
            } else {
                i += 1;
            }
        }
    }
    let unassigned: Vec<String> = constants
        .keys()
        .filter(|n| !assigned.contains(*n))
        .cloned()
        .collect();
    let absent_constants: Vec<String> = script
        .lua()
        .load("local out = {} for _, n in ipairs(...) do if _G[n] == nil then out[#out+1] = n end end return out")
        .call(unassigned)
        .unwrap();
    for name in absent_constants {
        eprintln!("CONST {name} {}", constants[&name]);
    }
    let names: Vec<String> = calls
        .keys()
        .filter(|n| !defined.contains(*n))
        .cloned()
        .collect();
    // The names only a FrameXML definition suppresses, for the Wow.exe shadow guard.
    let mut suppressed: Vec<(&str, String, usize)> = Vec::new();
    for (name, n) in &methods {
        if !has.contains(name) && defined.contains(name) {
            suppressed.push(("METHOD", name.clone(), *n));
        }
    }
    for (name, n) in &calls {
        if defined.contains(name) {
            suppressed.push(("GLOBAL", name.clone(), *n));
        }
    }
    let exe_path = std::path::Path::new(&std::env::var("WOW_DATA").expect("WOW_DATA"))
        .parent()
        .expect("WOW_DATA has a parent")
        .join("Wow.exe");
    let exe = std::fs::read(&exe_path)
        .unwrap_or_else(|e| panic!("the shadow guard needs {}: {e}", exe_path.display()));
    let candidate_names: Vec<String> = suppressed.iter().map(|(_, n, _)| n.clone()).collect();
    let globals_present: std::collections::HashSet<String> = script
        .lua()
        .load("local out = {} for _, n in ipairs(...) do if _G[n] ~= nil then out[#out+1] = n end end return out")
        .call::<Vec<String>>(candidate_names)
        .unwrap()
        .into_iter()
        .collect();
    let shadowed = shadow_guard(
        &suppressed,
        &exe,
        |n| has.contains(n) || globals_present.contains(n),
        SHADOW_REVIEWED,
    );
    let shadow_hits: std::collections::HashSet<&str> = suppressed
        .iter()
        .filter(|(_, name, _)| {
            let needle = [b"\0".as_slice(), name.as_bytes(), b"\0".as_slice()].concat();
            exe.windows(needle.len()).any(|w| w == needle.as_slice())
        })
        .map(|(_, n, _)| n.as_str())
        .collect();
    let stale_reviewed: Vec<&str> = SHADOW_REVIEWED
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !shadow_hits.contains(n))
        .collect();
    assert!(
        stale_reviewed.is_empty(),
        "SHADOW_REVIEWED rows no longer shadowing a Wow.exe string: {stale_reviewed:?}"
    );
    let absent: Vec<String> = script
        .lua()
        .load("local out = {} for _, n in ipairs(...) do if _G[n] == nil then out[#out+1] = n end end return out")
        .call(names)
        .unwrap();
    let native_absent = |name: &str| NATIVE_ABSENT.iter().any(|(n, _)| *n == name);
    let mut lines = Vec::new();
    // A listed name the census still sees missing, method or global, keeps its row live.
    let mut absent_listed = Vec::new();
    for (name, n) in &methods {
        if !has.contains(name) && !defined.contains(name) {
            if native_absent(name) {
                absent_listed.push(name.clone());
            } else {
                lines.push(format!("METHOD {name} {n}"));
            }
        }
    }
    for name in absent {
        if native_absent(&name) {
            absent_listed.push(name);
        } else {
            lines.push(format!("GLOBAL {name} {}", calls[&name]));
        }
    }
    // A listed name the VM now answers, or one the source no longer calls, is a stale row.
    let stale: Vec<&str> = NATIVE_ABSENT
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !absent_listed.iter().any(|a| a == n))
        .collect();
    assert!(
        stale.is_empty(),
        "NATIVE_ABSENT rows that are answered or no longer called: {stale:?}"
    );
    let shadow_guarded = shadowed.len();
    eprintln!("shadow-guarded: {} names", shadowed.len());
    for line in &shadowed {
        eprintln!("SHADOW-GUARDED {line}");
    }
    lines.extend(shadowed);
    eprintln!("native-absent: {} names", NATIVE_ABSENT.len());
    for (name, evidence) in NATIVE_ABSENT {
        let n = calls
            .get(*name)
            .or_else(|| methods.get(*name))
            .copied()
            .unwrap_or(0);
        eprintln!("NATIVE-ABSENT {name} {n}: {evidence}");
    }
    unhandled_attributes(&xml_texts);
    // Per origin, the missing names its own files call.
    let mut by_origin: std::collections::BTreeMap<&str, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (src, origin) in sources.iter().zip(&origins) {
        let (mut d, mut m, mut c) = Default::default();
        scan_calls(src, &mut d, &mut m, &mut c);
        for line in &lines {
            let name = line.split(' ').nth(1).unwrap_or("");
            if m.contains_key(name) || c.contains_key(name) {
                by_origin
                    .entry(origin)
                    .or_default()
                    .insert(name.to_string());
            }
        }
    }
    for (origin, names) in &by_origin {
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        eprintln!("ORIGIN {origin} {}: {}", names.len(), names.join(" "));
    }
    let missing = lines.len();
    eprintln!("missing surface: {} names", lines.len());
    for line in lines {
        eprintln!("{line}");
    }
    let mut wants = census_arity::Wants::new();
    for (src, path) in sources.iter().zip(&paths) {
        census_arity::scan(src, path, &census_lexer::local_names(src), &mut wants);
    }
    (wants, defined, missing, shadow_guarded)
}
