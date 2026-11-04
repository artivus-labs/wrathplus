//! The stock group-loot roll popups (`GroupLootFrame1..4` in `LootFrame.xml`), driven by a pushed
//! [`LootRollsState`] and `START_LOOT_ROLL`/`CANCEL_LOOT_ROLL`.

use wrathplus_ui::script::{
    DressUpIntent, ExtractedQuad, QuadContent, ScriptValue, UiScript, WrathLootRollEntry,
    WrathLootRollsState,
};

use super::test_ui::{load_ui as load_xml, load_ui_no_warnings as load_xml_no_warnings};

/// The centre of the first texture quad whose path contains `needle`, for clicking it.
fn quad_center(quads: &[ExtractedQuad], needle: &str) -> (f32, f32) {
    let r = quads
        .iter()
        .find(|q| {
            matches!(&q.content, QuadContent::Texture { path: Some(p), .. } if p.contains(needle))
        })
        .and_then(|q| q.rect)
        .unwrap_or_else(|| panic!("no quad for {needle}"));
    ((r.left + r.right) * 0.5, (r.bottom + r.top) * 0.5)
}

/// The colour of the first text quad reading `t`.
fn text_color(quads: &[ExtractedQuad], t: &str) -> Option<[f32; 4]> {
    quads.iter().find_map(|q| match &q.content {
        QuadContent::Text {
            text: Some(x),
            color,
            ..
        } if x == t => Some(*color),
        _ => None,
    })?
}

/// The four popups, declared in stock `LootFrame.xml`, which a kit loads after `UIParent.xml`,
/// whose `UIParent_OnEvent` routes `START_LOOT_ROLL` (`UIParent.lua:513-516`).
const GROUP_LOOT: &[&str] = &[
    r"Interface\FrameXML\UIDropDownMenu.xml",
    // `GroupLootDropDown`'s OnLoad runs its initializer at once, which reads `MAX_PARTY_MEMBERS`
    // (`LootFrame.lua:217`), a constant of `PartyMemberFrame.lua`.
    r"Interface\FrameXML\PartyMemberFrame.lua",
    r"Interface\FrameXML\LootFrame.xml",
];

/// The bars the popups stack over and the kits they reach for.
const BARS: &[&str] = &[
    // The loot window's labels (`ITEMS`, `PREV`, `NEXT`) are GlobalStrings keys; a missing one is
    // a loader warning, which `load_ui_no_warnings` fails.
    r"Interface\FrameXML\GlobalStrings.lua",
    "Interface\\FrameXML\\Fonts.xml",
    r"Interface\FrameXML\BasicControls.xml",
    r"Interface\FrameXML\UIParent.xml",
    r"Interface\FrameXML\MoneyFrame.lua",
    r"Interface\FrameXML\MoneyFrame.xml",
    "Interface\\FrameXML\\GameTooltip.xml", // the vote and item hovers
    // `UIPanelCloseButton`, which each popup's pass button inherits (`LootFrame.xml:364`).
    r"Interface\FrameXML\UIPanelTemplates.lua",
    r"Interface\FrameXML\UIPanelTemplates.xml",
    r"Interface\FrameXML\StaticPopup.xml",
    "Interface\\FrameXML\\TextStatusBar.lua",
    "Interface\\FrameXML\\TextStatusBar.xml",
    "Interface\\FrameXML\\MainMenuBar.xml",
    "Interface\\FrameXML\\Cooldown.xml",
    "Interface\\FrameXML\\ActionButtonTemplate.xml",
    "Interface\\FrameXML\\ActionBarFrame.xml",
    r"Interface\FrameXML\ItemButtonTemplate.xml",
    "Interface\\FrameXML\\BonusActionBarFrame.xml",
];

/// The bars and the popups, in the production order; returns the frames `LootFrame.xml` built,
/// with no loader warning of any kind.
fn setup_counted(extra: &[&str]) -> (UiScript, usize) {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    let mut built = 0;
    for file in super::test_ui::production_order(&[BARS, GROUP_LOOT, extra]) {
        if file.ends_with("LootFrame.xml") {
            built = load_xml_no_warnings(&mut s, file);
        } else {
            load_xml(&mut s, file);
        }
    }
    (s, built)
}

pub(super) fn setup() -> UiScript {
    setup_counted(&[]).0
}

/// The resolved rolls' links as `ui_loot_roll` builds them: quality colour, four-field `|Hitem:`.
const STAFF_LINK: &str = "|cffa335ee|Hitem:17182:0:0:0|h[Staff of Jordan]|h|r";
const SWORD_LINK: &str = "|cffffffff|Hitem:25:0:0:0|h[Worn Shortsword]|h|r";

/// One resolved 3.3.5 roll every vote may take (`vote_mask` pass/need/greed/disenchant, reasons
/// all eligible).
pub(super) fn roll(
    roll_id: u32,
    entry: u32,
    name: &str,
    icon: &str,
    quality: u32,
    bind_on_pickup: bool,
    link: &str,
) -> WrathLootRollEntry {
    WrathLootRollEntry {
        roll_id,
        entry,
        count: 1,
        vote_mask: 0b1111,
        texture: Some(format!("Interface\\Icons\\{icon}")),
        name: Some(name.into()),
        quality: Some(quality),
        bind_on_pickup,
        time_left_ms: 60_000,
        link: Some(link.into()),
        reasons: [Some(0); 3],
        disenchant_skill_required: Some(0),
        ..Default::default()
    }
}

/// Roll 7 resolved Epic and bind-on-pickup, roll 8 resolved Common, roll 9 with no template yet.
fn rolls() -> WrathLootRollsState {
    WrathLootRollsState {
        rolls: vec![
            roll(
                7,
                17182,
                "Staff of Jordan",
                "INV_Staff_12",
                4,
                true,
                STAFF_LINK,
            ),
            roll(
                8,
                25,
                "Worn Shortsword",
                "INV_Sword_04",
                1,
                false,
                SWORD_LINK,
            ),
            // No template yet: `GetLootRollItemInfo` answers nothing (loot_roll/wrath.rs).
            WrathLootRollEntry {
                roll_id: 9,
                entry: 4306,
                count: 1,
                vote_mask: 0b1111,
                ..Default::default()
            },
        ],
    }
}

/// The popups load with no loader warning of any kind and start hidden.
#[test]
fn shipped_group_loot_frame_loads_clean_and_starts_hidden() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let (s, built) = setup_counted(&[]);
    // The kit seats the whole session on its first file, so the count says nothing; the popups are
    // found by name and type instead.
    let _ = built;
    for i in 1..=4 {
        assert_eq!(
            s.eval::<String>(&format!("return GroupLootFrame{i}:GetObjectType()"))
                .unwrap(),
            "Frame"
        );
    }

    for i in 1..=4 {
        let name = format!("GroupLootFrame{i}");
        assert!(
            !s.eval::<bool>(&format!("return {name}:IsVisible()"))
                .unwrap(),
            "{name} starts hidden"
        );
    }
}

/// `START_LOOT_ROLL` claims the first free popup (`LootFrame.lua:246-258`) and paints the roll:
/// name, quality colour, bind-on-pickup decoration and timer range; Need clicks through.
#[test]
fn start_loot_roll_claims_frames_in_order_and_paints_the_roll() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    s.set_wrath_loot_rolls(rolls());

    s.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(7), ScriptValue::Int(42_000)],
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(s
        .eval::<bool>("return GroupLootFrame1:IsVisible()")
        .unwrap());
    assert_eq!(s.eval::<i64>("return GroupLootFrame1.rollID").unwrap(), 7);
    // The timer's range is set before `Show()` (`LootFrame.lua:253`).
    assert_eq!(
        s.eval::<(f64, f64)>("return GroupLootFrame1Timer:GetMinMaxValues()")
            .unwrap(),
        (0.0, 42_000.0)
    );
    // Bind-on-pickup shows the gold decoration (`LootFrame.lua:263-266`).
    assert!(s
        .eval::<bool>("return GroupLootFrame1Decoration:IsShown()")
        .unwrap());

    s.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(8), ScriptValue::Int(60_000)],
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(s
        .eval::<bool>("return GroupLootFrame1:IsVisible()")
        .unwrap());
    assert!(s
        .eval::<bool>("return GroupLootFrame2:IsVisible()")
        .unwrap());
    assert_eq!(s.eval::<i64>("return GroupLootFrame2.rollID").unwrap(), 8);
    assert!(!s
        .eval::<bool>("return GroupLootFrame2Decoration:IsShown()")
        .unwrap());

    s.resolve();
    let quads = s.extract();
    assert_eq!(
        s.eval::<String>("return GroupLootFrame1Name:GetText()")
            .unwrap(),
        "Staff of Jordan"
    );
    let purple = text_color(&quads, "Staff of Jordan").expect("Staff of Jordan colour");
    assert!(
        (purple[0] - 0.64).abs() < 0.02 && (purple[1] - 0.21).abs() < 0.02,
        "Epic item text is purple, got {purple:?}"
    );
    let white = text_color(&quads, "Worn Shortsword").expect("Worn Shortsword colour");
    assert!(
        (white[0] - 1.0).abs() < 0.02 && (white[1] - 1.0).abs() < 0.02,
        "Common item text is white, got {white:?}"
    );

    // Found by name: the popups are `toplevel` (`LootFrame.xml:243`), so draw order is not
    // declaration order. Roll 7 is bind-on-pickup, so the vote waits for the confirm.
    let (nx, ny) = s
        .eval::<(f64, f64)>(
            "return (GroupLootFrame1RollButton:GetLeft() + GroupLootFrame1RollButton:GetRight()) \
             / 2, (GroupLootFrame1RollButton:GetBottom() + GroupLootFrame1RollButton:GetTop()) / 2",
        )
        .unwrap();
    let (nx, ny) = (nx as f32, ny as f32);
    assert_eq!(
        s.hit_test_name(nx, ny).as_deref(),
        Some("GroupLootFrame1RollButton"),
        "the click has to land on frame 1's Need button for the rest of this to mean anything"
    );
    s.mouse_button(nx, ny, "LeftButton", true);
    s.mouse_button(nx, ny, "LeftButton", false);
    assert!(
        s.take_loot_roll_votes().is_empty(),
        "a BoP Need must not reach the wire before the confirm"
    );
    assert_eq!(s.take_loot_roll_confirms(), vec![(7, 1)], "Need on roll 7");
}

/// `CONFIRM_LOOT_ROLL` opens the dialog carrying `(rollID, rollType)` (`UIParent.lua:517-523`);
/// its OK calls `ConfirmLootRoll` (`StaticPopup.lua:1375-1377`), which sends the withheld vote.
#[test]
fn the_bop_confirm_popup_lands_the_withheld_vote() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    s.set_wrath_loot_rolls(rolls());

    // The app fires this after draining the confirm queue (`ui_loot_roll::drain_loot_rolls`).
    s.fire_event(
        "CONFIRM_LOOT_ROLL",
        vec![ScriptValue::Int(7), ScriptValue::Int(1)],
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());

    // `data2` is the rollType, so `OnAccept` can replay the click.
    assert!(
        s.eval::<bool>("return StaticPopup_FindVisible(\"CONFIRM_LOOT_ROLL\") ~= nil")
            .unwrap(),
        "the confirm popup should be visible"
    );
    let (data, data2) = s
        .eval::<(i64, i64)>(
            "local d = StaticPopup_FindVisible(\"CONFIRM_LOOT_ROLL\")\nreturn d.data, d.data2",
        )
        .unwrap();
    assert_eq!((data, data2), (7, 1), "rollID and rollType ride the dialog");

    // 3.3.5's `LOOT_NO_DROP`, filled with the quality-coloured name (UIParent.lua:846), and `OKAY`.
    let dialog = s
        .eval::<String>("return StaticPopup_FindVisible(\"CONFIRM_LOOT_ROLL\"):GetName()")
        .unwrap();
    assert_eq!(
        s.eval::<String>(&format!("return {dialog}Text:GetText()"))
            .unwrap(),
        "Looting |cffa335eeStaff of Jordan|r will bind it to you."
    );
    assert_eq!(
        s.eval::<String>(&format!("return {dialog}Button1:GetText()"))
            .unwrap(),
        "Okay"
    );

    assert!(s.take_loot_roll_votes().is_empty());

    s.eval::<bool>(
        "local d = StaticPopup_FindVisible(\"CONFIRM_LOOT_ROLL\")\n\
         _G[d:GetName() .. \"Button1\"]:Click()\nreturn true",
    )
    .unwrap();
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert_eq!(
        s.take_loot_roll_votes(),
        vec![(7, 1)],
        "OK on the confirm sends the Need the gate withheld"
    );
}

/// `CANCEL_LOOT_ROLL` hides the popup holding that rollID and no other (`LootFrame.lua:279-285`).
#[test]
fn cancel_loot_roll_hides_only_that_frame() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    s.set_wrath_loot_rolls(rolls());

    s.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(7), ScriptValue::Int(42_000)],
    );
    s.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(8), ScriptValue::Int(60_000)],
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());

    s.fire_event("CANCEL_LOOT_ROLL", vec![ScriptValue::Int(7)]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(!s
        .eval::<bool>("return GroupLootFrame1:IsVisible()")
        .unwrap());
    assert!(s
        .eval::<bool>("return GroupLootFrame2:IsVisible()")
        .unwrap());

    // A rollID no popup holds is a no-op.
    s.fire_event("CANCEL_LOOT_ROLL", vec![ScriptValue::Int(7)]);
    s.fire_event("CANCEL_LOOT_ROLL", vec![ScriptValue::Int(999)]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(s
        .eval::<bool>("return GroupLootFrame2:IsVisible()")
        .unwrap());

    s.fire_event("CANCEL_LOOT_ROLL", vec![ScriptValue::Int(8)]);
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    assert!(!s
        .eval::<bool>("return GroupLootFrame2:IsVisible()")
        .unwrap());
}

/// A roll whose item is unresolved, or unknown, shows nothing: `GroupLootFrame_OnShow` hides a
/// popup whose `GetLootRollItemInfo` name is nil before reading anything else (LootFrame.lua:339-345).
#[test]
fn an_unresolved_roll_hides_its_popup() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = setup();
    s.set_wrath_loot_rolls(rolls());
    for roll in [9, 99] {
        s.fire_event(
            "START_LOOT_ROLL",
            vec![ScriptValue::Int(roll), ScriptValue::Int(55_000)],
        );
        assert!(s.errors().is_empty(), "roll {roll}: {:?}", s.errors());
    }
    assert!(!s
        .eval::<bool>("return GroupLootFrame1:IsVisible() or GroupLootFrame2:IsVisible()")
        .unwrap());
}

/// `UIPARENT_MANAGED_FRAME_POSITIONS["GroupLootFrame1"]` (baseY true, bottomEither
/// `actionBarOffset`; `UIParent.lua:1183`) stacks the popups over the bottom bars; the pass finds
/// the frame by name.
#[test]
fn managed_positions_engage_for_the_bare_frame_name() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    // `GameTooltip.xml` brings `TOOLTIP_DEFAULT_COLOR`, which the dropdown lists read in their
    // OnLoad (`UIDropDownMenuTemplates.xml:186`).
    for file in super::test_ui::production_order(&[
        &[
            r"Interface\FrameXML\GlobalStrings.lua",
            "Interface\\FrameXML\\Fonts.xml",
            r"Interface\FrameXML\BasicControls.xml",
            r"Interface\FrameXML\UIParent.xml",
            r"Interface\FrameXML\MoneyFrame.lua",
            r"Interface\FrameXML\MoneyFrame.xml",
            "Interface\\FrameXML\\GameTooltip.xml",
            r"Interface\FrameXML\UIPanelTemplates.lua",
            r"Interface\FrameXML\UIPanelTemplates.xml",
            r"Interface\FrameXML\StaticPopup.xml",
        ],
        GROUP_LOOT,
    ]) {
        load_xml(&mut s, file);
    }

    let bottom = |s: &UiScript| s.eval::<f64>("return GroupLootFrame1:GetBottom()").unwrap();

    // No bars: `baseY = true` is `menuBarTop`, 55 (UIParent.lua:1164,1183,1245-1247).
    s.run("UIParent_ManageFramePositions()").unwrap();
    s.resolve();
    assert_eq!(bottom(&s), 55.0, "baseY");

    // A shown bottom bar adds `bottomEither`, `actionBarOffset` 45; the pass reads the bars
    // themselves (UIParent.lua:1163,1761-1763).
    s.run("MultiBarBottomLeft:Show() UIParent_ManageFramePositions()")
        .unwrap();
    s.resolve();
    assert_eq!(bottom(&s), 100.0, "55 + bottomEither 45 — the row engaged");
}

/// The roll icon's OnClick (`LootFrame.xml:353-361`): ctrl dresses up the item and shift posts
/// its link to an open chat box, both through `GetLootRollItemLink`; the icon never votes.
#[test]
fn ctrl_and_shift_on_the_roll_icon_preview_and_post_its_link() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let (mut s, _) = setup_counted(&[
        "Interface\\FrameXML\\UIMenu.xml", // the kit the chat menus build from
        "Interface\\FrameXML\\ChatFrame.xml",
        "Interface\\FrameXML\\FloatingChatFrame.xml",
        "Interface\\FrameXML\\DressUpFrame.xml",
    ]);
    s.set_wrath_loot_rolls(rolls());
    // The rolled item's template, which `IsDressableItem` reads (DressUpFrame.lua:2-5).
    s.set_item_template(
        25,
        wrathplus_ui::script::ItemTemplateView {
            name: "Worn Shortsword".into(),
            inventory_type: 13,
            icon: Some("Interface\\Icons\\INV_Sword_04".into()),
            ..Default::default()
        },
    );

    // Roll 8 is not bind-on-pickup, so the dice below vote without a confirm.
    s.fire_event(
        "START_LOOT_ROLL",
        vec![ScriptValue::Int(8), ScriptValue::Int(60_000)],
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
    s.resolve();
    let quads = s.extract();
    let (x, y) = quad_center(&quads, "INV_Sword_04");

    // A plain click does nothing: the handler has only the ctrl and shift arms.
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
    assert!(
        s.take_loot_roll_votes().is_empty() && s.take_loot_roll_confirms().is_empty(),
        "an unmodified icon click votes nothing"
    );

    // An open chat box, which `ChatEdit_InsertLink` writes into (ChatFrame.lua).
    s.run("ChatEdit_ActivateChat(ChatFrame1EditBox)").unwrap();
    s.set_modifiers(true, false, false);
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
    s.set_modifiers(false, false, false);
    assert_eq!(
        s.eval::<String>("return ChatEdit_GetActiveWindow():GetText()")
            .unwrap(),
        format!(" {SWORD_LINK}"),
        "the rolled item's link, after `ChatEdit_InsertLink`'s parsing space (ChatFrame.lua:3492-3493)"
    );

    s.set_modifiers(false, true, false);
    s.mouse_button(x, y, "LeftButton", true);
    s.mouse_button(x, y, "LeftButton", false);
    s.set_modifiers(false, false, false);
    assert_eq!(
        s.take_dressup_intents(),
        vec![DressUpIntent::Dress, DressUpIntent::TryOn(25)],
        "re-dress first, then try the rolled item on"
    );
    assert!(
        s.take_loot_roll_votes().is_empty() && s.take_loot_roll_confirms().is_empty(),
        "no modified icon click may vote"
    );

    let (nx, ny) = quad_center(&quads, "UI-GroupLoot-Dice-Up");
    s.mouse_button(nx, ny, "LeftButton", true);
    s.mouse_button(nx, ny, "LeftButton", false);
    assert_eq!(
        s.take_loot_roll_votes(),
        vec![(8, 1)],
        "the Need button still votes"
    );
    assert!(s.errors().is_empty(), "script errors: {:?}", s.errors());
}

/// Addons inherit `GroupLootFrameTemplate` by its 1.12 name and reach for its children; the icon
/// is `IconFrameIcon` because it is nested in the `IconFrame` button (`LootFrame.xml:314`).
#[test]
fn the_roll_template_carries_the_reference_name_and_the_parts_addons_reach_for() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let s = setup();

    // `CreateFrame` raises on an unknown template, so this call is the check.
    s.run(r#"Probe = CreateFrame("Frame", "ProbeRoll", nil, "GroupLootFrameTemplate")"#)
        .expect("the reference-named template must resolve");

    for part in [
        "IconFrame",
        "IconFrameIcon",
        "Name",
        "Corner",
        "Decoration",
        "Timer",
    ] {
        assert!(
            s.eval::<bool>(&format!(r#"return getglobal("ProbeRoll{part}") ~= nil"#))
                .unwrap(),
            "an addon inheriting the template must find ProbeRoll{part}"
        );
    }

    // The timer is a StatusBar (`LootFrame.lua:253` sets its range).
    s.run(r#"ProbeRollTimer:SetMinMaxValues(0, 60000)"#)
        .expect("the timer must answer StatusBar verbs");

    assert!(s
        .eval::<bool>(r#"return getglobal("WrathPlusGroupLootFrameTemplate") == nil"#)
        .unwrap());
}
