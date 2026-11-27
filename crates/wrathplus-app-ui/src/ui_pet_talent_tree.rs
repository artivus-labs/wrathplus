//! The pet stable pair's app half: `GetPetTalentTree` (`0x5d3d80`) from the active pet's family's
//! pet talent type and the `TalentTab` pet masks, and `IsAtStableMaster` (`0x5a1090`) from the
//! open stable master. The same tab is the hunter pet's talent page: its ranks and unspent points
//! from `SMSG_TALENTS_INFO`'s pet form, answering the talent getters' `isPet`, and a pet
//! `LearnTalent` sends `CMSG_PET_LEARN_TALENT`.
//!
//! The family is the pet's cached creature template's, as `UnitCreatureFamily("pet")` reads it
//! ([`crate::ui_pet_stats`]); a miss reads no tree until the creature query lands. Unresolved,
//! labelled: the reference's talent-capable test on the pet (`0x71b630`) is not modelled; a family
//! without a pet talent type (the warlock minions, -1) already answers nil.

use std::collections::HashMap;

use bevy::prelude::*;

use wrathplus_ui::script::{TalentUiState, UiScript};

use crate::net::wrath_notices::WrathProgress;
use crate::net::{ClientCommand, NetCommands};

use crate::app_core::catalogs::PetFamilyTables;
use crate::app_core::schedule::UnitFeed;
use crate::app_core::state::PetBar;
use crate::names::NameCache;
use crate::ui_pet::PetUnit;
use crate::ui_stable::StableOpen;
use crate::ui_talent::Talents;

pub struct UiPetTalentTreePlugin;

impl Plugin for UiPetTalentTreePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PetTalentPages>().add_systems(
            Update,
            (feed_pet_talent_tree, feed_pet_talents)
                .in_set(UnitFeed)
                .after(crate::ui_pet::pet_stop_on_old_target_clear)
                // PET_STABLE_SHOW handlers synchronously read IsAtStableMaster.
                .before(crate::ui_stable::feed_stable),
        );
    }
}

/// The hunter pet's talent page as last pushed, which a pet `LearnTalent` click resolves against.
#[derive(Resource, Default)]
pub(crate) struct PetTalentPages(pub(crate) Option<TalentUiState>);

/// The active pet's family, off its cached creature template.
fn pet_family(bar: &PetBar, pet: &PetUnit, names: &NameCache) -> Option<u32> {
    pet.store(bar.guid())
        .and_then(|s| s.0.object_entry())
        .filter(|&e| e != 0)
        .and_then(|entry| names.creature_record(entry))
        .map(|r| r.pet_family)
}

/// The pet's page: its tree's tab with `SMSG_TALENTS_INFO`'s ranks (0-based on the wire) and
/// unspent points; `None` for a pet with no talent tree or before the pet's talents arrive.
pub(crate) fn pet_pages(
    talents: &Talents,
    pet_talent_type: Option<i32>,
    progress: &WrathProgress,
    spells: &wrathplus_formats::SpellCatalog,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<TalentUiState> {
    let tab = talents.catalog.pet_tab(pet_talent_type?)?;
    let (unspent, active, groups) = progress.talents.get(&true)?;
    let ranks: HashMap<u32, u32> = groups
        .get(usize::from(*active))
        .or_else(|| groups.first())
        .map(|g| {
            g.talents
                .iter()
                .map(|&(id, r)| (id, u32::from(r) + 1))
                .collect()
        })
        .unwrap_or_default();
    let rank_of =
        |t: &wrathplus_formats::Talent| ranks.get(&t.id).copied().unwrap_or(0).min(t.max_rank());
    Some(crate::ui_talent::build_tab_pages(
        &talents.catalog,
        vec![tab],
        &rank_of,
        &|_| true,
        spells,
        (*unspent, 0),
        get,
    ))
}

/// Push the pet's page when it changes, then `PET_TALENT_UPDATE`, the stock talent frame's cue.
#[allow(clippy::too_many_arguments)]
fn feed_pet_talents(
    script: Option<NonSendMut<UiScript>>,
    bar: Res<PetBar>,
    pet: PetUnit,
    names: Res<NameCache>,
    families: Option<Res<PetFamilyTables>>,
    talents: Option<Res<Talents>>,
    spells: Option<Res<crate::app_core::state::Spells>>,
    progress: Res<WrathProgress>,
    mut pushed: ResMut<PetTalentPages>,
    mut seeded: Local<crate::app_core::vm_memo::VmMemo<bool>>,
) {
    let Some(mut script) = script else { return };
    let fresh_vm = seeded.claim(&script);
    let inputs = progress.is_changed() || bar.is_changed() || names.is_changed();
    let catalogs = talents.as_ref().is_some_and(|t| t.is_changed())
        || families.as_ref().is_some_and(|f| f.is_changed());
    if !fresh_vm && !inputs && !catalogs {
        return;
    }
    let ty = families.as_deref().and_then(|f| {
        Some(
            f.families
                .get(pet_family(&bar, &pet, &names)?)?
                .pet_talent_type,
        )
    });
    let fresh = match (talents.as_deref(), spells.as_deref()) {
        (Some(t), Some(s)) => {
            let get = |key: &str| wrathplus_ui::strings::global(script.lua(), key);
            pet_pages(t, ty, &progress, &s.catalog, &get)
        }
        _ => None,
    };
    if !fresh_vm && fresh == pushed.0 {
        return;
    }
    script.set_pet_talents(fresh.clone());
    script.fire_event("PET_TALENT_UPDATE", vec![]);
    pushed.0 = fresh;
}

/// A pet `LearnTalent(tab, index)`: the talent at that slot and its current rank, the 0-based
/// next rank AzerothCore's `LearnPetTalent` takes; a maxed talent sends nothing.
pub(crate) fn learn(pages: &PetTalentPages, pet: u64, tab: u32, index: u32, net: &NetCommands) {
    let Some(t) = pages.0.as_ref().and_then(|p| {
        let tab = p.talents.get(usize::try_from(tab).ok()?.checked_sub(1)?)?;
        tab.get(usize::try_from(index).ok()?.checked_sub(1)?)
    }) else {
        return;
    };
    if pet == 0 || t.rank >= t.max_rank {
        return;
    }
    debug!(
        "ui_pet_talent_tree: learn pet talent {} rank {}",
        t.id, t.rank
    );
    let _ = net.0.send(ClientCommand::NativeSpellControl(
        wrathplus_protocol::wrath_spell_control::Command::PetLearnTalent {
            pet,
            talent_id: t.id,
            rank: t.rank,
        },
    ));
}

/// The tree name for a pet talent type: the first tab whose pet mask has its bit.
fn tree_for(
    talents: Option<&Talents>,
    families: Option<&PetFamilyTables>,
    family: Option<u32>,
) -> Option<String> {
    let ty = families?.families.get(family?)?.pet_talent_type;
    talents?.catalog.pet_tab_name(ty).map(str::to_string)
}

#[allow(clippy::type_complexity)] // Bevy system parameters / a closure table, kept inline
fn feed_pet_talent_tree(
    script: Option<NonSendMut<UiScript>>,
    bar: Res<PetBar>,
    pet: PetUnit,
    names: Res<NameCache>,
    families: Option<Res<PetFamilyTables>>,
    talents: Option<Res<Talents>>,
    stable: Res<StableOpen>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<(Option<String>, bool)>>>,
) {
    let Some(mut script) = script else { return };
    let family = pet_family(&bar, &pet, &names);
    let fresh = (
        tree_for(talents.as_deref(), families.as_deref(), family),
        stable.npc.is_some(),
    );
    let last = last.get(&script);
    if last.as_ref() != Some(&fresh) {
        script.set_wrath_pet_stable(fresh.0.clone(), fresh.1);
        *last = Some(fresh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_formats::{TalentCatalog, TalentTabInfo};

    #[test]
    fn the_tree_follows_the_family_type_and_the_tab_masks() {
        let tab = |id, name: &str, pet_talent_mask| TalentTabInfo {
            id,
            name: name.into(),
            race_mask: 0,
            class_mask: 0,
            pet_talent_mask,
            background: String::new(),
            icon: None,
        };
        let talents = Talents {
            catalog: TalentCatalog::from_rows(
                vec![tab(409, "Tenacity", 2), tab(410, "Ferocity", 1)],
                Vec::new(),
            ),
        };
        assert_eq!(
            tree_for(Some(&talents), None, Some(1)),
            None,
            "no family table"
        );
        assert_eq!(tree_for(Some(&talents), None, None), None);
    }

    /// The pet's page is its tree's tab with the wire's ranks (0-based, so +1) and unspent points;
    /// a pet learn sends `CMSG_PET_LEARN_TALENT` with the current rank, a maxed talent nothing.
    #[test]
    fn the_pet_page_reads_the_wire_ranks_and_learns_the_next_rank() {
        use wrathplus_formats::Talent;
        let tab = |id, name: &str, pet_talent_mask| TalentTabInfo {
            id,
            name: name.into(),
            race_mask: 0,
            class_mask: 0,
            pet_talent_mask,
            background: String::new(),
            icon: None,
        };
        let talent = |id, col, ranks: [u32; 5]| Talent {
            id,
            tab: 410,
            row: 0,
            col,
            ranks,
            prereq_talent: 0,
            prereq_rank: 0,
            required_spell: 0,
            exceptional: false,
        };
        let talents = Talents {
            catalog: TalentCatalog::from_rows(
                vec![tab(409, "Tenacity", 2), tab(410, "Ferocity", 1)],
                vec![
                    talent(2107, 0, [61682, 61683, 0, 0, 0]),
                    talent(2109, 1, [61684, 0, 0, 0, 0]),
                ],
            ),
        };
        let mut progress = WrathProgress::default();
        let group = wrathplus_protocol::events::TalentGroup {
            talents: vec![(2107, 0), (2109, 0)],
            glyphs: Vec::new(),
        };
        progress.talents.insert(true, (3, 0, vec![group]));
        let spells = wrathplus_formats::SpellCatalog::from_displays(HashMap::new());
        let page = pet_pages(&talents, Some(0), &progress, &spells, &|_| None).unwrap();
        assert_eq!(page.tabs[0].name, "Ferocity");
        assert_eq!(page.points.0, 3);
        let ranks: Vec<(u32, u32, u32)> = page.talents[0]
            .iter()
            .map(|t| (t.id, t.rank, t.max_rank))
            .collect();
        assert_eq!(ranks, [(2107, 1, 2), (2109, 1, 1)]);
        assert!(pet_pages(&talents, Some(-1), &progress, &spells, &|_| None).is_none());

        let pages = PetTalentPages(Some(page));
        let (tx, rx) = crossbeam_channel::unbounded();
        let net = NetCommands(tx);
        learn(&pages, 0xF140_0000_0000_0042, 1, 1, &net);
        learn(&pages, 0xF140_0000_0000_0042, 1, 2, &net);
        let sent: Vec<ClientCommand> = rx.try_iter().collect();
        assert!(
            matches!(
                sent.as_slice(),
                [ClientCommand::NativeSpellControl(
                    wrathplus_protocol::wrath_spell_control::Command::PetLearnTalent {
                        pet: 0xF140_0000_0000_0042,
                        talent_id: 2107,
                        rank: 1
                    }
                )]
            ),
            "only the unmaxed talent learns: {sent:?}"
        );
    }
}
