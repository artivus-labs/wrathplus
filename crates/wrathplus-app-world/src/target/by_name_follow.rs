//! `/follow`'s name pass and start gate, as 12340's `FollowUnit` (`0x5224c0`) runs them.
//!
//! A name goes through `0x520190` twice: friendly living players first (typemask `0x10`, mode 3:
//! `CanAssist` `0x7293d0` and health above 0, `0x518ebd`), then any unit (typemask 8, mode 0).
//! Each pass takes the nearest unit whose name starts with the text (`0x51ffe0`: `SStrCmpI`
//! `0x76ea40` over the text's `SStrLen` `0x76eea0` characters, the whole name when exact).
//! Not built: the party and raid pre-scans (`0x5202e1`, `0x520370`), which add only unstreamed
//! groupmates that `FollowUnit` then misses (`0x52253a`), and the `name-realm` form (`0x5200fd`).

use bevy::prelude::*;

use wrathplus_protocol::guid;

use crate::app_core::errors::UiError;
use crate::app_core::session::ObjectStore;
use crate::net::GuidIndex;
use crate::player::FollowRequest;

use super::ByNameScan;

/// Our own unit's rank in a name pass (`0x52006a`), behind every other candidate.
const SELF_RANK: f32 = f32::from_bits(0x4f15_02f9);
/// Added to the rank of a unit at 0 health, `UNIT_DYNFLAG_DEAD` or stand state 7 (`0x5200a8`).
const DEAD_PENALTY: f32 = 10_000.0;
/// Follow's squared start range, its row of the auto-move table (`0xadaadc`): 30 yards.
const START_RANGE2: f32 = 900.0;

/// `0x51ffe0`'s name test: the first `SStrLen(query)` characters, or the whole name when exact.
fn name_matches(query: &str, name: &str, exact: bool) -> bool {
    let n = if exact {
        usize::MAX
    } else {
        query.chars().count()
    };
    wrathplus_ui::script::sstr_eq_i_n(name.as_bytes(), query.as_bytes(), n)
}

/// `0x51ffe0`'s rank, the lower the better: squared distance, a dead unit pushed back, us last.
fn rank(dist2: f32, is_self: bool, dead: bool) -> f32 {
    match (is_self, dead) {
        (true, _) => SELF_RANK,
        (false, true) => dist2 + DEAD_PENALTY,
        (false, false) => dist2,
    }
}

/// A miss (`0x522590`-`0x5225c7`): `0xc7` for none or `target` (`0x76e780`), else `0x13a`.
fn follow_miss_key(text: &str) -> &'static str {
    if text.is_empty() || text.eq_ignore_ascii_case("target") {
        "ERR_GENERIC_NO_TARGET"
    } else {
        "ERR_UNIT_NOT_FOUND"
    }
}

/// What `0x72b4a0` reads of us and the followee.
#[derive(Clone, Copy, Debug, Default)]
struct Start {
    is_self: bool,
    can_follow: bool,
    dead: bool,
    stunned: bool,
    channelling: bool,
    dist2: f32,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    Ignore,
    /// Refused before the arm, so a follow in progress runs on.
    Refuse(&'static str),
    /// The arm stops what runs (`0x727422`) before its range check refuses.
    TooFar,
    Follow,
}

/// `0x72b4a0` mode 3 (jump table `0x72b640`, `0x72b650`), then the arm `0x727400`:
///
/// | check | site | failure |
/// |---|---|---|
/// | the followee is not us | `0x72b4c1` | silent |
/// | `0x729bd0` | `0x72b525` | `0x13b` `ERR_INVALID_FOLLOW_TARGET` |
/// | our health above 0 | `0x72b54c` | `0x87` `ERR_PLAYER_DEAD` |
/// | we are not stunned (`UNIT_FLAG_STUNNED`) | `0x72b56a` | `0x1b2` `ERR_GENERIC_STUNNED` |
/// | `UNIT_CHANNEL_SPELL` is 0 | `0x72b58e` | `0x147` `ERR_TOOBUSYTOFOLLOW` |
/// | under 30 yards ([`START_RANGE2`]) | `0x715cc3` | `0x139` `ERR_AUTOFOLLOW_TOO_FAR` |
fn follow_verdict(s: Start) -> Verdict {
    match s {
        Start { is_self: true, .. } => Verdict::Ignore,
        Start {
            can_follow: false, ..
        } => Verdict::Refuse("ERR_INVALID_FOLLOW_TARGET"),
        Start { dead: true, .. } => Verdict::Refuse("ERR_PLAYER_DEAD"),
        Start { stunned: true, .. } => Verdict::Refuse("ERR_GENERIC_STUNNED"),
        Start {
            channelling: true, ..
        } => Verdict::Refuse("ERR_TOOBUSYTOFOLLOW"),
        Start { dist2, .. } if dist2 >= START_RANGE2 => Verdict::TooFar,
        _ => Verdict::Follow,
    }
}

impl ByNameScan<'_, '_> {
    /// `FollowUnit`'s two name passes (`0x5224f9`, `0x522524`).
    fn follow_by_name(&self, query: &str, exact: bool) -> Option<u64> {
        let (factions, reps) = (
            self.relations.factions.as_deref(),
            self.relations.reputations(),
        );
        let me = self.self_q.single().ok().and_then(|(_, _, _, s)| s);
        let friendly_alive = |s: &ObjectStore| {
            s.0.unit_health().unwrap_or(0) > 0
                && crate::target::relations::can_assist(
                    Some(s),
                    factions,
                    reps,
                    me,
                    self.relations.context(),
                )
        };
        self.nearest_named(query, exact, true, friendly_alive)
            .or_else(|| self.nearest_named(query, exact, false, |_| true))
    }

    /// One `0x520190` name pass; without our own unit it finds nothing (`0x5201b9`).
    fn nearest_named(
        &self,
        query: &str,
        exact: bool,
        players_only: bool,
        keep: impl Fn(&ObjectStore) -> bool,
    ) -> Option<u64> {
        let (_, me, origin, _) = self.self_q.single().ok()?;
        let mut best: Option<(f32, u64)> = None;
        for (_, g, tf, store) in &self.units {
            let Some(store) = store.filter(|s| s.is_unit()) else {
                continue;
            };
            if players_only && !guid::is_player(g.0) || !keep(store) {
                continue;
            }
            if !self
                .names
                .peek(g.0)
                .is_some_and(|n| name_matches(query, n, exact))
            {
                continue;
            }
            let dist2 = tf.translation.distance_squared(origin.translation);
            let key = rank(dist2, g.0 == me.0, store.0.unit_reads_dead());
            if best.is_none_or(|(b, _)| key < b) {
                best = Some((key, g.0));
            }
        }
        info!("follow: \"{query}\" (players only {players_only}) -> {best:?}");
        best.map(|(_, g)| g)
    }

    /// `0x729bd0`: a player, not charmed, of our faction group when both templates resolve
    /// (`0x729c4a`), and not one we can attack (`0x729740`).
    fn can_follow(&self, guid: u64, store: &ObjectStore, own: &ObjectStore) -> bool {
        let factions = self.relations.factions.as_deref();
        let group = |s| crate::target::ring::faction_group_mask(factions, Some(s));
        let same_group = match (group(store), group(own)) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        guid::is_player(guid)
            && store.0.unit_charmed_by().is_none()
            && same_group
            && !crate::target::relations::can_attack(
                Some(store),
                factions,
                self.relations.reputations(),
                Some(own),
                self.relations.context(),
            )
    }

    /// The gate's inputs; `None` without our own unit, where `FollowUnit` stops (`0x52256b`).
    fn start(&self, guid: u64, at: Vec3, store: &ObjectStore) -> Option<Start> {
        let (_, me, tf, own) = self.self_q.single().ok()?;
        let own = own?;
        Some(Start {
            is_self: guid == me.0,
            can_follow: self.can_follow(guid, store, own),
            dead: own.0.unit_health().unwrap_or(0) == 0,
            stunned: own.0.unit_flags() & crate::player::UNIT_FLAG_STUNNED != 0,
            channelling: own.0.unit_channel_spell() != 0,
            dist2: at.distance_squared(tf.translation),
        })
    }
}

/// Drain `/follow` into [`crate::player`]; follow sends nothing on the wire. The followee's name
/// is latched for `AUTOFOLLOW_BEGIN` (`0x72b62a`).
pub(in crate::target) fn follow_requests(
    mut requests: MessageReader<FollowRequest>,
    scan: ByNameScan,
    index: Res<GuidIndex>,
    mut errors: ResMut<crate::app_core::errors::UiErrorKeys>,
    mut follow: ResMut<crate::player::FollowState>,
    mut approach: ResMut<crate::player::Approach>,
) {
    for request in requests.read() {
        let (text, guid) = match request {
            FollowRequest::Unit { text, guid } => (text, Some(*guid)),
            FollowRequest::Name { name, exact } => (name, scan.follow_by_name(name, *exact)),
        };
        // `0x52253a`: a unit the object manager holds, else the miss line.
        let held = guid
            .and_then(|g| index.0.get(&g))
            .and_then(|e| scan.units.get(*e).ok())
            .and_then(|(_, g, tf, s)| Some((g.0, tf.translation, s.filter(|s| s.is_unit())?)));
        let Some((guid, at, store)) = held else {
            let key = follow_miss_key(text);
            info!("follow: nothing to follow — {key}");
            errors.0.push(UiError::key(key));
            continue;
        };
        let Some(start) = scan.start(guid, at, store) else {
            continue;
        };
        match follow_verdict(start) {
            Verdict::Ignore => info!("follow: that is us; nothing happens"),
            Verdict::Refuse(key) => {
                info!("follow: refused — {key}");
                errors.0.push(UiError::key(key));
            }
            Verdict::TooFar => {
                approach.stop();
                follow.stop();
                errors.0.push(UiError::key("ERR_AUTOFOLLOW_TOO_FAR"));
            }
            Verdict::Follow => {
                let name = scan.names.peek(guid).unwrap_or_default().to_string();
                info!("follow: now following \"{name}\" guid {guid:#x}");
                approach.stop();
                follow.start(guid, name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dead player following a creature sees `ERR_INVALID_FOLLOW_TARGET`, not `ERR_PLAYER_DEAD`:
    /// the followee check comes first.
    #[test]
    fn the_follow_gate_refuses_in_the_references_own_order() {
        let ok = Start {
            can_follow: true,
            ..Start::default()
        };
        assert_eq!(follow_verdict(ok), Verdict::Follow);
        // Ourselves: silent, before anything else.
        let me = Start {
            is_self: true,
            can_follow: false,
            dead: true,
            ..ok
        };
        assert_eq!(follow_verdict(me), Verdict::Ignore);
        // A creature, a pet, an enemy or a charmed player.
        let other = Start {
            can_follow: false,
            dead: true,
            stunned: true,
            channelling: true,
            dist2: 1e6,
            ..ok
        };
        assert_eq!(
            follow_verdict(other),
            Verdict::Refuse("ERR_INVALID_FOLLOW_TARGET")
        );
        let dead = Start {
            dead: true,
            stunned: true,
            ..ok
        };
        assert_eq!(follow_verdict(dead), Verdict::Refuse("ERR_PLAYER_DEAD"));
        let stunned = Start {
            stunned: true,
            channelling: true,
            ..ok
        };
        assert_eq!(
            follow_verdict(stunned),
            Verdict::Refuse("ERR_GENERIC_STUNNED")
        );
        // A channel, not a cast, is what is busy (`UNIT_CHANNEL_SPELL`).
        let busy = Start {
            channelling: true,
            dist2: 1e6,
            ..ok
        };
        assert_eq!(follow_verdict(busy), Verdict::Refuse("ERR_TOOBUSYTOFOLLOW"));
        // 30 yards is already too far.
        assert_eq!(
            follow_verdict(Start { dist2: 900.0, ..ok }),
            Verdict::TooFar
        );
        assert_eq!(
            follow_verdict(Start { dist2: 899.0, ..ok }),
            Verdict::Follow
        );
        assert_eq!(follow_miss_key(""), "ERR_GENERIC_NO_TARGET");
        assert_eq!(follow_miss_key("Target"), "ERR_GENERIC_NO_TARGET");
        assert_eq!(follow_miss_key("Nobody"), "ERR_UNIT_NOT_FOUND");
    }

    /// 12340 matches a name's start, with no longest-prefix tier, and ranks by distance alone.
    #[test]
    fn a_follow_name_is_a_case_folded_prefix_and_the_nearest_wins() {
        assert!(name_matches("pet", "Petra", false));
        assert!(!name_matches("Ragx", "Ragnaros", false));
        assert!(!name_matches("Ragnarosx", "Ragnaros", false));
        assert!(!name_matches("rag", "Ragnaros", true));
        assert!(name_matches("RAGNAROS", "Ragnaros", true));
        assert!(name_matches("é", "Élise", false), "SStrCmpI folds Latin-1");
        assert!(rank(4.0, false, false) < rank(1.0, false, true));
        assert!(rank(1e6, false, true) < rank(0.0, true, false));
    }
}
