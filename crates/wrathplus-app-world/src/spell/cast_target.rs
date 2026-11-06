//! Cast target resolution: what goes in `CMSG_CAST_SPELL`'s target block (reference: `ArmCast
//! 0x6e5250`, `BindTarget 0x6e5b40`). The flag word is seeded from `Spell.dbc` `Targets` and
//! adjusted by the implicit-target switch (jump table `0x6e5484`), then:
//!
//! - word 0: commit at once with mask 0 and no guid, never the selection; the server fills the
//!   target from the spell's implicit targeting.
//! - `Attributes & 0x200`: the candidate is the equipped main hand (`0x6e5361`), bound as an item
//!   with no cursor; an empty hand refuses with "Your weapon hand is empty" (`0x6e504b`).
//! - otherwise the selection meets `BindTarget`'s gates ([`bind_gates`]), then each bit is cleared
//!   by its relation check; then the player behind `autoSelfCast` (`0x6e53d7`) does the same. Only a
//!   fully cleared word commits, as a unit guid.
//! - a word with item, lock, GameObject or location bits enters the targeting cursor carrying
//!   the whole word: its location (`0x6e6320`, `& 0x60`), item (`0x6e6330`, `& 0x4010`),
//!   GameObject (`0x6e62d0`, `& 0x4800`) and unit (`0x6e6460`) predicates can hold at once, and
//!   the click picks the leg.
//!
//! The unit hand cursor receives a residual unit word without the enemy bit. A word carrying
//! `0x80` instead raises the local no-target/invalid-target refusal; STRING bit 13 remains
//! unmodeled and refuses locally with "Invalid target".

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use wrathplus_formats::SpellDisplay;

use super::bind_gates::{bind_gates, unit_alive};
use super::group_relation::{GroupInputs, GroupRoster};
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfGuid;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Factions;
use crate::app_core::state::Selection;
use crate::creature_type::CreatureTypeSources;
use crate::names::NameCache;
use crate::net::{GuidIndex, Reputations};
use crate::target::{can_assist, can_attack, RelationContext};
use wrathplus_protocol::types::OwnerFallback;

/// `TARGET_FLAG_*` bits of the client's targeting flag word (`0xcecac0`).
const TF_UNIT: u16 = 0x0002;
const TF_UNIT_RAID: u16 = 0x0004;
const TF_UNIT_PARTY: u16 = 0x0008;
const TF_UNIT_ENEMY: u16 = 0x0080;
const TF_UNIT_ASSIST: u16 = 0x0100;
const TF_CORPSE_ENEMY: u16 = 0x0200;
pub(super) const TF_EXPLICIT_GATE: u16 = 0x0400;
const TF_CORPSE_ALLY: u16 = 0x8000;
/// The corpse words, `0x8600`: `BindTarget`'s dead-unit gate lets a dead unit through for any of
/// them (`6e5c7a`).
pub(super) const CORPSE_WORD_BITS: u16 = TF_CORPSE_ENEMY | TF_EXPLICIT_GATE | TF_CORPSE_ALLY;
/// The unit-shaped bits a selected unit (alive or dead) can satisfy.
const UNIT_BITS: u16 = TF_UNIT
    | TF_UNIT_RAID
    | TF_UNIT_PARTY
    | TF_UNIT_ENEMY
    | TF_UNIT_ASSIST
    | TF_CORPSE_ENEMY
    | TF_EXPLICIT_GATE
    | TF_CORPSE_ALLY;

/// Client cast-failed reasons used when an enemy word cannot bind.
pub(crate) const ERR_NO_TARGET: u8 = crate::spell::cast_fail_keys::reason::BAD_IMPLICIT_TARGETS;
pub(crate) const ERR_INVALID_TARGET: u8 = crate::spell::cast_fail_keys::reason::BAD_TARGETS;

/// `SPELL_FAILED_MAINHAND_EMPTY`, "Your weapon hand is empty": the reference's own refusal for an
/// imbue with an empty main hand (`0x6e5050`).
pub(crate) const ERR_MAINHAND_EMPTY: u8 = crate::spell::cast_fail_keys::reason::MAINHAND_EMPTY;

/// The dest-location bit, the ground-cast wire mask (`BindLocation 0x6e60f0`).
const TF_DEST_LOCATION: u16 = 0x0040;
/// The source-location bit, `BindLocation 0x6e60f0`'s other arm (`0x6e6105`).
const TF_SOURCE_LOCATION: u16 = 0x0020;
/// `TargetingWantsLocation 0x6e6320`'s mask: the terrain click serves both location bits.
const LOCATION_BITS: u16 = TF_SOURCE_LOCATION | TF_DEST_LOCATION;
/// The item bit, asked by the bag and paper-doll click seams (`TargetingWantsItem 0x6e6330`).
const TF_ITEM: u16 = 0x0010;
/// Build 12340's `TARGET_FLAG_GLYPH_SLOT`, above the 16-bit word: the cast waits for a socket.
pub(crate) const TF_GLYPH_SLOT: u32 = 0x2_0000;
/// `TARGET_FLAG_LOCKED`: in both the item mask `0x4010` and the GameObject mask `0x4800`, so one
/// lock spell ends on a bag item or a world object. It never reaches the wire: `BindTarget`
/// writes ITEM (`0x6e5f2e`) or GAMEOBJECT (`0x6e5f69`) instead, which vmangos accepts for a
/// lock spell (`Spell.cpp:6755`).
const TF_LOCKED: u16 = 0x4000;
/// `TARGET_FLAG_GAMEOBJECT`, the other bit of `TargetingWantsGameObject 0x6e62d0`'s `0x4800`.
const TF_GAMEOBJECT: u16 = 0x0800;
/// The bits `BindTarget`'s item arm admits and clears (`0x6e5f1e`, `0x6e5f36`).
const ITEM_ARM_BITS: u16 = TF_ITEM | TF_LOCKED;

/// What the wire's target block should carry for this cast.
#[derive(Debug, PartialEq, Eq)]
pub enum CastWireTarget {
    /// Word 0: mask 0, no guid; the server resolves the target implicitly.
    SelfImplicit,
    /// Mask `TARGET_FLAG_UNIT` (0x2) and this guid, possibly the player's own.
    Unit(u64),
    /// Mask `TARGET_FLAG_ITEM` (0x10) and this item guid, sent on the press: only the main-hand
    /// imbue binds an item from the cast itself; every other item target comes from a click.
    Item(u64),
    /// No send yet: enter the targeting cursor with this flag word (reference: word != 0 is
    /// `IsTargeting 0x6e48a0`). Each click seam tests the word with its own mask, so one word can
    /// serve several: Opening and Pick Lock take a bag item or a world GameObject.
    Targeting(u16),
    /// Do not send; show this client error.
    Refused(u8),
    /// A refusal the reference raises in its cast tail after `ArmCast` returns false (`0x6e5045`).
    /// That tail runs after the requirement validator `0x6094f0`, so it fires at the cursor-entry
    /// point in [`super::send_spell_cast`], not at the resolver's return.
    RefusedAtArm(u8),
}

/// Where `ArmCast 0x6e5250` takes its candidate guid from (`0x6e5361`-`0x6e539f`). The main hand
/// is taken instead of the selection, never before it; `caster` is the separate `autoSelfCast`
/// fallback (`0x6e53d7`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CastCandidates {
    /// The current selection (the client's `0xb4e2d8`); WrathPlus never passes an explicit guid.
    pub selection: Option<u64>,
    pub caster: Option<u64>,
    /// The equipped main hand's item guid (`[player+0x1d3c][15]`), only when it resolves to a
    /// streamed item object (`0x468460` at `0x6e53bc`).
    pub main_hand_item: Option<u64>,
}

/// What the binder's gates and relation checks read: the selected unit's store and the player's.
#[derive(Clone, Copy)]
pub struct TargetRelations<'a> {
    pub target_store: Option<&'a ObjectStore>,
    /// The target's charmer, creator or summoner store, for `CanAssist`'s `IsPvP` owner chase.
    pub target_owner_store: Option<&'a ObjectStore>,
    pub self_store: Option<&'a ObjectStore>,
    /// `CanAttack`/`CanAssist`'s live inputs: creature metadata and the per-hop guid lookup.
    pub relations: RelationContext<'a>,
    pub factions: Option<&'a Factions>,
    pub reputations: &'a Reputations,
    /// The creature-type gate's inputs, for the candidate and for the player alike.
    pub types: CreatureTypeSources<'a>,
    /// The party and raid predicates' inputs (`0x606c20`, `0x606d20`).
    pub group: GroupInputs<'a>,
}

/// A unit's owner as `CanAssist`'s `IsPvP` chase (`0x5ee5a0`) and `0x606170` read it, its charmer
/// else its creator: that object's descriptor, if it has streamed (`0x468460`).
pub(super) fn owner_store<'a>(
    store: Option<&ObjectStore>,
    index: Option<&GuidIndex>,
    stores: &'a Query<&ObjectStore>,
) -> Option<&'a ObjectStore> {
    let owner = store?.0.unit_owner(OwnerFallback::CreatedBy)?;
    stores.get(*index?.0.get(&owner)?).ok()
}

impl TargetRelations<'_> {
    /// `CanAssist 0x6066f0` toward the candidate, which the caster always is for himself. The
    /// binder's assist arm and the world pick's relation leg both ask it.
    pub(super) fn assistable(&self, is_self: bool) -> bool {
        is_self
            || can_assist(
                self.target_store,
                self.factions,
                self.reputations,
                self.self_store,
                self.relations,
            )
    }

    /// `CanAttack 0x606980` toward the candidate, never the caster. The binder's enemy arm and the
    /// world pick's relation leg both ask it.
    pub(super) fn attackable(&self, is_self: bool) -> bool {
        !is_self
            && can_attack(
                self.target_store,
                self.factions,
                self.reputations,
                self.self_store,
                self.relations,
            )
    }
}

/// The targeting inputs [`super::send_spell_cast`] resolves with, built by both cast callers.
pub struct CastContext<'a> {
    pub(crate) selection_guid: Option<u64>,
    pub(crate) self_guid: Option<u64>,
    pub(crate) auto_self_cast: bool,
    pub rel: TargetRelations<'a>,
    /// The local range gate's inputs (`IsTargetInRange 0x6e47b0`).
    pub(crate) range: RangeInputs,
    /// The main hand's item guid off the player descriptor (`PLAYER_FIELD_INV_SLOT_HEAD + 2*15`);
    /// the send path checks that it names a streamed item.
    pub(crate) main_hand_item: Option<u64>,
    /// The caster's movement flags (the client's `[unit+0x9e8]`), read by the requirement
    /// validator's moving gate.
    pub(crate) self_move_flags: u32,
}

impl CastContext<'_> {
    /// Applies the self-cast modifier, `onSelf`: the caster replaces the selection as the bind
    /// candidate, guid and store both, the pair the autoSelfCast fallback swaps (`0x6e53d7`). It
    /// is the explicit guid `CastSpell` and `CastSpellByName` hand `ArmCast 0x6e5250` (`0x4b4345`,
    /// `0x4b4afa`, bound at `0x6e5393`); where `UseAction 0x4e5ee0` applies its own is untraced.
    pub fn self_bound(mut self, on_self: bool) -> Self {
        if on_self {
            self.selection_guid = self.self_guid;
            self.rel.target_store = self.rel.self_store;
            self.rel.group.target_guid = self.self_guid;
        }
        self
    }
}

/// Positions and the two units for the pre-send range refusal: the client's cast runs
/// `CanTargetUnit 0x6e4440` and `IsTargetInRange 0x6e47b0` before the commit, so an out-of-range
/// press refuses locally and none of the commit tail runs.
#[derive(Clone, Copy)]
pub(crate) struct RangeInputs {
    pub(crate) self_pos: Option<Vec3>,
    pub(crate) target_pos: Option<Vec3>,
    /// The caster's combat reach and motion.
    pub(crate) caster: wrathplus_formats::RangeUnit,
    /// The bound unit, which `GetMinMaxRange` reads for the ranged pad and the moving bonus.
    pub(crate) target: Option<wrathplus_formats::RangeUnit>,
}

impl RangeInputs {
    /// The one range compare: the squared 3D distance against `GetMinMaxRange 0x7ff480`'s bounds,
    /// "Out of range." (0x59) past max², "Target too close" (0x76) inside a nonzero min². The
    /// pre-send gate (`IsTargetInRange 0x6e47b0`) and the targeting cursor's unit leg (`0x6e6460`,
    /// `6e677c`–`6e6802`) both ask it. No row or no position passes; the server judges.
    pub(crate) fn refusal(
        &self,
        def: &SpellDisplay,
        row: Option<&wrathplus_formats::SpellRange>,
        mods: &super::SpellModifiers,
    ) -> Option<u8> {
        let dist_sq = self
            .self_pos
            .zip(self.target_pos)
            .map(|(a, b)| a.distance_squared(b));
        super::validator::cast_range_refusal(def, row, self.caster, self.target, dist_sq, mods)
    }
}

impl Default for RangeInputs {
    fn default() -> Self {
        Self {
            self_pos: None,
            target_pos: None,
            caster: wrathplus_formats::RangeUnit::still(super::DEFAULT_REACH),
            target: None,
        }
    }
}

/// Everything a cast-sending system needs to build a [`CastContext`].
#[derive(SystemParam)]
pub struct CastTargeting<'w, 's> {
    pub selection: Res<'w, Selection>,
    pub self_store: Query<'w, 's, &'static ObjectStore, With<SelfPlayer>>,
    stores: Query<'w, 's, &'static ObjectStore>,
    index: Option<Res<'w, GuidIndex>>,
    self_guid: Res<'w, SelfGuid>,
    auto_self_cast: Res<'w, AutoSelfCast>,
    factions: Option<Res<'w, Factions>>,
    reputations: Res<'w, Reputations>,
    /// The party slots and raid roster as the frame began; a harness without them has no group.
    roster: Option<Res<'w, GroupRoster>>,
    /// The creature-type gate's template cache and form table; a harness without them resolves
    /// no type.
    names: Option<Res<'w, NameCache>>,
    spells: Option<Res<'w, crate::app_core::state::Spells>>,
    self_transform: Query<'w, 's, &'static Transform, With<SelfPlayer>>,
    transforms: Query<'w, 's, &'static Transform>,
    /// `Option`: the body exists only in world, and the cast-result handler must fetch this param
    /// anywhere. With no body the moving gate passes and the server decides.
    player: Option<Res<'w, crate::app_core::player::Player>>,
    /// The caster's and the selection's reach and motion, for the range gate.
    range_units: super::RangeUnits<'w, 's>,
}

impl crate::target::StoreLookup for CastTargeting<'_, '_> {
    fn store(&self, guid: u64) -> Option<&ObjectStore> {
        crate::target::guid_store(self.index.as_deref(), &self.stores, guid)
    }
}

impl CastTargeting<'_, '_> {
    /// This frame's [`CastContext`].
    pub fn context(&self) -> CastContext<'_> {
        let target_store = self.selection.target.and_then(|e| self.stores.get(e).ok());
        let owner_store = |store| owner_store(store, self.index.as_deref(), &self.stores);
        let self_store = self.self_store.iter().next();
        CastContext {
            selection_guid: self.selection.guid,
            self_guid: self.self_guid.0,
            auto_self_cast: self.auto_self_cast.0,
            rel: TargetRelations {
                target_store,
                target_owner_store: owner_store(target_store),
                self_store,
                relations: RelationContext::new(self.names.as_deref(), self),
                factions: self.factions.as_deref(),
                reputations: &self.reputations,
                types: CreatureTypeSources {
                    names: self.names.as_deref(),
                    forms: self.spells.as_deref().map(|s| &s.forms),
                },
                group: GroupInputs {
                    self_guid: self.self_guid.0,
                    target_guid: self.selection.guid,
                    self_owner_store: owner_store(self_store),
                    roster: self.roster.as_deref(),
                },
            },
            range: RangeInputs {
                self_pos: self.self_transform.iter().next().map(|t| t.translation),
                target_pos: self
                    .selection
                    .target
                    .and_then(|e| self.transforms.get(e).ok())
                    .map(|t| t.translation),
                caster: self.range_units.caster(),
                target: self.selection.target.and_then(|e| self.range_units.unit(e)),
            },
            main_hand_item: self
                .self_store
                .iter()
                .next()
                .and_then(|s| s.0.player_inv_slot(EQUIPMENT_SLOT_MAINHAND))
                .filter(|&g| g != 0),
            self_move_flags: self.player.as_deref().map_or(0, |p| p.move_flags()),
        }
    }
}

/// `EQUIPMENT_SLOT_MAINHAND` (the client reads it at `0x6e538b`, offset `0x78 == 15 * 8`).
const EQUIPMENT_SLOT_MAINHAND: u8 = 15;

/// The `autoSelfCast` CVar (name `0x870dc0`, read at `0x6e53d7`); build 12340 registers `"1"`
/// (`0x51dbf8`).
#[derive(bevy::prelude::Resource)]
pub struct AutoSelfCast(pub(crate) bool);

impl Default for AutoSelfCast {
    fn default() -> Self {
        Self(true)
    }
}

/// `autoSelfCast`'s change callback: a flag.
pub fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut auto: ResMut<AutoSelfCast>) {
    if ev.is("autoSelfCast") {
        auto.0 = ev.flag();
    }
}

/// The flag word: `Targets`, then one arm keyed on `EffectImplicitTargetA[0]` (`0x6e525a`-
/// `0x6e52ef`); every implicit target not matched below leaves the word as it is.
pub fn cast_target_mask(def: &SpellDisplay) -> u16 {
    let mut word = def.targets as u16;
    match def.implicit_target_a1 {
        1 => word &= !TF_EXPLICIT_GATE,
        5 => word &= !TF_CORPSE_ALLY,
        6 | 53 => word |= TF_UNIT_ENEMY,
        // 16 (ground target) sets a cursor-mode flag, not a word bit; [`resolve_cast_target`]
        // tests it.
        21 | 45 => word |= TF_UNIT_ASSIST,
        23 => word |= 0x0800,
        25 | 63 => word |= TF_UNIT,
        26 => word |= 0x4000,
        35 => word |= TF_UNIT_PARTY,
        57 | 61 => word |= TF_UNIT_RAID,
        _ => {}
    }
    word
}

/// `BindTarget 0x6e5b40`'s unit branch past the gates: clear every flag-word bit the candidate
/// satisfies.
///
/// The assist bit asks `CanAssist 0x6066f0`: selectable, friendly or better, and an NPC's owner
/// (or the NPC itself) PvP-enabled. This keeps friendly ambient NPCs and critters unbindable.
/// The enemy bit asks `CanAttack` (`0x606980`). The party bit needs `0x606c20` and then `CanAssist`
/// (`6e5cad`-`6e5cec`): the player, a member of his party, or a unit that answers to one. The raid
/// bit needs `0x606d20` and `CanAssist` (`6e5cf1`-`6e5d2f`): the same, with the raid roster
/// counting as well as the party slots. The corpse check (`0x6067d0`) is assistable with health 0
/// here.
fn clear_satisfied_bits(word: u16, is_self: bool, rel: &TargetRelations) -> u16 {
    let mut word = word;
    let assist = rel.assistable(is_self);
    let dead = rel.target_store.is_some_and(|s| !unit_alive(&s.0));
    if word & TF_UNIT_PARTY != 0 && assist && rel.in_party(is_self) {
        word &= !TF_UNIT_PARTY;
    }
    if word & TF_UNIT_RAID != 0 && assist && rel.in_group(is_self) {
        word &= !TF_UNIT_RAID;
    }
    if word & TF_UNIT_ASSIST != 0 && assist {
        word &= !TF_UNIT_ASSIST;
    }
    if word & TF_UNIT_ENEMY != 0 && rel.attackable(is_self) {
        word &= !TF_UNIT_ENEMY;
    }
    // Bit 1 has no relation check: any unit binds. A dead unit's bind clears bit 10 (`6e5d89`,
    // `6e5ddb`); the gate refuses a living unit under that word before it gets here.
    if word & TF_UNIT != 0 {
        word &= !TF_UNIT;
    }
    if word & TF_EXPLICIT_GATE != 0 && dead {
        word &= !TF_EXPLICIT_GATE;
    }
    if word & TF_CORPSE_ALLY != 0 && assist && dead {
        word &= !TF_CORPSE_ALLY;
    }
    if word & TF_CORPSE_ENEMY != 0 && !is_self && dead {
        word &= !TF_CORPSE_ENEMY;
    }
    word
}

/// Whether the unit arm's relation checks clear the whole standing word for this unit.
fn unit_word_binds(word: u16, is_self: bool, rel: &TargetRelations) -> bool {
    word & UNIT_BITS != 0 && clear_satisfied_bits(word, is_self, rel) == 0
}

/// Whether `BindTarget 0x6e5b40`'s unit branch binds this unit under the standing word: the gates
/// ([`bind_gates`]), then the relation checks. The initial selection, the `autoSelfCast` fallback,
/// a world click, `SpellTargetUnit` and the hover verdict all ask this one predicate. An unknown
/// spell has no row to gate on and leaves the relation checks alone.
pub(super) fn unit_binds(
    def: Option<&SpellDisplay>,
    word: u16,
    is_self: bool,
    rel: &TargetRelations,
) -> bool {
    def.is_none_or(|def| bind_gates(def, word, is_self, rel)) && unit_word_binds(word, is_self, rel)
}

/// The wire target for casting `def`. An unknown spell sends the selection as is, or no target
/// without one, and the server validates.
/// The targeting-cursor word a pet's spell raises, as [`resolve_cast_target`] raises ours: an
/// implicit-target-16 spell or a location-only word; `None` where the pet's order names a unit
/// (`CMSG_PET_ACTION`). Build 12340 then sends the commit as `CMSG_PET_CAST_SPELL` with the pet as
/// caster (`0x80ac90`); that the bar raises the cursor for these is inferred, the press path
/// untraced.
pub fn pet_cursor_word(def: &SpellDisplay) -> Option<u16> {
    let word = cast_target_mask(def);
    if def.implicit_target_a1 == 16 {
        return Some(word | TF_DEST_LOCATION);
    }
    (word & LOCATION_BITS != 0 && word & UNIT_BITS == 0).then_some(word)
}

pub fn resolve_cast_target(
    def: Option<&SpellDisplay>,
    cand: &CastCandidates,
    auto_self_cast: bool,
    rel: &TargetRelations,
) -> CastWireTarget {
    let Some(def) = def else {
        return match cand.selection {
            Some(guid) => CastWireTarget::Unit(guid),
            None => CastWireTarget::SelfImplicit,
        };
    };
    let word = cast_target_mask(def);
    if word == 0 {
        return CastWireTarget::SelfImplicit;
    }
    // Implicit target 16 enters the cursor before any candidate bind (`0x6e535b`), but after the
    // word-0 commit above (`0x6e5338` comes first).
    if def.implicit_target_a1 == 16 {
        // The reference flags this outside the word; the click seams read only the word, so the
        // DEST bit carries it (shipped arm-16 rows already have it in `Targets`).
        return CastWireTarget::Targeting(word | TF_DEST_LOCATION);
    }
    // `Attributes & 0x200` takes the main hand as the candidate (`0x6e5361`-`0x6e5391`). This
    // must precede the non-unit cursor defer below: an item candidate clears bit 4.
    if def.targets_main_hand_item() {
        match cand.main_hand_item {
            // `BindTarget`'s item arm clears bits 4 and 14 (`0x6e5f35`) and the cleared word
            // commits at once (`0x6e60c1`).
            Some(item) if word & ITEM_ARM_BITS != 0 && word & !ITEM_ARM_BITS == 0 => {
                return CastWireTarget::Item(item)
            }
            // Empty hand: not a cursor. The cast tail re-tests the attribute (`0x6e504b`), clears
            // the word (`0x6e5057`) and raises MAINHAND_EMPTY, jumping over the cursor arm
            // (`0x6e50ab`).
            None => return CastWireTarget::RefusedAtArm(ERR_MAINHAND_EMPTY),
            // A residual the item arm cannot clear waits at the cursor in the reference; no
            // shipped row has one (all 22 carry `Targets == 0x10`).
            Some(_) => {}
        }
    }
    // A non-unit bit enters the cursor with the whole word. The reference only reaches the cursor
    // after the unit walk fails (`0x6e50c8`), so deferring first matches it only because no unit
    // candidate clears these bits; a location word with a unit bit beside it (0x42, NPC-cast
    // spell 28218) binds the unit first in the reference, so it refuses here instead.
    if word & !UNIT_BITS != 0 {
        if word & LOCATION_BITS != 0 && word & UNIT_BITS == 0
            || word & (TF_ITEM | TF_LOCKED | TF_GAMEOBJECT) != 0
        {
            return CastWireTarget::Targeting(word);
        }
        return CastWireTarget::Refused(ERR_INVALID_TARGET);
    }
    // The selection (`0xb4e2d8`, `0x6e539f`). A unit the gates or the relation checks refuse binds
    // nothing, so the fallback below gets its turn.
    if let Some(guid) = cand.selection {
        let is_self = cand.caster == Some(guid);
        if unit_binds(Some(def), word, is_self, rel) {
            return CastWireTarget::Unit(guid);
        }
    }
    // The fallback: the active player (`0x6e53d7`), behind autoSelfCast, through the same gates.
    if auto_self_cast {
        if let Some(guid) = cand.caster {
            let self_rel = TargetRelations {
                target_store: rel.self_store,
                target_owner_store: None,
                group: GroupInputs {
                    target_guid: Some(guid),
                    ..rel.group
                },
                ..*rel
            };
            if unit_binds(Some(def), word, true, &self_rel) {
                return CastWireTarget::Unit(guid);
            }
        }
    }
    // A standing enemy word refuses locally. Every other residual unit word proceeds to
    // `BindTarget`'s hand cursor.
    if word & TF_UNIT_ENEMY != 0 {
        CastWireTarget::Refused(if cand.selection.is_some() {
            ERR_INVALID_TARGET
        } else {
            ERR_NO_TARGET
        })
    } else {
        CastWireTarget::Targeting(word)
    }
}

#[path = "cast_target_facts.rs"]
mod facts;

#[cfg(test)]
#[path = "cast_target_tests.rs"]
mod tests;
