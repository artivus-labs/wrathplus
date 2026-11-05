use super::*;

pub fn fields(values: &[(Field, u16, u32)]) -> ObjectFields {
    ObjectFields::from_pairs(
        &values
            .iter()
            .map(|&(field, offset, value)| (field.span().unwrap().index + offset, value))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn native_mask_requires_both_words_and_keeps_high_bits() {
    let partial = fields(&[(Field::PlayerKnownCurrencies, 0, 0)]);
    assert_eq!(known_currencies(&partial), None);
    let complete = fields(&[
        (Field::PlayerKnownCurrencies, 0, 1),
        (Field::PlayerKnownCurrencies, 1, 0x80000000),
    ]);
    assert_eq!(known_currencies(&complete), Some((1 << 63) | 1));
    assert_eq!(known_currencies(&fields(&[])), None);
}

#[test]
fn hidden_bag_last_slot_and_streamed_stack_counts_without_templates() {
    let player = fields(&[
        (Field::PlayerCurrencytokenSlot1, 0, 7),
        (Field::PlayerCurrencytokenSlot1, 1, 0),
        (Field::PlayerCurrencytokenSlot1, 62, 8),
        (Field::PlayerCurrencytokenSlot1, 63, 0),
    ]);
    let a = fields(&[
        (Field::ObjectEntry, 0, 49426),
        (Field::ItemStackCount, 0, 12),
    ]);
    let b = fields(&[
        (Field::ObjectEntry, 0, 49426),
        (Field::ItemStackCount, 0, 4),
    ]);
    let stacks = token_stacks(&player, |g| match g {
        7 => Some(&a),
        8 => Some(&b),
        _ => None,
    });
    // GetItemCount sums every stack; the currency list keeps the first.
    assert_eq!(stacks, [(49426, 12), (49426, 4)]);
    assert_eq!(token_counts(&stacks), BTreeMap::from([(49426, 12)]));
    assert!(token_stacks(&player, |_| None).is_empty());
}

#[test]
fn known_roster_keeps_unknown_categories_zero_counts_and_pending_metadata() {
    use wrathplus_formats::currencies::CurrencyType;
    let types = [
        CurrencyType {
            id: 1,
            item_id: 10,
            category_id: 999,
            bit_index: 64,
        },
        CurrencyType {
            id: 2,
            item_id: 20,
            category_id: 22,
            bit_index: 1,
        },
        CurrencyType {
            id: 3,
            item_id: 30,
            category_id: 22,
            bit_index: 2,
        },
        CurrencyType {
            id: 4,
            item_id: 40,
            category_id: 22,
            bit_index: 0,
        },
    ];
    let rows = roster(
        (1 << 63) | 1,
        types.iter(),
        &BTreeMap::from([(20, 7)]),
        |id| (id == 22).then(|| "Dungeon".into()),
        |_| (None, None),
    );
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].name.as_deref(), Some("Dungeon"));
    assert_eq!((rows[1].item_id, rows[1].count), (20, 7));
    assert_eq!((rows[2].category_id, rows[2].is_header), (999, true));
    assert_eq!((rows[3].item_id, rows[3].count), (10, 0));
    assert!(rows[3].name.is_none());
    assert!(rows[3].icon.is_none());
    assert!(roster(
        0,
        types.iter(),
        &BTreeMap::new(),
        |_| None,
        |_| (None, None)
    )
    .is_empty());
}

#[test]
fn special_rows_require_known_catalog_entries_and_keep_native_kinds() {
    use wrathplus_formats::currencies::CurrencyType;
    let types = [
        CurrencyType {
            id: 1,
            item_id: 43307,
            category_id: 1,
            bit_index: 1,
        },
        CurrencyType {
            id: 2,
            item_id: 43308,
            category_id: 1,
            bit_index: 2,
        },
    ];
    let counts = BTreeMap::from([(43307, 9), (43308, 0)]);
    let rows = roster(3, types.iter(), &counts, |_| None, |_| (None, None));
    assert_eq!((rows[1].count, rows[1].extra_currency_type), (9, 1));
    assert_eq!((rows[2].count, rows[2].extra_currency_type), (0, 2));
    assert!(roster(0, types.iter(), &counts, |_| None, |_| (None, None)).is_empty());
}
