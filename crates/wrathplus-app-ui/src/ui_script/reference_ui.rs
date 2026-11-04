//! The stock 1.12 FrameXML this client runs off the player's own patch chain, as the third
//! [`super::addons::Source`]; the parse, `<Include>` and `<Script file=>` resolution and chunk
//! naming are [`super::addons::Addon`]'s.
//!
//! The core is the chain's own `FrameXML.toc` ([`core`]), its rows in its order, as `UI_Init`
//! walks it; the AzerothBench root (`assets/ui/wrath/bootstrap.toc`) follows it. An entry with a path separator
//! comes off the chain, a bare filename is a file we ship ([`is_chain_entry`]). Without client
//! data the chain files are absent, and the log says so once.

use std::sync::OnceLock;

use bevy::prelude::*;
use wrathplus_formats::Chain;
use wrathplus_ui::toc::Toc;

use super::addons::{Addon, Source};

/// The addon name the stock files load under. FrameXML is not an addon: it gets no
/// `ADDON_LOADED`, and [`Addon::chunk_name`] names its chunks by chain path, as the 1.12 client
/// does, so an addon's `\AddOns\` debugstack pattern never matches a FrameXML frame.
pub(super) const NAME: &str = "FrameXML";

/// Whether a test kit's entry comes off the player's chain: it has a path separator. Decidable
/// because `assets/ui` is flat, which `manifest::tests` pins.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn is_chain_entry(entry: &str) -> bool {
    entry.contains('\\') || entry.contains('/')
}

/// The stock interface's table of contents, which `UI_Init` (`0x48fbf0`) hands to the `.toc`
/// runner (`0x6edb90`) at `0x48ffed`, once per UI build, `ReloadUI` included.
pub(super) const TOC: &str = r"Interface\FrameXML\FrameXML.toc";

/// The stock key-binding commands, which `UI_Init` loads after the [`TOC`] walk (`0x490018`).
pub(super) const BINDINGS: &str = r"Interface\FrameXML\Bindings.xml";

/// The default key set, which `UI_Init` reads through `0x4b62b0` (`0x4900c7`).
pub(super) const DEFAULT_BINDINGS: &str = r"WTF\DefaultBindings.wtf";

/// The core interface: the chain's own [`TOC`], patched or not, as an [`Addon`] over the chain, or
/// `None` when the chain has no such file.
pub(super) fn core() -> Option<Addon> {
    read(TOC).map(|bytes| core_at(TOC, &wrathplus_ui::source::decode(&bytes)))
}

/// The core over the text of the stock toc at `path` (FrameXML's, GlueXML's): each file line
/// prefixed with the toc's own directory, in the toc's order, as the runner builds its paths
/// (`0x6edbe5`, `0x6edcf4`, `0x6edd0c`) and dispatches each in turn (`0x6edd51`).
pub(super) fn core_at(path: &str, toc: &str) -> Addon {
    let dir = path.rsplit_once('\\').map_or("", |(d, _)| d);
    addon(
        Toc::parse(toc)
            .files
            .iter()
            .map(|f| format!("{dir}\\{f}"))
            .collect(),
    )
}

/// The stock interface as an [`Addon`] over the chain; `files` are full chain paths.
pub(super) fn addon(files: Vec<String>) -> Addon {
    Addon::new(
        NAME.to_string(),
        Toc {
            directives: Vec::new(),
            files,
        },
        Source::Chain,
    )
}

/// One file's bytes off the player's patch chain, by internal path. Bytes, not a string: Lua
/// takes a chunk as stored, and a cp1252 file is not valid UTF-8.
pub(super) fn read(req: &str) -> Option<Vec<u8>> {
    #[cfg(test)]
    if let Some(overlaid) = fixture::read(req) {
        return overlaid;
    }
    let chain = chain()?;
    // Archive paths separate with `\`; addon manifests resolve with `/`.
    let req = &req.replace('/', "\\");
    match chain.read(req) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            debug!("ui_script: {req} is not in the patch chain: {e:#}");
            None
        }
    }
}

/// The player's patch chain, opened once per process and shared by every VM. Process-local, not
/// the one [`wrathplus_assets`] holds: tests, the addon harness and a bare `UiScript` load the
/// interface with no Bevy world to ask.
static OVERRIDES: OnceLock<wrathplus_formats::AssetOverrides> = OnceLock::new();

pub fn initialize_asset_overrides(overrides: wrathplus_formats::AssetOverrides) {
    // App boot precedes UI creation. Bare VMs/tests deliberately keep archive-only defaults.
    OVERRIDES
        .set(overrides)
        .expect("asset policy initialized once before UI creation");
}

fn chain() -> Option<&'static Chain> {
    static CHAIN: OnceLock<Option<Chain>> = OnceLock::new();
    CHAIN
        .get_or_init(|| {
            let Some(data) = wrathplus_formats::wow_data() else {
                warn!(
                    "ui_script: no client data — the stock interface, `FrameXML.toc` and every \
                     file it lists, is absent off the player's install, so its windows do not \
                     exist and addons that call their globals will raise"
                );
                return None;
            };
            match wrathplus_formats::open_configured_chain(&data) {
                Ok(chain) => {
                    Some(chain.with_overrides(OVERRIDES.get().cloned().unwrap_or_default()))
                }
                Err(e) => {
                    error!("ui_script: opening the patch chain to source the reference UI: {e:#}");
                    None
                }
            }
        })
        .as_ref()
}

/// Files a test lays over the chain, per test thread: an entry with bytes reads as a chain file,
/// one with `None` as a file the chain lacks, and any other path falls through to the chain.
#[cfg(any(test, feature = "test-support"))]
pub(super) mod fixture {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static OVERLAY: RefCell<HashMap<String, Option<Vec<u8>>>> = RefCell::new(HashMap::new());
    }

    #[cfg(test)]
    fn key(path: &str) -> String {
        path.replace('/', "\\").to_ascii_lowercase()
    }

    /// Lays `files` over the chain until the guard drops.
    #[cfg(test)]
    pub(in crate::ui_script) fn lay(files: &[(&str, Option<&[u8]>)]) -> Guard {
        OVERLAY.with(|o| {
            let mut o = o.borrow_mut();
            o.clear();
            for (path, bytes) in files {
                o.insert(key(path), bytes.map(<[u8]>::to_vec));
            }
        });
        Guard
    }

    /// The overlay's answer for `path`, `None` when it has no entry.
    #[cfg(test)]
    pub(super) fn read(path: &str) -> Option<Option<Vec<u8>>> {
        OVERLAY.with(|o| o.borrow().get(&key(path)).cloned())
    }

    /// Clears the overlay on drop.
    #[cfg(test)]
    pub(in crate::ui_script) struct Guard;

    #[cfg(test)]
    impl Drop for Guard {
        fn drop(&mut self) {
            OVERLAY.with(|o| o.borrow_mut().clear());
        }
    }
}

/// The defaults' modes 2-4: [`DEFAULT_BINDINGS`]'s lines after a `BINDINGMODE` line
/// (`0x5641c0`), none when the chain lacks the file.
pub(crate) fn default_binding_layers() -> Vec<(u32, String, String)> {
    read(DEFAULT_BINDINGS)
        .map(|bytes| {
            wrathplus_ui::script::keybind::parse_bindings_wtf_layers(&String::from_utf8_lossy(
                &bytes,
            ))
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "reference_ui_producers.rs"]
mod producers;
#[cfg(test)]
mod tests {
    use wrathplus_ui::script::UiScript;

    fn strip_comments(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut in_xml = false;
        let mut in_lua_block = false;
        for line in text.lines() {
            let mut rest = line;
            let mut kept = String::new();
            loop {
                if in_xml {
                    match rest.find("-->") {
                        Some(i) => {
                            in_xml = false;
                            rest = &rest[i + 3..];
                        }
                        None => break,
                    }
                } else if in_lua_block {
                    match rest.find("]]") {
                        Some(i) => {
                            in_lua_block = false;
                            rest = &rest[i + 2..];
                        }
                        None => break,
                    }
                } else {
                    let xml = rest.find("<!--");
                    let lua = rest.find("--");
                    match (xml, lua) {
                        (Some(x), Some(l)) if x <= l => {
                            kept.push_str(&rest[..x]);
                            in_xml = true;
                            rest = &rest[x + 4..];
                        }
                        (_, Some(l)) => {
                            kept.push_str(&rest[..l]);
                            if rest[l..].starts_with("--[[") {
                                in_lua_block = true;
                                rest = &rest[l + 4..];
                            } else {
                                // A plain `--` comment runs to end of line. Strings are not
                                // tracked, so a `"--"` in one also cuts the line: this can
                                // hide a call, never invent one.
                                rest = "";
                                break;
                            }
                        }
                        (Some(x), None) => {
                            kept.push_str(&rest[..x]);
                            in_xml = true;
                            rest = &rest[x + 4..];
                        }
                        (None, None) => break,
                    }
                }
            }
            if !in_xml && !in_lua_block {
                kept.push_str(rest);
            }
            out.push_str(&kept);
            out.push('\n');
        }
        out
    }

    /// A chain entry that resolved to nothing would leave its globals nil with no error, and a
    /// collision's winner is invisible until an addon calls the wrong body; both are asserted.
    #[test]
    fn a_chain_entry_loads_and_the_later_line_owns_the_collision() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        let mut s = UiScript::new().expect("VM");
        s.set_screen_size(1024.0, 768.0);

        seat_a_player(&mut s);
        let failures = super::super::manifest::load_default_ui(&mut s);
        assert!(failures.is_empty(), "the default UI: {failures:#?}");

        // Only the stock `ContainerFrame.lua` defines these.
        for name in [
            "ContainerFrameItemButton_OnEnter",
            "ContainerFrameItemButton_OnClick",
            "ContainerFrameItemButton_OnLoad",
            "ContainerFrameItemButton_OnModifiedClick",
        ] {
            assert!(
                s.eval::<bool>(&format!("return type({name}) == \"function\""))
                    .unwrap(),
                "{name} must come from the sourced reference file"
            );
        }
        // Its constants, which addons read directly.
        assert_eq!(s.eval::<i64>("return NUM_BAG_FRAMES").unwrap(), 4);
        assert_eq!(s.eval::<i64>("return NUM_CONTAINER_FRAMES").unwrap(), 13);
        // `PaperDollFrame.lua` arrives through `PaperDollFrame.xml`'s `<Script file=>`: a chain
        // `.xml` brings its `.lua`.
        assert!(s
            .eval::<bool>("return type(PaperDollItemSlotButton_OnLoad) == \"function\"")
            .unwrap());

        // The character sheet runs the stock bodies alone, with no `WrathPlusPaperDollSlot_OnLoad`.
        assert!(
            s.eval::<bool>(
                "return type(PaperDollFrame_SetLevel) == \"function\" \
                 and type(CHARACTERFRAME_SUBFRAMES) == \"table\" \
                 and table.getn(CHARACTERFRAME_SUBFRAMES) == 5 \
                 and WrathPlusPaperDollSlot_OnLoad == nil"
            )
            .unwrap(),
            "the character sheet's bodies must be the reference's own now"
        );

        // Of two definitions of a plain Lua global the later stands; `publish_global`'s
        // non-overwriting rule (`0x701bd0`) applies to frames only.
        s.run("function _order_probe() return 1 end").unwrap();
        s.run("function _order_probe() return 2 end").unwrap();
        assert_eq!(
            s.eval::<i64>("return _order_probe()").unwrap(),
            2,
            "the later definition of a colliding name is the live one"
        );
        assert!(s.errors().is_empty(), "{:#?}", s.errors());
    }

    /// Which unmigrated stock FrameXML window loads clean on top of the shipped manifest, each
    /// asked of the real loader in a fresh VM. Clean means it loads, not that it works: nothing
    /// behind a click or an event runs. A failure in a frame name a file of ours also declares is
    /// suspect: `publish_global` does not overwrite (`0x701bd0`), so `_G` keeps our frame while
    /// the stock handlers write to theirs; re-measure with ours removed. Run with `--ignored
    /// --nocapture`.
    #[test]
    #[ignore = "instrument: run by hand when choosing the next window to migrate"]
    fn chain_readiness_report() {
        let _data = wrathplus_formats::wow_data_or_skip!();

        // The stock toc's order, off the chain; a file's position there is where its manifest line
        // goes, so the report prints it.
        let toc = String::from_utf8_lossy(
            &super::read("Interface\\FrameXML\\FrameXML.toc").expect("the reference's own toc"),
        )
        .into_owned();
        let stock: Vec<String> = toc
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#') && l.ends_with(".xml"))
            .map(str::to_string)
            .collect();

        let migrated: Vec<String> = super::core()
            .expect("the player's FrameXML.toc")
            .toc
            .files
            .iter()
            .filter(|f| super::is_chain_entry(f))
            .map(|f| {
                f.rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(f.as_str())
                    .to_string()
            })
            .collect();

        println!(
            "\n=== migration readiness — {} stock windows ===",
            stock.len()
        );
        println!(
            "{:>3}  {:<32} what stops it (empty = loads clean)",
            "pos", "file"
        );

        let mut clean = Vec::new();
        for (i, name) in stock.iter().enumerate() {
            let pos = i + 1;
            if migrated.iter().any(|m| m == name) {
                println!("{pos:>3}  {name:<32} — already migrated");
                continue;
            }
            let mut s = UiScript::new().expect("VM");
            s.set_screen_size(1024.0, 768.0);
            seat_a_player(&mut s);
            let base = super::super::manifest::load_default_ui(&mut s);
            assert!(base.is_empty(), "the shipped manifest itself: {base:#?}");
            s.resolve();
            let before = s.errors().len();

            let path = format!("Interface\\FrameXML\\{name}");
            let addon = super::addon(vec![path.clone()]);
            let mut said = addon.load_files(&s, std::slice::from_ref(&path));
            s.resolve();
            said.extend(s.errors().into_iter().skip(before));

            if said.is_empty() {
                clean.push((pos, name.clone()));
                println!("{pos:>3}  {name:<32} CLEAN");
            } else {
                // One line per distinct complaint: a verb twelve frames miss is one fact.
                let mut seen: Vec<String> = Vec::new();
                for e in said {
                    let one = e.lines().next().unwrap_or("").trim().to_string();
                    let one = if one.len() > 140 {
                        format!("{}…", &one[..140])
                    } else {
                        one
                    };
                    if !one.is_empty() && !seen.contains(&one) {
                        seen.push(one);
                    }
                }
                println!("{pos:>3}  {name:<32} {} issue(s)", seen.len());
                for one in seen.iter().take(6) {
                    println!("         · {one}");
                }
                if seen.len() > 6 {
                    println!("         · … and {} more", seen.len() - 6);
                }
            }
        }

        println!("\n=== loads clean today: {} ===", clean.len());
        for (pos, name) in &clean {
            println!("  {pos:>3}  {name}");
        }
    }

    /// Every function a file of ours defines that a chain entry also defines, and which one stands
    /// by load order; every frame name a stock window we do not load shares with a file of ours
    /// (such a pair cannot load side by side); every function of ours whose parameter count
    /// differs from the reference's. Run before attempting a window swap.
    #[test]
    #[ignore = "instrument: run by hand before attempting a window swap"]
    fn shadowed_reference_functions() {
        let _data = wrathplus_formats::wow_data_or_skip!();

        // Every `name="X"` a document declares, minus the `$parent`-relative ones.
        let declares = |text: &str| -> Vec<String> {
            let mut out = Vec::new();
            for (i, _) in text.match_indices("name=\"") {
                let rest = &text[i + 6..];
                let Some(end) = rest.find('"') else { continue };
                let name = &rest[..end];
                if name.starts_with('$') || name.is_empty() {
                    continue;
                }
                // A template's name is a registry key, not a frame, but two files holding one is
                // the same collision, so templates count too.
                out.push(name.to_string());
            }
            out
        };

        // `function Name(a, b)` to its name and parameter count. Lua never raises on an argument
        // count, so a difference is silent: ours wider breaks when our file goes, ours narrower
        // when a stock caller arrives.
        let params_in = |text: &str| -> Vec<(String, usize)> {
            let mut out = Vec::new();
            for line in text.lines() {
                let Some(rest) = line.trim_start().strip_prefix("function ") else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.is_empty() {
                    continue;
                }
                let Some(open) = rest.find('(') else { continue };
                let Some(close) = rest[open..].find(')') else {
                    continue;
                };
                let args = rest[open + 1..open + close].trim();
                // `...` is a vararg, which has no fixed arity to compare.
                let n = if args.is_empty() || args == "..." {
                    0
                } else {
                    args.split(',').count()
                };
                out.push((name, n));
            }
            out
        };

        let defined_in = |text: &str| -> Vec<String> {
            text.lines()
                .filter_map(|l| l.trim_start().strip_prefix("function "))
                .map(|r| {
                    r.chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<String>()
                })
                .filter(|n| !n.is_empty())
                .collect()
        };

        // Every function each chain entry and its same-name `.lua` define.
        let toc = &super::super::manifest::manifest_files();
        // Load order: each manifest entry at its line, ours included (`ours_wins` reads both
        // sides), then each reached addon's files after everything.
        let chain = gated_chain_entries();
        let pos: std::collections::HashMap<&String, usize> = toc
            .iter()
            .enumerate()
            .map(|(k, f)| (f, k))
            .chain(
                chain
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !toc.contains(f))
                    .map(|(k, f)| (f, toc.len() + k)),
            )
            .collect();
        let mut chain_home: std::collections::HashMap<String, (String, usize)> =
            std::collections::HashMap::new();
        let mut chain_frames: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for entry in &chain {
            let leaf = entry.rsplit(['\\', '/']).next().unwrap_or(entry);
            let mut cands = vec![entry.clone()];
            if let Some(stem) = entry.strip_suffix(".xml") {
                cands.push(format!("{stem}.lua"));
            }
            for cand in cands {
                let Some(b) = super::read(&cand.replace('\\', "/")) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&b).into_owned();
                for name in defined_in(&text) {
                    let at = pos[entry];
                    chain_home.entry(name).or_insert_with(|| {
                        (cand.rsplit('/').next().unwrap_or(leaf).to_string(), at)
                    });
                }
            }
        }

        // Against everything our files define.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ui");
        let mut hits: Vec<(String, String, String, bool)> = Vec::new();
        for entry in toc.iter().filter(|f| !super::is_chain_entry(f)) {
            let Ok(text) = std::fs::read_to_string(dir.join(entry)) else {
                continue;
            };
            for name in defined_in(&text) {
                if let Some((home, at)) = chain_home.get(&name) {
                    // Load order settles it: the later definition is the one that stands.
                    let ours_wins = pos[entry] > *at;
                    hits.push((name, entry.clone(), home.clone(), ours_wins));
                }
            }
        }
        hits.sort();
        hits.dedup();
        // Frame names and arities from every window in the stock toc, loaded or not: one we do
        // not load is the one whose names can collide.
        let ref_toc = String::from_utf8_lossy(
            &super::read("Interface\\FrameXML\\FrameXML.toc").expect("the reference's own toc"),
        )
        .into_owned();
        let mut ref_arity: std::collections::HashMap<String, (usize, String)> =
            std::collections::HashMap::new();
        for line in ref_toc.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') || !line.ends_with(".xml") {
                continue;
            }
            for cand in [line.to_string(), format!("{}.lua", &line[..line.len() - 4])] {
                let Some(b) = super::read(&format!("Interface/FrameXML/{cand}")) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&b).into_owned();
                if cand.ends_with(".xml") {
                    for name in declares(&text) {
                        chain_frames.entry(name).or_insert_with(|| line.to_string());
                    }
                }
                for (name, n) in params_in(&text) {
                    ref_arity.entry(name).or_insert((n, cand.clone()));
                }
            }
        }
        assert!(
            chain_frames.contains_key("GameTooltip") && chain_frames.contains_key("PetFrame"),
            "the stock frame-name scan found nothing recognisable ({} names) — an empty answer \
             here reads as \"no collisions\", which is what the first version of this reported \
             for the wrong reason",
            chain_frames.len()
        );

        // The frame-name half, over stock windows the manifest does not already load.
        let mut frame_hits: Vec<(String, String, String)> = Vec::new();
        for entry in toc.iter().filter(|f| !super::is_chain_entry(f)) {
            let Ok(text) = std::fs::read_to_string(dir.join(entry)) else {
                continue;
            };
            for name in declares(&text) {
                if let Some(home) = chain_frames.get(&name) {
                    // The leaf compared whole: a suffix test would take `UIOptionsFrame.xml` for
                    // `OptionsFrame.xml`, a different window.
                    let already = toc
                        .iter()
                        .filter(|f| super::is_chain_entry(f))
                        .any(|f| f.rsplit(['\\', '/']).next() == Some(home.as_str()));
                    if !already {
                        frame_hits.push((name, entry.clone(), home.clone()));
                    }
                }
            }
        }
        frame_hits.sort();
        frame_hits.dedup();

        println!("\n=== names ours redefines that a CHAIN entry already defines ===");
        println!("{:<36} {:<28} {:<34} winner", "name", "ours", "chain");
        for (name, ours, home, ours_wins) in &hits {
            let w = if *ours_wins { "OURS" } else { "the chain's" };
            println!("{name:<36} {ours:<28} {home:<34} {w}");
        }

        // Grouped by our file, the unit of work.
        let mut by_file: std::collections::BTreeMap<&String, usize> =
            std::collections::BTreeMap::new();
        for (_, ours, _, _) in &hits {
            *by_file.entry(ours).or_default() += 1;
        }
        println!(
            "\n=== {} collisions, across {} of our files ===",
            hits.len(),
            by_file.len()
        );
        for (f, n) in &by_file {
            println!("  {n:>3}  {f}");
        }

        // The arity half: it bites without both files loading, so it is a section of its own.
        let mut arity: Vec<(String, String, usize, usize, String)> = Vec::new();
        for entry in toc.iter().filter(|f| !super::is_chain_entry(f)) {
            let Ok(text) = std::fs::read_to_string(dir.join(entry)) else {
                continue;
            };
            for (name, ours_n) in params_in(&text) {
                if let Some((ref_n, home)) = ref_arity.get(&name) {
                    if *ref_n != ours_n {
                        arity.push((name, entry.clone(), ours_n, *ref_n, home.clone()));
                    }
                }
            }
        }
        arity.sort();
        arity.dedup();
        println!(
            "\n=== {} signatures of ours differ in ARITY from the reference's ===",
            arity.len()
        );
        println!("{:<34} {:<26} ours ref  direction", "name", "ours");
        for (name, ours, a, b, home) in &arity {
            // Which swap the difference bites on; both directions are silent.
            let dir = if a > b {
                "ours WIDER  — bites when OUR file goes"
            } else {
                "ours NARROWER — bites when THEIRS arrives"
            };
            println!("{name:<34} {ours:<26} {a:>4} {b:>3}  {dir} — ref in {home}");
        }

        // The frame half, grouped by stock window: can this one be added?
        let mut by_stock: std::collections::BTreeMap<&String, std::collections::BTreeSet<&String>> =
            std::collections::BTreeMap::new();
        for (_, ours, home) in &frame_hits {
            by_stock.entry(home).or_default().insert(ours);
        }
        println!(
            "\n=== {} FRAME-NAME collisions: {} stock windows we do not load already have their \
             names declared by a file of ours ===",
            frame_hits.len(),
            by_stock.len()
        );
        for (stock, ours) in &by_stock {
            let n = frame_hits.iter().filter(|(_, _, h)| h == *stock).count();
            let mine: Vec<&str> = ours.iter().map(|s| s.as_str()).collect();
            println!("  {n:>3}  {stock:<34} vs {}", mine.join(", "));
        }
    }

    /// A chain `.xml` that does not source its own `.lua` needs the `.lua` as a manifest line too,
    /// as the stock toc lists `MoneyInputFrame.lua` and `TextStatusBar.lua` (lines 11 and 32).
    /// Without it every global the file should define reads nil, and nothing errors.
    #[test]
    fn every_chain_xml_brings_its_own_lua() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        let toc = &super::super::manifest::manifest_files();
        let listed: std::collections::HashSet<&str> = toc
            .iter()
            .map(|f| f.rsplit(['\\', '/']).next().unwrap_or(f))
            .collect();

        // A `<Script file=…/>` in any loaded document sources the file too, as
        // `VideoOptionsPanels.xml:6` and `AudioOptionsPanels.xml:5` both do `OptionsPanelTemplates.lua`.
        let mut scripted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in &gated_chain_entries() {
            for text in entry_sources(entry) {
                for chunk in text.split('<').skip(1) {
                    let tag = chunk.split('>').next().unwrap_or("");
                    if !tag.trim_start().starts_with("Script") {
                        continue;
                    }
                    if let Some(file) = tag_attr(tag, "file") {
                        let leaf = file.rsplit(['\\', '/']).next().unwrap_or(&file);
                        scripted.insert(leaf.to_ascii_lowercase());
                    }
                }
            }
        }

        let mut orphans = Vec::new();
        for entry in &gated_chain_entries() {
            let leaf = entry.rsplit(['\\', '/']).next().unwrap_or(entry);
            let Some(stem) = leaf.strip_suffix(".xml") else {
                continue;
            };
            let lua = format!("{stem}.lua");
            // No sibling in the archive means there is nothing to miss.
            if super::read(&format!("Interface/FrameXML/{lua}")).is_none() {
                continue;
            }
            let xml = super::read(&entry.replace('\\', "/"))
                .unwrap_or_else(|| panic!("{entry}: not in the chain"));
            let text = String::from_utf8_lossy(&xml);
            let sourced = text
                .to_ascii_lowercase()
                .contains(&format!("file=\"{}\"", lua.to_ascii_lowercase()));
            let elsewhere = scripted.contains(&lua.to_ascii_lowercase());
            if !sourced && !elsewhere && !listed.contains(lua.as_str()) {
                orphans.push(format!(
                    "{leaf} does not source {lua}, and {lua} is not a manifest entry"
                ));
            }
        }
        assert!(
            orphans.is_empty(),
            "a chain window whose code never loads — silent, every global it defines reads nil:\n  {}",
            orphans.join("\n  ")
        );
    }

    #[test]
    fn a_separator_is_what_makes_an_entry_the_players_own_file() {
        assert!(super::is_chain_entry(
            "Interface\\FrameXML\\ContainerFrame.xml"
        ));
        assert!(super::is_chain_entry(
            "Interface/FrameXML/ContainerFrame.xml"
        ));
        assert!(!super::is_chain_entry("BagFrame.xml"));
        assert!(!super::is_chain_entry("ScrollTemplates.xml"));
    }
    /// Every global function and virtual template a manifest entry declares, including the
    /// functions of each `.lua` a chain `.xml` sources.
    fn declared_by(entry: &str) -> std::collections::BTreeSet<String> {
        fn attr(tag: &str, key: &str) -> Option<String> {
            let pat = format!("{key}=\"");
            let mut from = 0;
            while let Some(i) = tag[from..].find(&pat) {
                let at = from + i;
                let before_ok = at == 0
                    || tag[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_whitespace());
                let rest = &tag[at + pat.len()..];
                if before_ok {
                    return rest.find('"').map(|j| rest[..j].to_string());
                }
                from = at + pat.len();
            }
            None
        }
        fn harvest(text: &str, out: &mut std::collections::BTreeSet<String>) {
            for line in text.lines() {
                if let Some(rest) = line.trim_start().strip_prefix("function ") {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() && rest[name.len()..].trim_start().starts_with('(') {
                        out.insert(name);
                    }
                }
            }
            for chunk in text.split('<').skip(1) {
                let Some(end) = chunk.find('>') else { continue };
                let tag = &chunk[..end];
                if tag.contains("virtual=\"true\"") {
                    if let Some(n) = attr(tag, "name") {
                        out.insert(n);
                    }
                }
            }
        }

        let mut out = std::collections::BTreeSet::new();
        let text = if super::is_chain_entry(entry) {
            let Some(bytes) = super::read(&entry.replace('\\', "/")) else {
                return out;
            };
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/ui")
                .join(entry);
            match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => return out,
            }
        };
        harvest(&text, &mut out);

        // A chain `.xml`'s `<Script file=>`, resolved in the `.xml`'s own folder.
        if super::is_chain_entry(entry) && entry.to_ascii_lowercase().ends_with(".xml") {
            let dir = {
                let p = entry.replace('\\', "/");
                p.rsplit_once('/')
                    .map(|(d, _)| d.to_string())
                    .unwrap_or_default()
            };
            for chunk in text.split('<').skip(1) {
                let Some(end) = chunk.find('>') else { continue };
                let tag = &chunk[..end];
                if !tag.trim_start().starts_with("Script") {
                    continue;
                }
                let Some(file) = attr(tag, "file") else {
                    continue;
                };
                if !file.to_ascii_lowercase().ends_with(".lua") {
                    continue;
                }
                if let Some(bytes) = super::read(&format!("{dir}/{}", file.replace('\\', "/"))) {
                    harvest(&String::from_utf8_lossy(&bytes), &mut out);
                }
            }
        }
        out
    }

    /// The later line wins (a template is a `HashMap::insert`, a function an overwrite), so a
    /// function or template of ours that a later chain entry redeclares is dead while still on
    /// disk. Ours overriding a stock one from a later line is not checked here.
    #[test]
    fn nothing_we_ship_is_shadowed_by_a_later_chain_entry() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        let toc = &super::super::manifest::manifest_files();
        let mut ours: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut shadowed: Vec<String> = Vec::new();
        for entry in toc.iter() {
            let names = declared_by(entry);
            if super::is_chain_entry(entry) {
                for n in names {
                    if let Some(file) = ours.remove(&n) {
                        shadowed.push(format!("{n}  (ours in {file}, the chain's in {entry})"));
                    }
                }
            } else {
                for n in names {
                    ours.insert(n, entry.clone());
                }
            }
        }
        shadowed.sort();
        assert!(
            shadowed.is_empty(),
            "dead copies — declared by one of ours, then overwritten by a later chain entry:\n  {}",
            shadowed.join("\n  ")
        );
    }

    /// An inherited template nothing declares is only a warning: the frame is built bare, with none
    /// of the template's regions, children or `<Scripts>`, so its `<OnLoad>` state stays nil.
    #[test]
    fn every_template_the_manifest_inherits_is_declared_by_the_manifest() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        let toc = &super::super::manifest::manifest_files();

        let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
        // Inherits nested in a virtual template resolve when that template is instantiated, which
        // may follow a load-on-demand load: `AchievementAlertFrameTemplate` (AlertFrames.xml:195)
        // inherits `AchievementIconFrameTemplate`, and AlertFrames.lua:237 builds it only after
        // `AchievementFrame_LoadUI`.
        let mut deferred: Vec<(String, String)> = Vec::new();
        let mut wanted: Vec<(String, String)> = Vec::new();
        for entry in toc.iter() {
            // `<Include>` counts: the stock toc lists two `*Templates.xml`; the rest arrive with
            // the window that includes them, as `HonorFrame.xml` does `HonorFrameTemplates.xml`.
            let mut text = String::new();
            for src in entry_sources(entry) {
                text.push_str(&src);
                text.push('\n');
            }
            if text.is_empty() {
                continue;
            }
            // Open elements, each marked when it or an ancestor is virtual.
            let mut open: Vec<bool> = Vec::new();
            for chunk in text.split('<').skip(1) {
                let Some(end) = chunk.find('>') else { continue };
                let tag = &chunk[..end];
                if tag.starts_with('/') {
                    open.pop();
                    continue;
                }
                if tag.starts_with(['?', '!']) {
                    continue;
                }
                let virtual_tag = tag.contains("virtual=\"true\"");
                if virtual_tag {
                    if let Some(n) = tag_attr(tag, "name") {
                        declared.insert(n);
                    }
                }
                let inside_virtual = open.last().copied().unwrap_or(false);
                if let Some(list) = tag_attr(tag, "inherits") {
                    let into = if inside_virtual {
                        &mut deferred
                    } else {
                        &mut wanted
                    };
                    for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        into.push((name.to_string(), entry.clone()));
                    }
                }
                if !tag.ends_with('/') {
                    open.push(inside_virtual || virtual_tag);
                }
            }
        }

        // What the reached load-on-demand addons declare, for the deferred inherits only.
        let mut on_demand: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in gated_chain_entries()
            .iter()
            .filter(|e| e.starts_with("Interface\\AddOns\\"))
        {
            for text in entry_sources(entry) {
                for chunk in text.split('<').skip(1) {
                    let tag = chunk.split('>').next().unwrap_or("");
                    if tag.contains("virtual=\"true\"") {
                        on_demand.extend(tag_attr(tag, "name"));
                    }
                }
            }
        }
        wanted.extend(deferred.into_iter().filter(|(n, _)| !on_demand.contains(n)));

        let mut missing: Vec<String> = wanted
            .into_iter()
            .filter(|(n, _)| !declared.contains(n))
            .map(|(n, e)| format!("{n}  (inherited in {e})"))
            .collect();
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "the manifest inherits templates it never loads — the frames are built BARE, with no \
             scripts, and nothing errors:\n  {}",
            missing.join("\n  ")
        );
    }

    /// One `name="…"`-style attribute out of a raw tag, matched only at a token boundary.
    fn tag_attr(tag: &str, key: &str) -> Option<String> {
        let pat = format!("{key}=\"");
        let mut from = 0;
        while let Some(i) = tag[from..].find(&pat) {
            let at = from + i;
            let boundary = at == 0
                || tag[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_whitespace());
            let rest = &tag[at + pat.len()..];
            if boundary {
                return rest.find('"').map(|j| rest[..j].to_string());
            }
            from = at + pat.len();
        }
        None
    }

    /// The LoadOnDemand `Blizzard_*` addons the interface reaches: each one a
    /// `UIParentLoadAddOn("…")` literal names in our files or a chain entry's sources
    /// (`UIParent.lua`'s `*_LoadUI` loaders, `UIOptionsFrame.lua`'s combat-text load). Like the
    /// stock toc, the manifest has no row for them.
    fn reached_addons() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ui");
        let mut out: Vec<String> = Vec::new();
        for entry in &super::super::manifest::manifest_files() {
            let texts = if super::is_chain_entry(entry) {
                entry_sources(entry)
            } else {
                std::fs::read_to_string(dir.join(entry))
                    .into_iter()
                    .collect()
            };
            for text in texts {
                let text = strip_comments(&text);
                for (i, _) in text.match_indices("UIParentLoadAddOn(\"Blizzard_") {
                    let rest = &text[i + "UIParentLoadAddOn(\"".len()..];
                    if let Some(end) = rest.find('"') {
                        let name = rest[..end].to_string();
                        if !out.contains(&name) {
                            out.push(name);
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// Every chain file the shipped interface loads, in load order: the manifest's chain entries,
    /// then each reached addon's files in its own toc order.
    fn gated_chain_entries() -> Vec<String> {
        let mut out: Vec<String> = super::core()
            .expect("the player's FrameXML.toc")
            .toc
            .files
            .iter()
            .filter(|f| super::is_chain_entry(f))
            .cloned()
            .collect();
        for name in reached_addons() {
            let bytes =
                super::read(&format!("Interface/AddOns/{name}/{name}.toc")).unwrap_or_else(|| {
                    panic!("{name}: reached by UIParentLoadAddOn, not on the chain")
                });
            let toc = wrathplus_ui::toc::Toc::parse(&wrathplus_ui::source::decode(&bytes));
            for file in &toc.files {
                out.push(format!(
                    "Interface\\AddOns\\{name}\\{}",
                    file.replace('/', "\\")
                ));
            }
        }
        out
    }

    fn entry_sources(entry: &str) -> Vec<String> {
        fn read_one(path: &str, chain: bool) -> Option<String> {
            if chain {
                let bytes = super::read(&path.replace('\\', "/"))?;
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/ui")
                    .join(path),
            )
            .ok()
        }
        let chain = super::is_chain_entry(entry);
        let dir = {
            let p = entry.replace('\\', "/");
            p.rsplit_once('/').map(|(d, _)| d.to_string())
        };
        let mut out = Vec::new();
        let mut queue = vec![entry.to_string()];
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        while let Some(path) = queue.pop() {
            if !seen.insert(path.clone()) {
                continue;
            }
            let Some(text) = read_one(&path, chain) else {
                continue;
            };
            for chunk in text.split('<').skip(1) {
                let Some(end) = chunk.find('>') else { continue };
                let tag = &chunk[..end];
                // `<Include>` brings a sibling document and `<Script file=>` the code, where
                // nearly every `RegisterEvent` lives; both resolve in the entry's own folder.
                let kind = tag.trim_start();
                if !kind.starts_with("Include") && !kind.starts_with("Script") {
                    continue;
                }
                if let Some(file) = tag_attr(tag, "file") {
                    let next = match &dir {
                        Some(d) => format!("{d}/{}", file.replace('\\', "/")),
                        None => file.replace('\\', "/"),
                    };
                    queue.push(next);
                }
            }
            out.push(text);
        }
        out
    }

    /// A faux list follows its bar only through `FauxScrollFrame_OnVerticalScroll`
    /// (`UIPanelTemplates.lua:228`), called from the owner's own `<OnVerticalScroll>`; 1.12 has no
    /// `updateFunc` field. Without the handler the bar moves, the list never does, and nothing
    /// errors.
    #[test]
    fn every_faux_scroll_frame_declares_its_own_on_vertical_scroll() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ui");
        let mut unwired: Vec<String> = Vec::new();
        let mut stale: Vec<String> = Vec::new();
        for file in std::fs::read_dir(&dir).expect("assets/ui").flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "xml") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap_or_default();

            for line in text.lines() {
                let code = line.trim_start();
                if code.starts_with("--") || code.starts_with("<!--") {
                    continue;
                }
                if code.contains(".updateFunc") && code.contains('=') && !code.contains("==") {
                    stale.push(format!("{name}: {}", code.trim()));
                }
            }

            // Each `<ScrollFrame … inherits="…FauxScrollFrameTemplate">` up to its close.
            let mut from = 0;
            while let Some(i) = text[from..].find("<ScrollFrame ") {
                let start = from + i;
                let Some(gt) = text[start..].find('>') else {
                    break;
                };
                let tag = &text[start..start + gt];
                from = start + gt;
                if !tag.contains("FauxScrollFrameTemplate") {
                    continue;
                }
                // A virtual template is not a list; each instance declares its own handler.
                if tag.contains("virtual=\"true\"") {
                    continue;
                }
                let Some(end) = text[start..].find("</ScrollFrame>") else {
                    continue;
                };
                let body = &text[start..start + end];
                if !body.contains("OnVerticalScroll") {
                    let who = tag_attr(tag, "name").unwrap_or_else(|| "?".into());
                    unwired.push(format!("{name}: {who}"));
                }
            }
        }
        assert!(
            unwired.is_empty(),
            "a faux list with no <OnVerticalScroll> — its bar moves and the list never follows:\n  {}",
            unwired.join("\n  ")
        );
        assert!(
            stale.is_empty(),
            "`frame.updateFunc` is our retired kit's field; the reference has none:\n  {}",
            stale.join("\n  ")
        );
    }

    /// Every event a chain file registers is fired by something here or listed in `UNPRODUCED`. An
    /// event name is a plain string at both ends, so a listener nothing feeds is silent; a new one
    /// means a window migration brought it.
    #[test]
    fn every_event_a_chain_file_registers_has_a_producer() {
        let _data = wrathplus_formats::wow_data_or_skip!();

        // Names built at runtime, which a literal scan cannot see; each entry is a promise that
        // something fires it.
        const CONSTRUCTED: &[&str] = &[
            // `ui_unit.rs` builds `UNIT_{}` and `UNIT_MAX{}` over `power_token`'s five resources.
            "UNIT_MANA",
            "UNIT_RAGE",
            "UNIT_FOCUS",
            "UNIT_ENERGY",
            "UNIT_HAPPINESS",
            "UNIT_MAXMANA",
            "UNIT_MAXRAGE",
            "UNIT_MAXFOCUS",
            "UNIT_MAXENERGY",
            "UNIT_MAXHAPPINESS",
        ];

        // Registered by a chain file we load and fired by nothing here; each says why.
        const UNPRODUCED: &[(&str, &str)] = &[
            // ── The stock `UIParent.lua`'s listeners first, then the other files' ──
            (
                "ADDON_ACTION_FORBIDDEN",
                "UIParent.lua — the protected-action refusal; WrathPlus has no protected-call \
                 taint model, so nothing can raise it",
            ),
            (
                "MACRO_ACTION_FORBIDDEN",
                "UIParent.lua — the macro half of ADDON_ACTION_FORBIDDEN, same reason",
            ),
            (
                "BILLING_NAG_DIALOG",
                "UIParent.lua — the subscription-time nag; vmangos never sends it",
            ),
            (
                "IGR_BILLING_NAG_DIALOG",
                "UIParent.lua — the internet-cafe billing nag, likewise never sent",
            ),
            (
                "GOSSIP_ENTER_CODE",
                "UIParent.lua — the code-entry gossip option (a door with a combination); \
                 WrathPlus's gossip feed carries no code-entry option kind yet",
            ),
            (
                "PLAYER_SKINNED",
                "UIParent.lua — the corpse-skinned notice; WrathPlus's loot feed does not derive it",
            ),
            (
                "TRADE_REQUEST",
                "UIParent.lua — the trade ASK dialog, and the one entry on this list that is \
                 UNPRODUCEABLE rather than unbuilt: the 5875 client registers the event and \
                 signals it from NOWHERE (a whole-image census: no signal site passes its id, \
                 `0x11d`), so StaticPopupDialogs[\"TRADE\"] is \
                 dead code THERE too, and WrathPlus leaves it unproduced: \
                 producing this would be a divergence, not a fix",
            ),
            (
                "TRADE_REPLACE_ENCHANT",
                "UIParent.lua — the enchant-replacement confirm inside a trade; WrathPlus's trade \
                 feed does not derive it",
            ),
            (
                "CLOSE_WORLD_MAP",
                "WorldMapFrame.lua — the engine-side close the reference fires when the map is \
                 shut from outside its own frame; WrathPlus closes the map through the frame's own \
                 hide path only",
            ),
            ("DISPLAY_SIZE_CHANGED", "the four paperdoll files"),
            ("ITEM_TEXT_TRANSLATION", "ItemTextFrame.lua"),
            ("PET_UI_CLOSE", "PetPaperDollFrame.lua"),
            ("PET_UI_UPDATE", "PetPaperDollFrame.lua"),
            ("PLAYER_DAMAGE_DONE_MODS", "PaperDollFrame.lua"),
            (
                "SHOW_COMPARE_TOOLTIP",
                "PaperDollFrame.lua — the second `TRADE_REQUEST`: event 377 is \
                 registered in 5875 and signalled from NOWHERE (zero fire sites in the whole \
                 image), so this listener is dead code THERE \
                 too. WrathPlus once fired it, and later drove the plates itself on a \
                 shift-held hover; both were supersets. Nothing in this engine seats a \
                 shopping plate now — the reference's own callers do (`MerchantFrame.xml:63-80`, \
                 the auction rows), which is the whole of the compare in 1.12.1. Producing this \
                 event would be a divergence, not a fix",
            ),
            ("SYSMSG", "UIErrorsFrame.lua"),
            ("UNIT_DEFENSE", "PetPaperDollFrame.lua"),
            (
                "UNIT_QUEST_LOG_CHANGED",
                "QuestLogFrame.lua — a party member's quest-log fields changing (the reference \
                 fires it off the unit's PLAYER_QUEST_LOG_* descriptor updates); WrathPlus's unit \
                 feed does not derive it yet",
            ),
            (
                "UNIT_MODEL_CHANGED",
                "four files — the paperdoll model refresh",
            ),
            ("UNIT_PORTRAIT_UPDATE", "three files — the portrait refresh"),
            (
                "ZONE_UNDER_ATTACK",
                "ChatFrame.lua — the reference's `SMSG_ZONE_UNDER_ATTACK` line (\"%s is under \
                 attack!\"); the wire handler is not built",
            ),
        ];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("crates");
        // Production fires only: test files and inline test modules are no producer.
        let fired = super::producers::produced(&root, CONSTRUCTED);

        let mut dead: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for entry in &gated_chain_entries() {
            for text in entry_sources(entry) {
                let mut from = 0;
                while let Some(i) = text[from..].find("RegisterEvent(") {
                    let at = from + i + "RegisterEvent(".len();
                    from = at;
                    let rest = text[at..].trim_start();
                    if let Some(body) = rest.strip_prefix('"') {
                        if let Some(end) = body.find('"') {
                            let ev = &body[..end];
                            if !fired.contains(ev) {
                                dead.entry(ev.to_string()).or_insert_with(|| entry.clone());
                            }
                        }
                    }
                }
            }
        }

        // The 3.3.5 windows' listeners still waiting on a producer, one per line; the list
        // shrinks as producers land.
        let baseline: Vec<&str> = include_str!("reference_ui_unproduced_335.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        let expected: std::collections::HashSet<&str> = UNPRODUCED
            .iter()
            .map(|(e, _)| *e)
            .chain(baseline.iter().copied())
            .collect();
        let surprises: Vec<String> = dead
            .iter()
            .filter(|(e, _)| !expected.contains(e.as_str()))
            .map(|(e, f)| format!("{e}  (registered in {f})"))
            .collect();
        assert!(
            surprises.is_empty(),
            "a chain file listens for an event NOTHING fires, and it is not one of the known gaps \
             — a window migration brought a listener with no producer:\n  {}",
            surprises.join("\n  ")
        );

        let fixed: Vec<&str> = UNPRODUCED
            .iter()
            .map(|(e, _)| *e)
            .chain(baseline.iter().copied())
            .filter(|e| !dead.contains_key(*e))
            .collect();
        assert!(
            fixed.is_empty(),
            "these now HAVE a producer — take them out of UNPRODUCED or \
             reference_ui_unproduced_335.txt so the lists keep meaning \
             what it says:\n  {fixed:?}"
        );
    }

    /// Seats a player before a probe loads the manifest, as the live client always has one by
    /// then: the stock macro window formats `UnitName("player")` into a tab label in its `OnLoad`,
    /// and without a player that raises like a load failure.
    fn seat_a_player(s: &mut UiScript) {
        s.set_unit(
            "player",
            Some(wrathplus_ui::script::UnitState {
                exists: true,
                name: Some("Probefour".into()),
                level: 60,
                ..Default::default()
            }),
        );
    }

    /// Every event WrathPlus fires exists in the 1.12 client: an event it can dispatch is a
    /// NUL-terminated string in `WoW.exe`, and a name missing there no stock listener can ever
    /// receive. Test files are skipped; they fire synthetic names (`E3`) on purpose.
    #[test]
    fn every_event_we_fire_is_an_event_the_reference_has() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let exe = data.parent().expect("install root").join("WoW.exe");
        let Ok(bytes) = std::fs::read(&exe) else {
            eprintln!("skipping: no WoW.exe at {exe:?}");
            return;
        };

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("crates");
        let mut fired: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                // A test's own synthetic events are not a surface we ship.
                if p.to_string_lossy().contains("test") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                // Split so this file cannot match its own walker.
                const CALL: &str = concat!("fire_event", "(");
                let mut from = 0;
                while let Some(i) = text[from..].find(CALL) {
                    let at = from + i + CALL.len();
                    from = at;
                    let rest = text[at..].trim_start();
                    if let Some(body) = rest.strip_prefix('"') {
                        if let Some(end) = body.find('"') {
                            fired
                                .entry(body[..end].to_string())
                                .or_insert_with(|| p.display().to_string());
                        }
                    }
                }
            }
        }
        assert!(
            fired.len() > 100,
            "the walker found only {} fired events — it stopped matching, which would make this \
             gate silently vacuous",
            fired.len()
        );

        let ghosts: Vec<String> = fired
            .iter()
            // A `WRATHPLUS_` prefix marks an event as ours, never mistaken for a client name.
            .filter(|(ev, _)| !ev.starts_with("WRATHPLUS_"))
            .filter(|(ev, _)| {
                let needle: Vec<u8> = ev.bytes().chain(std::iter::once(0)).collect();
                !bytes.windows(needle.len()).any(|w| w == needle)
            })
            .map(|(ev, at)| format!("{ev}  (fired from {at})"))
            .collect();
        assert!(
            ghosts.is_empty(),
            "WrathPlus fires {} event(s) the 12340 client does not have — a 1.12 name, or one \
             invented here. Nothing in the stock UI can ever listen for these:\n  {}",
            ghosts.len(),
            ghosts.join("\n  ")
        );
    }
}
