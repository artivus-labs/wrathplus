//! The trainer that would not reopen (session 115138, Llane Beshere): AzerothCore's packets in its
//! order through the real handlers and drains, then each close the player has, and the
//! interaction latch the right-click's re-click gate reads.

use bevy::prelude::*;
use wrathplus_protocol::types::{GossipOption, NpcTextBlock};
use wrathplus_protocol::SessionEvent;
use wrathplus_ui::script::UiScript;

use crate::app_core::interaction::{GossipState, TrainerOpen};
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::{InteractNpc, QuestGiver};
use crate::net::{ClientCommand, GuidIndex, NetCommands};
use crate::target::click::interaction_already_open_on;
use crate::ui_session::close_npc_session_out_of_range;

/// Llane Beshere, the warrior trainer the live session could not reopen.
const LLANE: u64 = 0xF130_0003_8F00_40C6;
/// His menu text, a single trainer option with no quests.
const TEXT: u32 = 1216;

fn reopen_app() -> (App, crossbeam_channel::Receiver<ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx))
        .init_resource::<GossipState>()
        .init_resource::<QuestGiver>()
        .init_resource::<TrainerOpen>()
        .init_resource::<crate::ui_trainer::TrainerErrors>()
        .init_resource::<GuidIndex>()
        .init_resource::<InteractNpc>()
        .init_resource::<crate::names::NameCache>()
        .init_resource::<crate::app_core::state::WorldStates>()
        .insert_non_send_resource(UiScript::new().unwrap())
        .add_systems(
            Update,
            (
                close_npc_session_out_of_range::<GossipState>,
                close_npc_session_out_of_range::<TrainerOpen>,
                // The select reads the menu the VM was fed, as 12340's reads its own table.
                super::super::feed_gossip,
                super::super::select::drain_gossip,
                crate::ui_trainer::drain_trainer,
                crate::ui_session::feed_interact_npc,
            )
                .chain(),
        );
    super::register(&mut app);
    crate::ui_trainer::net::register(&mut app);
    app.world_mut().spawn((SelfPlayer, Transform::default()));
    let npc = app
        .world_mut()
        .spawn(Transform::from_xyz(2.0, 0.0, 0.0))
        .id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(LLANE, npc);
    (app, rx)
}

fn receive(app: &mut App, event: SessionEvent) {
    crate::net::handlers::dispatch(app.world_mut(), vec![event]);
    app.update();
}

fn lua(app: &mut App, code: &str) {
    app.world()
        .non_send_resource::<UiScript>()
        .run(code)
        .unwrap();
    app.update();
}

/// Whether a right-click on Llane would be refused as "its window is already open".
fn refused(app: &App) -> bool {
    interaction_already_open_on(LLANE, app.world().resource::<InteractNpc>())
}

/// `CMSG_GOSSIP_HELLO`'s answers up to a bought spell: the menu and its text, the lone option
/// stock `GossipFrame_OnEvent` selects by itself, the list, the purchase.
fn open_and_buy(app: &mut App, rx: &crossbeam_channel::Receiver<ClientCommand>) {
    receive(
        app,
        SessionEvent::GossipMenu {
            npc: LLANE,
            text_id: TEXT,
            options: vec![GossipOption {
                index: 0,
                icon: 3,
                coded: false,
                message: "I require training, Llane.".into(),
                ..Default::default()
            }],
            quests: vec![],
            quest_info: vec![],
        },
    );
    receive(
        app,
        SessionEvent::NpcGreeting {
            text_id: TEXT,
            blocks: vec![NpcTextBlock {
                probability: 1.0,
                male: "Hail, $c.".into(),
                female: String::new(),
            }],
        },
    );
    lua(app, "SelectGossipOption(1)");
    let sent: Vec<ClientCommand> = rx.try_iter().collect();
    assert!(
        sent.iter()
            .any(|c| matches!(c, ClientCommand::GossipSelectOption { guid: LLANE, .. })),
        "the option goes out: {sent:?}"
    );
    receive(
        app,
        SessionEvent::TrainerList {
            trainer: LLANE,
            trainer_type: 0,
            services: vec![],
            greeting: "Hail, warrior.".into(),
        },
    );
    assert_eq!(app.world().resource::<TrainerOpen>().trainer, Some(LLANE));
    assert_eq!(
        app.world().resource::<GossipState>().npc,
        None,
        "the trainer's window ends the menu"
    );
    assert!(refused(app), "a re-click on the open trainer does nothing");
    receive(
        app,
        SessionEvent::TrainerBuySucceeded {
            trainer: LLANE,
            spell_id: 772,
        },
    );
}

/// The close button and Escape both hide `ClassTrainerFrame`, whose `OnHide` calls
/// `CloseTrainer()` (`Blizzard_TrainerUI.xml:515-516`; Escape reaches it through
/// `CloseAllWindows` → `HideUIPanel`, the frame being a left-area panel). Before the fix the
/// gossip menu stayed latched on Llane, and the next right-click was refused.
#[test]
fn closing_the_trainer_frees_the_re_click() {
    let (mut app, rx) = reopen_app();
    open_and_buy(&mut app, &rx);
    lua(&mut app, "CloseTrainer()");
    assert_eq!(app.world().resource::<TrainerOpen>().trainer, None);
    assert_eq!(app.world().resource::<InteractNpc>().1, None);
    assert!(!refused(&app), "the right-click goes out again");

    // And again: a second visit closes the same way.
    open_and_buy(&mut app, &rx);
    lua(&mut app, "CloseTrainer()");
    assert!(!refused(&app));
}

/// Walking out of the leash closes every session client-side, the one path that worked live.
#[test]
fn walking_away_frees_the_re_click() {
    let (mut app, rx) = reopen_app();
    open_and_buy(&mut app, &rx);
    let me = app
        .world_mut()
        .query_filtered::<Entity, With<SelfPlayer>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(me)
        .insert(Transform::from_xyz(30.0, 0.0, 0.0));
    app.update();
    assert_eq!(app.world().resource::<TrainerOpen>().trainer, None);
    assert!(!refused(&app));
}
