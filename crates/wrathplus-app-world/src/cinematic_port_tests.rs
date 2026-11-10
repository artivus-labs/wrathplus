//! What reaches outside the playback: Lua's `OpeningCinematic()` as one empty
//! `CMSG_OPENING_CINEMATIC` per call, and the agent port's reading of a held client.

use super::*;

#[test]
fn each_call_sends_one_opening_cinematic() {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx));
    app.insert_non_send_resource(UiScript::new().unwrap());
    app.add_systems(Update, send_opening_cinematic);
    app.world()
        .non_send_resource::<UiScript>()
        .run("OpeningCinematic() OpeningCinematic()")
        .unwrap();
    app.update();
    let sent = rx
        .try_iter()
        .filter(|c| matches!(c, ClientCommand::OpeningCinematic))
        .count();
    assert_eq!(sent, 2);
    app.update();
    assert_eq!(rx.try_iter().count(), 0, "drained once");
    // The opcode the reference puts at `0x48c8c9`.
    assert_eq!(
        wrathplus_protocol::wrath_opcode_names::opcode_name(0xf9),
        Some("CMSG_OPENING_CINEMATIC")
    );
}

/// The agent port's wider reading: held from the trigger to the world's return.
#[test]
fn a_cinematic_is_active_from_its_latch_to_the_end_of_its_settle() {
    assert!(!Cinematic::default().is_active());
    let latched = Cinematic {
        pending: Some(41),
        ..Default::default()
    };
    assert!(latched.is_active() && !latched.is_playing());
    assert!(Cinematic::playing_for_test().is_active());
    let settling = Cinematic {
        settling: Some(SETTLE_BUDGET),
        ..Default::default()
    };
    assert!(settling.is_active() && !settling.is_playing());
}
