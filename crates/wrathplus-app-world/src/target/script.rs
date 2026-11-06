//! The selection calls of the script call stream (the UI's `script_calls` pass), each committed
//! through the one SetSelection path when its turn comes, so a later call in the same pass reads
//! it.

use bevy::prelude::*;

use wrathplus_ui::script::SelectionRequest;

use super::by_name::{self, ByNameScan, SelectCommit};
use super::scan;

/// Everything the selection calls read and write.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ScriptSelect<'w, 's> {
    /// The app's unit-token resolver, 1.12's grammar ([`wrathplus_ui::script::parse_unit_token`]):
    /// bases and `target` hops. The VM's is build 12340's `0x60abf0`, which adds `pet` suffixes
    /// and group members' names, so `TargetUnit("party1pet")` or a member's name names no unit
    /// here.
    tokens: crate::target::unit_tokens::UnitTokens<'w, 's>,
    /// `TargetLastEnemy`'s memory (`[0xb4e2e8]`), stamped by `scan::remember_last_enemy`.
    last_enemy: Res<'w, scan::LastEnemy>,
    /// `TargetLastFriend`'s memory, stamped beside it.
    last_friend: Res<'w, scan::LastFriend>,
    /// The SetSelection tail shared with the world click, `/target` and `/assist`, classification
    /// included.
    commit: SelectCommit<'w, 's>,
    by_name: ByNameScan<'w, 's>,
    /// The TAB cycler's inputs, for the `TargetNearest*` calls.
    scan: scan::TargetScan<'w, 's>,
    history: ResMut<'w, scan::TabHistory>,
    time: Res<'w, Time>,
}

impl ScriptSelect<'_, '_> {
    /// `TargetUnit` (`0x4899d0`), `AssistUnit` (`0x489b80`) and `TargetLastEnemy` share the
    /// reference's helper `0x489a40`: a resolved unit or a roster member is committed, anything
    /// else is a no-op, never a deselect. Here a token must name a streamed unit, so an
    /// out-of-range roster member is a no-op (the guid-only selection is not built).
    pub fn select(&mut self, request: SelectionRequest) {
        match request {
            SelectionRequest::Unit(token) => {
                if let Some((entity, guid)) = self.tokens.resolve(&token, &self.commit.selection) {
                    self.commit.commit(entity, guid);
                }
            }
            // `0x489ba9 call 0x515940(token)`, then the shared tail. An unresolvable token is
            // silent here; the reference shows message `0xb8` `ERR_GENERIC_NO_TARGET` (`0x489c0e`).
            SelectionRequest::Assist(token) => {
                match self.tokens.resolve(&token, &self.commit.selection) {
                    Some((basis, _)) => self.commit.assist(basis, "AssistUnit"),
                    None => info!("assist (AssistUnit): \"{token}\" names nothing; silent no-op"),
                }
            }
            // `0x489c40`, `/assist <name>`: a player name, prefix-matched by the app's name scan.
            SelectionRequest::AssistByName(name) => {
                by_name::assist_named(&self.by_name, &mut self.commit, Some(&name), "AssistByName");
            }
            // `0x489b45` → `0x489a40`. Empty memory is a no-op (the shim `0x489b40` tests nothing),
            // unlike `TargetLastTarget 0x489b00`, which reaches `0x493540(0,0)` and deselects.
            SelectionRequest::LastEnemy => {
                let Some(guid) = self.last_enemy.0 else {
                    info!("TargetLastEnemy: nothing hostile has been targeted yet; no-op");
                    return;
                };
                match self.tokens.held(guid) {
                    Some((entity, guid)) => self.commit.commit(entity, guid),
                    // Stale: a no-op, and the memory stays, as the reference never clears it.
                    None => info!(
                        "TargetLastEnemy: {guid:#x} is no longer streamed; target left untouched"
                    ),
                }
            }
        }
    }

    /// `TargetByName(name, exactMatch)` (`0x489d60`), the stock `/target`.
    pub fn target_by_name(&mut self, name: &str, exact: bool) {
        by_name::target_named(&self.by_name, &mut self.commit, name, exact);
    }

    /// `TargetNearest*([reverse])` (`0x489a80`/`0x489aa0`/`0x489ac0`/`0x489ae0` →
    /// `0x493f60(reverse, mode)`), one cycle of the cycler the TAB key runs.
    pub fn target_nearest(&mut self, mode: wrathplus_ui::script::NearestMode, reverse: bool) {
        let engaged = self.commit.engaged();
        scan::cycle(
            scan::ScanSide::of(mode),
            reverse,
            self.time.elapsed_secs_f64(),
            &self.scan,
            &mut self.history,
            &mut self.commit.selection,
            &mut self.commit.seam,
            engaged,
        );
    }

    /// `TargetLastTarget()` (`0x489b00`): a held last-target pair goes to `0x489a40` as a stack
    /// copy, so a streamed unit is committed (which stamps the outgoing target, swapping the two)
    /// and an unstreamed one is a no-op (the roster fallback is not built, as for
    /// `TargetLastEnemy`); an empty pair deselects through `0x493540(0,0)` (`0x489b2d`).
    pub fn target_last_target(&mut self) {
        match self.commit.selection.last {
            Some(guid) => match self.tokens.held(guid) {
                Some((entity, guid)) => self.commit.commit(entity, guid),
                None => info!("TargetLastTarget: {guid:#x} is no longer streamed; no-op"),
            },
            None => self.commit.clear(),
        }
    }

    /// A unit token's guid, as the selection calls resolve it; `None` names nothing streamed.
    pub fn token_guid(&self, token: &str) -> Option<u64> {
        self.tokens
            .resolve(token, &self.commit.selection)
            .map(|(_, guid)| guid)
    }

    /// Build 12340's `TargetLastFriend()` (`0x525e50`): the remembered friend, committed when
    /// streamed; empty memory or an unstreamed friend is a no-op, as `TargetLastEnemy` is.
    pub fn target_last_friend(&mut self) {
        let Some(guid) = self.last_friend.0 else {
            info!("TargetLastFriend: no friend has been targeted yet; no-op");
            return;
        };
        match self.tokens.held(guid) {
            Some((entity, guid)) => self.commit.commit(entity, guid),
            None => info!("TargetLastFriend: {guid:#x} is no longer streamed; no-op"),
        }
    }

    /// `ClearTarget()` (`0x489ff0`): deselect whatever the calls before it left selected.
    pub fn clear_target(&mut self) {
        self.commit.clear();
    }

    /// Build 12340's `TargetTotem(slot)` (`0x5262e0`): the totem's guid into `SetSelection`
    /// (`0x524bf0`), which commits a streamed unit. An unstreamed guid is a no-op here, as for
    /// `TargetLastEnemy`; the reference stores the bare guid (`0x524efc`).
    pub fn target_guid(&mut self, guid: u64) {
        match self.tokens.held(guid) {
            Some((entity, guid)) => self.commit.commit(entity, guid),
            None => info!("TargetTotem: {guid:#x} is not streamed; target left untouched"),
        }
    }
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
