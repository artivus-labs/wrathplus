//! The in-game movie. `SMSG_TRIGGER_MOVIE` (captured by `net::wrath_world_objects_capture`) makes a
//! movie pending (`net::wrath_movie`, the reference's `[0xbd0800]`, its id `[0xbd0804]`). The
//! reference's play step (`0x51d710`) finishes at once for an id `Movie.dbc` lacks, else fires
//! `PLAY_MOVIE(name, volume)`. The stock `MovieFrame` plays it, and `GameMovieFinished()`
//! (`0x518870`) ends the pending movie with an empty `CMSG_COMPLETE_MOVIE`, only while one is
//! pending. WrathPlus has no playback path:
//! `MovieFrame:StartMovie` answers nil (`wrathplus_ui::script` `movie`), so the stock frame finishes
//! straight away, as the reference does for a file it cannot open. Wow.exe pass:
//! `/tmp/azb-wireb-ghidra/movie-decomp.log`.
//!
//! Labelled: the trigger handler (`0x526530`) is untraced. The name is the row's file name up to
//! its first `.` (`0x51d775`), its `"%s%s"` prefix (`+8`) unread; the volume is the row's (the
//! handler's `[0xbd0808]` source is unread); the player-control bit `0x518870` clears is not modeled.

pub(crate) use crate::net::wrath_movie::PendingMovie;
use crate::net::wrath_world_objects_capture::NativeWorldObjectsPackets;
use crate::net::{ClientCommand, NetCommands};
use bevy::prelude::*;
use wrathplus_protocol::wrath_world_objects::{Command, Packet};
use wrathplus_ui::script::{ScriptValue, UiScript};

#[derive(Resource)]
struct Movies(wrathplus_formats::Movies);

pub(crate) fn register(app: &mut App) {
    app.add_systems(Startup, load.after(wrathplus_assets::AssetSet::Open))
        .add_systems(
            Update,
            feed_movie
                .in_set(crate::app_core::schedule::UiFeed)
                .run_if(crate::app_core::ui_state::ingame_ui_up),
        );
}

fn load(mut commands: Commands, assets: Option<Res<wrathplus_assets::WorldAssets>>) {
    use wrathplus_assets::LockRecover;
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    match wrathplus_formats::load_movies(&mut chain) {
        Ok(movies) => commands.insert_resource(Movies(movies)),
        Err(e) => warn!("Movie.dbc unavailable, every movie finishes at once: {e:#}"),
    }
}

/// `PLAY_MOVIE`'s name: the file name up to its first `.`.
fn movie_name(file: &str) -> &str {
    file.split('.').next().unwrap_or(file)
}

fn complete(pending: &mut PendingMovie, commands: &NetCommands) {
    if pending.id.take().is_some() {
        pending.shown = false;
        let _ = commands
            .0
            .send(ClientCommand::NativeWorldObjects(Command::CompleteMovie));
    }
}

fn feed_movie(
    script: Option<NonSendMut<UiScript>>,
    mut packets: ResMut<NativeWorldObjectsPackets>,
    mut pending: ResMut<PendingMovie>,
    movies: Option<Res<Movies>>,
    commands: Res<NetCommands>,
) {
    let Some(mut script) = script else {
        return;
    };
    // The triggers leave the capture; its other receipts stay, in order.
    let mut kept = std::collections::VecDeque::new();
    for packet in packets.0.drain(..) {
        match packet {
            Packet::TriggerMovie { movie } => {
                pending.id = Some(movie);
                pending.shown = false;
            }
            other => kept.push_back(other),
        }
    }
    packets.0 = kept;
    if script.take_game_movie_finished() > 0 {
        complete(&mut pending, &commands);
    }
    let Some(id) = pending.id.filter(|_| !pending.shown) else {
        return;
    };
    match movies.as_deref().and_then(|m| m.0.get(id)) {
        Some(row) => {
            pending.shown = true;
            script.fire_event(
                "PLAY_MOVIE",
                vec![
                    ScriptValue::Str(movie_name(&row.file).to_string()),
                    ScriptValue::Int(i64::from(row.volume)),
                ],
            );
        }
        // `0x51d749`: no row, so the movie is finished before it starts.
        None => complete(&mut pending, &commands),
    }
}

#[cfg(test)]
#[path = "wrath_movie_tests.rs"]
mod tests;
