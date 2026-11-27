use super::*;
use wrathplus_formats::{EnchantCatalog, SpellCatalog, Talent, TalentCatalog, TalentTabInfo};

fn reply(guid: u64, groups: Vec<TalentGroup>) -> Packet {
    Packet::Inspect {
        player: guid,
        unspent: 2,
        active_group: 1,
        groups,
        items: Vec::new(),
    }
}

fn talent(id: u32, row: u32, ranks: &[u32], prereq: (u32, u32)) -> Talent {
    let mut all = [0; wrathplus_formats::MAX_TALENT_RANK];
    all[..ranks.len()].copy_from_slice(ranks);
    Talent {
        id,
        tab: 161,
        row,
        col: 0,
        ranks: all,
        prereq_talent: prereq.0,
        prereq_rank: prereq.1,
        required_spell: 0,
        exceptional: false,
    }
}

#[test]
fn the_newest_reply_wins_and_the_fifo_drains() {
    let mut fifo = VecDeque::from([reply(1, Vec::new()), reply(2, Vec::new())]);
    assert_eq!(drain_latest(&mut fifo).map(|r| r.guid), Some(2));
    assert!(fifo.is_empty());
    assert_eq!(drain_latest(&mut fifo), None);
}

/// Each group's wire ranks are 0-based; the pages hold them 1-based, sum the tab's spent points,
/// read prerequisites off the inspected ranks and offer nothing to learn.
#[test]
fn inspected_groups_build_ranked_pages_with_nothing_learnable() {
    let catalog = TalentCatalog::from_rows(
        vec![TalentTabInfo {
            id: 161,
            name: "Arms".into(),
            race_mask: 511,
            class_mask: 1,
            pet_talent_mask: 0,
            background: "WarriorArms".into(),
            icon: None,
        }],
        vec![
            talent(10, 0, &[100, 101, 102], (0, 0)),
            talent(11, 1, &[200], (10, 2)),
        ],
    );
    let spells = SpellCatalog::from_displays(Default::default());
    let mut fifo = VecDeque::from([reply(
        5,
        vec![
            TalentGroup {
                talents: vec![(10, 2), (11, 0)],
                glyphs: vec![0, 41],
            },
            TalentGroup {
                talents: vec![(10, 0)],
                glyphs: Vec::new(),
            },
        ],
    )]);
    let held = drain_latest(&mut fifo).unwrap();
    let out = talents(&held, &catalog, &spells, 1, 1, &|_| None);
    assert_eq!((out.guid, out.unspent, out.active_group), (5, 2, 2));
    let first = &out.groups[0].pages;
    assert_eq!(first.points, (2, 0));
    assert_eq!(first.tabs[0].points_spent, 4);
    assert_eq!(
        first.talents[0].iter().map(|t| t.rank).collect::<Vec<_>>(),
        [3, 1]
    );
    assert!(first.talents[0][1].prereqs[0].learnable);
    assert!(first.talents[0].iter().all(|t| !t.learnable));
    assert_eq!(out.groups[0].glyphs, [0, 41]);
    let second = &out.groups[1].pages;
    assert_eq!(second.tabs[0].points_spent, 1);
    assert!(!second.talents[0][1].prereqs[0].learnable);
}

/// The reply's gear keeps every enchant slot (a gem in slot 2) and links as build 12340 does.
#[test]
fn an_inspected_item_keeps_its_gems_and_links_with_them() {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let commands = NetCommands(tx);
    let mut items = Items::default();
    items.insert_template(
        40,
        Some(wrathplus_protocol::types::ItemInfo {
            quality: 4,
            ..crate::items::test_template("Helm")
        }),
    );
    let enchants = crate::items::Enchants(EnchantCatalog::from_rows(
        Default::default(),
        [
            (3820, "+30 Spell Power".to_string()),
            (3627, "+20 Strength".to_string()),
        ]
        .into_iter()
        .collect(),
        [(3820, 0), (3627, 0)].into_iter().collect(),
    ));
    let item = InspectedItem {
        slot: 0,
        entry: 40,
        enchants: vec![(0, 3820), (2, 3627)],
        random_property: 0,
        creator: 0,
        suffix_factor: 0,
    };
    let view = slot_view(
        &item,
        &items,
        None,
        RollCatalogs {
            props: None,
            enchants: Some(&enchants),
        },
        &commands,
        &NameCache::default(),
        80,
    );
    assert_eq!((view.item_id, view.count, view.creator), (40, 1, None));
    assert_eq!(
        view.enchants
            .iter()
            .map(|e| (e.slot, e.name.as_str()))
            .collect::<Vec<_>>(),
        [(0, "+30 Spell Power"), (2, "+20 Strength")]
    );
    assert_eq!(
        view.link.as_deref(),
        Some("|cffa335ee|Hitem:40:3820:3627:0:0:0:0:0:80|h[Helm]|h|r")
    );
}
