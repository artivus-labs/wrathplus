//! ObjectIcons: Vanilla classifier 0x4eaa90; Wrath classifier 0x57f7f0. Native quest
//! cells are yellow ! 9, yellow ? 10, blue ! 11 (UV constructor 0x583718).

use std::collections::HashMap;

use bevy::math::Rect;
use bevy::prelude::*;

use wrathplus_assets::coords::bevy_to_wow;
use wrathplus_formats::{LockCatalog, ShapeshiftForm, LOCK_KEY_SKILL};
use wrathplus_protocol::EntityKind;

use crate::app_core::session::ObjectStore;
use crate::creature_type::CreatureTypeSources;
use crate::go_templates::GameObjectTemplates;
use crate::names::NameCache;
use crate::net::{GuidIndex, NetEntity};
use crate::overlay::UiQuad;
use crate::overlay::UiQuads;
use crate::overlay::UvRect;

use super::{party_member_pos, BlipCtx, MinimapBlipHover, TrackedCandidates, BLIP_BASIS_PX};

/// Every ObjectIcons and PartyRaidBlips dot's quad, quest, tracking and party alike: 12340 draws
/// them all (`0x580380`, `0x57ff70`) as the corner set `0xbf7fc8` × the frame's `+0x7c`, and the
/// minimap ctor (`0x57dca0`) sizes that set `0.0125 × 0.6⁻¹·[0xac0cb8]`, 2.083 % of the screen
/// height whatever the aspect (`0x47bf90`): 16 px on the 768-px basis. Labelled: `+0x7c` is
/// taken as 1, the frame's scale at the default UI scale.
pub(super) const OBJECT_ICON_PX: f32 = 16.0;
/// `UNIT_DYNAMIC_FLAGS` bit 0x2, set per viewer on a Hunter's Mark victim for its caster; one of
/// `0x5ed210`'s two always-show clauses (`+0x224 & 0x2`).
const UNIT_DYNFLAG_TRACK_UNIT: u32 = 0x2;
/// Vanilla: only DIALOG_STATUS 7 draws a quest dot, the gold cell 3 (`cmp [obj+0xcb8],7` at `0x4eac31`).
#[cfg(test)]
fn quest_dot_cell(status: u32) -> Option<[f32; 4]> {
    (status == 7).then_some([0.75, 1.0, 0.0, 0.25])
}

/// The installed Wrath atlas is 256x64, 8x2 32px cells, versus Vanilla's 128x128.
/// Installed pixels and 0x57f7f0 agree: party=0, red=2, gold=6, !=9, ?=10, blue !=11.
/// No grey '!' or skull exists in this Wrath atlas; the corpse uses POIIcons separately.
#[derive(Clone, Copy, Default)]
pub(in crate::minimap) enum ObjectIconAtlas {
    #[default]
    Vanilla,
    Wrath,
}

#[derive(Clone, Copy)]
enum ObjectIcon {
    TrackedResource,
    TrackedUnit,
    TrackedTown,
    Party,
    CompleteQuest,
    AvailableQuest,
    RepeatableQuest,
}

impl ObjectIconAtlas {
    pub(in crate::minimap) fn from_size(width: u32, height: u32) -> Self {
        if (width, height) == (256, 64) {
            Self::Wrath
        } else {
            Self::Vanilla
        }
    }
    fn icon(self, icon: ObjectIcon) -> Option<[f32; 4]> {
        use ObjectIcon::*;
        let (index, columns, rows) = match self {
            Self::Vanilla => (
                match icon {
                    TrackedResource => 0,
                    TrackedUnit => 1,
                    TrackedTown => return None,
                    Party => 4,
                    CompleteQuest => 3,
                    AvailableQuest | RepeatableQuest => return None,
                },
                4,
                4,
            ),
            Self::Wrath => (
                match icon {
                    TrackedResource => 6,
                    TrackedUnit => 2,
                    TrackedTown => 8,
                    Party => 0,
                    CompleteQuest => 10,
                    AvailableQuest => 9,
                    RepeatableQuest => 11,
                },
                8,
                2,
            ),
        };
        let col = (index % columns) as f32;
        let row = (index / columns) as f32;
        Some([
            col / columns as f32,
            (col + 1.0) / columns as f32,
            row / rows as f32,
            (row + 1.0) / rows as f32,
        ])
    }
    #[cfg(test)]
    fn cell(self, status: u32) -> Option<[f32; 4]> {
        self.quest_cell(status, false)
    }

    /// Wow.exe 12340: unit classifier 0x57f9e1..0x57fa23 and GO 0x57f8d6..0x57f918.
    /// AC QuestDef.h:110-124. Status 3/6/9 do not produce quest blips; ordinary tracking may.
    /// Trivial offers (2/4) use the same ! cells, only with type-3 tracking (0x57bf30).
    fn quest_cell(self, status: u32, trivial: bool) -> Option<[f32; 4]> {
        use ObjectIcon::*;
        let icon = match self {
            Self::Vanilla if status == 7 => CompleteQuest,
            Self::Wrath => match status {
                10 => CompleteQuest,
                8 => AvailableQuest,
                7 => RepeatableQuest,
                2 if trivial => AvailableQuest,
                4 if trivial => RepeatableQuest,
                _ => return None,
            },
            _ => return None,
        };
        self.icon(icon)
    }
}

/// The three gates `0x4eaa90` applies to a unit or player ahead of the status-7 compare at
/// `0x4eac31`, so they bar the quest dot and the tracking dot alike, never a GameObject or party
/// dot. A unit with no descriptor yet fails, as a zeroed health field does in the reference.
fn unit_dot_eligible(store: Option<&ObjectStore>, me: Option<u64>) -> bool {
    let Some(f) = store.map(|s| &s.0) else {
        return false;
    };
    // 1. A signed `<= 0` on `UNIT_FIELD_HEALTH` (`jle` at `0x4eac1e`): the dead get no dot.
    if f.unit_health().unwrap_or(0) as i32 <= 0 {
        return false;
    }
    // 2. The owner, `UNIT_FIELD_CHARMEDBY` when non-zero else `UNIT_FIELD_SUMMONEDBY`, equal to
    //    our guid (`0x4d3790`) rejects: our own pet, minion or charmed unit is never a blip.
    let owner = match f.unit_charmed_by() {
        Some(g) if g != 0 => g,
        _ => f.unit_summoned_by().unwrap_or(0),
    };
    if owner != 0 && Some(owner) == me {
        return false;
    }
    // 3. `byte [eax+0x213] & 4`, `UNIT_FIELD_BYTES_1` byte 3 bit 2 (vmangos's untrackable name).
    !f.unit_is_untrackable()
}

/// Draw the quest status icon (Wrath yellow ?, Vanilla status-7 dot), culled in 3-D with
/// no rim arrow. Runs after the player arrow, so the dots draw on top.
pub(in crate::minimap) fn emit_quest_dots(
    ctx: &BlipCtx,
    statuses: &HashMap<u64, u32>,
    candidates: &TrackedCandidates,
    self_guid: Option<u64>,
    icons: &Handle<Image>,
    atlas: ObjectIconAtlas,
    tracking: SelfTracking,
    player_indoors: bool,
    unit_indoors: impl Fn(Vec3) -> bool,
    quads: &mut UiQuads,
    hover: &mut MinimapBlipHover,
) {
    for (guid, net, tf, store) in candidates.iter() {
        let npc = guid.0;
        if !matches!(net.kind, EntityKind::Unit | EntityKind::Player)
            && !(matches!(atlas, ObjectIconAtlas::Wrath) && net.kind == EntityKind::GameObject)
        {
            continue;
        }
        if town_tracked(tracking, net.kind, store) {
            continue; // the town dot outranks the quest mark
        }
        let Some(cell) = statuses
            .get(&npc)
            .copied()
            .and_then(|s| atlas.quest_cell(s, tracking.trivial))
        else {
            continue;
        };
        if net.kind != EntityKind::GameObject && !unit_dot_eligible(store, self_guid) {
            continue;
        }
        let w = bevy_to_wow(tf.translation());
        let d3 =
            ((w[0] - ctx.wx).powi(2) + (w[1] - ctx.wy).powi(2) + (w[2] - ctx.wz).powi(2)).sqrt();
        if d3 > ctx.radius_yd {
            continue;
        }
        // The grey `0xffb0b0b0` marks a dot across an interior boundary; the reference decides
        // through `0x670540`, untraced, and this uses an indoor-containment mismatch.
        let grey = unit_indoors(tf.translation()) != player_indoors;
        let tint = if grey { 0xb0 as f32 / 255.0 } else { 1.0 };
        let rect = Rect::from_center_size(
            ctx.center + ctx.offset(w),
            Vec2::splat(ctx.side * (OBJECT_ICON_PX / BLIP_BASIS_PX)),
        );
        quads.overlays.push(UiQuad {
            rect,
            z_key: ctx.z,
            texture: Some(icons.clone()),
            uv: UvRect::from_tex_coords(cell),
            color: [tint, tint, tint, ctx.alpha],
            ..default()
        });
        if let (Some(c), Some(ui)) = (ctx.cursor, ctx.cursor_ui) {
            if rect.contains(c) {
                *hover = MinimapBlipHover::Npc(npc, ui, grey);
            }
        }
    }
}

/// Resource tracking (`0x5ed2b0`): any skill-keyed `Lock.dbc` slot of the lock whose `LockType`
/// `n` has bit `1 << (n − 1)` set in `PLAYER_TRACK_RESOURCES`, the bit the server sets from the
/// aura's MiscValue (vmangos `HandleAuraTrackResources`).
fn tracked_resource(mask: u32, lock_id: u32, locks: &LockCatalog) -> bool {
    if mask == 0 || lock_id == 0 {
        return false;
    }
    locks.slots(lock_id).is_some_and(|slots| {
        slots.iter().any(|s| {
            s.key_type == LOCK_KEY_SKILL
                && (1..=32).contains(&s.index)
                && mask & (1u32 << (s.index - 1)) != 0
        })
    })
}

/// Our tracking masks and the track-stealthed bit (`PLAYER_FIELD_BYTES & 0x2`), with the tracking
/// menu's selection: trivial quests (`0x57bf30`) or a town type, `(kind, mask)` (`[0xbeba64]`).
#[derive(Clone, Copy, Default)]
pub(in crate::minimap) struct SelfTracking {
    pub(in crate::minimap) creatures: u32,
    pub(in crate::minimap) resources: u32,
    pub(in crate::minimap) stealthed: bool,
    pub(in crate::minimap) trivial: bool,
    pub(in crate::minimap) town: Option<(u8, u32)>,
}

/// Town tracking (`0x71acf0`, `0x70f850`): a kind-1 row's mask against a unit's `UNIT_NPC_FLAGS`,
/// or a kind-2 row's game-object type against an object's. Wrath draws either as the yellow dot,
/// cell 8, ahead of any quest or tracking dot (`0x57f8cc`, `0x57f9d7`). Labelled: the visibility
/// tests beside them (`0x729530`, `0x71f8b0`) and the profession-trainer flag a 0x40 mask sets
/// on the blip are untraced.
fn town_tracked(tracking: SelfTracking, kind: EntityKind, store: Option<&ObjectStore>) -> bool {
    let (Some((row, mask)), Some(s)) = (tracking.town, store) else {
        return false;
    };
    match (row, kind) {
        (1, EntityKind::Unit) => s.0.unit_npc_flags() & mask != 0,
        (2, EntityKind::GameObject) => s.0.gameobject_type_id() == mask as i32,
        _ => false,
    }
}

/// Creature tracking (`0x5ed210`): either always-show clause, `UNIT_DYNFLAG_TRACK_UNIT` or our
/// track-stealthed bit (aura 151) with the unit's CREEP flag, else the creature type's bit
/// `type − 1` in `PLAYER_TRACK_CREATURES`. The predicate itself tests no alive/dead or faction.
fn tracked_creature(
    tracking: SelfTracking,
    creature_type: Option<u32>,
    dynamic_flags: u32,
    unit_creeping: bool,
) -> bool {
    if dynamic_flags & UNIT_DYNFLAG_TRACK_UNIT != 0 {
        return true;
    }
    if tracking.stealthed && unit_creeping {
        return true;
    }
    tracking.creatures != 0
        && creature_type
            .is_some_and(|t| (1..=32).contains(&t) && tracking.creatures & (1u32 << (t - 1)) != 0)
}

/// Draw the tracking dots, gold cell 0 per GameObject then red cell 1 per unit not at quest
/// status 7 (`0x4eac31` tests that first). Drawn before the quest and party dots, as the cell
/// lists draw in order.
pub(in crate::minimap) fn emit_tracking_dots(
    ctx: &BlipCtx,
    tracking: SelfTracking,
    candidates: &TrackedCandidates,
    statuses: &HashMap<u64, u32>,
    self_guid: Option<u64>,
    names: &NameCache,
    templates: &GameObjectTemplates,
    locks: Option<&LockCatalog>,
    forms: Option<&HashMap<u32, ShapeshiftForm>>,
    icons: &Handle<Image>,
    atlas: ObjectIconAtlas,
    player_indoors: bool,
    unit_indoors: impl Fn(Vec3) -> bool,
    quads: &mut UiQuads,
    hover: &mut MinimapBlipHover,
) {
    // The unit pass runs even with an empty creature mask: the always-show clauses need none.
    let mut dot = |guid: u64, tf: &GlobalTransform, cell: [f32; 4], name: DotName| {
        let w = bevy_to_wow(tf.translation());
        let d3 =
            ((w[0] - ctx.wx).powi(2) + (w[1] - ctx.wy).powi(2) + (w[2] - ctx.wz).powi(2)).sqrt();
        if d3 > ctx.radius_yd {
            return;
        }
        let grey = unit_indoors(tf.translation()) != player_indoors;
        let tint = if grey { 0xb0 as f32 / 255.0 } else { 1.0 };
        let rect = Rect::from_center_size(
            ctx.center + ctx.offset(w),
            Vec2::splat(ctx.side * (OBJECT_ICON_PX / BLIP_BASIS_PX)),
        );
        quads.overlays.push(UiQuad {
            rect,
            z_key: ctx.z,
            texture: Some(icons.clone()),
            uv: UvRect::from_tex_coords(cell),
            color: [tint, tint, tint, ctx.alpha],
            ..default()
        });
        if let (Some(c), Some(ui)) = (ctx.cursor, ctx.cursor_ui) {
            if rect.contains(c) {
                *hover = match name {
                    DotName::Guid => MinimapBlipHover::Npc(guid, ui, grey),
                    DotName::Known(n) => MinimapBlipHover::TrackedGo(n, ui, grey),
                };
            }
        }
    };
    // The town dot first, Wrath cell 8: it takes the object from every other dot (`0x57f7f0`).
    if let Some(cell) = atlas.icon(ObjectIcon::TrackedTown) {
        for (guid, net, tf, store) in candidates.iter() {
            let unit = net.kind == EntityKind::Unit;
            if !town_tracked(tracking, net.kind, store)
                || (unit && !unit_dot_eligible(store, self_guid))
            {
                continue;
            }
            let name = match templates.get(guid.0) {
                Some(t) if !unit => DotName::Known(t.name.clone()),
                _ => DotName::Guid,
            };
            dot(guid.0, tf, cell, name);
        }
    }
    // Cell 0, GameObjects. No quest-status check: the GameObject leg (`0x4eab43`) goes straight
    // to `0x5ed2b0` and never reaches the status-7 compare.
    if tracking.resources != 0 {
        if let Some(locks) = locks {
            for (guid, net, tf, store) in candidates.iter() {
                if net.kind != EntityKind::GameObject || town_tracked(tracking, net.kind, store) {
                    continue;
                }
                let Some(t) = templates.get(guid.0) else {
                    continue; // template not answered yet
                };
                if tracked_resource(tracking.resources, t.lock_id, locks) {
                    dot(
                        guid.0,
                        tf,
                        atlas.icon(ObjectIcon::TrackedResource).unwrap(),
                        DotName::Known(t.name.clone()),
                    );
                }
            }
        }
    }
    // Cell 1, units.
    let types = CreatureTypeSources {
        names: Some(names),
        forms,
    };
    for (guid, net, tf, store) in candidates.iter() {
        if !matches!(net.kind, EntityKind::Unit | EntityKind::Player) {
            continue;
        }
        if !unit_dot_eligible(store, self_guid) || town_tracked(tracking, net.kind, store) {
            continue;
        }
        if statuses
            .get(&guid.0)
            .copied()
            .and_then(|s| atlas.quest_cell(s, tracking.trivial))
            .is_some()
        {
            continue; // the quest branch already drew this status icon
        }
        let (dyn_flags, creeping) = store
            .map(|s| (s.0.unit_dynamic_flags(), s.0.unit_is_stealthed()))
            .unwrap_or((0, false));
        let creature_type = store.map(|s| types.of(s));
        if tracked_creature(tracking, creature_type, dyn_flags, creeping) {
            dot(
                guid.0,
                tf,
                atlas.icon(ObjectIcon::TrackedUnit).unwrap(),
                DotName::Guid,
            );
        }
    }
}

/// How a tracking dot's hover names itself: a unit resolves by guid through the name cache;
/// a GameObject's template name is already in hand.
enum DotName {
    Guid,
    Known(String),
}

/// The party dots, the in-range half of the party placement `0x6dad10`: blue cell 4 at 1.3×,
/// drawn last with the object dots, above every arrow (`0x4ed7b7`).
pub(in crate::minimap) fn emit_party_dots(
    ctx: &BlipCtx,
    group: &crate::app_core::group::GroupState,
    guids: &GuidIndex,
    unit_pos: &Query<&GlobalTransform, With<NetEntity>>,
    icons: &Handle<Image>,
    atlas: ObjectIconAtlas,
    quads: &mut UiQuads,
) {
    for m in group.party_slots() {
        let Some((x, y)) = party_member_pos(m, group, guids, unit_pos) else {
            continue;
        };
        let d = ((x - ctx.wx).powi(2) + (y - ctx.wy).powi(2)).sqrt();
        if d / ctx.radius_yd > super::BLIP_EDGE_RATIO {
            continue; // out of range: the arrow pass drew it
        }
        quads.overlays.push(UiQuad {
            rect: Rect::from_center_size(
                ctx.center + ctx.offset([x, y, 0.0]),
                Vec2::splat(ctx.side * (OBJECT_ICON_PX / BLIP_BASIS_PX)),
            ),
            z_key: ctx.z,
            texture: Some(icons.clone()),
            uv: UvRect::from_tex_coords(atlas.icon(ObjectIcon::Party).unwrap()),
            color: [1.0, 1.0, 1.0, ctx.alpha],
            ..default()
        });
    }
}

#[cfg(test)]
#[path = "dots_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "quest_status_tests.rs"]
mod quest_status_tests;
