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
fn every_row_in_table_order_but_only_known_rows_ask_for_metadata() {
    use wrathplus_formats::currencies::CurrencyType;
    let row = |id, item_id, category_id, bit_index| CurrencyType {
        id,
        item_id,
        category_id,
        bit_index,
    };
    let types = [
        row(341, 10, 999, 64),
        row(2, 20, 22, 1),
        row(3, 30, 22, 2),
        row(4, 40, 22, 0),
    ];
    let mut asked = Vec::new();
    let rows = roster(
        (1 << 63) | 1,
        types.iter(),
        &BTreeMap::from([(20, 7)]),
        |id| {
            asked.push(id);
            (Some(format!("item {id}")), None)
        },
    );
    assert_eq!(asked, [10, 20]);
    let items: Vec<_> = rows.iter().map(|r| (r.item_id, r.bit_index)).collect();
    assert_eq!(items, [(10, 64), (20, 1), (30, 2), (40, 0)]);
    assert_eq!((rows[1].count, rows[1].category_id), (7, 22));
    assert_eq!(rows[0].name.as_deref(), Some("item 10"));
    assert!(rows[2].name.is_none() && rows[3].name.is_none());
}

#[test]
fn special_rows_keep_native_kinds() {
    use wrathplus_formats::currencies::CurrencyType;
    let row = |item_id, bit_index| CurrencyType {
        id: bit_index,
        item_id,
        category_id: 2,
        bit_index,
    };
    let types = [row(43307, 1), row(43308, 2)];
    let counts = BTreeMap::from([(43307, 9), (43308, 0)]);
    let rows = roster(3, types.iter(), &counts, |_| (None, None));
    assert_eq!((rows[0].count, rows[0].extra_currency_type), (9, 1));
    assert_eq!((rows[1].count, rows[1].extra_currency_type), (0, 2));
}
