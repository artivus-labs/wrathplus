use super::*;

fn script() -> UiScript {
    UiScript::new().unwrap()
}

#[test]
fn empty_defaults() {
    script()
        .run(
            r#"
        assert(GetCurrencyListSize() == 0)
        for _, i in ipairs({-1, 0, 1, 4, -2147483648}) do
            assert(select('#', GetCurrencyListInfo(i)) == 9 and GetCurrencyListInfo(i) == nil)
            assert(select('#', GetBackpackCurrencyInfo(i)) == 5 and GetBackpackCurrencyInfo(i) == nil)
            ExpandCurrencyList(i, 1); SetCurrencyUnused(i, 1); SetCurrencyBackpack(i, 1)
        end
        -- `isnumber` on each argument (`0x5b0690`, `0x5b094a`-`0x5b0a39`, `0x5b0aa0`).
        for call, usage in pairs({
            [function() GetCurrencyListInfo() end] = 'GetCurrencyListInfo(index)',
            [function() GetBackpackCurrencyInfo('x') end] = 'GetBackpackCurrencyInfo(index)',
            [function() ExpandCurrencyList(1, nil) end] = 'ExpandCurrencyList(index,expand)',
            [function() SetCurrencyUnused(nil, 1) end] = 'SetCurrencyUnused(index,unused)',
            [function() SetCurrencyBackpack(1, true) end] = 'SetCurrencyBackpack(index,backpack)',
        }) do
            local ok, err = pcall(call)
            assert(not ok and err:find('Usage: ' .. usage, 1, true), err)
        end
        local a,b = GetHonorCurrency(); assert(a == 0 and b == 75000)
        a,b = GetArenaCurrency(); assert(a == 0 and b == 10000)
        assert(select('#', GetHonorCurrency()) == 2)
        assert(select('#', GetArenaCurrency()) == 2)
        -- `0x51f01a`-`0x51f08c`: registered at "0".
        for _, n in ipairs({'currencyTokensUnused1', 'currencyTokensUnused2',
                'currencyTokensBackpack1', 'currencyTokensBackpack2'}) do
            assert(GetCVar(n) == '0' and GetCVarDefault(n) == '0', n)
        end
    "#,
        )
        .unwrap();
}

fn category(id: u32, flags: u32, name: &str) -> WrathCurrencyCategory {
    WrathCurrencyCategory {
        id,
        flags,
        name: name.into(),
        is_expanded: true,
    }
}

fn token(item_id: u32, category_id: u32, bit_index: u32, name: &str) -> WrathCurrencyEntry {
    WrathCurrencyEntry {
        name: Some(name.into()),
        category_id,
        bit_index,
        item_id,
        icon: Some(format!("icon-{item_id}")),
        ..Default::default()
    }
}

/// One category with four known tokens, rows 2-5; the Unused category (flags 3) and an empty one
/// add no rows.
fn snapshot() -> WrathCurrencySnapshot {
    let entries = (1..=4)
        .map(|i| WrathCurrencyEntry {
            count: i * 10,
            icon: Some(format!("test-icon-{i}")),
            ..token(100 + i, 22, i, &format!("Test token {i}"))
        })
        .collect();
    WrathCurrencySnapshot {
        categories: vec![
            category(3, 3, "Unused"),
            category(22, 0, "Test category"),
            category(1, 0, "Miscellaneous"),
        ],
        entries,
        known: 0xf,
        honor_current: Some(42),
        arena_current: Some(12),
        ..Default::default()
    }
}

#[test]
fn supplied_snapshot_shapes_and_clear() {
    let mut s = script();
    s.set_wrath_currency_snapshot(snapshot());
    s.run(
        r#"
        assert(GetCurrencyListSize() == 5)
        assert(select('#', GetCurrencyListInfo(2)) == 9)
        local name,h,e,u,w,count,kind,icon,id = GetCurrencyListInfo(2)
        assert(name == 'Test token 1' and h == false and e == false)
        assert(u == false and w == false and count == 10 and kind == 0)
        assert(icon == 'test-icon-1' and id == 101)
        assert(GetCurrencyListInfo(2.9) == 'Test token 1', 'the index truncates')
        local n, h, e, u, w, c, k, icon, id = GetCurrencyListInfo(1)
        assert(n == 'Test category' and h and e and icon == nil and id == 0)
        SetCurrencyBackpack(2.9, '1')
        assert(select('#', GetBackpackCurrencyInfo(1)) == 5)
        name,count,kind,icon,id = GetBackpackCurrencyInfo(1)
        assert(name == 'Test token 1' and count == 10 and kind == 0)
        assert(icon == 'test-icon-1' and id == 101)
        local a,b = GetHonorCurrency(); assert(a == 42 and b == 75000)
        a,b = GetArenaCurrency(); assert(a == 12 and b == 10000)
    "#,
    )
    .unwrap();
    s.set_wrath_currency_snapshot(Default::default());
    s.run("assert(GetCurrencyListSize()==0 and GetBackpackCurrencyInfo(1)==nil) local a,b=GetHonorCurrency() assert(a==0 and b==75000)").unwrap();
}

#[test]
fn numeric_flags_move_rows_to_the_unused_bucket_and_the_backpack_has_no_cap() {
    let mut s = script();
    s.set_wrath_currency_snapshot(snapshot());
    s.run(
        r#"
        SetCurrencyUnused(2, 1)
        assert(GetCurrencyListSize() == 6 and GetCurrencyListInfo(2) == 'Test token 2')
        local name,h,e,u = GetCurrencyListInfo(5)
        assert(name == 'Unused' and h and e and u)
        name,h,e,u = GetCurrencyListInfo(6)
        assert(name == 'Test token 1' and not h and u)
        SetCurrencyUnused(6, 0); assert(GetCurrencyListSize() == 5)
        assert(not select(4,GetCurrencyListInfo(2)))
        for i=2,5 do SetCurrencyBackpack(i, 1) end
        assert(GetBackpackCurrencyInfo(4) == 'Test token 4')
        assert(select(5,GetCurrencyListInfo(5)))
        SetCurrencyBackpack(2, 0)
        assert(GetBackpackCurrencyInfo(3) == 'Test token 4' and GetBackpackCurrencyInfo(4) == nil)
        ExpandCurrencyList(1, 0); assert(GetCurrencyListSize() == 1)
        assert(GetCurrencyListInfo(2) == nil and not select(3,GetCurrencyListInfo(1)))
        assert(GetBackpackCurrencyInfo(1) == 'Test token 2')
        SetCurrencyBackpack(1, 1); SetCurrencyUnused(1, 1)
        assert(not select(4,GetCurrencyListInfo(1)) and GetCVar('currencyTokensUnused1') == '0')
        ExpandCurrencyList(1, 1); assert(GetCurrencyListSize() == 5)
        ExpandCurrencyList(2, 0); assert(GetCurrencyListSize() == 5)
        assert(GetCurrencyListInfo(-9223372036854775808) == nil)
        assert(GetBackpackCurrencyInfo(-9223372036854775808) == nil)
    "#,
    )
    .unwrap();
}

#[test]
fn the_backpack_cvar_marks_rows_in_table_order_and_headers_are_never_watched() {
    let mut s = script();
    let mut data = snapshot();
    for (e, name) in data
        .entries
        .iter_mut()
        .zip(["delta", "Alpha", "charlie", "Bravo"])
    {
        e.name = Some(name.into());
    }
    s.run("SetCVar('currencyTokensBackpack1', '15')").unwrap();
    s.set_wrath_currency_snapshot(data);
    s.run(
        r#"
        local order = {}
        for i = 2, 5 do order[#order + 1] = GetCurrencyListInfo(i) end
        assert(table.concat(order, ',') == 'Alpha,Bravo,charlie,delta', table.concat(order, ','))
        assert(not select(5,GetCurrencyListInfo(1)) and select(5,GetCurrencyListInfo(5)))
        order = {}
        for i = 1, 4 do order[#order + 1] = GetBackpackCurrencyInfo(i) end
        assert(table.concat(order, ',') == 'delta,Alpha,charlie,Bravo', table.concat(order, ','))
    "#,
    )
    .unwrap();
}

#[test]
fn refresh_keeps_folds_by_category_and_marks_by_cvar() {
    let mut s = script();
    s.set_wrath_currency_snapshot(snapshot());
    s.run("SetCurrencyUnused(2,1); SetCurrencyBackpack(2,1); ExpandCurrencyList(1,0)")
        .unwrap();
    let mut next = snapshot();
    next.categories[1].name = "Renamed category".into();
    next.entries[1].name = Some("Resolved name".into());
    next.entries[1].icon = Some("resolved-icon".into());
    next.entries[1].count = 0;
    s.set_wrath_currency_snapshot(next);
    s.run(
        r#"
        assert(GetCurrencyListSize()==3 and GetCurrencyListInfo(1)=='Renamed category')
        local n,c,k,icon,id=GetBackpackCurrencyInfo(1)
        assert(n=='Resolved name' and id==102)
        ExpandCurrencyList(1,1)
        assert(GetCurrencyListInfo(2)=='Resolved name' and select(6,GetCurrencyListInfo(2))==0)
        assert(select(4,GetCurrencyListInfo(6)) and not select(5,GetCurrencyListInfo(6)))
    "#,
    )
    .unwrap();
}

#[test]
fn categories_without_a_row_never_show_and_clear_forgets_folds() {
    let mut s = script();
    let mut data = snapshot();
    data.entries.push(token(41749, 24, 5, "No category"));
    data.entries
        .push(token(43949, 2089878896, 6, "Bad category"));
    data.known |= 0x30;
    s.set_wrath_currency_snapshot(data);
    s.run("assert(GetCurrencyListSize()==5) ExpandCurrencyList(1,0)")
        .unwrap();
    s.set_wrath_currency_snapshot(snapshot());
    s.run("assert(GetCurrencyListSize()==1)").unwrap();
    s.set_wrath_currency_snapshot(Default::default());
    s.set_wrath_currency_snapshot(snapshot());
    s.run("assert(GetCurrencyListSize()==5)").unwrap();
}

/// `0x5b0030`: categories by `0x5afc40` (flag bit 0 last, then name), headers by `0x5afe30`'s
/// two legs, the Unused bucket last, children by name; the CVars hold `%d` text.
#[test]
fn the_list_is_built_as_12340_builds_it() {
    let mut s = script();
    s.set_wrath_currency_snapshot(WrathCurrencySnapshot {
        categories: vec![
            category(1, 0, "Miscellaneous"),
            category(2, 0, "Player vs. Player"),
            category(3, 3, "Unused"),
            category(22, 0, "Dungeon and Raid"),
            category(23, 0, "Burning Crusade"),
        ],
        entries: vec![
            token(201, 2, 1, "Zeta"),
            token(202, 2, 2, "alpha"),
            token(203, 22, 33, "Beta"),
            token(204, 3, 34, "In Unused"),
            token(205, 24, 5, "Orphan"),
            token(206, 23, 6, "Unknown"),
            token(207, 2, 32, "Omega"),
        ],
        known: (1 << 0) | (1 << 1) | (1 << 32) | (1 << 33) | (1 << 4) | (1 << 31),
        ..Default::default()
    });
    s.run(
        r#"
        local function list()
            local t = {}
            for i = 1, GetCurrencyListSize() do t[#t + 1] = GetCurrencyListInfo(i) end
            return table.concat(t, ',')
        end
        -- A known row of the Unused category itself shows its header, and no row.
        assert(list() == 'Dungeon and Raid,Beta,Player vs. Player,alpha,Omega,Zeta,Unused', list())
        SetCurrencyUnused(2, 1)
        -- A category whose known rows are all unused keeps its header.
        assert(list() == 'Dungeon and Raid,Player vs. Player,alpha,Omega,Zeta,Unused,Beta', list())
        assert(GetCVar('currencyTokensUnused1') == '0' and GetCVar('currencyTokensUnused2') == '1')
        SetCurrencyUnused(4, 1)
        assert(GetCVar('currencyTokensUnused1') == '-2147483648')
        SetCurrencyUnused(4, 1)
        assert(GetCVar('currencyTokensUnused1') == '-2147483647')
        SetCVar('currencyTokensUnused1', '32') SetCVar('currencyTokensUnused2', '0')
        assert(GetCurrencyListSize() == 7, 'a bare SetCVar does not rebuild')
        ExpandCurrencyList(0, 1)
        -- An unused bit marks the Unused header whether or not the token is known.
        assert(list() == 'Dungeon and Raid,Beta,Player vs. Player,alpha,Omega,Zeta,Unused', list())
        local name, h, e, u, w, count, kind, icon, id = GetCurrencyListInfo(7)
        assert(h and e and u and not w and count == 0 and kind == 0 and id == 0)
    "#,
    )
    .unwrap();
    let changes = s.take_cvar_changes();
    assert!(changes.contains(&("currencyTokensUnused2".into(), "1".into())));
}

#[test]
fn pending_metadata_is_nil_and_sorts_last_until_query_resolves() {
    let mut s = script();
    let mut data = snapshot();
    data.entries[0].name = None;
    data.entries[0].icon = None;
    s.run("SetCVar('currencyTokensBackpack1', '1')").unwrap();
    s.set_wrath_currency_snapshot(data);
    s.run(
        r#"
        local n,h,e,u,w,c,k,icon,id=GetCurrencyListInfo(5)
        assert(n==nil and icon==nil and id==101 and c==10)
        assert(select('#',GetCurrencyListInfo(5))==9)
        assert(GetCurrencyListInfo(2)=='Test token 2')
        assert(select('#',GetBackpackCurrencyInfo(1))==5 and GetBackpackCurrencyInfo(1)==nil)
    "#,
    )
    .unwrap();
    s.set_wrath_currency_snapshot(snapshot());
    s.run("assert(GetBackpackCurrencyInfo(1)=='Test token 1')")
        .unwrap();
}

#[test]
fn balance_currents_are_independent_optional_unsigned_values_with_fixed_caps() {
    let mut s = script();
    s.set_wrath_currency_snapshot(WrathCurrencySnapshot {
        honor_current: Some(u32::MAX),
        arena_current: None,
        ..Default::default()
    });
    s.run("local c,m=GetHonorCurrency(); assert(c==4294967295 and m==75000) local a,b=GetArenaCurrency(); assert(a==0 and b==10000)").unwrap();
    s.set_wrath_currency_snapshot(WrathCurrencySnapshot {
        honor_current: None,
        arena_current: Some(0),
        ..Default::default()
    });
    assert_eq!(s.model_ref().wrath_currency.honor_current, None);
    assert_eq!(s.model_ref().wrath_currency.arena_current, Some(0));
    s.run("local c,m=GetHonorCurrency(); assert(c==0 and m==75000) local a,b=GetArenaCurrency(); assert(a==0 and b==10000) assert(select('#',GetHonorCurrency())==2 and select('#',GetArenaCurrency())==2)").unwrap();
}

#[test]
fn the_token_tooltips_show_the_rows_item() {
    let mut s = script();
    s.set_wrath_currency_snapshot(snapshot());
    s.run(
        r#"
        local tip = CreateFrame("GameTooltip", "TokenTip", UIParent)
        local shown = {}
        tip.SetHyperlink = function(_, link) shown[#shown + 1] = link end
        tip:SetCurrencyToken(1)   -- the header: nothing
        tip:SetCurrencyToken(3.9) -- token 2: the index truncates
        tip:SetBackpackToken(1)   -- nothing watched yet
        for i = 2, 5 do SetCurrencyBackpack(i, 1) end
        tip:SetBackpackToken(4)   -- token 4: the walk has no cap
        tip:SetCurrencyToken(9)   -- out of range
        assert(#shown == 2 and shown[1] == 'item:102' and shown[2] == 'item:104', table.concat(shown, ','))
        assert(not pcall(tip.SetCurrencyToken, tip))
    "#,
    )
    .unwrap();
}
