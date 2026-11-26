//! The achievement toast, whose template inherits from a demand-loaded addon:
//! `AchievementAlertFrameTemplate` (`AlertFrames.xml:195`, FrameXML) holds an icon frame that
//! inherits `AchievementIconFrameTemplate`, declared only by `Blizzard_AchievementUI`
//! (`Blizzard_AchievementUI.xml:730`). The template is virtual and is instantiated by
//! `CreateFrame` (`AlertFrames.lua:237`) after `AlertFrame_OnEvent` demand-loads the addon
//! (`AlertFrames.lua:12-14`), so inherits must resolve at instantiation, not at template parse.

use wrathplus_ui::script::{UiScript, UnitState, WrathAchievementProgress};

fn session(data: &std::path::Path) -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(1024.0, 768.0);
    s.set_unit(
        "player",
        Some(UnitState {
            exists: true,
            name: Some("Probefour".into()),
            level: 80,
            ..Default::default()
        }),
    );
    let mut chain = wrathplus_formats::open_chain(data).expect("open chain");
    let metadata = crate::ui_achievements::metadata_from(&mut chain).expect("achievement catalog");
    s.set_wrath_achievement_metadata(metadata);
    let failures = super::load_default_ui(&mut s);
    assert!(failures.is_empty(), "manifest load errors: {failures:#?}");
    super::test_ui::seat_chain_addon(&mut s, "Blizzard_AchievementUI");
    s.resolve();
    assert!(s.errors().is_empty(), "script errors: {:#?}", s.errors());
    s
}

#[test]
fn an_earned_achievement_demand_loads_the_ui_and_builds_the_toast_with_its_icon() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut s = session(&data);
    assert_eq!(
        s.eval::<Option<i64>>(r#"return IsAddOnLoaded("Blizzard_AchievementUI")"#)
            .unwrap(),
        None,
        "nothing demand-loaded before the earn"
    );
    assert!(s
        .eval::<bool>("return AchievementAlertFrame1 == nil")
        .unwrap());

    // Level 10 (6), as SMSG_ACHIEVEMENT_EARNED publishes it, then the event.
    s.set_wrath_achievement_progress(WrathAchievementProgress {
        earned: vec![(6, 0)],
        criteria: Default::default(),
        faction: 1,
        received: true,
    });
    s.wrath_achievement_earned(6);
    s.tick(0.016);
    s.resolve();

    assert!(s.errors().is_empty(), "script errors: {:#?}", s.errors());
    assert_eq!(
        s.eval::<i64>(r#"return IsAddOnLoaded("Blizzard_AchievementUI")"#)
            .unwrap(),
        1,
        "AlertFrame_OnEvent demand-loaded the addon"
    );
    let built: (bool, bool, bool, bool) = s
        .eval(
            r#"local f = AchievementAlertFrame1
               return f ~= nil, AchievementAlertFrame1Icon ~= nil,
                   AchievementAlertFrame1IconTexture ~= nil, f ~= nil and f:IsShown() == 1"#,
        )
        .unwrap();
    assert_eq!(
        built,
        (true, true, true, true),
        "the toast, its icon frame, the icon's inherited texture, shown"
    );
    assert_eq!(
        s.eval::<String>("return AchievementAlertFrame1Name:GetText()")
            .unwrap(),
        s.eval::<String>("return (select(2, GetAchievementInfo(6)))")
            .unwrap(),
        "the toast names the earned achievement"
    );
}
