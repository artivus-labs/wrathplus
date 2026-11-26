//! The battleground and arena scoreboard feed, the app half of build 12340's `WorldStateFrame.lua`
//! score frame.
//!
//! The reference holds a `MSG_PVP_LOG_DATA` board until every row's name has resolved, and the last
//! name arrival rebuilds it (`0x54d280` → `0x54ca30` → `0x54aa30`); here the board is pushed the
//! frame the last name lands. `UPDATE_BATTLEFIELD_SCORE` fires on a pushed board and on the
//! status-3 arrival, before `UPDATE_BATTLEFIELD_STATUS`. In an arena (the active map's instance
//! type 4) a row's team is the wire byte; elsewhere it comes from the race (`0x54aa30`).

use std::time::{Duration, Instant};

use bevy::prelude::*;

use wrathplus_protocol::wrath_battleground::{PvpLog, ScoreKind};
use wrathplus_ui::script::{
    BattlefieldScoreRow, BattlefieldScores, BattlefieldStatColumn, BattlefieldTeamView, UiScript,
};

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::EnteredWorldMessage;
use crate::names::NameCache;
use crate::net::{ClientCommand, NetCommands};
use crate::ui_dialog_verbs::BattlefieldQueue;
use crate::world_state_ui::WorldStateUiRes;

/// `RequestBattlefieldScoreData`'s throttle (`0x54ce30`).
const REQUEST_THROTTLE: Duration = Duration::from_millis(5000);

/// The last `MSG_PVP_LOG_DATA`, the arena team blocks it last carried and the request stamp.
#[derive(Resource, Default)]
pub(crate) struct BattlefieldScoreboard {
    log: Option<PvpLog>,
    /// `0xbea0b0`/`0xbea594..`: written only by an arena board, zeroed at world enter.
    teams: [BattlefieldTeamView; 2],
    last_request: Option<Instant>,
}

impl BattlefieldScoreboard {
    pub(crate) fn apply(&mut self, log: PvpLog) {
        if let Some(arena) = &log.arena {
            for (team, (name, ratings)) in self
                .teams
                .iter_mut()
                .zip(arena.names.iter().zip(arena.ratings))
            {
                // The 96-byte name buffer (`0x47b480(.., 0x60)`).
                team.name = name.chars().take(95).collect();
                team.ratings = [ratings.0, ratings.1, ratings.2];
            }
        }
        self.log = Some(log);
    }

    /// The world-enter reset (`0x54ca50`): the board, the stamp and the team names clear.
    fn clear_world(&mut self) {
        self.log = None;
        self.last_request = None;
        for team in &mut self.teams {
            team.name.clear();
        }
    }

    /// `0x54ce30`: the first request goes at once; later ones only while the match runs outside
    /// an arena, at most every 5 s.
    fn request_due(&mut self, arena: bool, now: Instant) -> bool {
        let ended = self.log.as_ref().is_some_and(|l| l.winner.is_some());
        let due = match self.last_request {
            None => true,
            Some(t) => !ended && !arena && now.saturating_duration_since(t) >= REQUEST_THROTTLE,
        };
        if due {
            self.last_request = Some(now);
        }
        due
    }
}

/// The column headers for `map`: the first contiguous run of matching `WorldStateUI.dbc` rows in
/// table order, as the status-3 arm builds them (`0x54ae40`).
pub(crate) fn score_columns(catalog: &WorldStateUiRes, map: u32) -> Vec<BattlefieldStatColumn> {
    let mut out = Vec::new();
    for (_, row) in catalog.0.rows() {
        let matches = (row.map_id == map || row.map_id == u32::MAX) && row.ui_type == 2;
        if matches {
            out.push(BattlefieldStatColumn {
                text: row.text.clone(),
                icon: row.icon.clone(),
                tooltip: row.tooltip.clone(),
            });
        } else if !out.is_empty() {
            break;
        }
    }
    out
}

/// A class id's file token, the score's tenth value.
fn class_token(class: u8) -> Option<&'static str> {
    Some(match class {
        1 => "WARRIOR",
        2 => "PALADIN",
        3 => "HUNTER",
        4 => "ROGUE",
        5 => "PRIEST",
        6 => "DEATHKNIGHT",
        7 => "SHAMAN",
        8 => "MAGE",
        9 => "WARLOCK",
        11 => "DRUID",
        _ => return None,
    })
}

/// Resolve the raw board through the name cache; `None` while any name is still in flight.
fn resolve_board(
    log: &PvpLog,
    arena: bool,
    names: &NameCache,
    commands: &NetCommands,
) -> Option<Vec<BattlefieldScoreRow>> {
    let mut rows = Vec::with_capacity(log.scores.len());
    for r in &log.scores {
        let name = names.resolve(r.guid, commands)?.to_string();
        let (race, class) = names
            .player_traits(r.guid)
            .map_or((0, 0), |(race, class, _)| (race, class));
        let (honorable_kills, deaths, honor_gained, team) = match r.kind {
            ScoreKind::Battleground {
                honorable_kills,
                deaths,
                bonus_honor,
            } => (honorable_kills, deaths, bonus_honor, None),
            ScoreKind::Arena { team } => (0, 0, 0, Some(team)),
        };
        let faction = match (arena, team) {
            (true, Some(team)) => i32::from(team),
            _ => i32::from(crate::ui_unit::race_pvp_team(race)),
        };
        let mut stats = [0u32; 8];
        for (slot, v) in stats.iter_mut().zip(&r.objectives) {
            *slot = *v;
        }
        let known = |s: &'static str| (s != "?").then(|| s.to_string());
        rows.push(BattlefieldScoreRow {
            name,
            killing_blows: r.killing_blows,
            honorable_kills,
            deaths,
            honor_gained,
            faction,
            race: (race != 0)
                .then(|| crate::char_select::race_name(race))
                .and_then(known),
            class: (class != 0)
                .then(|| crate::char_select::class_name(class))
                .and_then(known),
            class_token: class_token(class).map(str::to_string),
            class_id: class,
            damage: r.damage,
            healing: r.healing,
            stats,
        });
    }
    Some(rows)
}

pub(crate) fn feed_battlefield_score(
    script: Option<NonSendMut<UiScript>>,
    board: Res<BattlefieldScoreboard>,
    mut queue: ResMut<BattlefieldQueue>,
    catalog: Option<Res<WorldStateUiRes>>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<BattlefieldScores>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let now = Instant::now();
    script.set_battlefield_run_time_ms(queue.run_time_ms(now));

    let arena = queue.active_instance_type() == 4;
    let fresh = board.log.as_ref().and_then(|log| {
        let rows = resolve_board(log, arena, &names, &commands)?;
        let columns = queue
            .active_map()
            .zip(catalog.as_deref())
            .map(|(map, cat)| score_columns(cat, map))
            .unwrap_or_default();
        Some(BattlefieldScores {
            rows,
            winner: log.winner,
            teams: board.teams.clone(),
            columns,
        })
    });
    script.set_battlefield_score_names_pending(board.log.is_some() && fresh.is_none());
    let last = last.get(&script);
    let status_rebuild = queue.take_score_dirty();
    if status_rebuild {
        // Status 3 resets the filter (`0x54ae40`) before its rebuild.
        script.reset_battlefield_score_filter();
    }
    let changed = fresh.is_some() && fresh != *last;
    if changed {
        script.set_battlefield_scores(fresh.clone().expect("checked"));
        *last = fresh;
    }
    if changed || status_rebuild {
        script.fire_event("UPDATE_BATTLEFIELD_SCORE", vec![]);
    }
}

fn drain_battlefield_score(
    script: Option<NonSendMut<UiScript>>,
    mut board: ResMut<BattlefieldScoreboard>,
    queue: Res<BattlefieldQueue>,
    commands: Res<NetCommands>,
) {
    let Some(mut script) = script else {
        return;
    };
    if script.take_battlefield_score_requests() > 0
        && board.request_due(queue.active_instance_type() == 4, Instant::now())
    {
        let _ = commands.0.send(ClientCommand::RequestBattlefieldScoreData);
    }
    // Unresolved, labelled: the reference sends the active slot's eight queue-id bytes; the
    // writer sends zeroes, which AzerothCore never reads.
    for _ in 0..script.take_battlefield_leave_requests() {
        let _ = commands.0.send(ClientCommand::LeaveBattlefield);
    }
}

pub(crate) fn clear_on_world_enter(
    mut entered: MessageReader<EnteredWorldMessage>,
    mut board: ResMut<BattlefieldScoreboard>,
) {
    if entered.read().next().is_some() {
        board.clear_world();
    }
}

/// The scoreboard's session handler.
mod net {
    use bevy::prelude::*;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use super::BattlefieldScoreboard;
    use crate::net::NetHandlerApp;

    pub(super) fn register(app: &mut App) {
        app.net_handler(SessionEventKind::Disconnected, on_session_end);
    }

    /// Zeroes the board and the stamp, as the module init (`0x54e720`) does at every login.
    fn on_session_end(In(_): In<SessionEvent>, mut board: ResMut<BattlefieldScoreboard>) {
        *board = BattlefieldScoreboard::default();
    }
}

pub struct BattlefieldScorePlugin;

impl Plugin for BattlefieldScorePlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        app.init_resource::<BattlefieldScoreboard>().add_systems(
            Update,
            (
                clear_on_world_enter
                    .in_set(UiFeed)
                    .before(feed_battlefield_score),
                // Before the queue feed: on status 3 the score event precedes the status event.
                feed_battlefield_score
                    .before(crate::ui_dialog_verbs::feed_dialog_verbs)
                    .in_set(UiFeed),
                drain_battlefield_score.after(UiInput),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_formats::{WorldStateUiCatalog, WorldStateUiRow};

    fn row(map_id: u32, ui_type: u32, text: &str) -> WorldStateUiRow {
        WorldStateUiRow {
            map_id,
            area_id: 0,
            icon: String::new(),
            text: text.into(),
            tooltip: String::new(),
            state_variable: 0,
            ui_type,
            dynamic_icon: String::new(),
            dynamic_tooltip: String::new(),
            extended_ui: String::new(),
            extended_ui_state: [0; 3],
        }
    }

    #[test]
    fn the_columns_are_the_first_contiguous_run_of_type_two_rows_for_the_map() {
        let cat = WorldStateUiRes(WorldStateUiCatalog::from_rows(vec![
            (1, row(489, 0, "always-up")),
            (2, row(529, 2, "other map")),
            (3, row(489, 2, "Flags Captured")),
            (4, row(u32::MAX, 2, "Flags Returned")),
            (5, row(489, 0, "a gap")),
            (6, row(489, 2, "after the gap")),
        ]));
        let cols: Vec<String> = score_columns(&cat, 489)
            .into_iter()
            .map(|c| c.text)
            .collect();
        assert_eq!(cols, ["Flags Captured", "Flags Returned"]);
        // Another map gets the wildcard row alone: `-1` rows serve every map.
        let cols: Vec<String> = score_columns(&cat, 30)
            .into_iter()
            .map(|c| c.text)
            .collect();
        assert_eq!(cols, ["Flags Returned"]);
    }

    #[test]
    fn the_session_end_zeroes_the_scoreboard() {
        let mut app = App::new();
        app.init_resource::<BattlefieldScoreboard>();
        net::register(&mut app);
        {
            let mut board = app.world_mut().resource_mut::<BattlefieldScoreboard>();
            board.apply(PvpLog {
                arena: None,
                winner: Some(1),
                scores: Vec::new(),
            });
            board.last_request = Some(Instant::now());
        }

        crate::net::handlers::dispatch(
            app.world_mut(),
            vec![wrathplus_protocol::SessionEvent::Disconnected {
                reason: "socket".into(),
                end: wrathplus_protocol::SessionEnd::Lost,
            }],
        );

        let board = app.world().resource::<BattlefieldScoreboard>();
        assert!(board.log.is_none());
        assert!(board.last_request.is_none());
    }
}
