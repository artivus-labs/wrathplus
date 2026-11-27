//! Build 12340's six glyph sockets for the UI (`wrathplus_ui::script::GlyphSocketView`), read from
//! the player's descriptor:
//! - `PLAYER_FIELD_GLYPH_SLOTS_1 + i` is the socket's `GlyphSlot.dbc` row;
//! - `PLAYER_FIELD_GLYPHS_1 + i` is its `GlyphProperties.dbc` glyph;
//! - `PLAYER_GLYPHS_ENABLED` holds the unlocked bits.
//!
//! The other talent group's sockets are the same slots holding that group's glyphs from
//! `SMSG_TALENTS_INFO` (12340 reads a group's glyph off its record, `0x5c5770`).
//!
//! A glyph's name is its spell's, and its art is `GlyphProperties`' `SpellIcon.dbc` row. A socket
//! fits the pending glyph cast ([`crate::spell::targeting::glyph`]'s wait) when the cast's first
//! `APPLY_GLYPH` effect with both rows agrees on the type flags (`GlyphMatchesSocket` `0x5b7410`).

use std::collections::HashMap;

use bevy::prelude::*;

use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};
use wrathplus_formats::glyphs::GlyphCatalog;
use wrathplus_formats::SpellCatalog;
use wrathplus_protocol::ObjectFields;
use wrathplus_ui::script::{GlyphSocketView, GlyphView, UiScript};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Spells;
use crate::net::wrath_notices::WrathProgress;
use crate::spell::targeting::glyph::GlyphTables;
use crate::spell::targeting::SpellTargeting;

/// `SPELL_EFFECT_APPLY_GLYPH`.
const APPLY_GLYPH: u32 = 74;

/// `SpellIcon.dbc`, id → texture path, for the glyph art.
#[derive(Resource, Default)]
pub(crate) struct GlyphIcons(HashMap<u32, String>);

pub struct UiGlyphsPlugin;

impl Plugin for UiGlyphsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_icons.after(AssetSet::Open))
            .add_systems(
                Update,
                feed_glyphs.in_set(crate::app_core::schedule::UnitFeed),
            );
    }
}

fn load_icons(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    if !matches!(
        chain.profile(),
        wrathplus_formats::DataProfile::Wrath { .. }
    ) {
        return;
    }
    // The shared `SpellIcon.dbc` reader.
    match wrathplus_formats::achievements::load_achievement_icons(&mut chain) {
        Ok(icons) => commands.insert_resource(GlyphIcons(icons)),
        Err(error) => warn!("ui_glyphs: SpellIcon.dbc: {error:#}"),
    }
}

/// The six sockets from the descriptor and the tables.
pub(crate) fn sockets(
    player: &ObjectFields,
    tables: &GlyphCatalog,
    spells: Option<&SpellCatalog>,
    icons: Option<&HashMap<u32, String>>,
    pending: Option<u32>,
) -> [GlyphSocketView; 6] {
    let enabled = player.player_glyphs_enabled().unwrap_or(0);
    let pending = pending.and_then(|id| spells?.get(id));
    std::array::from_fn(|i| {
        let slot_id = player.player_glyph_slot(i as u8).unwrap_or(0);
        let slot = tables.slot(slot_id);
        let glyph_id = player.player_glyph(i as u8).unwrap_or(0);
        let glyph = glyph_view(tables, spells, icons, glyph_id);
        let matches_pending = pending.is_some_and(|spell| {
            (0..3)
                .filter(|&k| spell.effects[k] == APPLY_GLYPH)
                .find_map(|k| {
                    let g = tables.property(spell.effect_misc_value[k] as u32)?;
                    Some(g.type_flags == slot?.type_flags)
                })
                .unwrap_or(false)
        });
        GlyphSocketView {
            enabled: enabled & (1 << i) != 0,
            slot_id,
            slot: slot.map(|s| (s.type_flags, s.order)),
            glyph_id,
            glyph,
            matches_pending,
        }
    })
}

/// A `GlyphProperties.dbc` row resolved for the UI; `None` with no row.
fn glyph_view(
    tables: &GlyphCatalog,
    spells: Option<&SpellCatalog>,
    icons: Option<&HashMap<u32, String>>,
    glyph_id: u32,
) -> Option<GlyphView> {
    tables.property(glyph_id).map(|g| GlyphView {
        spell_id: g.spell_id,
        name: spells
            .and_then(|c| c.get(g.spell_id))
            .map(|d| d.name.clone())
            .unwrap_or_default(),
        icon: icons.and_then(|m| m.get(&g.icon_id)).cloned(),
    })
}

/// The other talent group's sockets: the active ones' slots, the group's own glyphs.
pub(crate) fn inactive_sockets(
    active: &[GlyphSocketView; 6],
    progress: Option<&WrathProgress>,
    tables: &GlyphCatalog,
    spells: Option<&SpellCatalog>,
    icons: Option<&HashMap<u32, String>>,
) -> Option<[GlyphSocketView; 6]> {
    let (_, active_index, groups) = progress?.talents.get(&false)?;
    let (_, group) = groups
        .iter()
        .enumerate()
        .find(|&(i, _)| i != usize::from(*active_index))?;
    Some(std::array::from_fn(|i| {
        let glyph_id = group.glyphs.get(i).copied().map_or(0, u32::from);
        GlyphSocketView {
            glyph_id,
            glyph: glyph_view(tables, spells, icons, glyph_id),
            matches_pending: false,
            ..active[i].clone()
        }
    }))
}

/// The sockets last pushed: the active group's and the other's.
type Pushed = Option<(Option<[GlyphSocketView; 6]>, Option<[GlyphSocketView; 6]>)>;

#[allow(clippy::too_many_arguments)]
fn feed_glyphs(
    script: Option<NonSendMut<UiScript>>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    tables: Option<Res<GlyphTables>>,
    spells: Option<Res<Spells>>,
    icons: Option<Res<GlyphIcons>>,
    targeting: Option<Res<SpellTargeting>>,
    progress: Option<Res<WrathProgress>>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<Pushed>>,
) {
    let (Some(mut script), Some(tables)) = (script, tables) else {
        return;
    };
    let fresh = self_q.single().ok().map(|store| {
        sockets(
            &store.0,
            &tables.0,
            spells.as_deref().map(|s| &s.catalog),
            icons.as_deref().map(|m| &m.0),
            targeting
                .as_deref()
                .and_then(|t| t.glyph_wait())
                .map(|(id, _)| id),
        )
    });
    let spells = spells.as_deref().map(|s| &s.catalog);
    let icons = icons.as_deref().map(|m| &m.0);
    let other = fresh
        .as_ref()
        .and_then(|active| inactive_sockets(active, progress.as_deref(), &tables.0, spells, icons));
    let fresh = (fresh, other);
    let pushed = memory.get(&script);
    if pushed.as_ref() != Some(&fresh) {
        *pushed = Some(fresh.clone());
        script.set_glyph_sockets(fresh.0);
        script.set_inactive_glyph_sockets(fresh.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_formats::glyphs::{GlyphProperty, GlyphSlot};

    #[test]
    fn sockets_read_the_descriptor_rows_and_the_pending_glyph() {
        use wrathplus_protocol::types::Field;
        let tables = GlyphCatalog::from_rows(
            [
                GlyphProperty {
                    id: 182,
                    spell_id: 54815,
                    type_flags: 0,
                    icon_id: 323,
                },
                GlyphProperty {
                    id: 183,
                    spell_id: 54816,
                    type_flags: 1,
                    icon_id: 323,
                },
            ],
            [GlyphSlot {
                id: 21,
                type_flags: 0,
                order: 0,
            }],
        );
        let player = ObjectFields::from_fields(&[
            (Field::PlayerGlyphsEnabled, 0b1),
            (Field::PlayerGlyphSlots1, 21),
            (Field::PlayerGlyphs1, 182),
        ]);
        let glyph_spell = |id: u32, glyph: i32| {
            let d = wrathplus_formats::SpellDisplay {
                name: format!("Glyph {id}"),
                effects: [APPLY_GLYPH, 0, 0],
                effect_misc_value: [glyph, 0, 0],
                ..Default::default()
            };
            (id, d)
        };
        let spells = SpellCatalog::from_displays(
            [
                glyph_spell(54815, 182),
                glyph_spell(1, 182),
                glyph_spell(2, 183),
            ]
            .into_iter()
            .collect(),
        );
        let icons: HashMap<u32, String> =
            [(323, "Interface\\Spellbook\\UI-Glyph-Rune-1".into())].into();
        let s = sockets(&player, &tables, Some(&spells), Some(&icons), Some(1));
        assert!(s[0].enabled && !s[1].enabled);
        assert_eq!(
            (s[0].slot_id, s[0].slot, s[0].glyph_id),
            (21, Some((0, 0)), 182)
        );
        assert_eq!(
            s[0].glyph.clone(),
            Some(GlyphView {
                spell_id: 54815,
                name: "Glyph 54815".into(),
                icon: Some("Interface\\Spellbook\\UI-Glyph-Rune-1".into()),
            })
        );
        assert!(s[1].slot.is_none() && s[1].glyph.is_none());
        // A major glyph fits the major socket; a minor one does not; no socket row fits nothing.
        assert!(s[0].matches_pending && !s[1].matches_pending);
        let minor = sockets(&player, &tables, Some(&spells), None, Some(2));
        assert!(!minor[0].matches_pending);

        // The other group: the same slots, its own glyphs off the talents packet.
        use wrathplus_protocol::events::TalentGroup;
        let mut progress = WrathProgress::default();
        let group = |glyphs: Vec<u16>| TalentGroup {
            talents: vec![],
            glyphs,
        };
        let groups = vec![group(vec![182]), group(vec![183])];
        progress.talents.insert(false, (0, 0, groups));
        let other = inactive_sockets(&s, Some(&progress), &tables, Some(&spells), None).unwrap();
        assert_eq!((other[0].slot_id, other[0].glyph_id), (21, 183));
        assert_eq!(other[0].glyph.as_ref().map(|g| g.spell_id), Some(54816));
        assert!(other[0].enabled && !other[0].matches_pending);
        progress.talents.get_mut(&false).unwrap().2.pop();
        assert!(inactive_sockets(&s, Some(&progress), &tables, None, None).is_none());
    }
}
