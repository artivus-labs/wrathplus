//! The player's area identity for the UI: the `ZONE_CHANGED` event family and the host globals
//! behind `GetZoneText`, `GetSubZoneText`, `GetRealZoneText`, `GetMinimapZoneText` and
//! `GetZonePVPInfo`, all fired from the one updater `0x494780`.
//!
//! - A zone id change fires `ZONE_CHANGED_NEW_AREA` alone; else a text change fires
//!   `ZONE_CHANGED_INDOORS` or `ZONE_CHANGED` by the indoor bit. The first world-enter is a zone
//!   id change from the zeroed cache.
//! - The zone is the leaf's single-hop parent (AreaTable field 2, itself when 0).
//! - The indoor bit is the faces-only down-ray (`0x6a87f0`): the surface below is a WMO face whose
//!   group lacks MOGP `0x8` EXTERIOR.
//! - Indoor naming (`0x67e670`): while indoors in the same group the whole updater is skipped
//!   (`[0x868608]`, `[0x86860c]`). On a change, the whole-WMO row (`0x69d830`) overrides the zone
//!   slot and nulls the subzone when its name is non-empty and differs from the subzone; an
//!   unnamed row resolves through its `AreaTableID`, a missing row never overrides. Then the
//!   group's own row (`0x69d8f0`) refills the subzone. Both are exact-key lookups.
//! - `GetRealZoneText` reads the zone name before the indoor override.
//! - `GetMinimapZoneText` is subzone-else-zone. It has no event of its own in 3.3.5: 12340's event
//!   table has no `MINIMAP_ZONE_CHANGED`, and the minimap redraws on the `ZONE_CHANGED*` events.
//!
//! On the first world-enter `0x494780` also sets the world map to the player's zone
//! (`0x4a6650`); that half lives in the world map feed (`ui_world_map`).
//!
//! Host globals are written before the event fires, so a handler's `GetZoneText()` sees the new
//! state.
//!
//! Deviation: the indoor dedup keys `(WMOID, MOGP uniqueID)`, not the client's raw per-WMO group
//! index, because the raw index can alias across adjacent buildings.
//!
//! The reference's PvP display gate `[0x88272c]` (a cached boolean, not the realm type) is
//! untraced; ours is always open.

use bevy::ecs::system::NonSendMut;
use bevy::prelude::*;

use wrathplus_formats::AreaTableCatalog;
use wrathplus_ui::script::UiScript;

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Factions;
use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};

/// The shared `AreaTable.dbc` catalog, read by the world map and this module; absent if the DBC
/// failed to read.
#[derive(Resource)]
pub struct AreaTableRes(pub AreaTableCatalog);

fn load_area_table(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_area_table_catalog(&mut chain)
    };
    match loaded {
        Ok(cat) => {
            info!("area: {} rows in the shared AreaTable catalog", cat.len());
            commands.insert_resource(AreaTableRes(cat));
        }
        Err(e) => warn!("area: AreaTable catalog failed to load: {e:#}"),
    }
}

/// The client's zone-text cache: the compare targets of `0x494780`, the minimap line's cache
/// (`0x494970`) and the indoor dedup pair (`[0x868608]`, `[0x86860c]`). It lives per VM, so every
/// login starts zeroed and its first resolve is a NEW_AREA.
#[derive(Default)]
struct ZoneCache {
    /// `None` until the first resolve.
    zone_id: Option<u32>,
    zone_text: String,
    subzone_text: String,
    minimap_text: String,
    /// `[0x868608]`.
    indoor: bool,
    /// The previous hit group (`[0x86860c]`, keyed as the module doc's deviation says).
    wmo_id: u32,
    wmo_group: u32,
}

/// One resolve's compare inputs.
struct ZoneSignal {
    zone_id: u32,
    zone_text: String,
    subzone_text: String,
    indoor: bool,
}

/// The event election of `0x494780`.
fn elect_event(cache: &ZoneCache, next: &ZoneSignal) -> Option<&'static str> {
    if cache.zone_id != Some(next.zone_id) {
        return Some("ZONE_CHANGED_NEW_AREA");
    }
    if cache.zone_text != next.zone_text || cache.subzone_text != next.subzone_text {
        return Some(if next.indoor {
            "ZONE_CHANGED_INDOORS"
        } else {
            "ZONE_CHANGED"
        });
    }
    None
}

/// Resolves the texts and PvP info, pushes the zone caches, then fires the elected zone event.
fn feed_zone_events(
    script: Option<NonSendMut<UiScript>>,
    world: wrathplus_world::world_point::WorldPoint,
    areas: Option<Res<AreaTableRes>>,
    factions: Option<Res<Factions>>,
    self_store: Query<&ObjectStore, With<SelfPlayer>>,
    mut cache: Local<crate::app_core::vm_memo::VmMemo<ZoneCache>>,
) {
    let (Some(mut script), Some(areas)) = (script, areas) else {
        return;
    };
    let cache = cache.get(&script);
    let Some(leaf) = world.area() else { return };
    let Some(leaf_row) = areas.0.get(leaf) else {
        // An unknown id keeps the last state, as the resolver does for areaId 0.
        return;
    };
    let zone = if leaf_row.zone_id == 0 {
        leaf
    } else {
        leaf_row.zone_id
    };
    let real_zone_text = areas.0.name(zone).unwrap_or_default().to_string();
    let indoor = world.area_interior().is_some();

    // The outdoor texts `0x67e670` starts from.
    let mut zone_text = real_zone_text.clone();
    let mut subzone_text = if leaf == zone {
        String::new()
    } else {
        leaf_row.name.clone()
    };

    let mut wmo_group = 0u32;
    if let Some((k, group, default)) = world.area_interior_rows() {
        // The client keys `[groupRec+0x7c]`, nonzero-gated; see the module doc's deviation.
        wmo_group = k.group_area_id;
        if cache.indoor
            && k.group_area_id != 0
            && (cache.wmo_id, cache.wmo_group) == (k.wmo_id, wmo_group)
        {
            return;
        }
        cache.wmo_id = k.wmo_id;
        // The whole-WMO -1 row. An unnamed one with `AreaTableID` 0 takes the leaf's name, which
        // is the terrain areaId the client's area-0 arm reads.
        let a_name = default.as_ref().map(|d| {
            if !d.name.is_empty() {
                d.name.clone()
            } else if d.area_table_id != 0 {
                areas
                    .0
                    .name(d.area_table_id)
                    .unwrap_or_default()
                    .to_string()
            } else {
                leaf_row.name.clone()
            }
        });
        if let Some(a) = a_name.filter(|n| !n.is_empty()) {
            if a != subzone_text {
                zone_text = a;
                subzone_text = String::new();
            }
        }
        // The hit group's own row refills the subzone.
        if let Some(b) = group
            .as_ref()
            .map(|r| r.name.as_str())
            .filter(|n| !n.is_empty())
        {
            subzone_text = b.to_string();
        }
    }
    let signal = ZoneSignal {
        zone_id: zone,
        zone_text,
        subzone_text,
        indoor,
    };
    let minimap_text = if signal.subzone_text.is_empty() {
        signal.zone_text.clone()
    } else {
        signal.subzone_text.clone()
    };

    let event = elect_event(cache, &signal);
    let minimap_changed = cache.minimap_text != minimap_text;
    if event.is_none() && !minimap_changed {
        return;
    }

    // GetZonePVPInfo (`0x48d540`): isArena is the leaf's Flags `0x80`; the type is the zone's
    // FactionGroupMask against the player template's friend then enemy masks, "contested" when
    // neither; never "arena"; nil only on structural failure. factionName is FactionGroup.dbc's
    // Name for the zone's mask bit; the realm type never enters. Wrath runs `0x51ba50` instead
    // ([`wrath_zone_pvp`]), with its special-area types.
    let is_arena = leaf_row.flags & 0x80 != 0;
    let zone_row = areas.0.get(zone);
    let zone_mask = zone_row.map_or(0, |r| r.faction_group_mask);
    let template = factions
        .as_ref()
        .zip(self_store.single().ok())
        .map(|(f, store)| {
            store
                .0
                .unit_faction_template()
                .and_then(|t| f.catalog().template(t))
        });
    let faction_name = factions
        .as_deref()
        .and_then(|f| f.catalog().faction_group_name(zone_mask))
        .unwrap_or("");
    let (kind, pvp_is_subzone) = wrath_zone_pvp(Some(leaf_row), zone_row, template);
    let pvp = kind.map(|k| (k, faction_name));
    let pvp_type = pvp.map_or("", |(ty, _)| ty);

    script.set_zone_texts(wrathplus_ui::script::ZoneTexts {
        zone: signal.zone_text.clone(),
        real_zone: real_zone_text,
        subzone: signal.subzone_text.clone(),
        minimap: minimap_text.clone(),
        pvp_type: pvp.map(|(ty, _)| ty.to_string()),
        pvp_faction: pvp
            .map(|(_, faction)| faction)
            .filter(|f| !f.is_empty())
            .map(str::to_string),
        is_arena,
        pvp_is_subzone,
    });
    script.set_wrath_can_hearth_and_resurrect(can_hearth_and_resurrect(zone_row));

    if let Some(event) = event {
        script.fire_event(event, vec![]);
        debug!(
            "area: {event} → zone {:?} / sub {:?} ({pvp_type})",
            signal.zone_text, signal.subzone_text
        );
    }
    // No event of its own: 12340's event table has no `MINIMAP_ZONE_CHANGED`; the stock minimap
    // redraws its zone text on the `ZONE_CHANGED*` events (`Minimap.lua`'s `Minimap_Update`).
    if minimap_changed {
        debug!("area: minimap zone text → {minimap_text:?}");
    }
    cache.zone_id = Some(signal.zone_id);
    cache.zone_text = signal.zone_text;
    cache.subzone_text = signal.subzone_text;
    cache.minimap_text = minimap_text;
    cache.indoor = signal.indoor;
    cache.wmo_group = wmo_group;
}

/// AreaTable flags `GetZonePVPInfo 0x51ba50` tests, in its order (AzerothCore `AreaFlags`).
const AREA_FLAG_SANCTUARY: u32 = 0x800;
const AREA_FLAG_ARENA: u32 = 0x80;
/// Wintergrasp-style battlefield, the "combat" type.
const AREA_FLAG_COMBAT: u32 = 0x0100_0000;
const AREA_FLAG_CONTESTED: u32 = 0x0004_0000;
/// `CanHearthAndResurrectFromArea`'s bit.
const AREA_FLAG_HEARTH_AND_RESURRECT: u32 = 0x0800_0000;

/// Build 12340's `CanHearthAndResurrectFromArea` (`0x54c810`): the flag on the ZONE's row, the id
/// `0xbd080c` the zone updater stores (its parent-resolved first argument, `0x78f082`).
fn can_hearth_and_resurrect(zone: Option<&wrathplus_formats::AreaTableRow>) -> bool {
    zone.is_some_and(|r| r.flags & AREA_FLAG_HEARTH_AND_RESURRECT != 0)
}

/// Wrath `GetZonePVPInfo 0x51ba50`'s type: the leaf row's flags when it has one, else the zone's;
/// sanctuary, arena and combat first, with isSubZone set when the flags came from the leaf. Any
/// other area needs the zone row and the player: the contested flag, else the player's template
/// masks against the zone's (a player with no template reads friendly, `0x51bbaf`).
///
/// Unresolved and not applied: the ordinary-territory gate `[0xb2f998] == 0 && !(zone flags &
/// 0x10)` returns no values; its global is untraced, so ordinary territory always reports.
fn wrath_zone_pvp(
    leaf: Option<&wrathplus_formats::AreaTableRow>,
    zone: Option<&wrathplus_formats::AreaTableRow>,
    template: Option<Option<&wrathplus_formats::FactionTemplate>>,
) -> (Option<&'static str>, bool) {
    let (flags, from_leaf) = match (leaf, zone) {
        (Some(leaf), _) => (leaf.flags, true),
        (None, Some(zone)) => (zone.flags, false),
        (None, None) => (0, false),
    };
    for (bit, kind) in [
        (AREA_FLAG_SANCTUARY, "sanctuary"),
        (AREA_FLAG_ARENA, "arena"),
        (AREA_FLAG_COMBAT, "combat"),
    ] {
        if flags & bit != 0 {
            return (Some(kind), from_leaf);
        }
    }
    let (Some(zone), Some(template)) = (zone, template) else {
        return (None, false);
    };
    let kind = if flags & AREA_FLAG_CONTESTED != 0 {
        "contested"
    } else {
        match template {
            None => "friendly",
            Some(t) if t.friend_group_mask & zone.faction_group_mask != 0 => "friendly",
            Some(t) if t.enemy_group_mask & zone.faction_group_mask != 0 => "hostile",
            Some(_) => "contested",
        }
    };
    (Some(kind), false)
}

/// The shared area catalog and the zone-event feed.
pub struct AreaPlugin;

impl Plugin for AreaPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_area_table.after(AssetSet::Open))
            .add_systems(
                Update,
                // After the leaf authority: leaf, indoor bit and names must come from one frame,
                // as in the client's single-pass resolve (`0x67e510`), or a stale leaf fires a
                // spurious splash.
                feed_zone_events
                    .after(crate::app_core::schedule::UiInput)
                    .after(wrathplus_world::terrain_stream::AreaAuthoritySet),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(flags: u32, mask: u32) -> wrathplus_formats::AreaTableRow {
        wrathplus_formats::AreaTableRow {
            map_id: 0,
            zone_id: 0,
            explore_flag: 0,
            flags,
            faction_group_mask: mask,
            exploration_level: 0,
            name: String::new(),
        }
    }

    /// `0x51ba50`: leaf flags first, the special types in order with isSubZone from the leaf, then
    /// contested, then the player's masks; a player with no template reads friendly.
    #[test]
    fn wrath_zone_pvp_follows_the_native_ladder() {
        let human = wrathplus_formats::FactionTemplate {
            faction: 1,
            group_mask: 3,
            friend_group_mask: 2,
            enemy_group_mask: 4,
            ..Default::default()
        };
        let me = Some(Some(&human));
        let zone = row(0, 2);
        assert_eq!(
            wrath_zone_pvp(Some(&row(0x800 | 0x80, 0)), Some(&zone), me),
            (Some("sanctuary"), true)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0x80, 0)), Some(&zone), me),
            (Some("arena"), true)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0x100_0000, 0)), Some(&zone), me),
            (Some("combat"), true)
        );
        assert_eq!(
            wrath_zone_pvp(None, Some(&row(0x800, 2)), me),
            (Some("sanctuary"), false)
        );
        // A special type needs no player at all.
        assert_eq!(
            wrath_zone_pvp(Some(&row(0x80, 0)), None, None),
            (Some("arena"), true)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0x4_0000, 0)), Some(&zone), me),
            (Some("contested"), false)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0, 0)), Some(&zone), me),
            (Some("friendly"), false)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0, 0)), Some(&row(0, 4)), me),
            (Some("hostile"), false)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0, 0)), Some(&row(0, 0)), me),
            (Some("contested"), false)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0, 0)), Some(&row(0, 4)), Some(None)),
            (Some("friendly"), false)
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0, 0)), Some(&zone), None),
            (None, false),
            "no player"
        );
        assert_eq!(
            wrath_zone_pvp(Some(&row(0, 0)), None, me),
            (None, false),
            "no zone row"
        );
    }

    fn sig(zone_id: u32, zone: &str, sub: &str, indoor: bool) -> ZoneSignal {
        ZoneSignal {
            zone_id,
            zone_text: zone.into(),
            subzone_text: sub.into(),
            indoor,
        }
    }

    fn cache(zone_id: Option<u32>, zone: &str, sub: &str) -> ZoneCache {
        ZoneCache {
            zone_id,
            zone_text: zone.into(),
            subzone_text: sub.into(),
            ..ZoneCache::default()
        }
    }

    /// The `0x494780` election.
    #[test]
    fn election_matches_the_byte_law() {
        let c = ZoneCache::default();
        assert_eq!(
            elect_event(&c, &sig(12, "Elwynn Forest", "Northshire Valley", false)),
            Some("ZONE_CHANGED_NEW_AREA")
        );

        // Zone hop: NEW_AREA alone, though the texts changed too.
        let c = cache(Some(12), "Elwynn Forest", "");
        assert_eq!(
            elect_event(&c, &sig(40, "Westfall", "The Jansen Stead", false)),
            Some("ZONE_CHANGED_NEW_AREA")
        );

        let c = cache(Some(12), "Elwynn Forest", "");
        assert_eq!(
            elect_event(&c, &sig(12, "Elwynn Forest", "Goldshire", false)),
            Some("ZONE_CHANGED")
        );

        // Inn entry: the indoor override changed the zone text.
        let c = cache(Some(12), "Elwynn Forest", "Goldshire");
        assert_eq!(
            elect_event(&c, &sig(12, "Lion's Pride Inn", "", true)),
            Some("ZONE_CHANGED_INDOORS")
        );

        let c = cache(Some(12), "Lion's Pride Inn", "");
        assert_eq!(
            elect_event(&c, &sig(12, "Elwynn Forest", "Goldshire", false)),
            Some("ZONE_CHANGED")
        );

        let c = cache(Some(12), "Elwynn Forest", "Goldshire");
        assert_eq!(
            elect_event(&c, &sig(12, "Elwynn Forest", "Goldshire", false)),
            None
        );
    }

    /// Wintergrasp (4197) carries the hearth-and-resurrect flag and some of its subzones do not,
    /// so the zone's row, not the leaf's, decides there.
    #[test]
    fn hearth_and_resurrect_reads_the_zone_row_on_real_data() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("chain");
        let areas = wrathplus_formats::load_area_table_catalog(&mut chain).expect("AreaTable");
        assert!(can_hearth_and_resurrect(areas.get(4197)), "Wintergrasp");
        let bare = areas
            .rows()
            .filter(|r| r.zone_id == 4197 && !can_hearth_and_resurrect(Some(r)))
            .count();
        assert!(bare > 0, "a Wintergrasp subzone without the flag");
        assert!(!can_hearth_and_resurrect(None));
    }

    /// The `0x67e670` override skip (abbey) and fire (inn) branches on the install's data.
    #[test]
    fn indoor_naming_matches_the_byte_law_on_real_data() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("chain");
        let cat = wrathplus_formats::load_wmo_area_catalog(&mut chain).expect("WMOAreaTable");

        // Abbey (WMO 59, name-set 1): the default name equals the yard subzone, so no override.
        let default = cat.default_row(59, 1).expect("abbey default row");
        assert_eq!(default.name, "Northshire Abbey");
        let group = cat.group_row(59, 1, 1934).expect("abbey main hall row");
        assert_eq!(group.name, "Main Hall");
        assert_eq!(
            group.area_table_id, 24,
            "the leaf stays Northshire Abbey (24)"
        );
        // Rooms have distinct rows, so a room hop re-fires the dedup.
        let library = cat.group_row(59, 1, 1943).expect("library row");
        assert_ne!(group.id, library.id, "distinct rows exist per room");

        let unnamed = cat.group_row(59, 1, 1935).expect("unnamed room row");
        assert!(unnamed.name.is_empty());

        // Goldshire inn (WMO 53, name-set 2): only a whole-WMO row, named unlike the street, so
        // the override fires and the subzone stays nulled.
        let inn = cat.default_row(53, 2).expect("the inn's whole-WMO row");
        assert_eq!(inn.name, "Lion's Pride Inn");
        assert_eq!(inn.area_table_id, 0);
        assert!(
            cat.group_row(53, 2, 0).is_none_or(|g| g.name.is_empty()),
            "no named group rows — query B leaves the nulled subzone alone"
        );
    }
}
