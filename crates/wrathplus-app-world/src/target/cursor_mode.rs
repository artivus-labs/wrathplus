//! The world cursor's classifier: the hovered unit, GameObject or corpse resolves to a
//! [`CursorKind`] as the reference's `CGWorldFrame` classifier does (`0x4828d0`, unit branch
//! `0x482200`): the NPC service ladder, else loot, skin or attack by state, each grayed by its own
//! range gate. Loot rights never gray; they decide whether the loot cursor shows at all.

use bevy::prelude::*;

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::net::Reputations;

use super::ring::Factions;
use super::{go_is_nearest, Hovered, HoveredObject};

/// The world cursor modes WrathPlus shows, named by the reference's mode table (`0x853b8c`), whose
/// strings are the `Interface\Cursor\<Name>.blp` stems.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorKind {
    Point,
    Attack,
    Speak,
    /// Pickup(8): the vendor's pouch and the loot leg's base mode.
    Pickup,
    /// LootAll(16): the loot leg's triple pouch while the effective auto-loot is on; the vendor
    /// pouch never triples.
    LootAll,
    /// Interact(5): the gear, for a GameObject with no data-named cursor and for the innkeeper.
    Interact,
    Buy,
    /// Inspect(7): the UI's Ctrl-hover cursor (`ShowInspectCursor 0x48ac60`) and the world cursor
    /// over a readable TEXT(9) GameObject (`0x5f5890`).
    Inspect,
    Trainer,
    Taxi,
    Skin,
    /// Repair(17): UI repair mode in Vanilla (`ShowRepairCursor 0x4fbcc0`), also the
    /// existing repair asset used for Wrath repair-service NPCs.
    Repair,
    /// Mail(15): a MAILBOX(19) or type-28 GameObject (`0x5f6840`, `0x5f6e30`).
    Mail,
    /// Mine(11): a GameObject whose lock's first `LockType` is Mining (3), named by LockType.dbc
    /// (`0x5f3070`).
    Mine,
    /// GatherHerbs(13): the same, for Herbalism (2).
    GatherHerbs,
    /// PickLock(14): the same, for Pick Lock (1); never grayed, since `0x5f3070` skips the usable
    /// gate for `LockType.Id == 1`.
    PickLock,
    /// Cast(2): the spell-targeting cursor (`0x4820f0`), which pre-empts the whole object
    /// classifier while a spell awaits a target; the classifier here never sets it.
    Cast,
}

impl CursorKind {
    /// The cursor's BLP stem in `Interface\Cursor\`.
    fn name(self) -> &'static str {
        match self {
            CursorKind::Point => "Point",
            CursorKind::Attack => "Attack",
            CursorKind::Speak => "Speak",
            CursorKind::Pickup => "Pickup",
            CursorKind::LootAll => "LootAll",
            CursorKind::Interact => "Interact",
            CursorKind::Buy => "Buy",
            CursorKind::Inspect => "Inspect",
            CursorKind::Trainer => "Trainer",
            CursorKind::Taxi => "Taxi",
            CursorKind::Skin => "Skin",
            CursorKind::Repair => "Repair",
            CursorKind::Mail => "Mail",
            CursorKind::Mine => "Mine",
            CursorKind::GatherHerbs => "GatherHerbs",
            CursorKind::PickLock => "PickLock",
            CursorKind::Cast => "Cast",
        }
    }
}

/// This frame's world cursor, written by [`classify_cursor`]; `unable` picks the grayed
/// `Unable<Name>` twin (the reference's `mode + 20`).
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WorldCursor {
    pub(crate) kind: CursorKind,
    pub(crate) unable: bool,
}

impl Default for WorldCursor {
    fn default() -> Self {
        Self {
            kind: CursorKind::Point,
            unable: false,
        }
    }
}

impl WorldCursor {
    /// The BLP stem (`Attack`, `UnableAttack`), the key the platform cursor caches use.
    pub(crate) fn stem(&self) -> String {
        if self.unable && self.kind != CursorKind::Point {
            format!("Unable{}", self.kind.name())
        } else {
            self.kind.name().to_string()
        }
    }
}

/// `UNIT_NPC_FLAGS` bits, build 12340's values (AzerothCore `NPCFlags`), the raw word
/// [`wrathplus_protocol::types::NpcFlags`] reads; the ladder never tests REPAIR (`0x1000`). The live
/// code reads services through `NpcFlags`; outside the tests only the registrar's pair is used.
pub mod npc_flags {
    #[cfg(any(test, feature = "test-support"))]
    pub const GOSSIP: u32 = 0x1;
    #[cfg(test)]
    pub const QUESTGIVER: u32 = 0x2;
    #[cfg(test)]
    pub const TRAINER: u32 = 0x10;
    #[cfg(test)]
    pub const VENDOR: u32 = 0x80;
    #[cfg(test)]
    pub const FLIGHTMASTER: u32 = 0x2000;
    #[cfg(test)]
    pub const SPIRITHEALER: u32 = 0x4000;
    #[cfg(test)]
    pub const SPIRITGUIDE: u32 = 0x8000;
    #[cfg(test)]
    pub const INNKEEPER: u32 = 0x1_0000;
    #[cfg(test)]
    pub const BANKER: u32 = 0x2_0000;
    pub const PETITIONER: u32 = 0x4_0000;
    pub const TABARDDESIGNER: u32 = 0x8_0000;
    #[cfg(test)]
    pub const BATTLEMASTER: u32 = 0x10_0000;
    #[cfg(test)]
    pub const AUCTIONEER: u32 = 0x20_0000;
    #[cfg(test)]
    pub const STABLEMASTER: u32 = 0x40_0000;
}

pub(super) const UNIT_FLAG_SKINNABLE: u32 = 0x0400_0000;
const UNIT_FLAG_PACIFIED: u32 = 0x0002_0000;

/// The hovered unit is one the unit dispatcher `0x60bea0` forks to its attack arm for (`0x60c192`):
/// alive or feigning, refused by `CanInteract` and passed by `CanAttack`. The sword also needs the
/// player's own legs ([`player_attack_legs`]), which the arm tests again inside.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct AttackFork(pub(crate) bool);

/// The player's legs of the classifier's attack leg (`0x482648`–`0x4826a2`), in its order: not
/// dead or a ghost (`0x48264a`), not mounted (`0x482661`), not pacified (`0x48268f`) and in
/// control (`[0xb4b3e4]`, `0x48269b`). Any failure clears to Point (`0x4826cb`).
pub(crate) fn player_attack_legs(own: &ObjectStore, control_lost: bool) -> bool {
    !own.0.is_dead_or_ghost()
        && own.0.unit_mount_display_id() == 0
        && own.0.unit_flags() & UNIT_FLAG_PACIFIED == 0
        && !control_lost
}

/// NPC-service range, squared: gray beyond 5.5556 yd (`0xb4b32c` = `[0x804328]²`, checked
/// boundary-inclusive at `0x482320`). The NPC windows' walk-away close uses the same constant.
pub const SERVICE_RANGE_SQ: f32 = 30.864;
/// Attack's range, squared: gray beyond a fixed 10.45 yd (`0x80447c`, checked at `0x4826a7`), not
/// the melee reach.
const ATTACK_RANGE_SQ: f32 = 109.2025;
/// Melee interact reach `max(reachA + reachB + 1.333, 5.0)` (`0x80b058`, `0x80a1e8`): 5.0 is a
/// floor, so large creatures reach farther. Gates skin (`0x7ff480`) and loot (`CanLootNow
/// 0x5ec110`), center to center, boundary-inclusive.
pub(super) const MELEE_OFFSET: f32 = 1.333_33;
pub(super) const MELEE_FLOOR: f32 = 5.0;

pub(super) fn melee_reach(target: &ObjectStore, me: &ObjectStore) -> f32 {
    (target.0.unit_combat_reach() + me.0.unit_combat_reach() + MELEE_OFFSET).max(MELEE_FLOOR)
}

/// A corpse's interact reach, squared: a flat 5 yd, since a corpse has no combat reach and the
/// melee formula falls to its floor. `CanLootNow 0x5ec110` and the `CMSG_LOOT` sender `0x5df130`
/// both compare against 25.0.
pub(super) const CORPSE_INTERACT_RANGE_SQ: f32 = 25.0;

/// `GAMEOBJECT_TYPE_GENERIC`: decoration whose highlightable slot is constant false (`0x5f47f0`),
/// so it never shows an interact cursor.
pub const GO_TYPE_GENERIC: i32 = 5;
/// The transport family, TRANSPORT(11), MAP_OBJECT(14) and MO_TRANSPORT(15): both strategy slots
/// are constant false (vtables `0x80ba58`, `0x80b710`, `0x80b798`; `+0x14` is `0x5f5c70` or
/// `0x5f48b0`), so a boat or elevator never shows a cursor, a tooltip or a right-click use.
const GO_TYPE_TRANSPORT: i32 = 11;
const GO_TYPE_MAP_OBJECT: i32 = 14;
const GO_TYPE_MO_TRANSPORT: i32 = 15;
/// The marker set, SPELL_FOCUS(8), DUEL_ARBITER(16), FISHINGHOLE(25) and AURA_GENERATOR(30):
/// mouseover slot `+0xc` constant true, highlightable slot `+0x14` constant false (vtables
/// `0x80b8a8`, `0x80bb68`, `0x80bbf8`, `0x80c2a0`). An anvil or a duel flag tooltips and brightens
/// but shows the plain pointer and ignores the right-click (`OnUse 0x5f8660` asks `+0x14`).
const GO_TYPE_SPELL_FOCUS: i32 = 8;
const GO_TYPE_DUEL_ARBITER: i32 = 16;
const GO_TYPE_FISHINGHOLE: i32 = 25;
const GO_TYPE_AURA_GENERATOR: i32 = 30;
/// The highest type the type factory's jump table (`0x5f76cc`) covers; 21 GUARDPOST has no case
/// and takes `default:` with every out-of-range type.
const GO_TYPE_MAX: i32 = 30;
const GO_TYPE_GUARDPOST: i32 = 21;

/// Whether the type factory gives this type its `default:` strategy (`0x5f76a0`): a static
/// placeholder (vtable `0x80b188`) whose `+0x14` (`0x5f36f0`) and `+0xc` both answer false, so the
/// object shows nothing. No 1.12 data sends such a type; the flag predicate alone would pass it.
fn strategy_is_default(type_id: i32) -> bool {
    type_id == GO_TYPE_GUARDPOST || !(0..=GO_TYPE_MAX).contains(&type_id)
}
/// `GAMEOBJECT_TYPE_TEXT` (9), a readable book or plaque: the Inspect magnifier (`0x5f5890`).
pub const GO_TYPE_TEXT: i32 = 9;
/// MAILBOX(19) (`0x5f6840`) and type 28 (`0x5f6e30`) show Mail; no 1.12 data uses type 28.
/// RITUAL(18) shows Mail here, where the reference gives it the base cursor: its own vtable
/// `0x80bd10` keeps `0x5f3070`.
const GO_TYPE_RITUAL: i32 = 18;
pub(super) const GO_TYPE_MAILBOX: i32 = 19;
/// GUILD_BANK (34), the guild vault.
pub(super) const GO_TYPE_GUILD_BANK: i32 = 34;
const GO_TYPE_28: i32 = 28;
/// Every other type's interact range: the NPC-service reach, standing in for the reference's
/// per-type strategy constant (`[strat+0xc]`, compared squared by `usable 0x5f3130`; base 5.0,
/// overridden by several types), which is not transcribed.
const GO_INTERACT_RANGE_SQ: f32 = SERVICE_RANGE_SQ;
/// `GAMEOBJECT_TYPE_FISHINGNODE` (17), the bobber: its strategy (vtable `0x80bc80`) overrides only
/// the highlightable slot, and its interact range is 100 yd (`0x5f66b0`, `[0x80b0b0]`).
pub(crate) const GO_TYPE_FISHINGNODE: i32 = 17;
/// `GAMEOBJECT_TYPE_CHAIR` (7): its own range predicate `0x5f5670` accepts within 3.0 yd
/// (`[0xc4d808]`, written only by `0x5f98b0` as 3.0 squared).
pub(crate) const GO_TYPE_CHAIR: i32 = 7;
pub(super) fn go_interact_range_sq(type_id: i32) -> f32 {
    match type_id {
        GO_TYPE_FISHINGNODE => 100.0 * 100.0,
        // Measured to the object; the reference (seats from `0x5f5760`) and vmangos
        // (`GameObject.cpp:2231-2236`) measure to the nearest seat, so only a multi-seat bench
        // differs, by up to half its span.
        GO_TYPE_CHAIR => 9.0,
        _ => GO_INTERACT_RANGE_SQ,
    }
}
/// `GameObjectFlags` IN_USE (`0x1`) and NO_INTERACT (`0x10`), which `0x5f2f80` rejects together.
const GO_FLAG_IN_USE_OR_NO_INTERACT: u32 = 0x11;
/// `GO_FLAG_INTERACT_COND`: usable only while the per-player activate bit is set, the quest gate.
const GO_FLAG_INTERACT_COND: u32 = 0x4;
/// `GO_DYNFLAG_LO_ACTIVATE` in `GAMEOBJECT_DYN_FLAGS`, set per player by the server
/// (`GameObject::ActivateToQuest`); read only under `INTERACT_COND`.
const GO_DYNFLAG_ACTIVATE: u32 = 0x1;

/// The types whose highlightable slot `+0x14` is constant false, so `0x5f2f80` never runs.
fn strategy_never_highlightable(type_id: i32) -> bool {
    strategy_is_default(type_id)
        || matches!(
            type_id,
            GO_TYPE_GENERIC
                | GO_TYPE_TRANSPORT
                | GO_TYPE_MAP_OBJECT
                | GO_TYPE_MO_TRANSPORT
                | GO_TYPE_SPELL_FOCUS
                | GO_TYPE_DUEL_ARBITER
                | GO_TYPE_FISHINGHOLE
                | GO_TYPE_AURA_GENERATOR
                | GO_TYPE_AUCTIONHOUSE
                | GO_TYPE_CAPTURE_POINT
        )
}

/// `GAMEOBJECT_TYPE_AUCTIONHOUSE` (20): both slots false (`0x5f68a0`), so it shows nothing and
/// `OnUse` returns unsent (`0x5f8673`); the auctioneer players click is a unit.
const GO_TYPE_AUCTIONHOUSE: i32 = 20;
/// `GAMEOBJECT_TYPE_CAPTURE_POINT` (29): never a cursor (`0x5f6d40`), but a tooltip and brighten
/// when its highlight column `data[19]` is set (`0x5f6d80`), GENERIC's rule. No 1.12 data uses it.
const GO_TYPE_CAPTURE_POINT: i32 = 29;
/// Inputs of the two types whose `+0x14` is a predicate of its own rather than `0x5f2f80` or a
/// constant; the caller resolves them, and each is ignored for every other type.
#[derive(Clone, Copy, Default)]
pub struct GoOverrides {
    /// FISHINGNODE: the local player is channeling at this bobber.
    pub channel_owned: bool,
    /// MEETINGSTONE: this stone's area is the one we are queued at.
    pub meeting_stone_queued: bool,
    /// TRAP: its template's lock id names no `Lock.dbc` row, so 12340's gate (`0x7112a0` via
    /// `0x70ef30`) refuses it, cursor and mouseover alike. False while either is unknown.
    pub trap_lockless: bool,
}

/// `GAMEOBJECT_TYPE_MEETINGSTONE` (23): its `+0x14` (`0x5f6990`) is only
/// `template.data[2] != [0xb72038]`, the queued area; it never calls `0x5f2f80`.
pub(super) const GO_TYPE_MEETINGSTONE: i32 = 23;

/// Whether a GameObject shows an interact cursor at all: `0x5f2f80` plus the per-type overrides.
/// An INTERACT_COND object passes only while the server has set its per-player activate bit: the
/// quest gate. The FISHINGNODE override (`0x5f6710`) compares our channel object, not
/// `CREATED_BY`, then runs the shared gate.
fn highlightable_flags(
    type_id: i32,
    flags: u32,
    dyn_flags: u32,
    reaction: Option<u8>,
    overrides: GoOverrides,
) -> bool {
    if strategy_never_highlightable(type_id) {
        return false;
    }
    if type_id == GO_TYPE_FISHINGNODE && !overrides.channel_owned {
        return false;
    }
    // MEETINGSTONE's slot replaces the shared gate, so none of the terms below apply to it.
    if type_id == GO_TYPE_MEETINGSTONE {
        return !overrides.meeting_stone_queued;
    }
    // The faction term (`0x5f3026`): the GameObject's reaction toward us must beat hostile. An
    // unresolved reaction passes, so a data gap never blanks the world.
    let ordinary_faction_ok = |r: Option<u8>| r.is_none_or(|r| r > 1);
    if type_id == GO_TYPE_TRAP {
        // TRAP (12340 `0x7112a0`) needs a lock row, then inverts the term: highlightable only to
        // whoever it is hostile to. Labelled: a player-made trap's PvP and party terms are not.
        if overrides.trap_lockless || !reaction.is_none_or(|r| r == 1) {
            return false;
        }
    } else if !ordinary_faction_ok(reaction) {
        return false;
    }
    if flags & GO_FLAG_IN_USE_OR_NO_INTERACT != 0 {
        return false;
    }
    if flags & GO_FLAG_INTERACT_COND != 0 && dyn_flags & GO_DYNFLAG_ACTIVATE == 0 {
        return false;
    }
    true
}

/// `GAMEOBJECT_TYPE_TRAP` (6), the one type whose faction term is inverted.
const GO_TYPE_TRAP: i32 = 6;

/// A GameObject's reaction toward us on the 1/3/4 scale: the reference's `0x5f7fd0` → `0x606530` →
/// `0x606640` chain, the GameObject's template toward the player's. No faction is neutral
/// (`0x5f8025`); `None` when unresolvable, which callers pass. Only the template comparator runs:
/// every shipped GameObject faction (114, 35, 14 and 1375) resolves through it, so the reference's
/// reputation arm and a `GAMEOBJECT_CREATED_BY` creator's reaction are not applied.
pub fn go_reaction(
    factions: Option<&Factions>,
    go_faction: u32,
    self_store: Option<&ObjectStore>,
) -> Option<u8> {
    if go_faction == 0 {
        return Some(wrathplus_formats::Reaction::Neutral as u8);
    }
    let catalog = factions?.catalog();
    let go_tpl = catalog.template(go_faction)?;
    let self_tpl = catalog.template(self_store?.0.unit_faction_template()?)?;
    Some(go_tpl.reaction_toward(self_tpl) as u8)
}

/// Whether a picked object becomes the mouseover at all (`[vtbl+0x54]`): false publishes the null
/// mouseover (`0x482985`), so no tooltip (`0x52aa20`), brighten (`0x4945e0`) or cursor. For a
/// GameObject it is the type's slot `+0xc` (`0x5f8620`), not the cursor's slot. `highlight_column`
/// is GENERIC's `data[1]` or CAPTURE_POINT's `data[19]` (`0x5f4830`, `0x5f6d80`); `None`, a
/// template not yet answered, reads eligible.
pub fn mouseover_eligible(
    type_id: i32,
    flags: u32,
    dyn_flags: u32,
    highlight_column: Option<bool>,
    reaction: Option<u8>,
    overrides: GoOverrides,
) -> bool {
    match type_id {
        GO_TYPE_TRANSPORT | GO_TYPE_MAP_OBJECT | GO_TYPE_MO_TRANSPORT => false,
        GO_TYPE_SPELL_FOCUS
        | GO_TYPE_DUEL_ARBITER
        | GO_TYPE_FISHINGHOLE
        | GO_TYPE_AURA_GENERATOR => true,
        GO_TYPE_GENERIC | GO_TYPE_CAPTURE_POINT => highlight_column.unwrap_or(true),
        // Every other type's `+0xc` forwards to its own `+0x14` (`0x5f9db0`).
        _ => highlightable_flags(type_id, flags, dyn_flags, reaction, overrides),
    }
}

/// [`highlightable_flags`] off a GameObject's descriptor; an absent `GAMEOBJECT_TYPE_ID` is the
/// wire default 0, DOOR. This slot gates the cursor, the right-click use (`OnUse 0x5f8660`) and
/// the pick's pass-2 priority (`0x480c90` via `0x5f8800`); the tooltip and the brighten follow the
/// mouseover slot, [`mouseover_eligible`].
pub fn go_highlightable(store: &ObjectStore, reaction: Option<u8>, overrides: GoOverrides) -> bool {
    highlightable_flags(
        store.0.gameobject_type_id(),
        store.0.gameobject_flags(),
        store.0.gameobject_dynamic_flags(),
        reaction,
        overrides,
    )
}

impl GoOverrides {
    /// One GameObject's overrides: its guid and template (`None` while unanswered), our player,
    /// the meeting-stone queue's area and `Lock.dbc`.
    pub fn of(
        self_store: Option<&ObjectStore>,
        guid: Option<u64>,
        tmpl: Option<&crate::go_templates::GoTemplate>,
        queued_area: u32,
        locks: Option<&crate::go_templates::Locks>,
    ) -> Self {
        GoOverrides {
            channel_owned: fishing_channel_owned(self_store, guid),
            meeting_stone_queued: meeting_stone_queued(
                tmpl.and_then(|t| t.meeting_stone).map(|m| m.area),
                queued_area,
            ),
            trap_lockless: trap_lockless(tmpl.map(|t| t.lock_id), locks),
        }
    }
}

/// [`GoOverrides::trap_lockless`] from a template's lock id (`None` unanswered) and `Lock.dbc`.
pub fn trap_lockless(lock_id: Option<u32>, locks: Option<&crate::go_templates::Locks>) -> bool {
    match lock_id {
        Some(0) => true,
        Some(id) => locks.is_some_and(|l| l.0.slots(id).is_none()),
        None => false,
    }
}

/// MEETINGSTONE's override for one stone, 1.12's `template.data[2] == [0xb72038]`. `queued_area`
/// is always 0 in 3.3.5, which has no queue (`GoGateInputs::queued_area`, labelled). An
/// unanswered template reads not queued; a `data[2]` of 0 matches the unqueued 0.
pub fn meeting_stone_queued(area: Option<u32>, queued_area: u32) -> bool {
    area.is_some_and(|a| a == queued_area)
}

/// The FISHINGNODE override's input (`0x5f6710`): our `UNIT_FIELD_CHANNEL_OBJECT` is this
/// GameObject. False when either is unknown, as the reference answers false with no active player.
pub fn fishing_channel_owned(self_store: Option<&ObjectStore>, go_guid: Option<u64>) -> bool {
    match (self_store, go_guid) {
        (Some(s), Some(g)) => s.0.unit_channel_object() == Some(g),
        _ => false,
    }
}

/// A `LockType.dbc` CursorName to its cursor, the reference's `CursorModeFromName` (`0x523d40`)
/// over the three cursor-bearing lock types in 1.12.
fn cursor_kind_from_lock_name(name: &str) -> Option<CursorKind> {
    match name {
        "PickLock" => Some(CursorKind::PickLock),
        "GatherHerbs" => Some(CursorKind::GatherHerbs),
        "Mine" => Some(CursorKind::Mine),
        _ => None,
    }
}

/// The data-named cursor for a GameObject's lock (`0x5f3070`): the `Lock.dbc` row's first slot
/// only (`[lockRow+0x24]`, no scan), then that `LockType.dbc` row's CursorName.
fn go_lock_cursor(
    lock_id: u32,
    locks: Option<&crate::go_templates::Locks>,
    lock_types: Option<&crate::go_templates::LockTypes>,
) -> Option<CursorKind> {
    if lock_id == 0 {
        return None;
    }
    let slots = locks?.0.slots(lock_id)?;
    let lock_type_id = slots[0].index;
    let name = lock_types?.0.cursor_name(lock_type_id)?;
    cursor_kind_from_lock_name(name)
}

/// A highlightable GameObject's cursor (`GetCursorMode 0x5f8760`): TEXT and the Mail types by
/// type, any other its lock's data-named cursor or the gear.
fn go_cursor_kind(type_id: i32, lock_cursor: Option<CursorKind>) -> CursorKind {
    match type_id {
        GO_TYPE_TEXT => CursorKind::Inspect,
        GO_TYPE_RITUAL | GO_TYPE_MAILBOX | GO_TYPE_28 => CursorKind::Mail,
        _ => lock_cursor.unwrap_or(CursorKind::Interact),
    }
}

/// The QUESTGIVER bit's own gate (`0x5df490`): the cached `SMSG_QUESTGIVER_STATUS`
/// (`[unit+0xcb8]`) is neither NONE nor UNAVAILABLE, and a missing one reads as NONE since the
/// server sends it unprompted. Without the gate a quest-less questgiver opens an empty gossip frame
/// that vmangos fills with placeholder "Greetings $N" text (`QueryHandler.cpp:313-319`).
pub fn questgiver_has_quest(quest_status: Option<u32>) -> bool {
    use wrathplus_protocol::types::dialog_status::{NONE, UNAVAILABLE};
    !matches!(quest_status, None | Some(NONE) | Some(UNAVAILABLE))
}

/// The service cursor ladder (`FUN_004f7a50`), independent of the click ladder. `None` when no
/// tested bit is set.
fn service_cursor_native(
    flags: wrathplus_protocol::types::NpcFlags,
    status: Option<u32>,
) -> Option<CursorKind> {
    use wrathplus_protocol::types::NpcService as f;
    // Quest/custom cursor artwork is not represented here yet; retain Speak for quests.
    let bit = |service| flags.has(service);
    Some(if flags.bits() != 0 && matches!(status, Some(2..=10)) {
        CursorKind::Speak
    } else if bit(f::Repair) {
        CursorKind::Repair
    } else if bit(f::Innkeeper) {
        CursorKind::Interact
    } else if bit(f::FlightMaster) {
        CursorKind::Taxi
    } else if flags.has_trainer_base() {
        CursorKind::Trainer
    } else if bit(f::SpiritHealer) || bit(f::SpiritGuide) {
        CursorKind::Speak
    } else if bit(f::Banker) {
        CursorKind::Buy
    } else if bit(f::Petitioner) || bit(f::TabardDesigner) || bit(f::BattleMaster) {
        CursorKind::Speak
    } else if bit(f::Auctioneer) {
        CursorKind::Buy
    } else if bit(f::StableMaster) {
        CursorKind::Speak
    } else if flags.has_vendor_base() {
        CursorKind::Pickup
    } else if bit(f::Gossip) {
        CursorKind::Speak
    } else {
        return None;
    })
}

/// The loot leg's mode, `8 + (keyDown(0) ? 8 : 0)` in the reference (`0x48252c`): LootAll while
/// the effective auto-loot, `autoLootDefault` XOR shift, is on. `autoLootDefault` is not a 1.12
/// CVar; with it off the held key alone decides, as in 1.12.
fn loot_cursor(auto_loot: bool, shift_held: bool) -> CursorKind {
    if auto_loot != shift_held {
        CursorKind::LootAll
    } else {
        CursorKind::Pickup
    }
}

/// Whether a corpse becomes the mouseover (`[CGCorpse_C vtbl+0x54]` = `0x5d76d0`, asked at
/// `0x482982`): a body always, your own included; a bone pile only while it has loot. A refused
/// corpse is still picked (`0x480816`) but shows no name and no brighten.
pub fn corpse_mouseover_eligible(store: &ObjectStore) -> bool {
    !store.0.corpse_is_bones() || store.0.corpse_lootable()
}

/// What the corpse classifier `0x482740` reads. It has two legs and no own-corpse leg: your own
/// body sets neither flag and reads Point.
#[derive(Debug, Clone, Copy, Default)]
struct CorpseFacts {
    /// `CORPSE_FIELD_DYNAMIC_FLAGS` bit 0 (`0x5d6e20`).
    lootable: bool,
    /// `CORPSE_FIELD_FLAGS` bit 5, `CORPSE_FLAG_LOOTABLE`: the PvP insignia.
    insignia: bool,
    /// `[0xb700e8]`, latched by learning a `SPELL_EFFECT_SKIN_PLAYER_CORPSE` spell.
    skin_latch: bool,
    /// `d2`, centre to centre.
    dist_sq: f32,
    /// The already-looting override: `[player+0x1d28/2c] == corpse GUID`.
    looting_this: bool,
    /// `CanAutoInteract`, which `CanLootNow` asks first and then skips its range (`0x5ec11b`).
    can_auto_interact: bool,
    auto_loot: bool,
    /// Key 0, shift.
    shift_held: bool,
}

fn corpse_cursor(f: CorpseFacts) -> Option<(CursorKind, bool)> {
    // Leg 1, lootable: able within `CanLootNow 0x5ec110`'s reach or while this corpse's loot
    // window is open; Pickup or LootAll by key 0 (`0x41f8f0` at `0x4827f8`), grayed at `0x482818`.
    if f.lootable {
        let able = f.can_auto_interact || f.dist_sq <= CORPSE_INTERACT_RANGE_SQ || f.looting_this;
        return Some((loot_cursor(f.auto_loot, f.shift_held), !able));
    }
    // Leg 2, the PvP insignia (`0x482875..0x4828a2`), inert in 1.12.1 as no player holds a
    // latching spell. Not applied: its third term `!0x6067d0(player, corpse)`, and the faction's
    // SkinAlliance(20) or SkinHorde(19) cursor (`0x80439c`), shown here as Skin.
    if f.insignia && f.skin_latch {
        return Some((CursorKind::Skin, f.dist_sq > CORPSE_INTERACT_RANGE_SQ));
    }
    None // the reference's "else clear": Point
}

/// Resolve this frame's [`WorldCursor`] over the hover.
pub(super) fn classify_cursor(
    hovered: Res<Hovered>,
    hovered_object: Res<HoveredObject>,
    inputs: CursorInputs,
    (mut cursor, mut fork): (ResMut<WorldCursor>, ResMut<AttackFork>),
) {
    let (want, want_fork) = classify(&inputs, &hovered, &hovered_object);
    if *cursor != want {
        *cursor = want;
    }
    if *fork != want_fork {
        *fork = want_fork;
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct CursorInputs<'w, 's> {
    factions: Option<Res<'w, Factions>>,
    reputations: Res<'w, Reputations>,
    // `control_lost` is the attack leg's `[0xb4b3e4]` clear.
    player: Res<'w, crate::app_core::player::Player>,
    approach: Res<'w, crate::player::Approach>,
    #[allow(clippy::type_complexity)]
    units: Query<
        'w,
        's,
        (
            &'static Transform,
            Option<&'static ObjectStore>,
            Option<&'static crate::go_anim::GoAnim>,
        ),
    >,
    self_q: Query<'w, 's, (&'static Transform, &'static ObjectStore), With<SelfPlayer>>,
    go_inputs: super::lock::GoLockInputs<'w, 's>,
    player_actions: Res<'w, crate::app_core::state::PlayerActions>,
    // The gather latches and the corpse's trade (`0x53bce0`), and `[0xbe5d7c]`.
    gather: GatherSpells<'w>,
    quest: Res<'w, crate::app_core::state::QuestGiver>,
    loot_cfg: Res<'w, crate::app_core::state::LootConfig>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    // `[player+0x1d28/2c]`, the object whose loot window is open.
    loot_latch: Res<'w, crate::app_core::state::LootLatch>,
    /// `CanAttack`'s creature metadata and controller lookup.
    relations: super::RelationInputs<'w, 's>,
}

#[path = "gather.rs"]
mod gather;
pub(crate) use gather::GatherSpells;

/// A skinnable corpse's cursor by its gather trade (12340 `0x4f7ec0`, ids `0xc`/`0xd`/`0xb`).
pub(crate) fn gather_cursor(kind: usize) -> CursorKind {
    match kind {
        1 => CursorKind::GatherHerbs,
        2 | 3 => CursorKind::Mine,
        _ => CursorKind::Skin,
    }
}

/// The cursor and the attack fork over a pick; no pick, or nothing resolvable, reads Point.
pub(super) fn classify(
    inputs: &CursorInputs,
    hovered: &Hovered,
    hovered_object: &HoveredObject,
) -> (WorldCursor, AttackFork) {
    let CursorInputs {
        factions,
        reputations,
        player,
        approach,
        units,
        self_q,
        go_inputs,
        player_actions,
        gather,
        quest,
        loot_cfg,
        keys,
        loot_latch,
        relations,
    } = inputs;
    let can_auto_interact = crate::player::can_auto_interact(
        approach.enabled,
        self_q.single().ok().map(|(_, s)| s),
        player,
    );
    // A GameObject that is not highlightable clears the cursor, as the reference's handler does.
    let resolve_go = || {
        let (go_tf, store, anim) = units.get(hovered_object.target?).ok()?;
        let store = store?;
        let (self_tf, self_store) = self_q.single().ok()?;
        let reaction = go_reaction(
            factions.as_deref(),
            store.0.gameobject_faction(),
            Some(self_store),
        );
        // Read before the gate: MEETINGSTONE's override is a template slot.
        let tmpl = hovered_object.guid.and_then(|g| go_inputs.templates.get(g));
        // No queue in 3.3.5: the unqueued 0 (`GoGateInputs::queued_area`).
        let locks = go_inputs.locks.as_deref();
        let overrides = GoOverrides::of(Some(self_store), hovered_object.guid, tmpl, 0, locks);
        if !go_highlightable(store, reaction, overrides) {
            return None;
        }
        // Until its template answers, a locked GameObject reads the gear.
        let lock_id = tmpl.map_or(0, |t| t.lock_id);
        let lock_cursor = go_lock_cursor(
            lock_id,
            go_inputs.locks.as_deref(),
            go_inputs.lock_types.as_deref(),
        );
        let kind = go_cursor_kind(store.0.gameobject_type_id(), lock_cursor);
        let dist_sq = go_tf.translation.distance_squared(self_tf.translation);
        // `usable 0x5f3130`'s lock arm (`0x5f32a6`) then range arm (`0x5f330c`); its player-state
        // arm is not applied. The lock arm needs `GO_FLAG_LOCKED`, so an herb the player cannot
        // gather stays lit and refuses on the click, while a padlocked door grays.
        let facts = super::lock::go_facts(Some((store, crate::go_anim::go_state(anim, store))));
        let lock_unmet = go_inputs
            .locks
            .as_deref()
            .and_then(|l| l.0.slots(lock_id).filter(|_| lock_id != 0))
            .is_some_and(|slots| {
                super::lock::resolve_lock(
                    slots,
                    &player_actions.spells,
                    go_inputs.spells.as_deref(),
                    go_inputs.skill_lines.as_ref().map(|s| &s.catalog),
                    Some(self_store),
                    &go_inputs.objects,
                    facts,
                    &mut None,
                )
                .blocks_usable(facts.flag_locked)
            });
        let unable = kind != CursorKind::PickLock
            && (lock_unmet || dist_sq > go_interact_range_sq(store.0.gameobject_type_id()));
        Some((kind, unable))
    };
    // The reference picks once over all objects; WrathPlus picks units and GameObjects apart and
    // classifies whichever is nearer under the cursor.
    let attack_fork = std::cell::Cell::new(false);
    let resolve_unit = || {
        let (unit_tf, store, _) = units.get(hovered.target?).ok()?;
        let store = store?;
        let (self_tf, self_store) = self_q.single().ok()?;
        let dist_sq = unit_tf.translation.distance_squared(self_tf.translation);
        let reach = melee_reach(store, self_store);
        let in_melee = dist_sq <= reach * reach;

        let dead = store.0.unit_is_dead();
        if dead {
            if store.0.unit_lootable() {
                // Grayed by `CanLootNow 0x5ec110`'s melee reach; the reference's mid-loot state
                // block and open-loot-window override are not applied to units.
                let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
                return Some((
                    loot_cursor(loot_cfg.auto_loot, shift),
                    !(can_auto_interact || in_melee),
                ));
            }
            // The gather cursor needs the corpse's own trade learned (`0x4f7e17` → `0x53bce0`): a
            // miner over a boar sees Point. Its icon is the trade's (`0x4f7ec0`'s table on
            // `0x715e50`): Skin, GatherHerbs, Mine, and Mine for Engineering.
            if store.0.unit_flags() & UNIT_FLAG_SKINNABLE != 0 && gather.spell_for(store).is_some()
            {
                return Some((gather_cursor(gather.kind(store)), !in_melee));
            }
            return None; // a plain corpse: Point
        }

        // `CanInteract 0x6067f0`, asked at `0x482310` through `CanInteractNow 0x606880`, picks
        // the ladder; it is not a reaction threshold.
        if relations.can_interact(Some(store), Some(self_store)) {
            let status = hovered
                .guid
                .and_then(|g| quest.wrath_statuses().get(&g).copied());
            // No matching bit clears the cursor (`je 0x4826cb`) without reaching the attack leg:
            // Point, never the sword.
            return service_cursor_native(store.0.unit_npc_services(), status)
                .map(|kind| (kind, dist_sq > SERVICE_RANGE_SQ));
        }
        // `CanAttack 0x606980` (`0x482680`), the predicate TAB, the combat flash and
        // `UnitCanAttack` share; not attackable clears the cursor (`0x4826cb`): Point.
        if !super::can_attack(
            Some(store),
            factions.as_deref(),
            reputations,
            Some(self_store),
            relations.context(),
        ) {
            return None;
        }
        attack_fork.set(true);
        player_attack_legs(self_store, player.control_lost)
            .then_some((CursorKind::Attack, dist_sq > ATTACK_RANGE_SQ))
    };
    // The corpse classifier `0x482740`.
    let resolve_corpse = || {
        let (corpse_tf, store, _) = units.get(hovered.corpse?).ok()?;
        let store = store?;
        let (self_tf, _) = self_q.single().ok()?;
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        corpse_cursor(CorpseFacts {
            lootable: store.0.corpse_lootable(),
            insignia: store.0.corpse_pvp_insignia(),
            skin_latch: gather.learned.skin_player_corpse.is_some(),
            dist_sq: corpse_tf.translation.distance_squared(self_tf.translation),
            looting_this: loot_latch.0.is_some() && loot_latch.0 == hovered.corpse_guid,
            can_auto_interact,
            auto_loot: loot_cfg.auto_loot,
            shift_held: shift,
        })
    };
    let resolved = if go_is_nearest(hovered, hovered_object) {
        resolve_go()
    } else if hovered.corpse.is_some() {
        resolve_corpse()
    } else {
        resolve_unit()
    };
    let (kind, unable) = resolved.unwrap_or((CursorKind::Point, false));
    (WorldCursor { kind, unable }, AttackFork(attack_fork.get()))
}

#[cfg(test)]
#[path = "cursor_mode_gate_tests.rs"]
mod gate_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::Field;

    /// `field`'s native descriptor index.
    const fn at(field: Field) -> u16 {
        field.index()
    }

    /// `CanLootNow 0x5ec110` skips its range under `CanAutoInteract` (`0x5ec11b`), so the click
    /// that walks there finds a lit pouch.
    #[test]
    fn wrath_cursor_priority_is_not_the_click_priority() {
        use super::super::click::{service_arm_native, ServiceArm};
        use wrathplus_protocol::types::NpcFlags;
        for (bits, click, cursor) in [
            (
                0x2080,
                Some(ServiceArm::FlightMaster),
                Some(CursorKind::Taxi),
            ),
            (
                0x10080,
                Some(ServiceArm::Vendor),
                Some(CursorKind::Interact),
            ),
            (0x1080, Some(ServiceArm::Vendor), Some(CursorKind::Repair)),
            (0x81, Some(ServiceArm::Gossip), Some(CursorKind::Pickup)),
            (0x90, Some(ServiceArm::Vendor), Some(CursorKind::Trainer)),
            (0x100, None, None), // Vendor specialization is not the base-bit branch.
            (0x20, None, None),  // Neither is a trainer specialization.
        ] {
            let flags = NpcFlags::from_bits(bits);
            assert_eq!(service_arm_native(flags, None), click, "click {bits:#x}");
            assert_eq!(
                service_cursor_native(flags, None),
                cursor,
                "cursor {bits:#x}"
            );
        }
        let vendor = NpcFlags::from_bits(0x80);
        // Native quest cursor precedes services without requiring the QuestGiver bit.
        for status in 2..=10 {
            assert_eq!(
                service_cursor_native(vendor, Some(status)),
                Some(CursorKind::Speak)
            );
        }
        assert_eq!(
            service_cursor_native(vendor, Some(11)),
            Some(CursorKind::Pickup)
        );
        let quest = NpcFlags::from_bits(0x1082);
        assert_eq!(
            service_cursor_native(quest, Some(8)),
            Some(CursorKind::Speak)
        );
        assert_eq!(
            service_arm_native(quest, Some(8)),
            Some(ServiceArm::Questgiver)
        );
        assert_eq!(
            service_cursor_native(quest, Some(1)),
            Some(CursorKind::Repair)
        );
    }

    #[test]
    fn native_wrath_services_never_use_vanilla_bit_positions() {
        use super::super::click::{service_arm_native, ServiceArm};
        use wrathplus_protocol::types::Field;
        for (bits, arm, cursor) in [
            (0x80, Some(ServiceArm::Vendor), Some(CursorKind::Pickup)),
            (
                0x10000,
                Some(ServiceArm::Innkeeper),
                Some(CursorKind::Interact),
            ),
            (0x10, Some(ServiceArm::Trainer), Some(CursorKind::Trainer)),
            (0x1000, None, Some(CursorKind::Repair)),
            (
                0x2000,
                Some(ServiceArm::FlightMaster),
                Some(CursorKind::Taxi),
            ),
            (0x20000, Some(ServiceArm::Banker), Some(CursorKind::Buy)),
        ] {
            let index = Field::UnitNpcFlags.span().unwrap().index;
            let fields = wrathplus_protocol::ObjectFields::from_pairs(&[(index, bits)]);
            let flags = fields.unit_npc_services();
            assert_eq!(service_arm_native(flags, None), arm, "native {bits:#x}");
            assert_eq!(
                service_cursor_native(flags, None),
                cursor,
                "native {bits:#x}"
            );
        }
    }

    #[test]
    fn click_to_move_lights_a_far_corpses_pouch() {
        let far = CorpseFacts {
            lootable: true,
            dist_sq: 400.0,
            ..CorpseFacts::default()
        };
        assert_eq!(corpse_cursor(far), Some((CursorKind::Pickup, true)));
        assert_eq!(
            corpse_cursor(CorpseFacts {
                can_auto_interact: true,
                ..far
            }),
            Some((CursorKind::Pickup, false))
        );
    }

    #[test]
    fn the_corpse_classifier_has_two_legs_and_no_own_corpse_leg() {
        let near = 25.0; // exactly the gate, which is inclusive
        let far = 25.01;
        let bones = |dist_sq| CorpseFacts {
            lootable: true,
            dist_sq,
            ..CorpseFacts::default()
        };
        assert_eq!(
            corpse_cursor(bones(near)),
            Some((CursorKind::Pickup, false))
        );
        assert_eq!(
            corpse_cursor(CorpseFacts {
                auto_loot: true,
                ..bones(near)
            }),
            Some((CursorKind::LootAll, false))
        );
        assert_eq!(corpse_cursor(bones(far)), Some((CursorKind::Pickup, true)));
        assert_eq!(
            corpse_cursor(CorpseFacts {
                looting_this: true,
                ..bones(far)
            }),
            Some((CursorKind::Pickup, false))
        );
        let insignia = |skin_latch, dist_sq| CorpseFacts {
            insignia: true,
            skin_latch,
            dist_sq,
            ..CorpseFacts::default()
        };
        assert_eq!(corpse_cursor(insignia(false, near)), None);
        assert_eq!(
            corpse_cursor(insignia(true, near)),
            Some((CursorKind::Skin, false))
        );
        assert_eq!(
            corpse_cursor(insignia(true, far)),
            Some((CursorKind::Skin, true))
        );
        // The reference tests lootable first.
        assert_eq!(
            corpse_cursor(CorpseFacts {
                insignia: true,
                skin_latch: true,
                ..bones(near)
            }),
            Some((CursorKind::Pickup, false))
        );
        // Your own corpse sets neither flag.
        let mine = |dist_sq| CorpseFacts {
            skin_latch: true,
            dist_sq,
            ..CorpseFacts::default()
        };
        assert_eq!(corpse_cursor(mine(near)), None);
        assert_eq!(
            corpse_cursor(CorpseFacts {
                auto_loot: true,
                shift_held: true,
                ..mine(far)
            }),
            None
        );
    }

    #[test]
    fn the_loot_pouch_triples_with_the_effective_auto_loot() {
        assert_eq!(loot_cursor(false, false), CursorKind::Pickup);
        assert_eq!(loot_cursor(true, false), CursorKind::LootAll);
        // Shift inverts both ways; with the setting off it is 1.12's whole mechanism.
        assert_eq!(loot_cursor(false, true), CursorKind::LootAll);
        assert_eq!(loot_cursor(true, true), CursorKind::Pickup);
        let far = WorldCursor {
            kind: CursorKind::LootAll,
            unable: true,
        };
        assert_eq!(far.stem(), "UnableLootAll");
    }

    #[test]
    fn stems_name_the_shipped_blps() {
        let attack = WorldCursor {
            kind: CursorKind::Attack,
            unable: false,
        };
        assert_eq!(attack.stem(), "Attack");
        let far = WorldCursor {
            kind: CursorKind::Attack,
            unable: true,
        };
        assert_eq!(far.stem(), "UnableAttack");
        let point = WorldCursor {
            kind: CursorKind::Point,
            unable: true,
        };
        assert_eq!(point.stem(), "Point");
    }

    #[test]
    fn mouseover_eligibility_matches_the_per_type_slot_table() {
        for t in [
            GO_TYPE_SPELL_FOCUS,
            GO_TYPE_DUEL_ARBITER,
            GO_TYPE_FISHINGHOLE,
            GO_TYPE_AURA_GENERATOR,
        ] {
            assert!(mouseover_eligible(
                t,
                GO_FLAG_INTERACT_COND,
                0,
                None,
                None,
                GoOverrides::default()
            ));
            assert!(mouseover_eligible(
                t,
                0x10,
                0,
                None,
                None,
                GoOverrides::default()
            ));
        }
        for t in [GO_TYPE_TRANSPORT, GO_TYPE_MAP_OBJECT, GO_TYPE_MO_TRANSPORT] {
            assert!(!mouseover_eligible(
                t,
                0,
                GO_DYNFLAG_ACTIVATE,
                None,
                None,
                GoOverrides::default()
            ));
        }
        assert!(mouseover_eligible(
            GO_TYPE_GENERIC,
            0,
            0,
            Some(true),
            None,
            GoOverrides::default()
        ));
        assert!(!mouseover_eligible(
            GO_TYPE_GENERIC,
            0,
            0,
            Some(false),
            None,
            GoOverrides::default()
        ));
        assert!(
            mouseover_eligible(GO_TYPE_GENERIC, 0, 0, None, None, GoOverrides::default()),
            "template not answered yet reads eligible, so a signpost isn't blank while it queries"
        );
        assert!(!mouseover_eligible(
            0,
            GO_FLAG_INTERACT_COND,
            0,
            None,
            None,
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            0,
            GO_FLAG_INTERACT_COND,
            GO_DYNFLAG_ACTIVATE,
            None,
            None,
            GoOverrides::default()
        ));
        assert!(!mouseover_eligible(
            3,
            0x10,
            0,
            None,
            None,
            GoOverrides::default()
        )); // NO_INTERACT chest
        assert!(!mouseover_eligible(
            3,
            0x1,
            0,
            None,
            None,
            GoOverrides::default()
        )); // IN_USE chest
            // A door hostile to us, like the Deadmines Factory Door (faction 114, flags 0x20).
        assert!(!mouseover_eligible(
            0,
            0x20,
            0,
            None,
            Some(1),
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            0,
            0x20,
            0,
            None,
            Some(3),
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            0,
            0x20,
            0,
            None,
            Some(4),
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            GO_TYPE_TRAP,
            0,
            0,
            None,
            Some(1),
            GoOverrides::default()
        ));
        assert!(!mouseover_eligible(
            GO_TYPE_TRAP,
            0,
            0,
            None,
            Some(3),
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            0,
            0x20,
            0,
            None,
            None,
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            0,
            0,
            0,
            None,
            None,
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            19,
            0,
            0,
            None,
            None,
            GoOverrides::default()
        )); // mailbox
    }

    #[test]
    fn the_meeting_stone_answers_only_the_queued_area() {
        let queued = GoOverrides {
            meeting_stone_queued: true,
            ..Default::default()
        };
        assert!(highlightable_flags(
            GO_TYPE_MEETINGSTONE,
            0,
            0,
            None,
            GoOverrides::default()
        ));
        assert!(!highlightable_flags(
            GO_TYPE_MEETINGSTONE,
            0,
            0,
            None,
            queued
        ));
        assert!(!mouseover_eligible(
            GO_TYPE_MEETINGSTONE,
            0,
            0,
            None,
            None,
            queued
        ));
        for (flags, dyn_flags, reaction) in [
            (0x10, 0, Some(1)),
            (0x1, 0, Some(1)),
            (GO_FLAG_INTERACT_COND, 0, Some(1)),
        ] {
            assert!(
                highlightable_flags(
                    GO_TYPE_MEETINGSTONE,
                    flags,
                    dyn_flags,
                    reaction,
                    GoOverrides::default()
                ),
                "flags {flags:#x} must not reach a meeting stone — its slot never calls 0x5f2f80"
            );
        }
        assert!(!meeting_stone_queued(None, 0));
        assert!(!meeting_stone_queued(Some(1519), 0));
        assert!(meeting_stone_queued(Some(1519), 1519));
        assert!(!meeting_stone_queued(Some(1519), 1517));
        assert!(!meeting_stone_queued(None, 1519));
        // The reference's own `0 != 0`: a stone whose `data[2]` is 0 matches while unqueued.
        assert!(meeting_stone_queued(Some(0), 0));
    }

    #[test]
    fn the_bobber_is_channel_gated_and_reaches_a_hundred_yards() {
        assert!(highlightable_flags(
            GO_TYPE_FISHINGNODE,
            0,
            0,
            None,
            GoOverrides {
                channel_owned: true,
                ..Default::default()
            }
        ));
        assert!(!highlightable_flags(
            GO_TYPE_FISHINGNODE,
            0,
            0,
            None,
            GoOverrides::default()
        ));
        // NO_INTERACT still applies after the channel check.
        assert!(!highlightable_flags(
            GO_TYPE_FISHINGNODE,
            0x10,
            0,
            None,
            GoOverrides {
                channel_owned: true,
                ..Default::default()
            }
        ));
        assert!(!mouseover_eligible(
            GO_TYPE_FISHINGNODE,
            0,
            0,
            None,
            None,
            GoOverrides::default()
        ));
        assert!(mouseover_eligible(
            GO_TYPE_FISHINGNODE,
            0,
            0,
            None,
            None,
            GoOverrides {
                channel_owned: true,
                ..Default::default()
            }
        ));
        assert!(highlightable_flags(0, 0, 0, None, GoOverrides::default()));
        assert_eq!(go_interact_range_sq(GO_TYPE_FISHINGNODE), 100.0 * 100.0);
        assert_eq!(go_interact_range_sq(0), GO_INTERACT_RANGE_SQ);
        assert_eq!(go_interact_range_sq(GO_TYPE_CHAIR), 9.0);
        assert!(
            go_interact_range_sq(GO_TYPE_CHAIR) < GO_INTERACT_RANGE_SQ,
            "a chair's reach is shorter than the default GameObject reach"
        );
        assert!(!fishing_channel_owned(None, Some(7)));
        assert!(!fishing_channel_owned(None, None));
    }

    #[test]
    fn melee_reach_floors_at_five() {
        // A typical pair, 1.5 + 1.5 + 1.333, is lifted to the 5 yd floor.
        assert_eq!((1.5_f32 + 1.5 + MELEE_OFFSET).max(MELEE_FLOOR), 5.0);
        assert!(((3.0_f32 + 3.0 + MELEE_OFFSET).max(MELEE_FLOOR) - 7.333_33).abs() < 1e-4);
    }

    /// The sword's player legs (`0x48264a`–`0x4826a2`): a corpse, a ghost at health 1, a rider, a
    /// pacified player and one without control each see the pointer.
    #[test]
    fn the_sword_needs_a_live_unmounted_unpacified_player_in_control() {
        // Health 22, max health 28, unit flags 46, mount display 133, player flags 190.
        let own = |pairs: &[(u16, u32)]| {
            let mut all = vec![
                (at(Field::UnitHealth), 100),
                (at(Field::UnitMaxHealth), 100),
            ];
            all.extend_from_slice(pairs);
            ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(&all))
        };
        assert!(player_attack_legs(&own(&[]), false));
        assert!(
            !player_attack_legs(&own(&[(at(Field::UnitHealth), 0)]), false),
            "dead"
        );
        assert!(
            !player_attack_legs(
                &own(&[(at(Field::UnitHealth), 1), (at(Field::PlayerFlags), 0x10)]),
                false
            ),
            "a ghost"
        );
        assert!(
            !player_attack_legs(&own(&[(at(Field::UnitMountDisplayId), 1147)]), false),
            "mounted"
        );
        assert!(
            !player_attack_legs(&own(&[(at(Field::UnitFlags), UNIT_FLAG_PACIFIED)]), false),
            "pacified"
        );
        assert!(!player_attack_legs(&own(&[]), true), "no control");
    }

    #[test]
    fn lock_names_resolve_to_the_three_data_cursors() {
        assert_eq!(
            cursor_kind_from_lock_name("PickLock"),
            Some(CursorKind::PickLock)
        );
        assert_eq!(
            cursor_kind_from_lock_name("GatherHerbs"),
            Some(CursorKind::GatherHerbs)
        );
        assert_eq!(cursor_kind_from_lock_name("Mine"), Some(CursorKind::Mine));
        assert_eq!(cursor_kind_from_lock_name(""), None);
        assert_eq!(cursor_kind_from_lock_name("Fishing"), None);
    }

    #[test]
    fn go_cursor_kind_maps_type_then_lock() {
        // MAILBOX(19), RITUAL(18) and type 28 take Mail whatever their lock.
        assert_eq!(go_cursor_kind(19, None), CursorKind::Mail);
        assert_eq!(go_cursor_kind(18, None), CursorKind::Mail);
        assert_eq!(go_cursor_kind(28, Some(CursorKind::Mine)), CursorKind::Mail);
        assert_eq!(go_cursor_kind(9, None), CursorKind::Inspect); // TEXT
        for t in [0, 1, 3, 10, 6, 24] {
            assert_eq!(go_cursor_kind(t, None), CursorKind::Interact);
        }
        assert_eq!(go_cursor_kind(3, Some(CursorKind::Mine)), CursorKind::Mine);
        assert_eq!(
            go_cursor_kind(3, Some(CursorKind::GatherHerbs)),
            CursorKind::GatherHerbs
        );
        assert_eq!(
            go_cursor_kind(3, Some(CursorKind::PickLock)),
            CursorKind::PickLock
        );
    }

    #[test]
    fn go_cursor_stems_name_the_shipped_blps() {
        for (kind, stem) in [
            (CursorKind::Mail, "Mail"),
            (CursorKind::Mine, "Mine"),
            (CursorKind::GatherHerbs, "GatherHerbs"),
            (CursorKind::PickLock, "PickLock"),
        ] {
            assert_eq!(
                WorldCursor {
                    kind,
                    unable: false
                }
                .stem(),
                stem
            );
            assert_eq!(
                WorldCursor { kind, unable: true }.stem(),
                format!("Unable{stem}")
            );
        }
    }
}
