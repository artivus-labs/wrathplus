//! The experience bar at login, relog and `/reload`: stock shows it only from
//! `ReputationWatchBar_Update` (`ReputationFrame.lua:323-413`), which the watch bar runs on
//! `UPDATE_FACTION`; our reputation feed fires that once per VM, on its first push.

use bevy::prelude::*;
use wrathplus_ui::script::UiScript;

/// A stock session as world entry builds it, the player at level 10 with 2500 of 7600 xp.
fn session(data: &std::path::Path) -> UiScript {
    let (mut s, _failures) = super::super::stock_session(data);
    s.clear_instruction_budget();
    let me = super::super::stock_player(10);
    s.set_unit_guids(&wrathplus_ui::script::UnitGuids {
        player: me.guid,
        ..Default::default()
    });
    s.set_unit("player", Some(me));
    s.set_player_xp(2_500, 7_600);
    s.fire_event("PLAYER_XP_UPDATE", vec![]);
    s
}

fn exp_bar(app: &App) -> String {
    app.world()
        .non_send_resource::<UiScript>()
        .eval(
            "local lo, hi = MainMenuExpBar:GetMinMaxValues() \
             return tostring(MainMenuExpBar:IsShown())..' '..lo..'/'..hi..' '..MainMenuExpBar:GetValue()",
        )
        .unwrap()
}

/// The real feed over a loaded VM: the in-world VM is minted and loaded in one exclusive pass
/// (`lifecycle::load_ingame_ui_on_world_entry`) with the boot VM parked meanwhile, so the feed's
/// first push meets the loaded watch bar. Each new VM (a relog, a `/reload`) gets its own first
/// push, its memo keyed on the VM's session; each shows the bar with the player's values.
#[test]
fn login_relog_and_reload_each_show_the_exp_bar() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let catalog = wrathplus_formats::load_faction_catalog(&mut chain).expect("factions");
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(crate::net::Reputations::default())
        .insert_resource(crate::app_core::state::Factions(catalog))
        .add_systems(Update, crate::ui_reputation::feed_reputation);
    app.world_mut().spawn((
        crate::app_core::session::ObjectStore(Default::default()),
        crate::app_core::session::SelfPlayer,
    ));
    for round in ["login", "relog", "reload"] {
        app.insert_non_send_resource(session(&data));
        app.update();
        assert_eq!(exp_bar(&app), "1 0/7600 2500", "{round}");
        let errors = app.world().non_send_resource::<UiScript>().errors();
        assert!(errors.is_empty(), "{round}: {errors:?}");
    }
}

/// Stock Wrath MainMenuExpBar_Update reads the descriptor XP pair; roster and name-cache
/// pushes must neither hide its strip nor leave the extracted fill at the previous fraction.
#[test]
fn real_unit_and_party_feeds_preserve_and_update_the_xp_strip() {
    use crate::app_core::{
        group::GroupState,
        session::{ObjectStore, SelfPlayer},
        state::{Factions, Selection},
    };
    use crate::names::NameCache;
    use crate::net::{Guid, GuidIndex, NetCommands, Reputations};
    use wrathplus_protocol::types::{member_status, Field, GroupMemberEntry, ObjectFields};
    use wrathplus_ui::script::QuadContent;

    let data = wrathplus_formats::wow_data_or_skip!();
    let (script, failures) = super::super::stock_session(&data);
    assert!(failures.is_empty(), "load failures: {failures:?}");
    script.clear_instruction_budget();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let catalog = wrathplus_formats::load_faction_catalog(&mut chain).expect("factions");
    let (tx, _rx) = crossbeam_channel::unbounded();
    // Same minimal world inputs as ui_unit's real-Update fixture; the party plugin supplies
    // its production feed rather than synthesizing PARTY_MEMBERS_CHANGED in Lua.
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Selection>()
        .init_resource::<NameCache>()
        .init_resource::<GuidIndex>()
        .init_resource::<Reputations>()
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::guild::GuildState>()
        .insert_resource(NetCommands(tx))
        .insert_resource(Factions(catalog))
        .insert_non_send_resource(script)
        .add_plugins(crate::ui_party::UiPartyPlugin)
        .add_plugins(crate::ui_reputation::UiReputationPlugin);
    wrathplus_world::world_point::init_world_point_resources(app.world_mut());
    crate::ui_unit::add_unit_feed(&mut app);
    const ME: u64 = 1;
    const MATE: u64 = 2;
    let fields = |xp, next| {
        ObjectFields::from_pairs(&[
            (Field::UnitLevel.index(), 10),
            (Field::UnitBytes0.index(), 1 | (1 << 8)),
            (Field::UnitHealth.index(), 60),
            (Field::UnitMaxHealth.index(), 60),
            (Field::UnitFactionTemplate.index(), 1),
            (Field::PlayerBytes2.index(), 2 << 24),
            (Field::PlayerXp.index(), xp),
            (Field::PlayerNextLevelXp.index(), next),
        ])
    };
    let me = app
        .world_mut()
        .spawn((SelfPlayer, Guid(ME), ObjectStore(fields(2500, 7600))))
        .id();
    app.world_mut().resource_mut::<GuidIndex>().0.insert(ME, me);
    // The guid table the aura feed seats beside the unit feed's snapshots; this app runs no aura
    // feed.
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .set_unit_guids(&wrathplus_ui::script::UnitGuids {
            player: ME,
            ..Default::default()
        });
    app.world_mut().resource_mut::<NameCache>().insert_player(
        ME,
        "Benchone".into(),
        Some((1, 1, 0)),
    );

    let frame = |app: &mut App, xp, next| {
        app.update();
        let mut s = app.world_mut().non_send_resource_mut::<UiScript>();
        s.tick(0.016);
        s.resolve();
        assert!(s.errors().is_empty(), "Lua errors: {:?}", s.errors());
        let state: (bool, f64, f64, f64) = s.eval("local lo,hi=MainMenuExpBar:GetMinMaxValues() return MainMenuExpBar:IsVisible() and true or false,lo,hi,MainMenuExpBar:GetValue()").unwrap();
        assert_eq!(state, (true, 0.0, next as f64, xp as f64));
        let fill = s.extract().into_iter().find(|q| {
            s.quad_owner_name(q.target).as_deref() == Some("MainMenuExpBar")
                && matches!(&q.content, QuadContent::Texture { path: Some(p), .. } if p.ends_with("UI-StatusBar"))
        }).expect("XP fill must emit render geometry");
        let rect = fill.rect.expect("XP fill must have resolved geometry");
        let width: f64 = s.eval("return MainMenuExpBar:GetWidth()").unwrap();
        let expected = width * f64::from(xp) / f64::from(next);
        assert!(
            (f64::from(rect.right - rect.left) - expected).abs() < 0.01,
            "extracted fill width must match XP/range: {rect:?}, expected {expected}"
        );
        assert!(fill.alpha > 0.0, "XP fill must not be transparent");
        fill
    };
    // Let the stock MainMenuBar slide-in OnUpdate finish before comparing positions.
    for _ in 0..120 {
        frame(&mut app, 2500, 7600);
    }
    let baseline = frame(&mut app, 2500, 7600);
    assert!(
        baseline.rect.unwrap().bottom >= 0.0,
        "settled XP fill must be on screen"
    );
    for cycle in 0..3 {
        app.world_mut().resource_mut::<GroupState>().apply_list(
            0,
            0,
            vec![GroupMemberEntry {
                name: "PartyMate".into(),
                guid: MATE,
                status: member_status::ONLINE,
                flags: 0,
            }],
            ME,
            None,
            Some(ME),
        );
        assert_eq!(frame(&mut app, 2500, 7600), baseline, "join {cycle}");
        // Stream the member before its name reply, then land the reply on a separate frame.
        let mate = app
            .world_mut()
            .spawn((Guid(MATE), ObjectStore(fields(0, 7600))))
            .id();
        app.world_mut()
            .resource_mut::<GuidIndex>()
            .0
            .insert(MATE, mate);
        assert_eq!(frame(&mut app, 2500, 7600), baseline, "stream {cycle}");
        app.world_mut().resource_mut::<NameCache>().insert_player(
            MATE,
            "PartyMate".into(),
            Some((1, 1, 0)),
        );
        assert_eq!(frame(&mut app, 2500, 7600), baseline, "name reply {cycle}");
        let name: String = app
            .world()
            .non_send_resource::<UiScript>()
            .eval("return UnitName('party1')")
            .unwrap();
        assert_eq!(name, "PartyMate");
        app.world_mut().resource_mut::<GroupState>().leave_group();
        app.world_mut().resource_mut::<GuidIndex>().0.remove(&MATE);
        app.world_mut().despawn(mate);
        assert_eq!(frame(&mut app, 2500, 7600), baseline, "leave {cycle}");
    }
    app.world_mut()
        .entity_mut(me)
        .get_mut::<ObjectStore>()
        .unwrap()
        .0
        .merge(fields(3500, 7600));
    let updated = frame(&mut app, 3500, 7600);
    assert_ne!(
        updated.rect, baseline.rect,
        "XP field update must change the extracted fill"
    );
    assert_eq!(frame(&mut app, 3500, 7600), updated, "steady frame");
    // A roster update and XP update land together, exercising both feed gates in one Update.
    app.world_mut().resource_mut::<GroupState>().apply_list(
        0,
        0,
        vec![GroupMemberEntry {
            name: "PartyMate".into(),
            guid: MATE,
            status: member_status::ONLINE,
            flags: 0,
        }],
        ME,
        None,
        Some(ME),
    );
    // Range-only changes exercise SetMinMaxValues even when SetValue has no value edge.
    app.world_mut()
        .entity_mut(me)
        .get_mut::<ObjectStore>()
        .unwrap()
        .0
        .merge(fields(3500, 9000));
    let ranged = frame(&mut app, 3500, 9000);
    assert_ne!(
        ranged.rect, updated.rect,
        "range update must change the extracted fill"
    );
}
