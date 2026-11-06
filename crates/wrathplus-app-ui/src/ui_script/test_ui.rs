//! The tests' interface loader. A stock FrameXML file ([`production_rank`]) is part of the session's
//! interface, which the first one loads whole, as a session does ([`seat_session_ui`]); any other
//! path is one off the player's patch chain (an on-demand addon's file), and a bare filename a
//! fixture under `assets/ui`. It reads the source tree, not the compiled-in copy.

#[cfg(test)]
use wrathplus_ui::script::QuadContent;
use wrathplus_ui::script::UiScript;

/// Load one interface file into `s`, panicking on any loader error; returns the frames it built
/// (0 for a `.lua` and for a stock FrameXML file, which seats the whole interface). A chain entry
/// needs client data, so a test naming one opens with
/// `wrathplus_formats::wow_data_or_skip!()`.
pub(crate) fn load_ui(s: &mut UiScript, entry: &str) -> usize {
    load_entry(s, entry, false, false)
}

/// The cooldown indicator's file, as the stock `CooldownFrameTemplate` names it.
#[cfg(test)]
pub(super) const COOLDOWN_MODEL: &str = r"Interface\Cooldown\UI-Cooldown-Indicator.mdx";

/// The cooldown indicator's file facts (`wrathplus-extract m2seq`), handed to the engine as the app
/// does: sequence 0 (id 0, 1000 ms, clamp) is the sweep `CooldownFrame_OnUpdateModel` scrubs,
/// sequence 1 (id 1, 1000 ms, clamp) the finish flash whose end hides the frame.
#[cfg(test)]
pub(super) fn cooldown_facts(s: &mut UiScript) {
    use wrathplus_ui::widget::{ModelFileFacts, SequenceFacts};
    let seq = |anim_id, duration_ms| SequenceFacts {
        anim_id,
        duration_ms,
        looping: false,
    };
    s.set_model_facts(
        COOLDOWN_MODEL,
        ModelFileFacts {
            sequences: vec![seq(0, 1000), seq(1, 1000)],
            bbox: ([0.0; 3], [0.0; 3]),
            cameras: 0,
        },
    );
}

/// How far the shown cooldown sweep `owner` names has run, as `(0, elapsed ms of a 1000 ms
/// sweep)`: the shape the 1.12 model pane's play head had. Build 12340's `Cooldown` is an engine
/// frame type, painted as a [`QuadContent::CooldownSweep`] off `SetCooldown`'s record.
#[cfg(test)]
pub(super) fn cooldown_play(s: &UiScript, owner: &str) -> Option<(u16, u32)> {
    s.extract().into_iter().find_map(|q| match q.content {
        QuadContent::CooldownSweep {
            remaining_milli, ..
        } if s.quad_owner_name(q.target).as_deref() == Some(owner) => {
            Some((0, 1000 - u32::from(remaining_milli)))
        }
        _ => None,
    })
}

/// [`cooldown_play`] for whichever cooldown is shown: `ContainerFrame_GenerateFrame` numbers a
/// bag's slots from the far end, so a one-item test asks for the sweep, not a name.
#[cfg(test)]
pub(super) fn cooldown_play_any(s: &UiScript) -> Option<(u16, u32)> {
    s.extract().into_iter().find_map(|q| match q.content {
        QuadContent::CooldownSweep {
            remaining_milli, ..
        } => Some((0, 1000 - u32::from(remaining_milli))),
        _ => None,
    })
}

/// [`load_ui`], and a missing template fails too: the loader only warns, and the art is lost.
#[cfg(test)]
pub(super) fn load_ui_strict(s: &mut UiScript, entry: &str) -> usize {
    load_entry(s, entry, true, false)
}

/// [`load_ui`], and any loader warning fails.
#[cfg(test)]
pub(super) fn load_ui_no_warnings(s: &mut UiScript, entry: &str) -> usize {
    load_entry(s, entry, false, true)
}

fn load_entry(s: &mut UiScript, entry: &str, strict_templates: bool, no_warnings: bool) -> usize {
    // A stock FrameXML file is part of the session's interface, which loads whole.
    if production_rank(entry).is_some() {
        seat_session_ui(s);
        return 0;
    }
    // A FrameXML path the install does not ship is a 1.12 file (`OptionsFrame.lua`,
    // `UIOptionsFrame.xml`, `LocaleProperties.lua`): 3.3.5 has no such file, so it loads nothing.
    // A test that needs what it defined fails on its own assertion.
    let norm = entry.replace('/', "\\").to_ascii_lowercase();
    if norm.starts_with(r"interface\framexml\") && read(entry).is_none() {
        return 0;
    }
    load_file(s, entry, strict_templates, no_warnings)
}

thread_local! {
    /// The VM sessions whose whole stock interface [`seat_session_ui`] has loaded.
    static SEATED: std::cell::RefCell<std::collections::HashSet<u64>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The stock interface as a session loads it ([`super::load_default_ui`]: the installed
/// `FrameXML.toc` whole, the archive's startup addon and the bootstrap, on a VM seeded with its
/// display mode and player), once per VM: a kit's first stock file loads all of it, and the rest
/// are already there. A test sees the interface a player has, in the order the client loads it.
fn seat_session_ui(s: &mut UiScript) {
    if SEATED.with(|seated| seated.borrow().contains(&s.session())) {
        return;
    }
    let failures = super::load_default_ui(s);
    assert!(
        failures.is_empty(),
        "the session interface failed to load: {failures:#?}"
    );
    SEATED.with(|seated| seated.borrow_mut().insert(s.session()));
}

/// One file's load, the order already checked.
fn load_file(s: &UiScript, entry: &str, strict_templates: bool, no_warnings: bool) -> usize {
    // The app registers its CVars before any file loads: stock `UIOptionsFrame.xml` reads two
    // camera CVars at `OnLoad` and raises on a nil. A value a test already set stands.
    s.register_cvars(crate::shipped_cvars::pairs());
    let path = entry.replace('\\', "/");
    let bytes = read(&path).unwrap_or_else(|| panic!("{entry}: not found"));
    if path.to_ascii_lowercase().ends_with(".lua") {
        s.run_chunk_named(&bytes, &format!("@{entry}"))
            .unwrap_or_else(|e| panic!("{entry}: {e}"));
        return 0;
    }
    let doc = wrathplus_ui::framexml::parse(&wrathplus_ui::source::decode(&bytes))
        .unwrap_or_else(|e| panic!("{entry}: {e}"));
    let provider = |req: &str| -> Option<Vec<u8>> { read(req) };
    let report = wrathplus_ui::loader::load_in(s, &doc, &path, &provider);
    assert!(
        report.errors.is_empty(),
        "{entry}: loader errors: {:?}",
        report.errors
    );
    if no_warnings {
        assert!(
            report.warnings.is_empty(),
            "{entry}: loader warnings: {:?}",
            report.warnings
        );
    }
    if strict_templates {
        let missing: Vec<&String> = report
            .warnings
            .iter()
            .filter(|w| w.contains("unknown template"))
            .collect();
        assert!(
            missing.is_empty(),
            "{entry}: inherits a template this house does not ship (the frame loads, its ART does \
             not): {missing:?}"
        );
    }
    report.frames
}

/// Where `entry` falls in the production load: a `FrameXML.toc` row at its row, a file a row pulls
/// in (`<Include>` or `<Script file=>`, at any depth) just above the first row that does, and
/// `None` for anything else (a fixture, an on-demand addon's file). Read off the player's own toc;
/// `Some` marks a stock file for [`seat_session_ui`].
pub(super) fn production_rank(entry: &str) -> Option<usize> {
    static RANKS: std::sync::OnceLock<std::collections::HashMap<String, usize>> =
        std::sync::OnceLock::new();
    let ranks = RANKS.get_or_init(|| {
        let key = |path: &str| path.replace('/', "\\").to_ascii_lowercase();
        let mut ranks = std::collections::HashMap::new();
        let rows = super::reference_ui::core()
            .map(|core| core.toc.files)
            .unwrap_or_default();
        for (i, row) in rows.iter().enumerate() {
            ranks.insert(key(row), 2 * i + 1);
        }
        for (i, row) in rows.iter().enumerate() {
            let mut queue = vec![row.clone()];
            while let Some(file) = queue.pop() {
                let Some(bytes) = super::reference_ui::read(&file) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&bytes);
                for chunk in text.split('<').skip(1) {
                    let tag = chunk.split('>').next().unwrap_or("").trim_start();
                    if !tag.starts_with("Include") && !tag.starts_with("Script") {
                        continue;
                    }
                    let Some(name) = tag
                        .split("file=\"")
                        .nth(1)
                        .and_then(|r| r.split('"').next())
                    else {
                        continue;
                    };
                    let path = format!(r"Interface\FrameXML\{name}");
                    if let std::collections::hash_map::Entry::Vacant(slot) = ranks.entry(key(&path))
                    {
                        slot.insert(2 * i);
                        queue.push(path);
                    }
                }
            }
        }
        ranks
    });
    ranks
        .get(&entry.replace('/', "\\").to_ascii_lowercase())
        .copied()
}

/// A kit's files in the production order ([`production_rank`]): the lists merged, each file once,
/// and a file with no rank (a fixture) kept behind the file listed before it.
#[cfg(test)]
pub(crate) fn production_order<'a>(parts: &[&[&'a str]]) -> Vec<&'a str> {
    let mut seen = std::collections::HashSet::new();
    let mut keyed = Vec::new();
    let mut last = 0;
    for (i, file) in parts.iter().flat_map(|p| p.iter()).enumerate() {
        if !seen.insert(file.replace('/', "\\").to_ascii_lowercase()) {
            continue;
        }
        last = production_rank(file).unwrap_or(last);
        keyed.push((last, i, *file));
    }
    keyed.sort_by_key(|&(rank, i, _)| (rank, i));
    keyed.into_iter().map(|(_, _, file)| file).collect()
}

/// One file's bytes, off the chain for a path or from this crate's `assets/ui` for a bare name;
/// also the loader's `<Include>`/`<Script file=>` provider.
pub(super) fn read(req: &str) -> Option<Vec<u8>> {
    if super::reference_ui::is_chain_entry(req) {
        return super::reference_ui::read(req);
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ui");
    std::fs::read(dir.join(req)).ok()
}

/// What stock `MerchantFrame.xml` needs to load and behave, including `BasicControls.xml` for the
/// `TEXT()` that `MerchantFrame.lua:70` calls on first show and `ItemButtonTemplate.xml`, whose
/// absence only warns. Needs client data: open with `wrathplus_formats::wow_data_or_skip!()`.
#[cfg(test)]
pub(super) const MERCHANT_UI: &[&str] = &[
    "Interface\\FrameXML\\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    "Interface\\FrameXML\\BasicControls.xml", // TEXT()
    r"Interface\FrameXML\UIParent.xml",
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    "Interface\\FrameXML\\GameTooltip.xml", // app load order: tooltip before merchant
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    // The stock window tab, whose `<OnShow>` needs the `UIPanelTemplates` pair above it.
    r"Interface\FrameXML\CharacterFrameTemplates.xml",
    "Interface\\FrameXML\\StaticPopup.xml",
    "Interface\\FrameXML\\ItemButtonTemplate.xml",
];

/// Stock `GossipFrame.xml`'s dependencies in the production order, shared with `ui_gossip`'s feed
/// tests. The greeting pane inherits `UIPanelScrollFrameTemplate`, and a missing template only
/// warns, losing the scrollbar silently. Needs client data.
#[cfg(test)]
pub(crate) const GOSSIP_UI: &[&str] = &[
    r"Interface\FrameXML\GlobalStrings.lua",
    r"Interface\FrameXML\BasicControls.xml",
    r"Interface\FrameXML\UIParent.xml",
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    r"Interface\FrameXML\StaticPopup.xml",
];

/// What stock `LootFrame.xml` needs to load and behave. `GlobalStrings.lua` because a bare VM
/// lacks what the app loads at setup (the master-loot menu reads `GROUP` and `GIVE_LOOT`),
/// `ItemButtonTemplate.xml` because a missing template only warns, and `PartyFrame.xml` because
/// `LootFrame.lua:217` reads `MAX_PARTY_MEMBERS` at load. Needs client data.
#[cfg(test)]
pub(crate) const LOOT_UI: &[&str] = &[
    "Interface\\FrameXML\\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    "Interface\\FrameXML\\BasicControls.xml", // `TEXT`, which UnitPopup.lua reads at file scope
    r"Interface\FrameXML\UIParent.xml",       // UIParent.lua: the panel slot manager and the fades
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    "Interface\\FrameXML\\GameTooltip.xml", // TOOLTIP_DEFAULT_COLOR, read by the dropdown backdrop
    "Interface\\FrameXML\\UIDropDownMenu.xml", // GroupLootDropDown's OnLoad calls UIDropDownMenu_Initialize
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    "Interface\\FrameXML\\StaticPopup.xml",
    // Each `PartyMemberFrame<N>` and its pet frame run `UnitFrame_Initialize` (UnitFrame.lua) at
    // OnLoad, which calls `SetTextStatusBarText` (TextStatusBar.lua).
    "Interface\\FrameXML\\TextStatusBar.lua",
    "Interface\\FrameXML\\TextStatusBar.xml",
    // `RefreshBuffs`, which each party row's OnLoad reaches (`PartyMemberFrame.lua:60`), is in
    // BuffFrame.lua: the reference's toc has BuffFrame at 40 and PartyFrame at 45.
    "Interface\\FrameXML\\BuffFrame.xml",
    // `UnitPopup.lua` reads `ITEM_QUALITY_COLORS` at file scope (`UnitPopup.lua:47-49`), so its
    // declarer, `UIParent.lua:65`, precedes it.
    "Interface\\FrameXML\\UnitPopup.xml",
    "Interface\\FrameXML\\UnitFrame.xml",
    "Interface\\FrameXML\\PartyFrame.xml",
    "Interface\\FrameXML\\ItemButtonTemplate.xml",
];

/// What stock `CharacterFrame.xml`, `PaperDollFrame.xml` and `PetPaperDollFrame.xml` need to load
/// and behave. `CharacterFrame_OnLoad` reaches `TextStatusBar.lua`, `PlayerFrame.xml`,
/// `MainMenuBar.xml` (`MainMenuExpBar`) and the `UIPanelTemplates` pair; two bite only on show:
/// `HonorFrame.xml`, whose labels `PaperDollFrame.lua:103` and `:122` write, and
/// `MainMenuBarMicroButtons.xml` (`MicroButtonTooltipText` on a tab hover). Needs client data.
#[cfg(test)]
pub(super) const CHARACTER_UI: &[&str] = &[
    // The stock strings, which `PaperDollFrame_OnLoad` reads at load; there are no fallbacks.
    "Interface\\FrameXML\\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    "Interface\\FrameXML\\BasicControls.xml", // TEXT(), which those labels go through
    // `GetText`, the gendered-string helper `ReputationFrame.lua:65` calls for each standing.
    r"Interface\FrameXML\UIParent.xml", // Model_OnLoad/_Rotate*/_OnUpdate: the model turntable
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    "Interface\\FrameXML\\GameTooltip.xml",
    "Interface\\FrameXML\\UIMenu.xml",
    // The unit frames' dropdowns call `UIDropDownMenu_Initialize` at load, so the kit and the menu
    // table precede them.
    "Interface\\FrameXML\\UIDropDownMenu.xml",
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    // ReputationFrame's detail check boxes inherit its `OptionsCheckButtonTemplate`.
    "Interface\\FrameXML\\OptionsFrameTemplates.xml",
    // The stock window tab, whose `<OnShow>` needs the `UIPanelTemplates` pair above it.
    r"Interface\FrameXML\CharacterFrameTemplates.xml",
    "Interface\\FrameXML\\StaticPopup.xml",
    "Interface\\FrameXML\\TextStatusBar.lua",
    "Interface\\FrameXML\\TextStatusBar.xml",
    "Interface\\FrameXML\\MainMenuBar.xml",
    r"Interface\FrameXML\MainMenuBarMicroButtons.xml",
    "Interface\\FrameXML\\Cooldown.xml", // CooldownFrameTemplate + CooldownFrame_SetTimer, per equipment slot
    "Interface\\FrameXML\\ActionButtonTemplate.xml",
    "Interface\\FrameXML\\ActionBarFrame.xml",
    "Interface\\FrameXML\\CombatFeedback.xml",
    "Interface\\FrameXML\\UnitPopup.xml",
    // `UnitFrame_Initialize` and `CombatFeedback_Initialize`, called by the frames below at load.
    "Interface\\FrameXML\\UnitFrame.xml",
    "Interface\\FrameXML\\PlayerFrame.xml",
    // `PetFrame.xml`'s debuff buttons inherit `PartyBuffButtonTemplate`, declared here; the
    // manifest reaches this file only through `PartyFrame.xml`'s `<Include>`.
    "Interface\\FrameXML\\PartyFrameTemplates.xml",
    "Interface\\FrameXML\\PetFrame.xml",
    "Interface\\FrameXML\\ItemButtonTemplate.xml", // PaperDollItemSlotButtonTemplate's base
    // Every page must exist before the window opens: `CharacterFrame_ShowSubFrame` hides each
    // `CHARACTERFRAME_SUBFRAMES` page it is not showing, unguarded (`CharacterFrame.lua:25-32`).
    "Interface\\FrameXML\\CharacterFrame.xml",
    "Interface\\FrameXML\\PaperDollFrame.xml",
    "Interface\\FrameXML\\PetPaperDollFrame.xml",
    "Interface\\FrameXML\\SkillFrame.xml",
    r"Interface\FrameXML\ReputationFrame.xml",
    "Interface\\FrameXML\\HonorFrame.xml",
    // `updateContainerFrameAnchors`, which `ReputationWatchBar_Update` calls when the bar moves
    // (`ReputationFrame.lua:248`): in the reference the bar's presence reflows the bag row.
    "Interface\\FrameXML\\ContainerFrame.xml",
    "Interface\\FrameXML\\BonusActionBarFrame.xml",
];

/// The manifest slice stock `FriendsFrame.xml` and `RaidFrame.xml` reach at load or on show, in its
/// order; [`load_social_ui`] adds the raid tab's LoadOnDemand addon.
#[cfg(test)]
pub(super) const SOCIAL_UI: &[&str] = &[
    "Interface\\FrameXML\\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    "Interface\\FrameXML\\BasicControls.xml",
    r"Interface\FrameXML\UIParent.xml",
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    "Interface\\FrameXML\\GameTooltip.xml",
    "Interface\\FrameXML\\UIMenu.xml",
    "Interface\\FrameXML\\UIDropDownMenu.xml",
    "Interface\\FrameXML\\UIPanelTemplates.lua",
    "Interface\\FrameXML\\UIPanelTemplates.xml",
    "Interface\\FrameXML\\OptionsFrameTemplates.xml",
    // The stock window tab, whose `<OnShow>` needs the `UIPanelTemplates` pair above it.
    r"Interface\FrameXML\CharacterFrameTemplates.xml",
    "Interface\\FrameXML\\StaticPopup.xml",
    "Interface\\FrameXML\\TextStatusBar.lua",
    "Interface\\FrameXML\\TextStatusBar.xml",
    "Interface\\FrameXML\\MainMenuBar.xml",
    r"Interface\FrameXML\MainMenuBarMicroButtons.xml",
    "Interface\\FrameXML\\Cooldown.xml",
    "Interface\\FrameXML\\ActionButtonTemplate.xml",
    "Interface\\FrameXML\\ActionBarFrame.xml",
    "Interface\\FrameXML\\MultiActionBars.xml",
    "Interface\\FrameXML\\BuffFrame.xml",
    "Interface\\FrameXML\\CombatFeedback.xml",
    "Interface\\FrameXML\\UnitPopup.xml",
    "Interface\\FrameXML\\UnitFrame.xml",
    "Interface\\FrameXML\\PartyFrame.xml",
    "Interface\\FrameXML\\ChatFrame.xml",
    "Interface\\FrameXML\\FloatingChatFrame.xml",
    "Interface\\FrameXML\\ReputationFrame.xml",
    "Interface\\FrameXML\\FriendsFrame.xml",
    "Interface\\FrameXML\\RaidFrame.xml",
    "Interface\\FrameXML\\BonusActionBarFrame.xml",
];

/// Load [`SOCIAL_UI`], then the raid tab's LoadOnDemand addon as the app reaches it: seated off the
/// chain and loaded by stock `RaidFrame_LoadUI`. Needs client data.
#[cfg(test)]
pub(super) fn load_social_ui(s: &mut UiScript) {
    for f in SOCIAL_UI {
        load_ui_strict(s, f);
    }
    seat_chain_addon(s, "Blizzard_RaidUI");
    s.run("RaidFrame_LoadUI()").unwrap();
}

/// The files a test needs before it can open a bag window, in the production order, trimmed to what
/// the bags reach for. Needs client data: open with `wrathplus_formats::wow_data_or_skip!()`.
#[cfg(test)]
pub(super) const BAG_UI: &[&str] = &[
    // The stock strings: the bar's hovers pass them to `GameTooltip:SetText`, which raises on nil.
    "Interface\\FrameXML\\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    // `TEXT()`, which the backpack button's OnEnter and `BagSlotButton_OnEnter` call.
    "Interface\\FrameXML\\BasicControls.xml",
    // `UIParent`: the twelve `ContainerFrame`s are its children, `updateContainerFrameAnchors`
    // anchors each open bag to its parent, and `OpenAllBags` returns unless `UIParent` is visible.
    r"Interface\FrameXML\UIParent.xml",
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    "Interface\\FrameXML\\GameTooltip.xml",
    "Interface\\FrameXML\\UIMenu.xml", // the kit ChatMenu/EmoteMenu/VoiceMacroMenu build from
    "Interface\\FrameXML\\UIDropDownMenu.xml",
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    // The dialog engine, after the `UIPanelCloseButton` it inherits.
    r"Interface\FrameXML\StaticPopup.xml",
    // `ContainerFrameItemButton_OnClick` hides `StackSplitFrame` on every plain click
    // (`ContainerFrame.lua:581`) and opens it on the shift fork.
    "Interface\\FrameXML\\StackSplitFrame.xml",
    "Interface\\FrameXML\\TextStatusBar.lua",
    "Interface\\FrameXML\\TextStatusBar.xml",
    // `MainMenuBar.xml` and its templates: the bag bar's `parent="MainMenuBarArtFrame"` resolves
    // at load, and `MainMenuBar_UpdateKeyRing` puts the keyring on the bar.
    "Interface\\FrameXML\\MainMenuBar.xml",
    // `UpdateMicroButtons`, which the keyring's OnHide and OnShow call
    // (`ContainerFrame.lua:117`, `:137`).
    r"Interface\FrameXML\MainMenuBarMicroButtons.xml",
    "Interface\\FrameXML\\Cooldown.xml",
    "Interface\\FrameXML\\ActionButtonTemplate.xml",
    "Interface\\FrameXML\\ActionBarFrame.xml",
    // The chat edit box: the shift arm tests `ChatFrameEditBox:IsShown()`
    // (`ContainerFrame.lua:568`) to choose between linking the item and splitting the stack.
    "Interface\\FrameXML\\ChatFrame.xml",
    "Interface\\FrameXML\\FloatingChatFrame.xml",
    "Interface\\FrameXML\\ItemButtonTemplate.xml",
    // `BagSlotButtonTemplate` inherits `PaperDollItemSlotButtonTemplate`, and its OnLoad
    // (`PaperDollItemSlotButton_OnLoad`) gives each bag button its inventory-slot id, 20..23.
    // `CharacterFrame.xml` stays out: a missing `parent=` only warns.
    "Interface\\FrameXML\\PaperDollFrame.xml",
    // The stock bag bar, with `BagSlotButtonTemplate` and `KEYRING_CONTAINER`.
    "Interface\\FrameXML\\MainMenuBarBagButtons.xml",
    "Interface\\FrameXML\\ContainerFrame.xml",
    // `updateContainerFrameAnchors` measures every open bag against `BankFrame:GetRight()`
    // (`ContainerFrame.lua:505`) on every open and close, so the bank window is a hard dependency.
    "Interface\\FrameXML\\BankFrame.xml",
    "Interface\\FrameXML\\BonusActionBarFrame.xml",
    // Our adapters over the stock container files, loaded after them and the bar so they win.
];

/// The name of the `ContainerFrame` showing bag `id`, asked of `IsBagOpen`: the reference recycles
/// twelve windows across every container (`ContainerFrame_GetOpenFrame`).
#[cfg(test)]
pub(super) fn bag_window(s: &UiScript, id: i64) -> Option<String> {
    s.eval::<Option<i64>>(&format!("return IsBagOpen({id})"))
        .unwrap()
        .map(|i| format!("ContainerFrame{i}"))
}

/// Is bag `id` open?
#[cfg(test)]
pub(super) fn bag_open(s: &UiScript, id: i64) -> bool {
    bag_window(s, id).is_some()
}

/// The item button in bag `id`'s open window that holds game slot `slot`, found by `GetID`:
/// `ContainerFrame_GenerateFrame` numbers the buttons backwards (`index = size - j + 1`).
#[cfg(test)]
pub(super) fn bag_slot_button(s: &mut UiScript, id: i64, slot: u32) -> String {
    let w = bag_window(s, id).unwrap_or_else(|| panic!("bag {id} is not open"));
    s.eval::<String>(&format!(
        "for j = 1, MAX_CONTAINER_ITEMS do \
           local b = getglobal(\"{w}Item\"..j) \
           if b and b:IsShown() and b:GetID() == {slot} then return \"{w}Item\"..j end \
         end return \"\""
    ))
    .inspect(|n| assert!(!n.is_empty(), "no {w}Item* is bag {id} slot {slot}"))
    .expect("the item-button scan")
}

/// The centre of a named frame, in the y-up UI space `mouse_move`/`mouse_button` take.
#[cfg(test)]
pub(super) fn centre_of(s: &mut UiScript, name: &str) -> (f32, f32) {
    s.resolve();
    let r: Vec<f32> = s
        .eval(&format!(
            "local f = getglobal(\"{name}\") \
             return {{ f:GetLeft(), f:GetBottom(), f:GetWidth(), f:GetHeight() }}"
        ))
        .unwrap_or_else(|e| panic!("{name}: no resolved rect: {e}"));
    assert_eq!(r.len(), 4, "{name}: unresolved rect {r:?}");
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

/// Move the mouse onto the centre of `name` through the engine's hit test and `OnEnter`: stock
/// handlers read `this`, which only the engine sets.
#[cfg(test)]
pub(super) fn hover(s: &mut UiScript, name: &str) {
    let (x, y) = centre_of(s, name);
    s.mouse_move(x, y);
}

/// Move the mouse well clear of everything: the `OnLeave` half of [`hover`].
#[cfg(test)]
pub(super) fn unhover(s: &mut UiScript) {
    s.mouse_move(-500.0, -500.0);
}

/// Press and release `button` over the centre of `name`, the way a player's mouse does.
#[cfg(test)]
pub(super) fn click(s: &mut UiScript, name: &str, button: &str) {
    let (x, y) = centre_of(s, name);
    s.mouse_move(x, y);
    s.mouse_button(x, y, button, true);
    s.mouse_button(x, y, button, false);
}

/// Stock `WorldFrame.xml` and `MirrorTimer.xml`: `WorldFrame_OnUpdate` loops to
/// `MIRRORTIMER_NUMTIMERS`, which `MirrorTimer.lua` declares, and raises every tick without it.
/// Its other loop reads `STATICPOPUP_NUMDIALOGS`, which every caller already loads.
#[cfg(test)]
pub(super) const WORLD_FRAME_UI: &[&str] = &[
    r"Interface\FrameXML\WorldFrame.xml",
    r"Interface\FrameXML\MirrorTimer.xml",
];

/// A completed left click on the game world, at a point the loaded `WorldFrame` owns: searched on
/// a grid, since a harness's windows may cover any fixed point.
#[cfg(test)]
pub(super) fn world_click(s: &mut UiScript) {
    let (x, y) = world_point(s);
    s.mouse_move(x, y);
    s.mouse_button(x, y, "LeftButton", true);
    assert!(
        s.mouse_button(x, y, "LeftButton", false),
        "a world drop consumes the completed click"
    );
}

/// [`world_click`]'s search.
#[cfg(test)]
fn world_point(s: &mut UiScript) -> (f32, f32) {
    s.resolve();
    let r: Vec<f32> = s
        .eval(
            "local f = WorldFrame \
             return { f:GetLeft(), f:GetBottom(), f:GetWidth(), f:GetHeight() }",
        )
        .expect(
            "the fixture must load Interface\\FrameXML\\WorldFrame.xml (load_world_frame): a \
             world drop is a click on THAT frame, not on nothing",
        );
    assert_eq!(r.len(), 4, "WorldFrame: unresolved rect {r:?}");
    let (steps, mut blockers) = (7, Vec::new());
    for row in 0..steps {
        for col in 0..steps {
            let x = r[0] + r[2] * (col as f32 + 0.5) / steps as f32;
            let y = r[1] + r[3] * (row as f32 + 0.5) / steps as f32;
            match s.hit_test(x, y) {
                Some(id) if s.is_world_frame(id) => return (x, y),
                _ => blockers.push(
                    s.hit_test_name(x, y)
                        .unwrap_or_else(|| "<anonymous>".into()),
                ),
            }
        }
    }
    blockers.sort();
    blockers.dedup();
    panic!("no point on the world frame is clickable — the UI covers all {steps}x{steps} of them: {blockers:?}")
}

/// Seat one of the reference's LoadOnDemand Blizzard addons off the chain, so a harness's
/// `UIParentLoadAddOn(name)` loads it as the app does. Needs client data.
#[cfg(test)]
pub(crate) fn seat_chain_addon(s: &mut UiScript, name: &str) {
    let toc = super::reference_ui::read(&format!("Interface/AddOns/{name}/{name}.toc"))
        .map(|b| wrathplus_ui::toc::Toc::parse(&wrathplus_ui::source::decode(&b)))
        .unwrap_or_else(|| panic!("{name}: no toc off the chain"));
    let mut info = super::addons::info_from_toc(name, &toc);
    info.chain = true;
    s.set_addon_chain_reader(Box::new(super::reference_ui::read));
    s.register_addons(vec![info], None, None, None);
}

/// The stock unit frames' event-driven bars: build 12340 registers `predictedHealth` and
/// `predictedPower` "1", whose bars poll on `OnUpdate` instead (`UnitFrame.lua:211`, `:327`). Run
/// before the frames load, which read them there.
#[cfg(test)]
pub(crate) fn event_driven_bars(s: &UiScript) {
    s.run("SetCVar('predictedHealth', '0') SetCVar('predictedPower', '0')")
        .unwrap();
}

/// The loot window at its left UI-panel slot: build 12340 registers `lootUnderMouse` "1", which
/// opens it at the cursor (`LootFrame_Show`).
#[cfg(test)]
pub(crate) fn loot_at_left_slot(s: &UiScript) {
    s.run("SetCVar('lootUnderMouse', '0')").unwrap();
}

#[cfg(test)]
pub(crate) fn kits(space_separated: &str) -> Vec<wrathplus_ui::script::SoundRequest> {
    let kit = |n: &str| wrathplus_ui::script::SoundRequest::KitName(n.into());
    space_separated.split(' ').map(kit).collect()
}

#[cfg(test)]
mod tests {
    /// The merged kit runs toc rows in the toc's order, each once.
    #[test]
    fn a_merged_kit_takes_the_production_order() {
        let _data = wrathplus_formats::wow_data_or_skip!();
        assert_eq!(
            super::production_order(&[
                &[r"Interface\FrameXML\UIParent.xml"],
                &[
                    r"Interface\FrameXML\GlobalStrings.lua",
                    r"Interface\FrameXML\UIParent.xml",
                ],
            ]),
            [
                r"Interface\FrameXML\GlobalStrings.lua",
                r"Interface\FrameXML\UIParent.xml",
            ]
        );
    }
}
