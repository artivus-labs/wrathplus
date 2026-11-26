use super::*;
use crate::script::UiScript;

/// An ordinary bar row: visible, not a header. `parent` 0 puts it in the "Other" bucket.
fn entry(faction_id: u32, rep_list_id: u32, parent_id: u32, name: &str) -> FactionEntry {
    FactionEntry {
        faction_id,
        rep_list_id,
        parent_id,
        name: name.into(),
        description: format!("About the {name}."),
        standing: 4000,
        standing_id: 5,
        bar_min: 3000,
        bar_max: 9000,
        visible: true,
        is_header: false,
        at_war: false,
        can_toggle_at_war: true,
        inactive: false,
        has_rep: false,
    }
}

/// A header row as the wire delivers one: flag `0x08` without `VISIBLE`, as the Steamwheedle
/// Cartel's byte is; a header is never drawn as a bar, so its own visibility is irrelevant.
fn header(faction_id: u32, rep_list_id: u32, name: &str) -> FactionEntry {
    FactionEntry {
        visible: false,
        is_header: true,
        ..entry(faction_id, rep_list_id, 0, name)
    }
}

/// Pushed out of order so the engine's sort shows: Alliance (469) over Stormwind and Ironforge,
/// Steamwheedle (169) over Booty Bay, and two parentless factions. Bloodsail Buccaneers holds
/// slot 0, which must never read as "watch nothing".
fn state() -> ReputationState {
    ReputationState {
        entries: vec![
            entry(72, 19, 469, "Stormwind"),
            entry(529, 13, 0, "Argent Dawn"),
            header(469, 11, "Alliance"),
            entry(21, 1, 169, "Booty Bay"),
            entry(47, 20, 469, "Ironforge"),
            header(169, 10, "Steamwheedle Cartel"),
            entry(87, 0, 0, "Bloodsail Buccaneers"),
        ],
        watched: None,
    }
}

fn seated() -> UiScript {
    let mut s = UiScript::new().expect("VM");
    s.set_reputation(state());
    s
}

fn names(s: &UiScript) -> Vec<String> {
    s.eval::<Vec<String>>(
        "local t = {} for i = 1, GetNumFactions() do t[i] = (GetFactionInfo(i)) end return t",
    )
    .expect("names")
}

/// The fixture's headers lack the visible flag, as on the wire, so this also pins that a
/// header's own visibility does not gate its group.
#[test]
fn the_header_flag_builds_the_tree_and_other_comes_last() {
    let s = seated();
    assert_eq!(
        names(&s),
        [
            "Alliance",
            "Ironforge",
            "Stormwind",
            "Steamwheedle Cartel",
            "Booty Bay",
            "Other",
            "Argent Dawn",
            "Bloodsail Buccaneers",
        ],
        "two named groups sorted by header, then the bucket; children sorted within"
    );
    let (name, _desc, standing_id, _min, _max, _val, at_war, toggle, is_header) = s
        .eval::<(String, String, i64, i64, i64, i64, Value, Value, i64)>("return GetFactionInfo(1)")
        .unwrap();
    assert_eq!(name, "Alliance", "named from the flagged header row");
    assert_eq!(is_header, 1);
    // 3.3.5's builder reads a header's own faction like a bar's (`0x5d0da0`).
    assert_eq!(standing_id, 5, "the header faction's own standing");
    assert!(
        at_war.is_nil() && toggle == Value::Integer(1),
        "and its war state"
    );
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn a_header_with_no_visible_children_is_dropped() {
    let mut s = UiScript::new().expect("VM");
    let mut st = state();
    // Unmeet both of Alliance's cities; Alliance itself stays in the push.
    for e in &mut st.entries {
        if e.parent_id == 469 {
            e.visible = false;
        }
    }
    s.set_reputation(st);
    assert!(
        !names(&s).iter().any(|n| n == "Alliance"),
        "an empty header is not drawn; got {:?}",
        names(&s)
    );
    assert_eq!(names(&s)[0], "Steamwheedle Cartel");
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn a_bar_row_returns_the_references_thirteen_and_so_does_a_miss() {
    let s = seated();
    let got = s
        .eval::<(
            String,
            String,
            i64,
            i64,
            i64,
            i64,
            Value,
            Value,
            Value,
            Value,
            Value,
            Value,
            Value,
        )>("return GetFactionInfo(3)")
        .unwrap();
    assert_eq!(got.0, "Stormwind");
    assert_eq!(got.1, "About the Stormwind.");
    assert_eq!(got.2, 5, "standingID is 1-based: FACTION_STANDING_LABEL5");
    assert_eq!((got.3, got.4, got.5), (3000, 9000, 4000), "absolute");
    assert!(got.6.is_nil(), "atWarWith");
    assert_eq!(got.7, Value::Integer(1), "canToggleAtWar");
    assert!(got.8.is_nil() && got.9.is_nil(), "isHeader/isCollapsed");
    assert!(got.10.is_nil(), "hasRep");
    assert!(got.11.is_nil(), "isWatched");
    assert!(got.12.is_nil(), "isChild");

    for miss in ["GetFactionInfo(99)", "GetFactionInfo(0)"] {
        let n = s.arity(miss).unwrap();
        assert_eq!(n, 13, "{miss} must still be thirteen wide");
        let sid = s
            .eval::<i64>(&format!("local _,_,s = {miss} return s"))
            .unwrap();
        assert_eq!(sid, 1, "{miss}'s standingID slot is the client's 1");
    }
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn folding_shrinks_the_list_and_index_zero_folds_everything() {
    let s = seated();
    s.run("CollapseFactionHeader(1)").unwrap();
    assert_eq!(
        names(&s),
        [
            "Alliance",
            "Steamwheedle Cartel",
            "Booty Bay",
            "Other",
            "Argent Dawn",
            "Bloodsail Buccaneers"
        ]
    );
    assert_eq!(
        s.eval::<Value>("local _,_,_,_,_,_,_,_,_,c = GetFactionInfo(1) return c")
            .unwrap(),
        Value::Integer(1),
        "and reports itself collapsed"
    );
    s.run("ExpandFactionHeader(1)").unwrap();
    assert_eq!(names(&s).len(), 8, "expanded again");

    // A bar row is not a header, so it takes the same fall-through: index 2 is Ironforge.
    s.run("CollapseFactionHeader(2)").unwrap();
    assert_eq!(
        names(&s),
        ["Alliance", "Steamwheedle Cartel", "Other"],
        "a non-header index folds every header"
    );
    // So do index 0 and an index past the end.
    s.run("ExpandFactionHeader(0)").unwrap();
    assert_eq!(names(&s).len(), 8, "index 0 unfolds every header");
    s.run("CollapseFactionHeader(99)").unwrap();
    assert_eq!(names(&s).len(), 3, "so does an out-of-range index");
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn a_push_resets_the_folds() {
    let mut s = seated();
    s.run("CollapseFactionHeader(1)").unwrap();
    assert_eq!(names(&s).len(), 6, "folded");
    let mut ticked = state();
    ticked.entries[0].standing = 12345;
    s.set_reputation(ticked);
    assert_eq!(names(&s).len(), 8, "the rebuild unfolded it again");
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn an_inactive_faction_moves_under_the_inactive_header() {
    let mut s = UiScript::new().expect("VM");
    let mut st = state();
    st.entries[1].inactive = true; // Argent Dawn, an "Other" row
    s.set_reputation(st);
    assert_eq!(
        names(&s),
        [
            "Alliance",
            "Ironforge",
            "Stormwind",
            "Steamwheedle Cartel",
            "Booty Bay",
            "Other",
            "Bloodsail Buccaneers",
            "Inactive",
        ],
        "Inactive sorts after Other, and arrives COLLAPSED so its child is not listed"
    );
    // Unfolding it reveals exactly the re-parented row.
    s.run("ExpandFactionHeader(8)").unwrap();
    assert_eq!(names(&s).last().unwrap(), "Argent Dawn");
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn selection_follows_the_faction_not_the_row_index() {
    let mut s = seated();
    assert_eq!(s.eval::<i64>("return GetSelectedFaction()").unwrap(), 0);
    s.run("SetSelectedFaction(3)").unwrap(); // Stormwind
    assert_eq!(s.eval::<i64>("return GetSelectedFaction()").unwrap(), 3);
    // Fold Alliance: Stormwind is no longer a visible row, so there is no index to answer.
    s.run("CollapseFactionHeader(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedFaction()").unwrap(), 0);
    s.run("ExpandFactionHeader(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedFaction()").unwrap(), 3);
    // Selecting a header clears the selection rather than selecting the group.
    s.run("SetSelectedFaction(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedFaction()").unwrap(), 0);
    // A push that drops the faction entirely drops the selection with it.
    s.run("SetSelectedFaction(3)").unwrap();
    let mut without = state();
    without.entries.retain(|e| e.faction_id != 72);
    s.set_reputation(without);
    assert_eq!(s.eval::<i64>("return GetSelectedFaction()").unwrap(), 0);
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

/// A peace-forced faction refuses: the stock pane disables the box, but an addon can still call
/// the global.
#[test]
fn the_war_and_inactive_toggles_flip_locally_and_queue_their_sends() {
    let mut s = seated();
    s.run("FactionToggleAtWar(3)").unwrap(); // Stormwind
    assert_eq!(
        s.eval::<Value>("local _,_,_,_,_,_,w = GetFactionInfo(3) return w")
            .unwrap(),
        Value::Integer(1),
        "the pane shows war immediately — nothing acks this"
    );
    s.run("SetFactionInactive(3)").unwrap();
    assert_eq!(
        names(&s).last().unwrap(),
        "Inactive",
        "and the row re-parents on the spot, without a server rebuild"
    );
    assert_eq!(
        s.take_reputation_sends(),
        [
            ReputationSend::AtWar {
                rep_list_id: 19,
                at_war: true
            },
            ReputationSend::Inactive {
                rep_list_id: 19,
                inactive: true
            },
        ]
    );
    assert!(s.take_reputation_sends().is_empty(), "the drain empties");

    // A peace-forced row: no flip, no send.
    let mut forced = state();
    forced.entries[0].can_toggle_at_war = false;
    s.set_reputation(forced);
    s.run("FactionToggleAtWar(3)").unwrap();
    assert!(s
        .eval::<Value>("local _,_,_,_,_,_,w = GetFactionInfo(3) return w")
        .unwrap()
        .is_nil());
    assert!(s.take_reputation_sends().is_empty());
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn watching_queues_a_send_but_does_not_move_the_bar_itself() {
    let mut s = seated();
    assert!(
        s.eval::<Value>("return GetWatchedFactionInfo()")
            .unwrap()
            .is_nil(),
        "nothing watched → a nil name, which stock `if ( name )` gates on"
    );

    s.run("SetWatchedFactionIndex(3)").unwrap(); // Stormwind
    assert!(
        s.eval::<Value>("return GetWatchedFactionInfo()")
            .unwrap()
            .is_nil(),
        "still nothing: the server owns this field, so the click alone cannot move the bar"
    );
    assert_eq!(s.take_reputation_sends(), [ReputationSend::Watch(Some(19))]);

    // The descriptor update arrives as the next push, and now the bar reads.
    let mut watched = state();
    watched.watched = Some(19);
    s.set_reputation(watched);
    assert_eq!(
        s.eval::<(String, i64, i64, i64, i64)>("return GetWatchedFactionInfo()")
            .unwrap(),
        ("Stormwind".into(), 5, 3000, 9000, 4000)
    );
    assert_eq!(
        s.eval::<Value>("local _,_,_,_,_,_,_,_,_,_,_,w = GetFactionInfo(3) return w")
            .unwrap(),
        Value::Integer(1),
        "and the row reports itself watched"
    );

    // Watching slot 0, then clearing: the two must stay distinct.
    s.run("SetWatchedFactionIndex(8)").unwrap(); // Bloodsail Buccaneers, rep slot 0
    assert_eq!(s.take_reputation_sends(), [ReputationSend::Watch(Some(0))]);
    s.run("SetWatchedFactionIndex(0)").unwrap();
    assert_eq!(s.take_reputation_sends(), [ReputationSend::Watch(None)]);
    assert!(s.errors().is_empty(), "{:#?}", s.errors());
}

#[test]
fn a_sub_header_with_its_own_bar_nests_its_children() {
    let mut s = UiScript::new().unwrap();
    let mut expedition = header(1052, 30, "Horde Expedition");
    expedition.parent_id = 1097;
    expedition.has_rep = true;
    s.set_reputation(ReputationState {
        entries: vec![
            header(1097, 40, "Wrath of the Lich King"),
            expedition,
            entry(1064, 31, 1052, "The Taunka"),
            entry(1085, 32, 1052, "Warsong Offensive"),
            entry(1090, 33, 1097, "Kirin Tor"),
        ],
        watched: Some(30),
    });
    let names = |s: &UiScript| {
        s.eval::<Vec<String>>(
            "local t = {} for i = 1, GetNumFactions() do t[i] = (GetFactionInfo(i)) end return t",
        )
        .unwrap()
    };
    assert_eq!(
        names(&s),
        [
            "Wrath of the Lich King",
            "Horde Expedition",
            "The Taunka",
            "Warsong Offensive",
            "Kirin Tor"
        ]
    );
    assert!(s
        .eval::<bool>("local _,_,_,_,_,_,_,_,h,c,r,w,k = GetFactionInfo(2) return h == 1 and c == nil and r == 1 and w == 1 and k == nil")
        .unwrap());
    assert!(s
        .eval::<bool>("local k = select(13, GetFactionInfo(3)) local k5 = select(13, GetFactionInfo(5)) return k == 1 and k5 == nil")
        .unwrap());
    assert!(s.eval::<bool>("return select(1, GetFactionInfoByID(1064)) == 'The Taunka' and select(13, GetFactionInfoByID(1064)) == 1").unwrap());
    s.run("CollapseFactionHeader(2)").unwrap();
    assert_eq!(
        names(&s),
        ["Wrath of the Lich King", "Horde Expedition", "Kirin Tor"]
    );
    assert!(s
        .eval::<bool>("return select(10, GetFactionInfo(2)) == 1")
        .unwrap());
    s.run("SetWatchedFactionIndex(2)").unwrap();
    assert_eq!(s.take_reputation_sends(), [ReputationSend::Watch(Some(30))]);
}
