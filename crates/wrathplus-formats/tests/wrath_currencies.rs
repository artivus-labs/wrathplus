//! Installed build-12340 currency metadata, read through the effective archive chain.
use std::path::Path;
use wrathplus_formats::{
    currencies::{load_currency_categories, load_currency_types},
    Chain,
};

#[test]
fn installed_wrath_currency_metadata() {
    if std::env::var("WOW_CLIENT_PROFILE").as_deref() != Ok("wrath") {
        eprintln!("SKIP installed currency test: select WOW_CLIENT_PROFILE=wrath");
        return;
    }
    let data = std::env::var("WOW_DATA").expect("WOW_DATA");
    let mut chain = Chain::open_wrath(Path::new(&data), "enUS").unwrap();
    for (name, expected) in [
        ("CurrencyTypes", [26, 4, 16]),
        ("CurrencyCategory", [8, 19, 76]),
    ] {
        let bytes = chain
            .read_file(&format!("DBFilesClient\\{name}.dbc"))
            .unwrap();
        assert_eq!(&bytes[..4], b"WDBC");
        for (i, value) in expected.into_iter().enumerate() {
            assert_eq!(
                u32::from_le_bytes(bytes[4 + i * 4..8 + i * 4].try_into().unwrap()),
                value,
                "{name}"
            );
        }
    }
    let types = load_currency_types(&mut chain).unwrap();
    let categories = load_currency_categories(&mut chain).unwrap();
    assert_eq!(types.len(), 26);
    assert_eq!(categories.len(), 8);
    // The installed table is not in id order, and its order is the backpack's.
    let ids: Vec<u32> = types.iter().map(|row| row.id).take(8).collect();
    assert_eq!(ids, [1, 2, 42, 61, 81, 241, 103, 104]);
    // Emblem of Frost: item identity is not the currency DBC id or the known bit.
    let frost = types.get_by_item(49426).unwrap();
    assert_eq!(
        (frost.id, frost.category_id, frost.bit_index),
        (341, 22, 29)
    );
    assert_eq!(frost.known_mask(), Some(1 << 28));
    assert_eq!(categories.get(22).unwrap().name, "Dungeon and Raid");
    // PvP currency item; this row must not be confused with descriptor-backed Honor Points.
    let honor_token = types.get_by_item(43307).unwrap();
    assert_eq!(
        (
            honor_token.id,
            honor_token.category_id,
            honor_token.bit_index
        ),
        (103, 2, 12)
    );
    assert_eq!(categories.get(2).unwrap().name, "Player vs. Player");
    assert_eq!(categories.get(3).unwrap().flags, 3);
    assert_eq!(categories.get(3).unwrap().name, "Unused");
    // Descriptor-backed Honor/Arena points are not fabricated as token rows.
    assert!(types.get_by_item(0).is_none());
    assert!(types.iter().all(|row| row.known_mask().is_some()));
    // Effective installed archives contain unresolved categories; retain those rows.
    assert_eq!(types.get_by_item(43949).unwrap().category_id, 2089878896);
    assert!(categories.get(2089878896).is_none());
    assert_eq!(types.get_by_item(41749).unwrap().category_id, 24);
    assert!(categories.get(24).is_none());
}
