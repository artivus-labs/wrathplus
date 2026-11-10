//! [`query_statuses`] across a session's edges: the login burst, whose units arrive in the frame
//! before our own body is tagged, and a second world session in the same process.
//!
//! Found on the beta (2026-11-10): the create burst spawns our body and every unit in view in one
//! frame, and `tag_self_player` attaches [`SelfPlayer`] with a deferred command, a frame later. The
//! system returned early on that frame, which still spent the units' creation ticks, and the next
//! frame read our own store as unchanged, so no sweep ran: a giver that never changed a field
//! (Deputy Willem) was never asked about, and showed no `!` until a player field moved.

use super::*;
use crate::net::{Guid, NetEntity};
use wrathplus_protocol::types::Field;
use wrathplus_protocol::ObjectFields;

/// Northshire's questgivers, as the login burst sends them: Deputy Willem and two more.
const GIVERS: [u64; 3] = [
    0xf130_0003_3700_160b,
    0xf130_0000_c500_1626,
    0xf130_0001_cb00_1622,
];
/// A creature with no service, in the same burst.
const WOLF: u64 = 0xf130_0001_2b00_576a;
const ME: u64 = 912;

fn app() -> (App, crossbeam_channel::Receiver<ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx))
        .init_resource::<GuidIndex>()
        .init_resource::<QuestGiver>()
        .init_resource::<crate::net::Reputations>()
        .add_systems(Update, query_statuses);
    (app, rx)
}

fn unit(guid: u64, fields: &[(u16, u32)]) -> (Guid, NetEntity, ObjectStore) {
    (
        Guid(guid),
        NetEntity {
            kind: EntityKind::Unit,
            display_id: None,
            scale: 1.0,
        },
        ObjectStore(ObjectFields::from_pairs(fields)),
    )
}

/// The create burst: our body (untagged) and the units around us, all in one frame. Returns our
/// body, for the tag a frame later.
fn login_burst(app: &mut App) -> Entity {
    let level = Field::UnitLevel.index();
    let npc_flags = Field::UnitNpcFlags.index();
    let me = app.world_mut().spawn(unit(ME, &[(level, 1)])).id();
    for guid in GIVERS {
        let e = app.world_mut().spawn(unit(guid, &[(npc_flags, 0x3)])).id();
        app.world_mut()
            .resource_mut::<GuidIndex>()
            .0
            .insert(guid, e);
    }
    let wolf = app.world_mut().spawn(unit(WOLF, &[(level, 1)])).id();
    app.world_mut()
        .resource_mut::<GuidIndex>()
        .0
        .insert(WOLF, wolf);
    app.world_mut().resource_mut::<GuidIndex>().0.insert(ME, me);
    me
}

fn asked(app: &mut App, rx: &crossbeam_channel::Receiver<ClientCommand>) -> Vec<u64> {
    app.update();
    let mut out: Vec<u64> = rx
        .try_iter()
        .filter_map(|c| match c {
            ClientCommand::QuestgiverStatusQuery { npc } => Some(npc),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out
}

/// Logout: the world goes, every object with it.
fn logout(app: &mut App) {
    let everything: Vec<Entity> = app
        .world()
        .resource::<GuidIndex>()
        .0
        .values()
        .copied()
        .collect();
    for e in everything {
        app.world_mut().entity_mut(e).despawn();
    }
    app.world_mut().resource_mut::<GuidIndex>().0.clear();
}

fn givers() -> Vec<u64> {
    let mut g = GIVERS.to_vec();
    g.sort_unstable();
    g
}

#[test]
fn every_giver_in_the_login_burst_is_asked_once_our_body_is_tagged() {
    let (mut app, rx) = app();
    let me = login_burst(&mut app);
    // The burst's frame: no `SelfPlayer` yet, so nothing can be asked.
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
    // `tag_self_player`'s command lands: every giver is asked, with no player field moving.
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new(), "once");
    // The answers stand: a quiet frame tears nothing down.
    app.world_mut()
        .resource_mut::<QuestGiver>()
        .set_status(GIVERS[0], 5);
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
    assert_eq!(
        app.world().resource::<QuestGiver>().status(GIVERS[0]),
        Some(5)
    );
}

/// Several untagged frames (a slow tag, a worldport's re-stream) are no different from one.
#[test]
fn a_longer_wait_for_our_body_still_ends_in_one_sweep() {
    let (mut app, rx) = app();
    let me = login_burst(&mut app);
    for _ in 0..5 {
        assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
    }
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
}

/// Logout and back in, same process, same character (same spawn guids, same keys, our fields
/// hashing as before), our body first and the givers streamed a frame later: asked afresh. This
/// order passed before the reset too: the index change at our body's arrival pruned the old keys.
#[test]
fn a_second_session_streamed_after_our_body_asks_afresh() {
    let (mut app, rx) = app();
    let me = login_burst(&mut app);
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());

    // Logout: the world goes, every object with it.
    let everything: Vec<Entity> = app
        .world()
        .resource::<GuidIndex>()
        .0
        .values()
        .copied()
        .collect();
    for e in everything {
        app.world_mut().entity_mut(e).despawn();
    }
    app.world_mut().resource_mut::<GuidIndex>().0.clear();
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());

    // Back in: our body first and tagged in its own frame, the givers streamed a frame later.
    let level = Field::UnitLevel.index();
    let me = app
        .world_mut()
        .spawn((unit(ME, &[(level, 1)]), SelfPlayer))
        .id();
    app.world_mut().resource_mut::<GuidIndex>().0.insert(ME, me);
    assert_eq!(
        asked(&mut app, &rx),
        Vec::<u64>::new(),
        "nobody in view yet"
    );
    let npc_flags = Field::UnitNpcFlags.index();
    for guid in GIVERS {
        let e = app.world_mut().spawn(unit(guid, &[(npc_flags, 0x3)])).id();
        app.world_mut()
            .resource_mut::<GuidIndex>()
            .0
            .insert(guid, e);
    }
    assert_eq!(
        asked(&mut app, &rx),
        givers(),
        "asked afresh, not matched to the old session"
    );
}

/// The other order, where the stale state was real: our body, already tagged, arrives in the same
/// frame as the givers it last saw. Before the reset their old asked keys survived the prune (they
/// are in view), matched (same spawn, same key), and our fields hashed as before, so no sweep: none
/// was asked.
#[test]
fn a_second_session_arriving_with_our_body_asks_afresh() {
    let (mut app, rx) = app();
    let me = login_burst(&mut app);
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());
    logout(&mut app);
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
    let me = login_burst(&mut app);
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());
}

/// A second login whose burst races the tag, like the first: the reset at logout is what makes it
/// a fresh session again.
#[test]
fn a_second_login_burst_is_swept_like_the_first() {
    let (mut app, rx) = app();
    let me = login_burst(&mut app);
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());
    let everything: Vec<Entity> = app
        .world()
        .resource::<GuidIndex>()
        .0
        .values()
        .copied()
        .collect();
    for e in everything {
        app.world_mut().entity_mut(e).despawn();
    }
    app.world_mut().resource_mut::<GuidIndex>().0.clear();
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
    let me = login_burst(&mut app);
    assert_eq!(asked(&mut app, &rx), Vec::<u64>::new());
    app.world_mut().entity_mut(me).insert(SelfPlayer);
    assert_eq!(asked(&mut app, &rx), givers());
}

#[test]
fn pvp_refresh_coalesces_edges_and_retains_pending_work() {
    use std::time::{Duration, Instant};
    let now = Instant::now();
    let mut state = PvpRefresh::default();
    assert!(!state.poll(Some((false, false)), now));
    assert!(state.poll(Some((true, false)), now));
    assert!(!state.poll(Some((true, true)), now + Duration::from_millis(100)));
    assert!(!state.poll(Some((false, false)), now + Duration::from_millis(200)));
    assert!(!state.poll(Some((false, false)), now + Duration::from_millis(999)));
    assert!(state.poll(Some((false, false)), now + Duration::from_secs(1)));
    assert!(!state.poll(Some((false, false)), now + Duration::from_secs(2)));
    assert!(!state.poll(None, now + Duration::from_secs(2)));
    assert!(!state.poll(Some((true, true)), now + Duration::from_secs(2)));
}
