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
        for _, i in ipairs({-1, 0, 1, 4}) do
            assert(GetCurrencyListInfo(i) == nil)
            assert(GetBackpackCurrencyInfo(i) == nil)
            ExpandCurrencyList(i, 1); SetCurrencyUnused(i, 1); SetCurrencyBackpack(i, 1)
        end
        local a,b = GetHonorCurrency(); assert(a == 0 and b == 75000)
        a,b = GetArenaCurrency(); assert(a == 0 and b == 10000)
        assert(select('#', GetHonorCurrency()) == 2)
        assert(select('#', GetArenaCurrency()) == 2)
    "#,
        )
        .unwrap();
}

fn snapshot() -> WrathCurrencySnapshot {
    let mut entries = vec![WrathCurrencyEntry {
        name: Some("Test category".into()),
        category_id: 22,
        is_header: true,
        is_expanded: true,
        ..Default::default()
    }];
    for i in 1..=4 {
        entries.push(WrathCurrencyEntry {
            name: Some(format!("Test token {i}")),
            category_id: 22,
            count: i * 10,
            item_id: 100 + i,
            icon: Some(format!("test-icon-{i}")),
            extra_currency_type: 0,
            ..Default::default()
        });
    }
    WrathCurrencySnapshot {
        entries,
        honor_current: Some(42),
        arena_current: Some(12),
        token_stacks: Vec::new(),
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
        SetCurrencyBackpack(2, 1)
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
fn numeric_flags_visible_indexes_and_three_watch_slots() {
    let mut s = script();
    s.set_wrath_currency_snapshot(snapshot());
    s.run(
        r#"
        SetCurrencyUnused(2, 1); assert(select(4,GetCurrencyListInfo(2)))
        SetCurrencyUnused(2, 0); assert(not select(4,GetCurrencyListInfo(2)))
        for i=2,5 do SetCurrencyBackpack(i, 1) end
        assert(GetBackpackCurrencyInfo(3) == 'Test token 3')
        assert(GetBackpackCurrencyInfo(4) == nil)
        assert(not select(5,GetCurrencyListInfo(5)))
        SetCurrencyBackpack(2, 0); SetCurrencyBackpack(5, true)
        assert(GetBackpackCurrencyInfo(3) == 'Test token 4')
        ExpandCurrencyList(1, 0); assert(GetCurrencyListSize() == 1)
        assert(GetCurrencyListInfo(2) == nil)
        assert(GetBackpackCurrencyInfo(1) == 'Test token 2')
        SetCurrencyBackpack(1, 1); SetCurrencyUnused(1, 1)
        assert(not select(4,GetCurrencyListInfo(1)))
        ExpandCurrencyList(1, 1); assert(GetCurrencyListSize() == 5)
        ExpandCurrencyList(2, 0); assert(GetCurrencyListSize() == 5)
        assert(GetCurrencyListInfo(-9223372036854775808) == nil)
        assert(GetBackpackCurrencyInfo(-9223372036854775808) == nil)
    "#,
    )
    .unwrap();
}

#[test]
fn supplied_watch_flags_are_bounded_and_headers_cannot_be_watched() {
    let mut s = script();
    let mut data = snapshot();
    for row in &mut data.entries {
        row.is_watched = true;
    }
    s.set_wrath_currency_snapshot(data);
    s.run("assert(not select(5,GetCurrencyListInfo(1))) assert(not select(5,GetCurrencyListInfo(5))) assert(GetBackpackCurrencyInfo(3)=='Test token 3') assert(GetBackpackCurrencyInfo(4)==nil)").unwrap();
}

#[test]
fn refresh_preserves_identity_flags_but_updates_authoritative_metadata() {
    let mut s = script();
    s.set_wrath_currency_snapshot(snapshot());
    s.run("SetCurrencyUnused(2,1); SetCurrencyBackpack(2,1); ExpandCurrencyList(1,0)")
        .unwrap();
    let mut next = snapshot();
    next.entries[0].name = Some("Renamed category".into());
    next.entries[1].name = Some("Resolved name".into());
    next.entries[1].icon = Some("resolved-icon".into());
    next.entries[1].count = 0;
    next.entries.swap(1, 4);
    s.set_wrath_currency_snapshot(next);
    s.run(
        r#"
        assert(GetCurrencyListSize()==1)
        assert(GetCurrencyListInfo(1)=='Renamed category')
        local n,c,k,icon,id=GetBackpackCurrencyInfo(1)
        assert(n=='Resolved name' and c==0 and icon=='resolved-icon' and id==101)
        ExpandCurrencyList(1,1)
        assert(select(4,GetCurrencyListInfo(5)))
        assert(select(5,GetCurrencyListInfo(5)))
    "#,
    )
    .unwrap();
    let mut removed = snapshot();
    removed.entries.retain(|r| r.item_id != 101);
    s.set_wrath_currency_snapshot(removed);
    s.set_wrath_currency_snapshot(snapshot());
    s.run("assert(GetBackpackCurrencyInfo(1)==nil); assert(not select(4,GetCurrencyListInfo(2)))")
        .unwrap();
}

#[test]
fn unknown_category_ids_remain_distinct_and_clear_forgets_expansion() {
    let mut s = script();
    let mut data = snapshot();
    data.entries.push(WrathCurrencyEntry {
        category_id: 2089878896,
        name: Some("Unknown".into()),
        is_header: true,
        is_expanded: true,
        ..Default::default()
    });
    data.entries.push(WrathCurrencyEntry {
        category_id: 24,
        name: Some("Unknown".into()),
        is_header: true,
        is_expanded: true,
        ..Default::default()
    });
    s.set_wrath_currency_snapshot(data.clone());
    s.run("ExpandCurrencyList(6,0)").unwrap();
    data.entries.swap(5, 6);
    s.set_wrath_currency_snapshot(data);
    s.run("assert(select(3,GetCurrencyListInfo(6))); assert(not select(3,GetCurrencyListInfo(7)))")
        .unwrap();
    s.set_wrath_currency_snapshot(Default::default());
    s.set_wrath_currency_snapshot(snapshot());
    s.run("assert(GetCurrencyListSize()==5)").unwrap();
}

#[test]
fn pending_metadata_projects_empty_strings_until_query_resolves() {
    let mut s = script();
    let mut data = snapshot();
    data.entries[1].name = None;
    data.entries[1].icon = None;
    data.entries[1].is_watched = true;
    s.set_wrath_currency_snapshot(data);
    s.run(
        r#"
        local n,h,e,u,w,c,k,icon,id=GetCurrencyListInfo(2)
        assert(n=='' and icon=='' and id==101 and c==10)
        assert(select('#',GetCurrencyListInfo(2))==9)
        assert(GetBackpackCurrencyInfo(1)=='')
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
        tip:SetCurrencyToken(3)   -- token 2
        tip:SetBackpackToken(1)   -- nothing watched yet
        SetCurrencyBackpack(4, 1)
        tip:SetBackpackToken(1)   -- token 3
        tip:SetCurrencyToken(9)   -- out of range
        assert(#shown == 2 and shown[1] == 'item:102' and shown[2] == 'item:103', table.concat(shown, ','))
        assert(not pcall(tip.SetCurrencyToken, tip))
    "#,
    )
    .unwrap();
}
