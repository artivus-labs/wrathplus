//! The skills-pane bindings over pushed snapshots.

use super::*;
use crate::script::UiScript;

fn entry(
    skill_id: u32,
    name: &str,
    value: u32,
    max: u32,
    temp_bonus: i32,
    category_id: u32,
    category_name: &str,
    category_order: u32,
) -> SkillEntry {
    SkillEntry {
        skill_id,
        name: name.into(),
        value,
        max,
        temp_bonus,
        perm_bonus: 0,
        min_level: 0,
        cost_index: 0,
        category_id,
        category_name: category_name.into(),
        category_order,
        description: format!("About {name}."),
        // Fixture rule: Professions (id 2) is abandonable, as the 0x20 flag splits it.
        abandonable: category_id == 2,
        // Fixture rule: Class Skills (id 3) is single-rank, as the 0x400 flag splits it.
        mono: category_id == 3,
    }
}

/// Weapon Skills (order 1) and Professions (order 2), pushed unordered: Swords before Defense.
fn state() -> SkillsState {
    SkillsState {
        entries: vec![
            entry(43, "Swords", 200, 300, 0, 1, "Weapon Skills", 1),
            entry(95, "Defense", 180, 300, 5, 1, "Weapon Skills", 1),
            entry(129, "First Aid", 57, 75, 3, 2, "Professions", 2),
            entry(356, "Fishing", 1, 300, 0, 2, "Professions", 2),
        ],
    }
}

/// Read `(name, type)` at a visible index, `type` = "header"/"entry".
fn row_kind(s: &mut UiScript, i: i64) -> (String, String) {
    s.eval::<(String, String)>(&format!(
        "local n,h = GetSkillLineInfo({i}) local t = h and 'header' or 'entry' return n,t"
    ))
    .unwrap()
}

#[test]
fn grouped_visible_rows_interleave_headers_ordered_by_category() {
    let mut s = UiScript::new().unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 0);

    s.set_skills(state());
    // 2 headers + 4 entries, category_order ascending, name ascending within.
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 6);
    assert_eq!(
        row_kind(&mut s, 1),
        ("Weapon Skills".into(), "header".into())
    );
    assert_eq!(row_kind(&mut s, 2), ("Defense".into(), "entry".into()));
    assert_eq!(row_kind(&mut s, 3), ("Swords".into(), "entry".into()));
    assert_eq!(row_kind(&mut s, 4), ("Professions".into(), "header".into()));
    assert_eq!(row_kind(&mut s, 5), ("First Aid".into(), "entry".into()));
    assert_eq!(row_kind(&mut s, 6), ("Fishing".into(), "entry".into()));

    // Every group starts expanded.
    let (_, h1, e1) = s
        .eval::<(String, i64, Option<i64>)>("local n,h,e = GetSkillLineInfo(1) return n,h,e")
        .unwrap();
    assert_eq!((h1, e1), (1, Some(1)));
}

#[test]
fn abandon_skill_queues_the_entrys_skill_id_and_mutates_nothing() {
    let mut s = UiScript::new().unwrap();
    s.set_skills(state());
    // The 8th return, 1/nil: only Professions rows are abandonable (fixture rule).
    let ab = |s: &mut UiScript, i: i64| {
        s.eval::<Option<i64>>(&format!(
            "local _,_,_,_,_,_,_,ab = GetSkillLineInfo({i}) return ab"
        ))
        .unwrap()
    };
    assert_eq!(ab(&mut s, 2), None, "Defense is not abandonable");
    assert_eq!(ab(&mut s, 5), Some(1), "First Aid is abandonable");
    assert_eq!(ab(&mut s, 1), None, "a header never is");

    // Queued by skill id; a header or an out-of-range index does nothing; the list stays.
    s.run("AbandonSkill(5)").unwrap();
    s.run("AbandonSkill(1)").unwrap();
    s.run("AbandonSkill(99)").unwrap();
    assert_eq!(s.take_skill_abandons(), vec![129]);
    assert!(
        s.take_skill_abandons().is_empty(),
        "drain empties the queue"
    );
    assert_eq!(
        s.eval::<i64>("return GetNumSkillLines()").unwrap(),
        6,
        "no local removal — the visible tree is unchanged"
    );
}

#[test]
fn collapse_hides_a_groups_entries_and_remaps_indices() {
    let mut s = UiScript::new().unwrap();
    s.set_skills(state());

    // Fold Weapon Skills (row 1): its two entries go, the header stays with isExpanded nil.
    s.run("CollapseSkillHeader(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 4);
    let (name, _, expanded) = s
        .eval::<(String, i64, Option<i64>)>("local n,h,e = GetSkillLineInfo(1) return n,h,e")
        .unwrap();
    assert_eq!((name.as_str(), expanded), ("Weapon Skills", None));
    assert_eq!(
        row_kind(&mut s, 2),
        ("Professions".into(), "header".into()),
        "Weapon Skills' entries are folded; Professions is now row 2"
    );

    // Expand it back.
    s.run("ExpandSkillHeader(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 6);

    // Collapse-all (id 0), then expand-all (id 0).
    s.run("CollapseSkillHeader(0)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 2);
    s.run("ExpandSkillHeader(0)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 6);
}

#[test]
fn a_repush_keeps_the_selection_and_re_expands_every_group() {
    let mut s = UiScript::new().unwrap();
    s.set_skills(state());

    // Selecting a header clears the selection.
    s.run("SetSelectedSkill(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedSkill()").unwrap(), 0);

    // Select Swords (row 3), fold Professions (row 4).
    s.run("SetSelectedSkill(3)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedSkill()").unwrap(), 3);
    s.run("CollapseSkillHeader(4)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 4);

    // A re-push unfolds every group and keeps Swords selected at row 3.
    let mut ticked = state();
    ticked.entries[0].value = 201; // Swords
    s.set_skills(ticked);
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 6);
    assert_eq!(s.eval::<i64>("return GetSelectedSkill()").unwrap(), 3);
    let (name, rank) = s
        .eval::<(String, i64)>("local n,_,_,r = GetSkillLineInfo(3) return n,r")
        .unwrap();
    assert_eq!((name.as_str(), rank), ("Swords", 201));

    // A re-push that drops the selected skill entirely clears the selection.
    let mut without_swords = state();
    without_swords.entries.remove(0);
    s.set_skills(without_swords);
    assert_eq!(s.eval::<i64>("return GetSelectedSkill()").unwrap(), 0);
}

#[test]
fn header_and_entry_tuple_shapes() {
    let mut s = UiScript::new().unwrap();
    s.set_skills(state());

    // Header row 1: twelve values, no description (`0x4d3768`).
    let (name, is_header, is_expanded, rank, temp, modifier, max) = s
        .eval::<(String, i64, Option<i64>, i64, i64, i64, i64)>(
            "local n,h,e,r,t,m,mx = GetSkillLineInfo(1) return n,h,e,r,t,m,mx",
        )
        .unwrap();
    assert_eq!(
        (
            name.as_str(),
            is_header,
            is_expanded,
            rank,
            temp,
            modifier,
            max
        ),
        ("Weapon Skills", 1, Some(1), 0, 0, 0, 0)
    );
    let (abandon_nil, step_nil, rank_cost_nil, min_level, cost_type) = s
        .eval::<(bool, bool, bool, i64, i64)>(
            "local _,_,_,_,_,_,_,a,st,rc,ml,ct = GetSkillLineInfo(1) \
             return a==nil, st==nil, rc==nil, ml, ct",
        )
        .unwrap();
    let count = s.arity("GetSkillLineInfo(1)").unwrap();
    assert!(abandon_nil);
    assert!(
        step_nil && rank_cost_nil,
        "stepCost/rankCost are nil, not 0"
    );
    assert_eq!((min_level, cost_type), (0, 0));
    assert_eq!(count, 12, "a header row returns 12 values, not 13");

    // Entry row 2 (Defense 180/300, temp +5): thirteen values.
    let (name, is_header, is_expanded, rank, temp, modifier, max) = s
        .eval::<(String, Option<i64>, Option<i64>, i64, i64, i64, i64)>(
            "local n,h,e,r,t,m,mx = GetSkillLineInfo(2) return n,h,e,r,t,m,mx",
        )
        .unwrap();
    assert_eq!(
        (
            name.as_str(),
            is_header,
            is_expanded,
            rank,
            temp,
            modifier,
            max
        ),
        ("Defense", None, None, 180, 0, 5, 300)
    );
    let count = s.arity("GetSkillLineInfo(2)").unwrap();
    let desc = s
        .eval::<String>("local _,_,_,_,_,_,_,_,_,_,_,_,d = GetSkillLineInfo(2) return d")
        .unwrap();
    assert_eq!((count, desc.as_str()), (13, "About Defense."));
    // The cost type is the cost index plus one, so index 0 reads 1.
    let (min_level, cost_type) = s
        .eval::<(i64, i64)>("local _,_,_,_,_,_,_,_,_,_,ml,ct = GetSkillLineInfo(2) return ml,ct")
        .unwrap();
    assert_eq!((min_level, cost_type), (0, 1));
}

#[test]
fn the_permanent_bonus_folds_into_the_numbers_and_the_temporary_one_does_not() {
    let mut s = UiScript::new().unwrap();
    let mut st = state();
    st.entries[1].perm_bonus = 10; // Defense 180/300, temp +5
    st.entries
        .push(entry(182, "Herbalism", 0, 0, 0, 2, "Professions", 2));
    st.entries.last_mut().unwrap().perm_bonus = 10;
    s.set_skills(st);

    // Defense (row 2): 190/310, the modifier still the temp +5 alone.
    let (rank, modifier, max) = s
        .eval::<(i64, i64, i64)>("local _,_,_,r,_,m,mx = GetSkillLineInfo(2) return r,m,mx")
        .unwrap();
    assert_eq!((rank, modifier, max), (190, 5, 310));

    // Herbalism 0/0 (row 7, untrained so last): the bonus is not added to a zero.
    let (name, rank, max) = s
        .eval::<(String, i64, i64)>("local n,_,_,r,_,_,mx = GetSkillLineInfo(7) return n,r,mx")
        .unwrap();
    assert_eq!((name.as_str(), rank, max), ("Herbalism", 0, 0));
}

#[test]
fn untrained_lines_sort_under_the_trained_ones_of_their_category() {
    let mut s = UiScript::new().unwrap();
    let mut st = state();
    // "Alchemy" would sort first in Professions by name; at rank 0 it goes last.
    st.entries
        .push(entry(171, "Alchemy", 0, 300, 0, 2, "Professions", 2));
    s.set_skills(st);
    let names: Vec<String> = (5..=7)
        .map(|i| {
            s.eval::<String>(&format!("return (GetSkillLineInfo({i}))"))
                .unwrap()
        })
        .collect();
    assert_eq!(names, ["First Aid", "Fishing", "Alchemy"]);
}

#[test]
fn a_mono_line_reports_max_rank_one_whatever_the_server_said() {
    let mut s = UiScript::new().unwrap();
    let mut st = state();
    // Beast Mastery: category 3, so mono by the fixture rule; 300/300 from the server.
    st.entries.push(entry(
        50,
        "Beast Mastery",
        300,
        300,
        0,
        3,
        "Class Skills",
        0,
    ));
    s.set_skills(st);

    // Rows 1 and 2: the Class Skills header and its entry (category_order 0 sorts first).
    let (name, rank, modifier, max) = s
        .eval::<(String, i64, i64, i64)>(
            "local n,_,_,r,_,m,mx = GetSkillLineInfo(2) return n,r,m,mx",
        )
        .unwrap();
    assert_eq!(
        (name.as_str(), rank, modifier, max),
        ("Beast Mastery", 1, 0, 1),
        "the descriptor's 300/300 reads 1/1 — max overridden, rank clamped under it"
    );
    // Defense, not mono, still reports its real 300.
    assert_eq!(
        s.eval::<i64>("local _,_,_,_,_,_,mx = GetSkillLineInfo(4) return mx")
            .unwrap(),
        300
    );
}

/// The count is the assertion: `GetSkillLineInfo(0) == nil` holds for a single nil too, since
/// Lua compares only the first value.
#[test]
fn out_of_range_answers_the_thirteen_value_tuple() {
    let s = UiScript::new().unwrap();
    for idx in ["0", "1", "99"] {
        assert_eq!(
            s.arity(&format!("GetSkillLineInfo({idx})")).unwrap(),
            13,
            "GetSkillLineInfo({idx}) must answer 13 values"
        );
        assert_eq!(
            s.eval::<i64>(&format!(
                "local _,_,_,r,t = GetSkillLineInfo({idx}) return r + t"
            ))
            .unwrap(),
            0,
            "GetSkillLineInfo({idx}) slots 4+5 must be NUMBERS — the reference adds them unguarded"
        );
        assert!(
            s.eval::<bool>(&format!("return (GetSkillLineInfo({idx})) == nil"))
                .unwrap(),
            "slot 1 must stay nil — it drives the stock file's own `if (not skillName)` early-out"
        );
    }
}

#[test]
fn no_push_reports_zero_rows() {
    let s = UiScript::new().unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 0);
    // The empty pane answers the out-of-range tuple; this pins slot 1.
    assert!(s
        .eval::<bool>("return (GetSkillLineInfo(1)) == nil")
        .unwrap());
    assert_eq!(s.eval::<i64>("return GetSelectedSkill()").unwrap(), 0);
    assert_eq!(s.eval::<i64>("return GetAdjustedSkillPoints()").unwrap(), 0);
    // Collapse, expand and select on an empty pane do nothing.
    s.run("CollapseSkillHeader(0) ExpandSkillHeader(1) SetSelectedSkill(1)")
        .unwrap();
    assert_eq!(s.eval::<i64>("return GetNumSkillLines()").unwrap(), 0);
}
