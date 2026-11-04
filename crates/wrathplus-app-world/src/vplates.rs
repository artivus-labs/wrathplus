//! V-key nameplates, `CGNamePlateFrame` (ctor `0x7cb250`): health-bar plates drawn as a 2-D
//! overlay that replaces a unit's overhead name. This file owns the world side (gate, anchor,
//! projection, seat, basis); the widgets live in [`wrathplus_ui::script::nameplate`].
//!
//! - Toggles: the eight `nameplateShow*` CVars ([`VPlateMode`]), which the stock V keys write;
//!   build 12340 has no plate verbs.
//! - Gate, build 12340's `0x72b060`: never the own unit, a `NOT_SELECTABLE` or dead one, a critter
//!   or non-combat pet, or a stealthed enemy; friendly is not `CanAttack` from the player, and for
//!   a player subject also an equal faction-group mask (`0x729b30`), and a player that is neither
//!   friendly nor attackable has none; then the category's CVar, and a player-owned totem, pet or
//!   guardian its sub-gate ([`unit_sort`]); 41 yd, none in an arena; no occlusion; a unit not
//!   projected into view loses its plate.
//! - Anchor: the overhead head point (`0x608640`) plus 2/3 yd (`[0x80abfc]`), projected every
//!   frame (`0x483ee0`), the plate's top centre on it (`0x509ec0`), sized in gx units of the
//!   screen diagonal (`0x41ad10`), outside uiScale. Deviation: the basis is damped past
//!   [`PLATE_DIAG_KNEE`], because the reference's unbounded growth (`0x7705b0`) draws plates too
//!   large and soft at high resolutions.
//! - Highlight: the mouseover or the target (`0x606f20` into `0x607080`) lights the plate.
//!   Deviation: the bar fill brightens by `LIT_BOOST` in place of the reference's ADD
//!   `Nameplate-Glow` rim, which reads as hard edge lines under WrathPlus's linear blending.
//!   Hovering a plate makes its unit the mouseover (`0x7cb850`) and turns the name yellow; a
//!   click on it selects.
//! - With a target, every other plate dims ([`DIM_ALPHA`]).

use bevy::ecs::entity::EntityHashSet;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use wrathplus_protocol::{field, EntityKind};
use wrathplus_ui::script::{unit_is_grey, PlateGeometry, PlateState};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Factions;
use crate::app_core::state::Selection;
use crate::entities::{overhead_anchor, BoneAttach, OverheadFallback};
use crate::names::NameCache;
use crate::net::{Guid, NetCommands, NetEntity, Reputations};
use crate::target::{Hovered, TargetUpdate};
use wrathplus_world::view::WorldCamera;

/// The plate border, sharp-resampled so its 128x32 art stays crisp when magnified.
pub mod border;
mod cast;
use cast::{target_cast, threat_flash};

/// The eight `nameplateShow*` CVars the plate gate reads (`[0xbd09b8]`..`[0xbd09d4]`, registered
/// in `0x51d9b0`): a category on/off for enemies and friends, each with pet, guardian and totem
/// sub-gates for player-owned units. Defaults are the registered ones, and the CVars persist.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct VPlateMode {
    pub enemies: bool,
    pub enemy_pets: bool,
    pub(crate) enemy_guardians: bool,
    pub enemy_totems: bool,
    pub friends: bool,
    pub friendly_pets: bool,
    pub friendly_guardians: bool,
    pub friendly_totems: bool,
    /// `showVKeyCastbar`: the target's plate carries its cast bar.
    pub cast_bar: bool,
    /// `threatWarning`: when the plates' threat flash may show (`0x519df0`).
    pub threat_warning: u8,
}

impl Default for VPlateMode {
    fn default() -> Self {
        Self {
            enemies: false,
            enemy_pets: true,
            enemy_guardians: true,
            enemy_totems: true,
            friends: false,
            friendly_pets: true,
            friendly_guardians: true,
            friendly_totems: true,
            cast_bar: true,
            threat_warning: 3,
        }
    }
}

impl VPlateMode {
    /// Whether a unit of this reaction and sort may have a plate: the category CVar, then the
    /// sub-gate for a player-owned totem, pet or guardian (`0x72b1a5`-`0x72b28c`).
    pub(crate) fn shows(&self, friendly: bool, sort: UnitSort) -> bool {
        let (base, pets, guardians, totems) = if friendly {
            (
                self.friends,
                self.friendly_pets,
                self.friendly_guardians,
                self.friendly_totems,
            )
        } else {
            (
                self.enemies,
                self.enemy_pets,
                self.enemy_guardians,
                self.enemy_totems,
            )
        };
        base && match sort {
            UnitSort::Pet => pets,
            UnitSort::Guardian => guardians,
            UnitSort::Totem => totems,
            UnitSort::Player | UnitSort::NonCombat | UnitSort::Creature => true,
        }
    }
}

/// The plate CVars' change callback.
pub fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut mode: ResMut<VPlateMode>) {
    if ev.key() == "threatwarning" {
        mode.threat_warning = ev.num().clamp(0.0, 255.0) as u8;
        return;
    }
    let slot = match ev.key().as_str() {
        "nameplateshowenemies" => &mut mode.enemies,
        "nameplateshowenemypets" => &mut mode.enemy_pets,
        "nameplateshowenemyguardians" => &mut mode.enemy_guardians,
        "nameplateshowenemytotems" => &mut mode.enemy_totems,
        "nameplateshowfriends" => &mut mode.friends,
        "nameplateshowfriendlypets" => &mut mode.friendly_pets,
        "nameplateshowfriendlyguardians" => &mut mode.friendly_guardians,
        "nameplateshowfriendlytotems" => &mut mode.friendly_totems,
        "showvkeycastbar" => &mut mode.cast_bar,
        _ => return,
    };
    *slot = ev.flag();
}

/// What the plate gate and the overhead names sort a unit into, by build 12340's checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitSort {
    Player,
    /// Creature type 8 (Critter) or 12 (Non-combat Pet): never a plate (`0x72b1c8`, `0x72b1d6`).
    NonCombat,
    /// A player-owned totem, creature type 11.
    Totem,
    /// A player-owned unit whose only link is its creator (`0x71b600`: no charmer or summoner).
    Guardian,
    /// Any other player-owned unit: a summoned or charmed one.
    Pet,
    /// Everything else.
    Creature,
}

/// Sort a unit off its own fields and its template's creature type (`0x71f300` reads the
/// template's, past a shapeshift's own). Unresolved, labelled: the owner lookup `0x4f5f20` is
/// untraced, so the owner is the charmer, else the summoner, else the creator, and a player one
/// is told by its guid.
pub(crate) fn unit_sort(
    is_player: bool,
    fields: &wrathplus_protocol::ObjectFields,
    creature_type: Option<u32>,
) -> UnitSort {
    if is_player {
        return UnitSort::Player;
    }
    if matches!(creature_type, Some(8 | 12)) {
        return UnitSort::NonCombat;
    }
    let linked = |g: Option<u64>| g.filter(|&g| g != 0);
    let (charmer, summoner, creator) = (
        linked(fields.unit_charmed_by()),
        linked(fields.unit_summoned_by()),
        linked(fields.unit_created_by()),
    );
    let owner = charmer.or(summoner).or(creator);
    if !owner.is_some_and(wrathplus_protocol::guid::is_player) {
        return UnitSort::Creature;
    }
    if creature_type == Some(11) {
        UnitSort::Totem
    } else if charmer.is_none() && summoner.is_none() {
        UnitSort::Guardian
    } else {
        UnitSort::Pet
    }
}

/// The units carrying a live plate this frame; the overhead-name driver draws no name for them
/// (the reference's ShouldShowName).
#[derive(Resource, Default)]
pub struct VPlates(pub EntityHashSet);

/// The unit whose plate the pointer is inside, from last frame's layout: the plate's OnEnter
/// publishing the mouseover (`0x7cb850` into `[0xb4e2c8]`), since the world pick stands down
/// over UI.
#[derive(Resource, Default)]
pub struct PlateHover(pub Option<Entity>);

/// Completed plate clicks, physical or an addon's `plate:Click`, waiting for the targeting chain:
/// the reference's click slot (`0x7cb910`) ends in the same `SetSelection` as a body click.
#[derive(Resource, Default)]
pub(crate) struct PlateClicks {
    pub(crate) left: Vec<Entity>,
    pub(crate) right: Vec<Entity>,
}

/// The plate frame in gx units (`[0x87d9cc]`, `[0x87d9d0]`); child offsets from `0x7cb250` and
/// `0x7cb6d0`.
const PLATE_W: f32 = 0.1;
const PLATE_H: f32 = 0.025;
/// The health bar: BOTTOMLEFT at the plate's BOTTOMLEFT plus the offset.
const BAR_OFF_X: f32 = 0.0031;
const BAR_OFF_Y: f32 = 0.003125;
const BAR_W: f32 = 0.0804;
const BAR_H: f32 = 0.007025;
/// The name's BOTTOM sits at the plate CENTER; the level's CENTER at plate BOTTOMRIGHT plus
/// (-0.0092, +0.0071); the skull overlays the level. Deviation: `LEVEL_H` is 0.0086, one em under
/// the reference's 0.009, because the level number read too large.
const NAME_H: f32 = 0.01;
const LEVEL_H: f32 = 0.0086;
const LEVEL_OFF_X: f32 = 0.0092;
const LEVEL_OFF_Y: f32 = 0.0071;
const SKULL_SIZE: f32 = 0.01;
/// The raid-target icon (`0x7cb250`): its RIGHT on the border's LEFT, hanging off the plate's
/// left edge.
const RAID_ICON_SIZE: f32 = 0.02;
/// `[0x80abfc]`.
const PLATE_LIFT: f32 = 2.0 / 3.0;
/// 41 yd squared, `[0xadaa7c]` = 1681.0, compared at `0x72b33d`; an arena (`Map.dbc` instance
/// type 4) has no limit (`0x72b2df`).
const MAX_DIST_SQ: f32 = 1681.0;
/// `Map.dbc` `InstanceType` 4, an arena.
const ARENA: u32 = 4;
/// The creature template `type_flags` bit that never takes a plate (`[unit+0x964]+0xc`,
/// `0x72b10f`). Unresolved, labelled: its name and the template slot are untraced.
const TYPE_FLAG_NO_PLATE: u32 = 0x10_0000;
/// Deviation: the reference dims non-target plates to `0x7F`; raised because that fades them too
/// far to read.
const DIM_ALPHA: f32 = 178.0 / 255.0;

/// The bar-fill palette, the dwords at `0xcf60d0`/`e8`/`c8`/`dc`, in sRGB.
const PLATE_HOSTILE: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const PLATE_NEUTRAL: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
const PLATE_FRIENDLY: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const PLATE_PLAYER: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// `0x7cbaa0`'s test order: reaction 1 or less red, else a player blue, else 4 or more green,
/// else (2 and 3) yellow.
fn plate_tint(rank: u8, is_player: bool) -> [f32; 4] {
    if rank <= 1 {
        PLATE_HOSTILE
    } else if is_player {
        PLATE_PLAYER
    } else if rank >= 4 {
        PLATE_FRIENDLY
    } else {
        PLATE_NEUTRAL
    }
}

/// The level con palette, `0x7cbd50`'s dwords, not FrameXML's `QuestDifficultyColor`.
const CON_RED: [f32; 4] = [1.0, 25.0 / 255.0, 25.0 / 255.0, 1.0];
const CON_ORANGE: [f32; 4] = [1.0, 127.0 / 255.0, 63.0 / 255.0, 1.0];
const CON_YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];
const CON_GREEN: [f32; 4] = [63.0 / 255.0, 178.0 / 255.0, 63.0 / 255.0, 1.0];
const CON_GRAY: [f32; 4] = [127.0 / 255.0, 127.0 / 255.0, 127.0 / 255.0, 1.0];

/// The con colour (`0x7cbd50`): +5 or more red, +3/+4 orange, -2..+2 yellow, then green until
/// the shared grey test ([`unit_is_grey`], table `[0x81dda8]`) says gray.
fn con_color(pl_level: u32, unit_level: u32) -> [f32; 4] {
    let diff = i64::from(unit_level) - i64::from(pl_level);
    if diff >= 5 {
        CON_RED
    } else if diff >= 3 {
        CON_ORANGE
    } else if diff >= -2 {
        CON_YELLOW
    } else if !unit_is_grey(pl_level, unit_level) {
        CON_GREEN
    } else {
        CON_GRAY
    }
}

/// Snap a logical-pixel coordinate onto the device pixel grid, so the sharp-resampled border
/// blits 1:1 at any scale factor. Deviation: the reference draws at fractional device
/// coordinates; the snap keeps the magnified border crisp. Shared with the chat bubble.
pub(crate) fn device_snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

/// Deviation: the reference's plates grow with the diagonal without limit (`0x7705b0`), which
/// reads too big and soft; past this diagonal, where the frame is the border art's native
/// 128x32 px, the basis grows at [`PLATE_GROWTH_DAMP`] of that rate.
const PLATE_DIAG_KNEE: f32 = 1280.0;
/// 0 pins the native size, 1 is the reference's rate.
const PLATE_GROWTH_DAMP: f32 = 0.5;

/// One gx unit in pixels: the screen diagonal (`0x41ad10`), never uiScale, damped past
/// [`PLATE_DIAG_KNEE`]. Shared with the chat bubble so its text holds the plate's em.
pub(crate) fn plate_basis(viewport: Vec2) -> f32 {
    let d = viewport.x.hypot(viewport.y);
    if d <= PLATE_DIAG_KNEE {
        d
    } else {
        PLATE_DIAG_KNEE + (d - PLATE_DIAG_KNEE) * PLATE_GROWTH_DAMP
    }
}

/// gx units to pixels for frame geometry; text takes [`text_px`].
pub(crate) fn gx_px(v: f32, basis: f32) -> f32 {
    (v * basis).round()
}

/// A plate FontString height to its em, `min(32, round(h * basis))`: the reference's
/// `0x44d040` and `0x5ca030` net `h` times the diagonal, capped at the 32 px atlas cell, and
/// re-rasterize on every resize (`0x5c2b50`).
pub(crate) fn text_px(h: f32, basis: f32) -> f32 {
    (h * basis).round().min(32.0)
}

/// The world and reaction inputs of the plate gate, bundled under the 16-param ceiling.
#[derive(SystemParam)]
#[allow(clippy::type_complexity)] // one bundled system param
struct PlateWorld<'w, 's> {
    units: Query<
        'w,
        's,
        (
            Entity,
            &'static NetEntity,
            &'static Guid,
            &'static Transform,
            Option<&'static ObjectStore>,
        ),
        Without<SelfPlayer>,
    >,
    self_q: Query<'w, 's, (&'static Transform, Option<&'static ObjectStore>), With<SelfPlayer>>,
    factions: Option<Res<'w, Factions>>,
    reputations: Res<'w, Reputations>,
    selection: Res<'w, Selection>,
    // With `selection`, the highlight's pair: the watcher's globals `[0xb4e2c8]`/`[0xb4e2d8]`.
    hovered: Res<'w, Hovered>,
    camera: Query<'w, 's, (&'static Camera, &'static Transform), With<WorldCamera>>,
    window: Query<'w, 's, &'static Window, With<bevy::window::PrimaryWindow>>,
    // The pending ground-target cast, for the plate's hit-test veto (`0x7cba30`).
    targeting: Res<'w, crate::spell::SpellTargeting>,
    /// `CanAttack`'s controller lookup, for the plate category.
    relations: crate::target::RelationInputs<'w, 's>,
    /// The arena exemption from the range limit.
    map: Option<Res<'w, wrathplus_world::world_map::CurrentMap>>,
    maps: Option<Res<'w, wrathplus_assets::MapCatalogRes>>,
    /// The target's cast bar: its cast slots and the spell's icon and attributes.
    casts: Res<'w, crate::spell::unit_cast::UnitCasts>,
    spells: Option<Res<'w, crate::app_core::state::Spells>>,
    /// The threat flash: the threat lists and whose situation they are read for.
    threat: Res<'w, crate::net::wrath_threat::ThreatStore>,
    self_guid: Res<'w, crate::app_core::session::SelfGuid>,
}

/// Every frame: gate the units, seat each plate and hand the result to the widget layer as
/// [`PlateState`]s. Runs after the targeting chain, whose selection it reads.
#[allow(clippy::type_complexity)] // one Bevy system's full input set
fn drive_vplates(
    mode: Res<VPlateMode>,
    mut plates: ResMut<VPlates>,
    mut plate_hover: ResMut<PlateHover>,
    mut plate_clicks: ResMut<PlateClicks>,
    rig: Res<crate::player::CameraControl>,
    world: PlateWorld,
    names: Res<NameCache>,
    net_commands: Res<NetCommands>,
    // `None` in a run with no UI VM.
    script: Option<NonSendMut<wrathplus_ui::script::UiScript>>,
    ui_scale: Res<crate::app_core::ui_flags::UiScaleCvar>,
    anchor_q: (
        Query<&BoneAttach>,
        Query<&wrathplus_world::rig_anim::RigPose>,
        Query<&OverheadFallback>,
        Query<&GlobalTransform>,
        Query<(), With<crate::entities::mount::MountChild>>,
    ),
    // Claim bucket 0, the plates', rebuilt every frame; combat text owns bucket 1.
    mut bucket: Local<crate::smart_rect::SmartBucket>,
    group: Res<crate::app_core::group::GroupState>,
    mut mouse_told: Local<crate::app_core::vm_memo::VmMemo<Option<bool>>>,
) {
    plates.0.clear();
    plate_hover.0 = None;
    bucket.clear();
    // Every early return retires the plates first, or the widgets stay on screen; the reference
    // hides each live plate and returns it to the pool (`0x608a10`).
    let retire_all = |script: Option<NonSendMut<wrathplus_ui::script::UiScript>>| {
        if let Some(mut script) = script {
            script.retire_nameplates();
        }
    };
    if !mode.enemies && !mode.friends {
        retire_all(script);
        return;
    }
    let in_arena = world
        .map
        .as_deref()
        .zip(world.maps.as_deref())
        .and_then(|(map, maps)| maps.0.instance_type(map.0))
        == Some(ARENA);
    let (Ok((cam, cam_pose)), Ok((self_tf, self_store))) =
        (world.camera.single(), world.self_q.single())
    else {
        retire_all(script);
        return;
    };
    let Some(mut script) = script else {
        return;
    };
    let cam_tf = GlobalTransform::from(*cam_pose);
    let Some(viewport) = cam.logical_viewport_size() else {
        script.retire_nameplates();
        return;
    };
    let basis = plate_basis(viewport);
    let gx = |v: f32| gx_px(v, basis);
    let window = world.window.single().ok();
    // Plates stop taking the mouse in freelook (`0x60f830`, called from `0x483e80`/`0x483e70` on
    // the transitions), written on the edge per VM.
    let looking = rig.is_looking();
    if *mouse_told.get(&script) != Some(looking) {
        *mouse_told.get(&script) = Some(looking);
        script.set_nameplate_mouse(!looking);
    }
    // While a ground-targeted spell is armed (`0x6e6320`'s `flag & 0x60`), plates refuse the hit
    // test (`0x7cba30`, the `+0x3c` override) without clearing the mouse-enabled bit, so
    // `IsMouseEnabled()` is unchanged.
    script.set_nameplate_hit_test_veto(
        world
            .targeting
            .wants(crate::spell::targeting::TargetingWants::Location),
    );
    let hovered_key = script.hovered_nameplate();
    // The cast slots' clock (`spell::unit_cast`).
    let now_ms = (script.now() * 1000.0) as u32;
    // `[0xbd08c0]`, the threat warning's switch (`0x519df0`): mode 1 in a dungeon or raid, 2 in
    // a group, 3 always. Unresolved, labelled: mode 2's third condition (`[0xbeb608]`) is untraced.
    let warning = match mode.threat_warning {
        0 => false,
        1 => world
            .map
            .as_deref()
            .zip(world.maps.as_deref())
            .and_then(|(map, maps)| maps.0.instance_type(map.0))
            .is_some_and(|t| matches!(t, 1 | 2)),
        2 => group.in_group,
        _ => true,
    };
    let clicked = script.take_nameplate_clicks();
    let my_level = self_store.and_then(|s| s.0.unit_level()).unwrap_or(1);
    let has_target = world.selection.target.is_some();

    let (pw, ph) = (gx(PLATE_W), gx(PLATE_H));
    let scale = window.map_or(1.0, |w| w.scale_factor());
    // Everything below is in window px, divided by this once for the widget layer's units.
    let seam = window.map_or(1.0, |w| {
        crate::app_core::ui_state::seam_scale(w.height(), ui_scale.0)
    });
    let mut states: Vec<PlateState> = Vec::new();

    // Plates seat in squared-distance order from the gx point (0.4, 0.3), Y mirrored from the
    // reference's Y-up space (`0x608870`, `0x6089a0`, walked by `0x608ce0`); each later plate is
    // solved off those already claimed. The undamped diagonal: this is position, not size.
    let diag = viewport.length();
    let sort_pt = Vec2::new(0.4 * diag, viewport.y - 0.3 * diag);
    let mut cands = Vec::new();
    for (entity, net, guid, tf, store) in &world.units {
        if !matches!(net.kind, EntityKind::Unit | EntityKind::Player) {
            continue;
        }
        // `NOT_SELECTABLE` (flags bit 25) never gets a plate, re-tested every tick (`0x60f622`,
        // from CGUnit's OnUpdate `0x607ed0`). A plate hover publishes the mouseover with no
        // selectability check (`0x7cb869`, unlike the world hover at `0x482982`), so this gate is
        // what keeps such a unit's tooltip away.
        if store.is_some_and(|s| s.0.unit_flags() & (1 << 25) != 0) {
            continue;
        }
        // Health 0 or less (absent reads 0) has no plate (`0x72b07a`).
        if store.is_none_or(|s| s.0.unit_health().unwrap_or(0) == 0) {
            continue;
        }
        let is_player = net.kind == EntityKind::Player;
        let template = store
            .and_then(|s| s.0.object_entry())
            .filter(|_| !is_player)
            .and_then(|entry| names.creature_record(entry));
        let sort = store.map_or(UnitSort::Creature, |s| {
            unit_sort(is_player, &s.0, template.map(|t| t.creature_type))
        });
        // Critters and non-combat pets never (`0x72b1c8`, `0x72b1d6`).
        if sort == UnitSort::NonCombat {
            continue;
        }
        if template.is_some_and(|t| t.type_flags & TYPE_FLAG_NO_PLATE != 0) {
            continue;
        }
        // The category is `CanAttack` from the player (a rep faction answers by its at-war bit);
        // the bar colour is the unit's reaction toward us (by standing). A not-at-war neutral NPC
        // is a friendly plate with a yellow bar.
        let rank = world.relations.ring_reaction(store, self_store);
        let friendly = crate::target::ring::plate_is_friendly(
            world.factions.as_deref(),
            &world.reputations,
            store,
            self_store,
            is_player,
            world.relations.context(),
        );
        // A stealthed unit that is not friendly (`UNIT_FIELD_BYTES_1` byte 2 `0x2`, `0x72b0e6`).
        if !friendly && store.is_some_and(|s| s.0.unit_is_stealthed()) {
            continue;
        }
        if !mode.shows(friendly, sort) {
            continue;
        }
        // `UNIT_DYNFLAG_DEAD` (feign death included) takes the plate on either side (`0x72b1e4`).
        // Unresolved, labelled: the reference keeps it where `0x512a30` passes the unit's
        // position, an untraced map test; here it never does.
        if store.is_some_and(|s| s.0.unit_dynamic_flags() & field::UNIT_DYNFLAG_DEAD != 0) {
            continue;
        }
        // A player-owned unit with a creator, no summoner and flags bit 9 (`0x200`): no plate in
        // either category (`0x72b1bb`); the bit's name is unknown.
        if matches!(sort, UnitSort::Pet | UnitSort::Guardian | UnitSort::Totem)
            && store.is_some_and(|s| {
                s.0.unit_created_by().is_some_and(|g| g != 0)
                    && s.0.unit_summoned_by().is_none_or(|g| g == 0)
                    && s.0.unit_flags() & 0x200 != 0
            })
        {
            continue;
        }
        // A player that is neither friendly nor attackable has none (`0x72b290`).
        if is_player
            && !friendly
            && !crate::target::ring::can_attack_from_player(
                world.factions.as_deref(),
                &world.reputations,
                store,
                self_store,
                world.relations.context(),
            )
        {
            continue;
        }
        if !in_arena && (tf.translation - self_tf.translation).length_squared() > MAX_DIST_SQ {
            continue;
        }
        // Behind the camera or outside the viewport there is no plate: `0x60f600` destroys it
        // before the seat, never clamping it in from off-screen.
        let anchor = overhead_anchor(
            entity,
            tf,
            &anchor_q.0,
            &anchor_q.1,
            &anchor_q.2,
            &anchor_q.3,
            &anchor_q.4,
        );
        let Some(screen) =
            crate::overlay::project_overlay(cam, &cam_tf, anchor + Vec3::Y * PLATE_LIFT, viewport)
        else {
            continue;
        };
        cands.push((
            screen.distance_squared(sort_pt),
            screen,
            anchor,
            rank,
            is_player,
            entity,
            guid,
            store,
        ));
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, screen, anchor, rank, is_player, entity, guid, store) in cands {
        plates.0.insert(entity);

        let alpha = if !has_target || world.selection.target == Some(entity) {
            1.0
        } else {
            DIM_ALPHA
        };
        let tint = plate_tint(rank, is_player);
        // `WOW_VPLATE_TRACE=1` prints each plate rect in logical px.
        static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let trace = *TRACE.get_or_init(|| std::env::var("WOW_VPLATE_TRACE").as_deref() == Ok("1"));
        // The seat (`0x509ec0`): the plate hangs from its top centre on the projected point, is
        // solved off the plates already claimed, then clamped half a plate inside the screen.
        let desired = Rect::new(
            screen.x - pw * 0.5,
            screen.y,
            screen.x + pw * 0.5,
            screen.y + ph,
        );
        let solved = bucket.resolve(desired, viewport);
        // Snap the left edge, not the centre, which an odd width would put between texels; the
        // claim takes the snapped rect, so later plates dodge what is drawn.
        let top = device_snap(solved.min.y, scale);
        let left = device_snap(solved.min.x, scale);
        let plate = Rect::new(left, top, left + pw, top + ph);
        // Trace tag `vpl`, one line per plate per frame: anchor, camera, projected point, solve
        // and snapped rect, so plate jitter can be attributed to one of them.
        if wrathplus_assets::trace::enabled_for("vpl") {
            let (cp, cf) = (cam_pose.translation, cam_pose.forward());
            wrathplus_assets::trace::line(
                "vpl",
                &format!(
                    "e={} vp=({:.0},{:.0}) anchor=[{:.4},{:.4},{:.4}] cam=[{:.4},{:.4},{:.4}] \
                     fwd=[{:.4},{:.4},{:.4}] scr=({:.3},{:.3}) solved=({:.3},{:.3}) plate=({:.1},{:.1})",
                    entity.index(),
                    viewport.x,
                    viewport.y,
                    anchor.x,
                    anchor.y,
                    anchor.z,
                    cp.x,
                    cp.y,
                    cp.z,
                    cf.x,
                    cf.y,
                    cf.z,
                    screen.x,
                    screen.y,
                    solved.min.x,
                    solved.min.y,
                    plate.min.x,
                    plate.min.y,
                ),
            );
        }
        bucket.claim(plate);
        let hover = hovered_key == Some(guid.0);
        // The mouseover or the target lights the bar; `hover` joins directly so the brighten does
        // not lag the yellow name by the frame the mouseover takes to publish.
        let lit =
            hover || world.hovered.target == Some(entity) || world.selection.target == Some(entity);
        if trace {
            eprintln!(
                "vplate-trace: viewport={viewport:?} screen=({:.2},{:.2}) plate=({:.1},{:.1})..({:.1},{:.1}) lit={lit}",
                screen.x, screen.y, plate.min.x, plate.min.y, plate.max.x, plate.max.y
            );
        }
        // ── The plate's state, handed to the widget layer ──
        //
        // The reference's plates sit outside the uiScale cascade, but WrathPlus folds uiScale into
        // one global seam, so dividing by it here keeps plate pixels uiScale-blind.
        // The raid-target index is 1-based here, 0-based for the atlas cell.
        let mark = group.raid_target_index(guid.0);
        let x_units = (plate.min.x + plate.max.x) * 0.5 / seam;
        let y_units = (viewport.y - plate.min.y) / seam;
        if hovered_key == Some(guid.0) {
            plate_hover.0 = Some(entity);
        }
        for click in &clicked {
            if click.key == guid.0 {
                match click.button.as_str() {
                    "RightButton" => plate_clicks.right.push(entity),
                    _ => plate_clicks.left.push(entity),
                }
            }
        }
        states.push(PlateState {
            // A plate stays bound to its unit for its life, as the reference's `[unit+0xe60]`;
            // addons cache per plate on it.
            key: guid.0,
            top_centre: (x_units, y_units),
            // Raw health, not a fraction: `healthbar:GetValue()` is `[bar+0x320]` (`0x78f5d0`),
            // and addons divide it by the max.
            health: store.and_then(|s| s.0.unit_health()).unwrap_or(0) as f32,
            max_health: store
                .and_then(|s| s.0.unit_max_health())
                .unwrap_or(1)
                .max(1) as f32,
            bar_colour: [tint[0], tint[1], tint[2]],
            name: names
                .resolve_unit(guid.0, store, &net_commands)
                .map(str::to_owned)
                .unwrap_or_default(),
            // The skull (`0x7cbb40`): rank 3 through `gated_rank` (so a charmed boss shows its
            // level), or a hostile 10 or more levels up that is not grey (`0x5f0700`, vacuous
            // there but transcribed). A unit with no level yet shows neither.
            level: store.and_then(|s| s.0.unit_level()),
            skull: store.and_then(|s| s.0.unit_level()).is_some_and(|level| {
                crate::names::gated_rank(
                    store
                        .and_then(|s| s.0.object_entry())
                        .and_then(|e| names.creature_record(e)),
                    store,
                ) == 3
                    || (rank <= 1 && level >= my_level + 10 && !unit_is_grey(my_level, level))
            }),
            level_colour: {
                let c = store
                    .and_then(|s| s.0.unit_level())
                    .map_or([1.0, 1.0, 1.0, 1.0], |level| con_color(my_level, level));
                [c[0], c[1], c[2]]
            },
            // 0-based for the atlas: `col = idx & 3`, `row = idx >> 2`.
            raid_icon: (mark >= 1).then(|| mark - 1),
            alpha,
            lit,
            hovered: hover,
            // Rank 1 (elite) or 2 (rare elite) through the template (`0x98e6e0`).
            elite: store
                .and_then(|s| s.0.object_entry())
                .filter(|_| !is_player)
                .and_then(|e| names.creature_record(e))
                .is_some_and(|t| matches!(t.rank, 1 | 2)),
            cast: (mode.cast_bar && world.selection.target == Some(entity))
                .then(|| target_cast(&world.casts, world.spells.as_deref(), guid.0, now_ms))
                .flatten(),
            threat: warning
                .then(|| {
                    let me = world.self_guid.0?;
                    let mob = world.threat.0.mobs.get(&guid.0)?;
                    threat_flash(mob.situation(me)?)
                })
                .flatten(),
        });
    }

    // The widget layer rewrites the static anchors only when this moves, so an addon's own
    // re-anchoring survives between resizes.
    script.sync_nameplates(
        PlateGeometry {
            width: pw / seam,
            height: ph / seam,
            bar_off_x: gx(BAR_OFF_X) / seam,
            bar_off_y: gx(BAR_OFF_Y) / seam,
            bar_width: gx(BAR_W) / seam,
            bar_height: gx(BAR_H) / seam,
            level_off_x: gx(LEVEL_OFF_X) / seam,
            level_off_y: gx(LEVEL_OFF_Y) / seam,
            skull_size: gx(SKULL_SIZE) / seam,
            raid_size: gx(RAID_ICON_SIZE) / seam,
            name_height: text_px(NAME_H, basis) / seam,
            level_height: text_px(LEVEL_H, basis) / seam,
            // Deviation: a constant one-logical-pixel shadow, not the reference's 0.001 gx, which
            // draws 2 px thick and too heavy at larger windows. Divided by the seam, which the
            // shared text arm would otherwise scale it by.
            shadow_offset: 1.0 / seam,
        },
        &states,
    );
}

/// The plate driver's set; the overhead-name driver runs after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct VPlateSet;

/// V-key nameplates: the toggles and the per-frame gate and draw.
pub struct VPlatesPlugin;

impl Plugin for VPlatesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_cvar);
        app.init_resource::<VPlateMode>()
            .init_resource::<VPlates>()
            .init_resource::<PlateHover>()
            .init_resource::<PlateClicks>()
            .add_systems(
                Update,
                // After the targeting chain, itself after `WorldStage::Input`, so this projects
                // through this frame's camera; `ui_script::extract::paint_script` runs after it, so
                // the plates paint the same frame.
                drive_vplates.after(TargetUpdate).in_set(VPlateSet),
            );
    }
}

#[cfg(test)]
mod tests;
