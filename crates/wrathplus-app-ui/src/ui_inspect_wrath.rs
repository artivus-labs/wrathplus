//! Build 12340's inspect reply (`SMSG_INSPECT_TALENT`, `0x3F4`, `https://github.com/artivus-labs/wrathplus/wiki/WRATH-INSPECT-TALENTS`):
//! the inspected player's talent groups, glyphs and equipped gear. The latest reply drained from
//! [`NativeInspectPackets`] is held for its guid; [`crate::ui_inspect`] lays its gear over the
//! visible-item slots and pushes its talents ([`UiScript::set_wrath_inspect_talents`]), then
//! fires `INSPECT_TALENT_READY`.
//!
//! Labelled: a talent's `requiredSpell` reads as met (the reply carries no spells); a negative
//! random property (`ItemRandomSuffix.dbc`) names no suffix; gems print as enchant lines; no
//! inspected pet talents.

use std::collections::{HashMap, VecDeque};

use wrathplus_protocol::wrath_inspect::{InspectedItem, Packet, TalentGroup};
use wrathplus_ui::script::{InvSlotView, WrathInspectGroup, WrathInspectTalents};

use crate::entities::ItemDisplays;
use crate::items::{Items, RollCatalogs};
use crate::names::NameCache;
use crate::net::NetCommands;

/// One held reply.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reply {
    pub(crate) guid: u64,
    pub(crate) unspent: u32,
    /// 0-based, as on the wire.
    pub(crate) active_group: u8,
    pub(crate) groups: Vec<TalentGroup>,
    pub(crate) items: Vec<InspectedItem>,
}

/// Drains the FIFO in arrival order and answers the newest reply, if any arrived.
pub(crate) fn drain_latest(packets: &mut VecDeque<Packet>) -> Option<Reply> {
    packets
        .drain(..)
        .map(|packet| match packet {
            Packet::Inspect {
                player,
                unspent,
                active_group,
                groups,
                items,
            } => Reply {
                guid: player,
                unspent,
                active_group,
                groups,
                items,
            },
        })
        .next_back()
}

/// The pushed talents: each group's pages built from its (talent, 0-based rank) pairs for the
/// inspected race and class, with no talent learnable.
pub(crate) fn talents(
    reply: &Reply,
    catalog: &wrathplus_formats::TalentCatalog,
    spells: &wrathplus_formats::SpellCatalog,
    race: u8,
    class: u8,
    get: &dyn Fn(&str) -> Option<String>,
) -> WrathInspectTalents {
    let groups = reply
        .groups
        .iter()
        .map(|group| {
            let ranks: HashMap<u32, u32> = group
                .talents
                .iter()
                .map(|&(id, rank)| (id, u32::from(rank) + 1))
                .collect();
            let mut pages = crate::ui_talent::build_pages_by(
                catalog,
                &|t| ranks.get(&t.id).copied().unwrap_or(0).min(t.max_rank()),
                &|_| true,
                spells,
                race,
                class,
                (0, 0),
                get,
            );
            pages.points = (reply.unspent, 0);
            WrathInspectGroup {
                pages,
                glyphs: group.glyphs.clone(),
            }
        })
        .collect();
    WrathInspectTalents {
        guid: reply.guid,
        unspent: reply.unspent,
        active_group: u32::from(reply.active_group) + 1,
        groups,
    }
}

/// One inspected slot from the reply's item: the template by entry, every enchant slot (gems
/// included), the creator's name and the roll, linked as build 12340 links it.
pub(crate) fn slot_view(
    item: &InspectedItem,
    items: &Items,
    icons: Option<&ItemDisplays>,
    rolls: RollCatalogs,
    commands: &NetCommands,
    names: &NameCache,
    link_level: u32,
) -> InvSlotView {
    let roll = i32::from(item.random_property);
    let (name, quality, display) = match items.template(item.entry, 0, commands) {
        Some(t) => (
            Some(crate::items::item_display_name(&t.name, roll, rolls.props)),
            t.quality,
            t.display_info_id,
        ),
        None => (None, 0, 0),
    };
    let enchant = |slot: u8| {
        item.enchants
            .iter()
            .find(|&&(s, _)| s == slot)
            .map_or(0, |&(_, id)| u32::from(id))
    };
    let link = name.as_ref().map(|n| {
        crate::items::wrath_item_link(
            item.entry,
            n,
            quality,
            [enchant(0), enchant(2), enchant(3), enchant(4)],
            roll,
            item.suffix_factor,
            link_level,
        )
    });
    InvSlotView {
        item_id: item.entry,
        icon: icons
            .and_then(|i| i.catalog.get(display))
            .and_then(|d| d.icon.clone()),
        count: 1,
        quality: quality as i32,
        name,
        link,
        creator: (item.creator != 0)
            .then(|| names.resolve(item.creator, commands).map(str::to_string))
            .flatten(),
        enchants: crate::items::enchant_lines(
            item.enchants
                .iter()
                .map(|&(slot, id)| (slot, i32::from(id), 0, None)),
            rolls.enchants,
        ),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "ui_inspect_wrath_tests.rs"]
mod tests;
