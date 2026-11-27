//! The Wrath companion lists behind `PetPaperDollFrame.lua`'s companion tab, and the companion
//! preview pane.
//!
//! The reference appends a learned spell to its type's list (`0x542030` at `0x5426b7`) when
//! `0x8009b0` admits it: CRITTER when `Effect[0]` is SUMMON (28) and `EffectMiscValueB[0]` names
//! a `SummonProperties.dbc` row with Slot 5, MOUNT when `EffectApplyAuraName[0]` is MOUNTED (78).
//! The lists keep learn order: the login book arrives ascending (the server's `std::map`), later
//! learns append. A companion is out (`0x802cb0`) when the player's `UNIT_FIELD_CRITTER` guid
//! carries its creature entry, or a mount when the player holds its aura.
//!
//! Unresolved, labelled: which `COMPANION_UPDATE` argument the reference passes (here the type
//! whose state moved); `COMPANION_LEARNED`'s tutorial popups (`0x530840`, tips 0x33/0x34).

use std::collections::BTreeSet;

use bevy::prelude::*;

use wrathplus_formats::{SpellDisplay, SummonProperties, SUMMON_SLOT_MINIPET};
use wrathplus_protocol::types::Field;
use wrathplus_ui::script::{
    CompanionView, ScriptValue, UiScript, WrathCompanions, COMPANION_TYPES,
};

use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::PlayerActions;
use crate::app_core::state::Spells;
use crate::names::NameCache;
use crate::net::NetCommands;
use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};

/// The pane `PetPaperDollFrame_UpdateCompanionPreview` points at the selected companion.
const COMPANION_PANE: &str = "CompanionModelFrame";
const SPELL_EFFECT_SUMMON: u32 = 28;
const SPELL_AURA_MOUNTED: u32 = 78;

/// `SummonProperties.dbc`, loaded once under the Wrath profile.
#[derive(Resource)]
pub(crate) struct SummonSlots(pub(crate) SummonProperties);

pub struct UiCompanionsPlugin;

impl Plugin for UiCompanionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_summon_slots.after(AssetSet::Open))
            .add_systems(
                Update,
                feed_companions
                    .in_set(UnitFeed)
                    .after(crate::ui_action::ActionBook)
                    // It pushes cooldown triples, which the cooldown events must find landed.
                    .before(crate::ui_action::CooldownEvents),
            )
            .add_systems(
                Update,
                feed_companion_booth
                    .after(crate::ui_stable::StableBoothFeed)
                    .after(crate::app_core::schedule::UiInput)
                    .before(crate::portrait::BoothSync)
                    .before(crate::entities::DisplayBuildSet),
            );
    }
}

fn load_summon_slots(
    mut commands: Commands,
    assets: Option<Res<WorldAssets>>,
    profile: Option<Res<wrathplus_assets::ClientDataProfile>>,
) {
    let (Some(assets), Some(profile)) = (assets, profile) else {
        return;
    };
    if !matches!(profile.0, wrathplus_formats::DataProfile::Wrath { .. }) {
        return;
    }
    let loaded = wrathplus_formats::load_summon_properties(&mut assets.chain.lock_recover());
    match loaded {
        Ok(table) => commands.insert_resource(SummonSlots(table)),
        Err(e) => {
            warn!("companions: SummonProperties.dbc failed to load — no critter lists: {e:#}")
        }
    }
}

/// `0x8009b0` and the type split at `0x5426b7`: 0 CRITTER, 1 MOUNT, `None` no companion.
pub(crate) fn companion_type(spell: &SpellDisplay, slots: &SummonProperties) -> Option<usize> {
    let minipet = spell.effects[0] == SPELL_EFFECT_SUMMON
        && u32::try_from(spell.effect_misc_value_b[0])
            .ok()
            .and_then(|row| slots.slot(row))
            == Some(SUMMON_SLOT_MINIPET);
    if minipet {
        Some(0)
    } else {
        (spell.effect_apply_aura[0] == SPELL_AURA_MOUNTED).then_some(1)
    }
}

/// `GetCompanionInfo`'s icon (`0x53e3e2`-`0x53e3f2`): `ActiveIconID` for a companion that is out
/// and has one, else `SpellIconID`.
fn companion_icon(spell: &SpellDisplay, active: bool) -> Option<String> {
    if active && spell.active_icon_id != 0 {
        spell.active_icon.clone()
    } else {
        spell.icon.clone()
    }
}

/// The entry a creature guid carries (`0x802d24`: `low >> 24 | (high & 0xfffff) << 8`).
fn guid_entry(guid: u64) -> u32 {
    ((guid >> 24) & 0x0fff_ffff) as u32
}

/// Keep `order` in learn order over `known`: drop forgotten spells, append new ones ascending.
fn learn_order(order: &mut Vec<u32>, known: &BTreeSet<u32>) {
    order.retain(|id| known.contains(id));
    let had: BTreeSet<u32> = order.iter().copied().collect();
    order.extend(known.iter().copied().filter(|id| !had.contains(id)));
}

#[derive(Default)]
struct Memory {
    order: Vec<u32>,
    pushed: Option<WrathCompanions>,
}

#[allow(clippy::too_many_arguments)]
fn feed_companions(
    script: Option<NonSendMut<UiScript>>,
    actions: Res<PlayerActions>,
    spells: Option<Res<Spells>>,
    slots: Option<Res<SummonSlots>>,
    me: Query<(&ObjectStore, Option<&crate::net::Guid>), With<SelfPlayer>>,
    auras: Res<crate::net::wrath_auras::PacketAuras>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    cooldowns: Res<crate::app_core::cooldowns::Cooldowns>,
    clock: Res<crate::app_core::ui_flags::UiClock>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<Memory>>,
) {
    let (Some(mut script), Some(spells), Some(slots)) = (script, spells, slots) else {
        return;
    };
    let memory = memory.get(&script);
    learn_order(&mut memory.order, &actions.spells);
    let (store, guid) = match me.iter().next() {
        Some((s, g)) => (Some(s), g.map(|g| g.0)),
        None => (None, None),
    };
    let critter_guid = store
        .and_then(|s| s.0.get_field_guid(Field::UnitCritter, 0))
        .unwrap_or(0);
    let player_auras: BTreeSet<u32> = guid
        .and_then(|g| auras.by_guid.get(&g))
        .map(|list| list.iter().map(|a| a.aura.spell_id).collect())
        .unwrap_or_default();
    let mut fresh = WrathCompanions {
        critter_guid,
        ..Default::default()
    };
    for &spell_id in &memory.order {
        let Some(spell) = spells.catalog.get(spell_id) else {
            continue;
        };
        let Some(ty) = companion_type(spell, &slots.0) else {
            continue;
        };
        let creature_id = u32::try_from(spell.effect_misc_value[0]).unwrap_or(0);
        let active = if ty == 0 {
            critter_guid != 0 && guid_entry(critter_guid) == creature_id
        } else {
            player_auras.contains(&spell_id)
        };
        fresh.lists[ty].push(CompanionView {
            creature_id,
            // `0x67b6a0` asks a miss and answers nil until the record lands.
            creature_name: names
                .resolve_creature(creature_id, 0, &commands)
                .map(str::to_string),
            spell_id,
            icon: companion_icon(spell, active),
            active,
            cooldown: cooldowns
                .info(spell_id, 0, Some(spell), clock.anchor)
                .ui_triple(clock.anchor, clock.ui_now),
        });
    }
    let last = memory.pushed.replace(fresh.clone());
    if last.as_ref() == Some(&fresh) {
        return;
    }
    script.set_wrath_companions(fresh.clone());
    let Some(last) = last else {
        return; // the first push of a VM is the login book, not a learn
    };
    let ids = |c: &WrathCompanions, ty: usize| -> Vec<u32> {
        c.lists[ty].iter().map(|v| v.spell_id).collect()
    };
    for (ty, token) in COMPANION_TYPES.iter().enumerate() {
        let (was, now) = (ids(&last, ty), ids(&fresh, ty));
        if now.len() > was.len() {
            script.fire_event("COMPANION_LEARNED", vec![]);
        } else if now.len() < was.len() {
            script.fire_event("COMPANION_UNLEARNED", vec![]);
        }
        let out = |c: &WrathCompanions| -> Vec<u32> {
            c.lists[ty]
                .iter()
                .filter(|v| v.active)
                .map(|v| v.spell_id)
                .collect()
        };
        if out(&last) != out(&fresh) {
            script.fire_event("COMPANION_UPDATE", vec![ScriptValue::Str((*token).into())]);
        }
    }
}

/// `SetCreature`'s pane draws in the stable booth, which the left-panel exclusivity of
/// `CharacterFrame` and `PetStableFrame` leaves free while no stable is open: the creature
/// template's cached display (`0x597700` reads record `+0x24`), asked for on a miss.
fn feed_companion_booth(
    script: Option<NonSend<UiScript>>,
    open: Res<crate::ui_stable::StableOpen>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    mut booth: ResMut<crate::portrait::StableBooth>,
) {
    let Some(script) = script else { return };
    if open.npc.is_some() {
        return;
    }
    let Some(creature) = script.model_pane_creature(COMPANION_PANE) else {
        return;
    };
    let _ = names.resolve_creature(creature, 0, &commands);
    let display = names
        .creature_record(creature)
        .map(|r| r.display_id)
        .filter(|&d| d != 0);
    let yaw = script.model_pane_facing(COMPANION_PANE);
    if booth.unit.is_some() || booth.display_id != display || booth.yaw != yaw {
        booth.unit = None;
        booth.display_id = display;
        booth.yaw = yaw;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(effect: u32, misc_b: i32, aura: u32) -> SpellDisplay {
        let mut s = SpellDisplay::default();
        s.effects[0] = effect;
        s.effect_misc_value_b[0] = misc_b;
        s.effect_apply_aura[0] = aura;
        s
    }

    #[test]
    fn the_native_predicate_splits_critters_and_mounts() {
        let slots = SummonProperties::from_slots([(41, 5), (61, 1)]);
        assert_eq!(companion_type(&spell(28, 41, 0), &slots), Some(0));
        assert_eq!(
            companion_type(&spell(28, 61, 0), &slots),
            None,
            "a guardian slot"
        );
        assert_eq!(companion_type(&spell(28, 999, 0), &slots), None, "no row");
        assert_eq!(companion_type(&spell(6, 0, 78), &slots), Some(1));
        assert_eq!(companion_type(&spell(6, 0, 4), &slots), None);
    }

    #[test]
    fn an_active_companion_draws_its_active_icon_when_it_has_one() {
        let mut s = spell(28, 41, 0);
        s.icon = Some("Interface\\Icons\\Pet".into());
        s.active_icon = Some("Interface\\Icons\\PetOut".into());
        assert_eq!(
            companion_icon(&s, true),
            s.icon,
            "no ActiveIconID: the spell icon"
        );
        s.active_icon_id = 7;
        assert_eq!(companion_icon(&s, true), s.active_icon);
        assert_eq!(companion_icon(&s, false), s.icon);
    }

    #[test]
    fn learn_order_appends_and_drops() {
        let mut order = Vec::new();
        learn_order(&mut order, &BTreeSet::from([30, 10, 20]));
        assert_eq!(order, [10, 20, 30]);
        learn_order(&mut order, &BTreeSet::from([5, 10, 30]));
        assert_eq!(
            order,
            [10, 30, 5],
            "a later learn appends even below the others"
        );
    }

    #[test]
    fn a_creature_guid_carries_its_entry() {
        // HighGuid 0xF130, entry 7385 (Bombay Cat) in bits 24..47, low counter 0x42.
        let guid = (0xF130u64 << 48) | (7385u64 << 24) | 0x42;
        assert_eq!(guid_entry(guid), 7385);
    }

    /// The predicate over the installed Wrath Spell.dbc: Brown Horse (458) is a mount, Mechanical
    /// Squirrel (4055) a critter summoning creature 2671, and both lists are populous.
    #[test]
    fn installed_spells_split_into_companion_lists() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let profile = wrathplus_formats::DataProfile::from_env().expect("profile");
        if !matches!(profile, wrathplus_formats::DataProfile::Wrath { .. }) {
            eprintln!("skipped: companions are a Wrath catalog");
            return;
        }
        let mut chain = wrathplus_formats::Chain::open_profile(&data, &profile).expect("chain");
        let catalog = wrathplus_formats::load_spell_catalog(&mut chain).expect("Spell.dbc");
        let slots =
            wrathplus_formats::load_summon_properties(&mut chain).expect("SummonProperties");
        let mut counts = [0usize; 2];
        for (_, spell) in catalog.iter() {
            if let Some(ty) = companion_type(spell, &slots) {
                counts[ty] += 1;
            }
        }
        eprintln!(
            "installed companions: {} critters, {} mounts",
            counts[0], counts[1]
        );
        assert!(counts[0] > 100 && counts[1] > 100, "{counts:?}");
        assert_eq!(companion_type(catalog.get(458).unwrap(), &slots), Some(1));
        let squirrel = catalog.get(4055).unwrap();
        assert_eq!(companion_type(squirrel, &slots), Some(0));
        assert_eq!(squirrel.effect_misc_value[0], 2671);
    }
}
