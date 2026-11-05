//! Wrath token roster: known-currency bits select CurrencyTypes rows; the hidden 32-slot
//! token bag supplies instance counts. Honor/arena are separate native descriptor balances.
use std::collections::BTreeMap;

use bevy::prelude::*;
use wrathplus_protocol::{types::Field, ObjectFields};
use wrathplus_ui::script::{UiScript, WrathCurrencyEntry, WrathCurrencySnapshot};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::vm_memo::VmMemo;
use crate::entities::ItemDisplays;
use crate::items::Items;
use crate::net::{NetCommands, Objects};

#[derive(Resource)]
pub struct CurrencyData {
    types: wrathplus_formats::currencies::CurrencyCatalog,
    categories: wrathplus_formats::currencies::CurrencyCategoryCatalog,
}

pub(super) fn load(mut commands: Commands, assets: Option<Res<wrathplus_assets::WorldAssets>>) {
    use wrathplus_assets::LockRecover;
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    if !matches!(
        chain.profile(),
        wrathplus_formats::DataProfile::Wrath { .. }
    ) {
        return;
    }
    match (
        wrathplus_formats::currencies::load_currency_types(&mut chain),
        wrathplus_formats::currencies::load_currency_categories(&mut chain),
    ) {
        (Ok(types), Ok(categories)) => commands.insert_resource(CurrencyData { types, categories }),
        (types, categories) => warn!(
            "currency DBCs: types={:?}, categories={:?}",
            types.err(),
            categories.err()
        ),
    }
}

/// Require both words: a partially streamed mask is not an authoritative empty roster.
fn known_currencies(player: &ObjectFields) -> Option<u64> {
    Some(
        u64::from(player.get_at(Field::PlayerKnownCurrencies, 0)?)
            | (u64::from(player.get_at(Field::PlayerKnownCurrencies, 1)?) << 32),
    )
}

/// The hidden token bag's stacks in slot order, `(entry, count)`.
fn token_stacks<'a>(
    player: &ObjectFields,
    mut object: impl FnMut(u64) -> Option<&'a ObjectFields>,
) -> Vec<(u32, u32)> {
    // AzerothCore Player.h: CURRENCYTOKEN_SLOT_START..END has 32 GUID positions.
    (0..32u16)
        .filter_map(|slot| {
            let guid = player.get_field_guid(Field::PlayerCurrencytokenSlot1, slot * 2);
            let item = object(guid.filter(|&g| g != 0)?)?;
            item.object_entry().zip(item.item_stack_count())
        })
        .collect()
}

/// Installed GetCurrencyListInfo (0x5b0680): first matching hidden stack wins.
fn token_counts(stacks: &[(u32, u32)]) -> BTreeMap<u32, u32> {
    let mut counts = BTreeMap::<u32, u32>::new();
    for &(entry, count) in stacks {
        counts.entry(entry).or_insert(count);
    }
    counts
}

fn roster<'a>(
    known: u64,
    types: impl Iterator<Item = &'a wrathplus_formats::currencies::CurrencyType>,
    counts: &BTreeMap<u32, u32>,
    mut category_name: impl FnMut(u32) -> Option<String>,
    mut metadata: impl FnMut(u32) -> (Option<String>, Option<String>),
) -> Vec<WrathCurrencyEntry> {
    let mut groups = BTreeMap::<u32, Vec<WrathCurrencyEntry>>::new();
    for currency in types {
        if currency.item_id == 0 || !currency.known_mask().is_some_and(|bit| known & bit != 0) {
            continue;
        }
        let (name, icon) = metadata(currency.item_id);
        groups
            .entry(currency.category_id)
            .or_default()
            .push(WrathCurrencyEntry {
                name,
                icon,
                category_id: currency.category_id,
                item_id: currency.item_id,
                count: counts.get(&currency.item_id).copied().unwrap_or(0),
                extra_currency_type: match currency.item_id {
                    43307 => 1,
                    43308 => 2,
                    _ => 0,
                },
                ..Default::default()
            });
    }
    let mut entries = Vec::new();
    for (category_id, children) in groups {
        entries.push(WrathCurrencyEntry {
            category_id,
            name: category_name(category_id),
            is_header: true,
            is_expanded: true,
            ..Default::default()
        });
        entries.extend(children);
    }
    entries
}

pub fn feed(
    script: Option<NonSendMut<UiScript>>,
    me: Query<&ObjectStore, With<SelfPlayer>>,
    objects: Objects,
    data: Option<Res<CurrencyData>>,
    items: Res<Items>,
    icons: Option<Res<ItemDisplays>>,
    commands: Res<NetCommands>,
    mut memo: Local<VmMemo<Option<WrathCurrencySnapshot>>>,
) {
    let Some(mut script) = script else { return };
    let last = memo.get(&script);
    let mut fresh = WrathCurrencySnapshot::default();
    if let Ok(player) = me.single() {
        fresh.honor_current = player.0.get(Field::PlayerHonorCurrency);
        fresh.arena_current = player.0.get(Field::PlayerArenaCurrency);
        // GetItemCount counts the bag whatever the known bits (`0x753a50`, bit `0x200`).
        fresh.token_stacks = token_stacks(&player.0, |guid| objects.object(guid));
        if let (Some(known), Some(data)) = (known_currencies(&player.0), data.as_deref()) {
            let mut counts = token_counts(&fresh.token_stacks);
            // Native special rows bypass the hidden bag; never create rows without known bits.
            counts.insert(43307, fresh.arena_current.unwrap_or(0));
            counts.insert(43308, fresh.honor_current.unwrap_or(0));
            fresh.entries = roster(
                known,
                data.types.iter(),
                &counts,
                |id| data.categories.get(id).map(|c| c.name.clone()),
                |id| {
                    let Some(template) = items.template(id, 0, &commands) else {
                        return (None, None);
                    };
                    (
                        Some(template.name.clone()),
                        icons
                            .as_deref()
                            .and_then(|icons| icons.catalog.get(template.display_info_id))
                            .and_then(|display| display.icon.clone()),
                    )
                },
            );
        }
    }
    // Rebuild pending metadata every frame; only authoritative snapshot changes cross the seam.
    // VmMemo re-seeds /reload; missing self pushes an empty snapshot on logout.
    if last.as_ref() != Some(&fresh) {
        // `PVPFrame.lua` repaints its honor and arena points on `HONOR_CURRENCY_UPDATE`, after the
        // snapshot so the repaint reads the new values. Unresolved, labelled: the 12340 field
        // watch behind it is untraced; this fires on either point field moving.
        let points = |s: &WrathCurrencySnapshot| (s.honor_current, s.arena_current);
        let currency_moved = last.as_ref().is_none_or(|l| points(l) != points(&fresh));
        script.set_wrath_currency_snapshot(fresh.clone());
        *last = Some(fresh);
        if currency_moved {
            script.fire_event("HONOR_CURRENCY_UPDATE", vec![]);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests;
