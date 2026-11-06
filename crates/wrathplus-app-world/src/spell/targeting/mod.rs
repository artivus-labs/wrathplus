//! The targeting cursor: a cast waiting for the click that binds its target. The reference's
//! targeting mode is a nonzero flag_word (`IsTargeting 0x6e48a0`); [`SpellTargeting`] holds that
//! word and each click seam asks it its own mask test ([`TargetingWants`]).
//!
//! - [`cursor`]: the hover verdict per seam. Terrain is `0x4820f0`'s `CheckGroundPointInRange
//!   0x6e6810`, a GameObject, a unit or a corpse is `0x4828d0`'s `0x6e6460` (the spell-vs-lock
//!   predicate `0x5f8260`, [`SpellTargeting::can_target_unit`] or
//!   [`SpellTargeting::can_target_corpse`], then range), and a word no seam handles is UnableCast.
//! - [`world`]: the world-click dispatcher `0x492ce0`, its terrain leg (`0x492580` → `BindLocation
//!   0x6e60f0`) and its object leg (`0x4925d0` → `SetSelection 0x493540` → `BindTarget 0x6e5b40`),
//!   whose unit arm [`bind_target_unit`] shares with `SpellTargetUnit`
//!   ([`ScriptCursor::spell_target_unit`]), and whose corpse arm is [`corpse`]'s.
//! - [`pick`]: the world pick's unit and player filter, `0x481050`'s flags from the word and each
//!   candidate's `0x480610`, so a unit the word cannot take is out of the hover and the click.
//! - [`item`]: the bag click (`PickupContainerItem 0x4f9b30`) and the paper-doll click
//!   (`0x4c7300`), both `IsTargeting`, `TargetingWantsItem 0x6e6330`, then `0x495d60`, whose
//!   confirm popups park the clicked guid (`0xb4e3c0`) with the word still standing.
//!
//! Every seam commits through [`crate::spell::CastLadder::commit_targeted`] (`SendCast 0x6e54f0`).
//! Only the unit and corpse arms have a range gate ([`merge`]): the terrain and GameObject clicks
//! send and the server judges range, and `CheckGroundPointInRange` only colours the cursor. While
//! targeting, the pick flags come from the word alone, so a click over a unit with a dest-only
//! word commits on the ground behind it.
//!
//! Cancels: ESC through `UIParent.lua:1490` (the UI's targeting push in `ui_cast`, and
//! [`ScriptCursor::stop_targeting`]), the right-button down edge
//! ([`cancel_targeting_on_right_press`]), a new spell's press, which aborts and proceeds
//! (`TryCast 0x6e4b60` at `0x6e4d62`), and the bar's re-press of the same spell
//! (`UseAction 0x4e5ee0`). A cancel clears the word and sends nothing; movement never
//! cancels (`0x515090`).

mod corpse;
mod cursor;
pub mod glyph;
mod item;
mod pick;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod world;

#[cfg(any(test, feature = "test-support"))]
pub use corpse::fixture as corpse_fixture;
pub use corpse::{corpse_pick_admits, publish_corpse_pick, CorpsePick};
pub use cursor::{drive_targeting_cursor, ground_cast_radius};
pub use item::{commit_item_cast_on_pick, EnchantConfirmItem, EnchantResume};
pub(crate) use pick::{publish_unit_pick, PickChecks, UnitPick};
pub(crate) use world::{commit_ground_cast_on_click, commit_object_cast_on_click};

use bevy::prelude::*;

use wrathplus_world::interact::WorldRightPress;

/// The inputs of `BindTarget 0x6e5b40`'s unit and corpse arms and of their read-only mirror
/// `0x6e6460` ([`SpellTargeting::can_target_unit`], [`bind_target_unit`],
/// [`SpellTargeting::can_target_corpse`], [`corpse::bind_target_corpse`]): the gates and relation
/// checks the cast arm's selection bind runs ([`super::cast_target::unit_binds`]), the corpse's two
/// facts, and the pre-send range gate's inputs ([`super::cast_target::RangeInputs`]), read as
/// [`super::cast_target::CastTargeting`] reads them.
#[derive(bevy::ecs::system::SystemParam)]
pub struct BindChecks<'w, 's> {
    stores: Query<'w, 's, &'static crate::app_core::session::ObjectStore>,
    index: Option<Res<'w, crate::net::GuidIndex>>,
    self_q: Query<
        'w,
        's,
        (
            Entity,
            Option<&'static crate::app_core::session::ObjectStore>,
        ),
        With<crate::app_core::session::SelfPlayer>,
    >,
    guids: Query<'w, 's, &'static crate::net::Guid>,
    factions: Option<Res<'w, crate::app_core::state::Factions>>,
    reputations: Res<'w, crate::net::Reputations>,
    /// The party slots and raid roster as the frame began; a harness without them has no group.
    roster: Option<Res<'w, super::group_relation::GroupRoster>>,
    names: Option<Res<'w, crate::names::NameCache>>,
    /// The range leg's positions: the pose the hover picks against, as last frame propagated it,
    /// so the VM feed, the cursor and both binds read one pose. Never the camera's, which
    /// `publish_camera_pose` rewrites mid-frame.
    poses: Query<'w, 's, &'static GlobalTransform, Without<wrathplus_world::view::WorldCamera>>,
    spells: Option<Res<'w, crate::app_core::state::Spells>>,
    spell_mods: Res<'w, super::SpellModifiers>,
    /// The caster, a unit and the caster's auto-attack target (`[caster+0xc48]`, which
    /// `GetMinMaxRange` looks up) as the range compare reads them.
    range_units: super::RangeUnits<'w, 's>,
}

impl crate::target::StoreLookup for BindChecks<'_, '_> {
    fn store(&self, guid: u64) -> Option<&crate::app_core::session::ObjectStore> {
        crate::target::guid_store(self.index.as_deref(), &self.stores, guid)
    }
}

impl BindChecks<'_, '_> {
    fn is_self(&self, entity: Entity) -> bool {
        self.self_q
            .iter()
            .next()
            .is_some_and(|(me, _)| me == entity)
    }

    /// `BindTarget 0x6e5b40`'s unit branch: whether this unit binds the word under this spell,
    /// its gates and then its relation checks ([`super::cast_target::unit_binds`]). An unknown
    /// spell has no row to gate on.
    fn unit_binds(&self, spell_id: u32, word: u16, entity: Entity) -> bool {
        let target_store = self.stores.get(entity).ok();
        let owner_store =
            |store| super::cast_target::owner_store(store, self.index.as_deref(), &self.stores);
        let me = self.self_q.iter().next();
        let self_store = me.and_then(|(_, store)| store);
        let rel = super::cast_target::TargetRelations {
            target_store,
            target_owner_store: owner_store(target_store),
            self_store,
            relations: crate::target::RelationContext::new(self.names.as_deref(), self),
            factions: self.factions.as_deref(),
            reputations: &self.reputations,
            types: crate::creature_type::CreatureTypeSources {
                names: self.names.as_deref(),
                forms: self.spells.as_deref().map(|s| &s.forms),
            },
            group: super::group_relation::GroupInputs {
                self_guid: me
                    .and_then(|(me, _)| self.guids.get(me).ok())
                    .map(|guid| guid.0),
                target_guid: self.guids.get(entity).ok().map(|guid| guid.0),
                self_owner_store: owner_store(self_store),
                roster: self.roster.as_deref(),
            },
        };
        let def = self.spells.as_deref().and_then(|s| s.catalog.get(spell_id));
        super::cast_target::unit_binds(def, word, self.is_self(entity), &rel)
    }

    /// The spell's min/max range against this unit, through the one compare the pre-send gate
    /// asks ([`super::cast_target::RangeInputs::refusal`]); our own body is distance 0. An
    /// unknown spell or a missing row passes.
    fn range_refusal(&self, spell_id: u32, entity: Entity) -> Option<u8> {
        let mut range = self.range_inputs(entity);
        range.target = self.range_units.unit(entity);
        self.refusal(spell_id, range)
    }

    /// The same compare against a corpse. `GetMinMaxRange 0x7ff480` pads a corpse's bounds as a
    /// unit's (`6e35fe`), but with no unit to read the second reach from it reads the caster's
    /// again (`6e3605`–`6e361e`). On the ranged arm, which the corpse spells use, no unit means no
    /// moving bonus; a melee row would fall back to the auto-attack target (`0x6e3580`).
    fn corpse_range_refusal(&self, spell_id: u32, entity: Entity) -> Option<u8> {
        let mut range = self.range_inputs(entity);
        range.target = Some(wrathplus_formats::RangeUnit::still(range.caster.reach));
        self.refusal(spell_id, range)
    }

    /// Our position, reach and motion and the candidate's position, each position the pose the
    /// hover picks against.
    fn range_inputs(&self, entity: Entity) -> super::cast_target::RangeInputs {
        let me = self.self_q.iter().next();
        super::cast_target::RangeInputs {
            self_pos: me
                .and_then(|(e, _)| self.poses.get(e).ok())
                .map(GlobalTransform::translation),
            target_pos: self
                .poses
                .get(entity)
                .ok()
                .map(GlobalTransform::translation),
            caster: self.range_units.caster(),
            target: None,
        }
    }

    fn refusal(&self, spell_id: u32, range: super::cast_target::RangeInputs) -> Option<u8> {
        let spells = self.spells.as_deref()?;
        let def = spells.catalog.get(spell_id)?;
        range.refusal(def, spells.ranges.get(def.range_index), &self.spell_mods)
    }

    /// What the corpse legs read of this corpse: `CORPSE_FLAG_BONES` and the reaction gate
    /// `0x6067d0`. A corpse whose store has not streamed binds nothing.
    fn corpse_facts(&self, entity: Entity) -> Option<corpse::CorpseFacts> {
        Some(corpse::CorpseFacts::of(
            self.stores.get(entity).ok()?,
            self.factions.as_deref(),
            self.self_q.iter().next().and_then(|(_, s)| s),
        ))
    }
}

/// A click seam's mask test on the flag_word `0xcecac0`, one per reference predicate:
///
/// - `Location`: `TargetingWantsLocation 0x6e6320`, `word & 0x60`, the terrain click.
/// - `Item`: `TargetingWantsItem 0x6e6330`, `word & 0x4010`, the bag and paper-doll clicks.
/// - `GameObject`: `TargetingWantsGameObject 0x6e62d0`, `word & 0x4800`, the world object click.
/// - `Unit`: `SpellCanTargetUnit 0x6e6460`, the unit arm of the world click.
/// - `Corpse`: `0x6e6230`, `word & 0x8600`, the pick's corpse flag `0x40` ([`corpse`]).
///
/// The masks overlap on `TARGET_FLAG_LOCKED`, so a lock spell answers both the item and the
/// GameObject seam; the reference settles it only at the click, where `BindTarget 0x6e5b40` picks
/// its arm by the clicked object's typemask.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetingWants {
    Location,
    Item,
    GameObject,
    Unit,
    Corpse,
}

/// The unit-shaped bits of the flag_word, the first gate `SpellCanTargetUnit` tests before it runs
/// the candidate through the unit arm's relation and liveness checks.
const UNIT_WORD_BITS: u16 = 0x0002 | 0x0004 | 0x0008 | 0x0080 | 0x0100 | 0x0200 | 0x0400 | 0x8000;

impl TargetingWants {
    fn matches(self, word: u16) -> bool {
        let mask = match self {
            Self::Location => 0x0060,
            Self::Item => 0x4010,
            Self::GameObject => 0x4800,
            Self::Unit => UNIT_WORD_BITS,
            Self::Corpse => 0x8600,
        };
        word & mask != 0
    }
}

/// The word bits `0x6e61a0` lets the world pick take the local player for: unit, raid, party,
/// assist, the explicit gate and the ally corpse, never enemy (`0x80`) or enemy corpse (`0x200`).
const SELF_PICK_BITS: u16 = 0x850e;

/// The targeting mode, `Some` while the flag_word is nonzero. Entered by the cast-send path
/// ([`super::cast_target::CastWireTarget::Targeting`]), cleared by a commit or a cancel.
#[derive(Resource, Default)]
pub struct SpellTargeting(Option<Targeting>);

struct Targeting {
    spell_id: u32,
    /// The whole pending cast survives the cursor, the cast item's guid (`0xceac48`) included, so
    /// `0x6e54f0` still sends `CMSG_USE_ITEM` for a grenade, a poison bottle or a key.
    commit: super::cast_send::CastCommit,
    /// The standing flag_word `0xcecac0`; more than one seam can answer yes to it.
    word: u16,
    /// Build 12340's glyph wait (`0xd3f4e0 & 0x20000`): only `PlaceGlyphInSocket` commits it.
    glyph: bool,
}

impl SpellTargeting {
    /// `IsTargeting 0x6e48a0`.
    pub fn active(&self) -> bool {
        self.0.is_some()
    }

    /// `GetTargetingSpellId 0x6e48e0`, whatever the word wants: for the bar's checked state, the
    /// re-press toggle and the `CURRENT_SPELL_CAST_CHANGED` edge. A seam reads [`Self::spell_for`].
    pub fn spell(&self) -> Option<u32> {
        self.0.as_ref().map(|t| t.spell_id)
    }

    /// Whether the standing word answers `wants`' mask test.
    pub fn wants(&self, wants: TargetingWants) -> bool {
        self.0.as_ref().is_some_and(|t| wants.matches(t.word))
    }

    /// `SpellCanTargetUnit`'s predicate `0x6e6460`, its unit leg, with the range flag both callers
    /// pass (the world hover `0x4828d0` at `48290b`, the Lua `SpellCanTargetUnit 0x6e6d00`):
    /// `BindTarget`'s gates and relation checks (`6e6507`–`6e65ab`), then min² ≤ d² ≤ max²
    /// (`6e677c`–`6e6802`), so a unit inside the minimum is out too. There is no line-of-sight
    /// test.
    pub fn can_target_unit(&self, entity: Entity, checks: &BindChecks) -> bool {
        self.0.as_ref().is_some_and(|t| {
            checks.unit_binds(t.spell_id, t.word, entity)
                && checks.range_refusal(t.spell_id, entity).is_none()
        })
    }

    /// `0x6e61a0`: whether the world pick takes the local player, the pick flag `0x20` that
    /// `0x480610` tests at `48062c`. Never outside targeting (`480638`); while targeting, a word in
    /// `0x850e` whose spell lacks `AttributesEx & 0x80000` (`6e61cf`). An unknown spell passes.
    pub(crate) fn picks_self(&self, spells: Option<&crate::app_core::state::Spells>) -> bool {
        self.0.as_ref().is_some_and(|t| {
            t.word & SELF_PICK_BITS != 0
                && !spells
                    .and_then(|s| s.catalog.get(t.spell_id))
                    .is_some_and(|d| d.excludes_caster())
        })
    }

    /// The pending cast and its standing word, whatever seam it wants.
    fn pending(&self) -> Option<(u32, super::cast_send::CastCommit, u16)> {
        self.0.as_ref().map(|t| (t.spell_id, t.commit, t.word))
    }

    /// The pending spell when the standing word answers `wants`. A surface that draws or binds for
    /// one seam reads this, never [`Self::spell`], or an armed lockpick gets an AoE reticle.
    pub(crate) fn spell_for(&self, wants: TargetingWants) -> Option<u32> {
        self.0
            .as_ref()
            .filter(|t| wants.matches(t.word))
            .map(|t| t.spell_id)
    }

    pub fn enter(&mut self, spell_id: u32, commit: super::cast_send::CastCommit, word: u16) {
        self.0 = Some(Targeting {
            spell_id,
            commit,
            word,
            glyph: false,
        });
    }

    /// Hold a glyph's item cast for its socket.
    pub(crate) fn enter_glyph(&mut self, spell_id: u32, commit: super::cast_send::CastCommit) {
        self.0 = Some(Targeting {
            spell_id,
            commit,
            word: 0,
            glyph: true,
        });
    }

    /// The glyph wait's spell and commit.
    pub fn glyph_wait(&self) -> Option<(u32, super::cast_send::CastCommit)> {
        self.0
            .as_ref()
            .filter(|t| t.glyph)
            .map(|t| (t.spell_id, t.commit))
    }

    /// The pending cast when the standing word answers `wants`, so no commit fires on a word its
    /// seam cannot bind.
    fn pending_for(&self, wants: TargetingWants) -> Option<(u32, super::cast_send::CastCommit)> {
        self.0
            .as_ref()
            .filter(|t| wants.matches(t.word))
            .map(|t| (t.spell_id, t.commit))
    }

    /// `BindLocation 0x6e60f0`'s fork: SOURCE (`0x20`) is tested before DEST (`0x40`). The
    /// reference takes a second click for a word carrying both; no 1.12 spell does, so only the
    /// precedence is built.
    fn location_bind(&self, point: [f32; 3]) -> Option<super::cast_send::TargetedBind> {
        let word = self.0.as_ref()?.word;
        if word & 0x0020 != 0 {
            Some(super::cast_send::TargetedBind::Source(point))
        } else if word & 0x0040 != 0 {
            Some(super::cast_send::TargetedBind::Dest(point))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// Right-click cancels on the down edge (WorldFrame `OnMouseDown 0x483c40` → `0x492c20` →
/// `StopTargeting 0x6e4900`), sends nothing and consumes nothing: the press still turns the camera.
/// A held cursor payload pre-empts it (`0x492b50`), and a press over a UI frame never reaches it.
/// Whether a UI-frame right-click also cancels in the reference is untraced. The rotate-placement
/// skip on flag `0xceca90` (effect `0x51`) is not built.
pub(crate) fn cancel_targeting_on_right_press(
    mut presses: MessageReader<WorldRightPress>,
    payload_held: Res<crate::app_core::state::CursorPayloadHeld>,
    mut targeting: ResMut<SpellTargeting>,
) {
    if !targeting.active() {
        // A press buffered while idle must not replay as a cancel once the mode turns on.
        presses.clear();
        return;
    }
    if presses.read().last().is_none() || payload_held.0 {
        return;
    }
    debug!("ui_action: targeting cancelled (right-click)");
    targeting.clear();
}

/// [`SpellTargeting::picks_self`] as the frame began, which the world pick reads
/// ([`crate::target`]'s hover): the word the VM's `SpellIsTargeting` answers from, in a declared
/// order against every cast drain that arms or ends the cursor after the input pass.
#[derive(Resource, Default)]
pub struct PicksSelf(pub(crate) bool);

/// Publish [`PicksSelf`] before the input pass.
pub(crate) fn publish_picks_self(
    targeting: Res<SpellTargeting>,
    spells: Option<Res<crate::app_core::state::Spells>>,
    mut picks: ResMut<PicksSelf>,
) {
    let now = targeting.picks_self(spells.as_deref());
    if picks.0 != now {
        picks.0 = now;
    }
}

/// `BindTarget 0x6e5b40`'s unit arm, for the world click (`0x493540` at `4935d5`) and
/// `SpellTargetUnit` (`0x6e5b10`): a unit the gates or the relation checks refuse binds nothing
/// and the cursor stays up; otherwise the merge. The player's selection never moves.
fn bind_target_unit(
    ladder: &mut crate::spell::CastLadder,
    checks: &BindChecks,
    entity: Entity,
    guid: u64,
) {
    let Some((spell_id, commit, word)) = ladder.ground.pending() else {
        return;
    };
    if !checks.unit_binds(spell_id, word, entity) {
        return;
    }
    let range = checks.range_refusal(spell_id, entity);
    merge(
        ladder,
        spell_id,
        commit,
        range,
        super::cast_send::TargetedBind::Unit(guid),
    );
}

/// `BindTarget`'s merge (`6e602c`–`6e60d7`): a bind whose mask is in `0x8202`, a unit or a corpse,
/// runs the range test `0x6e47b0` (`6e6063`). Out of range raises "Out of range." or "Target too
/// close" and binds nothing; otherwise the emptied word commits (`SendCast 0x6e54f0`). The
/// reference's range failure also aborts the cast (`6e6089`), which sends CMSG_CANCEL_CAST and
/// strands the cursor (`6e5b93`) until a cancel: a bug, not copied; targeting stays armed and
/// working.
fn merge(
    ladder: &mut crate::spell::CastLadder,
    spell_id: u32,
    commit: super::cast_send::CastCommit,
    range: Option<u8>,
    bound: super::cast_send::TargetedBind,
) {
    if let Some(reason) = range {
        debug!("ui_action: cast {spell_id} not bound at {bound:x?} — range ({reason:#x})");
        ladder.cast_errors.push_local(spell_id, reason);
        return;
    }
    debug!("ui_action: cast {spell_id} committed at {bound:x?}");
    ladder.commit_targeted(spell_id, commit, bound);
}

/// The targeting cursor's script calls, applied in call order by the UI's `script_calls` pass.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ScriptCursor<'w, 's> {
    tokens: crate::target::unit_tokens::UnitTokens<'w, 's>,
    selection: Res<'w, crate::app_core::state::Selection>,
    checks: BindChecks<'w, 's>,
    ladder: crate::spell::CastLadder<'w, 's>,
}

impl ScriptCursor<'_, '_> {
    /// `SpellTargetUnit(unit)` (`0x6e6d90`). The binding already raised the usage and
    /// unknown-token errors and dropped a call made while not targeting. Here a token that names
    /// no unit raises "Out of range." (OUT_OF_RANGE) and ends targeting, as the reference's abort clears
    /// the word, and a unit goes to [`bind_target_unit`].
    pub fn spell_target_unit(&mut self, token: &str) {
        // An earlier call may have bound or ended the cast: not targeting, no-op.
        let Some(spell_id) = self.ladder.ground.spell() else {
            return;
        };
        match self.tokens.resolve(token, &self.selection) {
            Some((entity, guid)) => bind_target_unit(&mut self.ladder, &self.checks, entity, guid),
            None => {
                debug!("ui_action: SpellTargetUnit({token}) names no unit — cast {spell_id} ends");
                self.ladder
                    .cast_errors
                    .push_local(spell_id, super::validator::ERR_OUT_OF_RANGE);
                self.ladder.ground.clear();
            }
        }
    }

    /// The ESC chain's `SpellStopTargeting()` (`0x6e6e30`): `StopTargeting 0x6e4900`, word
    /// cleared, no packet.
    pub fn stop_targeting(&mut self) {
        debug!("ui_action: targeting cancelled (ESC chain)");
        self.ladder.ground.clear();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
