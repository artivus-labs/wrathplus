//! Aura tooltip searches share the reader parser and filters.
use super::common::script;
use crate::script::*;

#[test]
fn unit_buff_and_debuff_tooltips_share_filtered_reader_search_and_spell_requests() {
    for (reader, setter, base, opposite) in [
        ("UnitBuff", "SetUnitBuff", 100, "HARMFUL"),
        ("UnitDebuff", "SetUnitDebuff", 200, "HELPFUL"),
    ] {
        for (args, offset) in [
            (format!("1, '{opposite}|PLAYER|RAID|CANCELABLE'"), 2),
            (format!("2, '{opposite}|PLAYER'"), 3),
            ("'1', 'RAID'".into(), 2),
            ("1.5, 'PLAYER'".into(), 3),
            ("'shared aura', 'rAnK 2', 'PLAYER|RAID'".into(), 2),
            ("'Shared Aura', nil, 'PLAYER|NOT_CANCELABLE'".into(), 3),
            (format!("'Shared Aura', 'Rank 2', '{opposite}'"), 2),
        ] {
            let mut s = sibling_search_script();
            let expected = base + offset;
            s.run(&format!(
                r#"
                local name, rank, _, _, _, _, _, _, _, _, id = {reader}('target', {args})
                assert(id == {expected}, 'reader selects expected filtered aura')
                TT:{setter}('target', {args})
                assert(SiblingTipTextLeft1:GetText() == name, 'tooltip selects reader name')
                "#
            ))
            .unwrap();
            assert_eq!(
                s.take_spell_tooltip_asks(),
                vec![expected],
                "{setter}('target', {args}) must request the reader's spell"
            );
            assert!(s.take_errors().is_empty());
        }
    }
}

#[test]
fn unit_buff_and_debuff_tooltip_search_misses_clear_without_spell_requests() {
    for (reader, setter) in [("UnitBuff", "SetUnitBuff"), ("UnitDebuff", "SetUnitDebuff")] {
        for (token, args) in [
            ("target", "0"),
            ("target", "-1"),
            ("target", "4"),
            ("target", "2, 'PLAYER|RAID'"),
            ("target", "1, 'CANCELABLE|NOT_CANCELABLE'"),
            ("target", "'Missing Aura', nil, 'PLAYER'"),
            ("target", "'Shared Aura', 'Rank 9'"),
            ("target", "'Shared Aura', 'Rank 1', 'PLAYER'"),
            ("nosuchunit", "1"),
        ] {
            let mut s = sibling_search_script();
            s.run(&format!(
                r#"
                TT:SetText('old content')
                TT:Show()
                assert(select('#', {reader}('{token}', {args})) == 0, 'reader misses')
                TT:{setter}('{token}', {args})
                assert(TT:NumLines() == 0 and not TT:IsShown(), 'miss clears and hides')
                "#
            ))
            .unwrap();
            assert!(
                s.take_spell_tooltip_asks().is_empty(),
                "{setter}('{token}', {args})"
            );
            assert!(s.take_errors().is_empty());
        }
    }
}

#[test]
fn unit_buff_and_debuff_tooltip_usage_names_the_method_and_tooltip() {
    let mut s = script();
    s.run(
        r#"
        local named = CreateFrame('GameTooltip', 'SiblingTip')
        local unnamed = CreateFrame('GameTooltip')
        for _, method in ipairs({'SetUnitBuff', 'SetUnitDebuff'}) do
            for _, tip in ipairs({named, unnamed}) do
                local prefix = 'Usage: ' .. (tip:GetName() or '<unnamed>') .. ':' .. method
                for _, args in ipairs({{}, {'target'}, {'target', false}, {false, 1}}) do
                    local ok, err = pcall(tip[method], tip, unpack(args))
                    assert(not ok and string.find(err, prefix, 1, true), 'method Usage names tooltip')
                    assert(string.find(err, '[index] or ["name", "rank"][, "filter"]', 1, true))
                end
            end
        end
        "#,
    )
    .unwrap();
    assert!(s.take_errors().is_empty());
}

// Same-name ranks and leading foreign/non-raid auras make ignoring any part of the query
// observable. No tooltip views are seeded: the ask identifies exactly which spell was selected.
fn sibling_search_script() -> UiScript {
    let mut s = script();
    s.set_unit_guids(&UnitGuids {
        player: 1,
        target: 7,
        ..Default::default()
    });
    let mut auras = Vec::new();
    for (helpful, base) in [(true, 100), (false, 200)] {
        for (offset, caster, raid, cancelable, rank) in [
            (1, 9, false, false, "Rank 1"),
            (2, 1, true, true, "Rank 2"),
            (3, 1, false, false, "Rank 3"),
        ] {
            auras.push(AuraState {
                spell_id: base + offset,
                name: Some("Shared Aura".into()),
                rank: Some(rank.into()),
                helpful,
                caster,
                raid,
                cancelable,
                ..Default::default()
            });
        }
    }
    s.set_unit_auras(7, Some(auras));
    s.run("TT = CreateFrame('GameTooltip', 'SiblingTip')")
        .unwrap();
    s
}
