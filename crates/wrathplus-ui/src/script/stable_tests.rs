use super::*;
use crate::script::UiScript;

fn pet(number: u32, name: &str, level: u32) -> StablePetSlot {
    StablePetSlot {
        pet_number: number,
        icon: Some("Interface\\Icons\\Ability_Hunter_Pet_Wolf".into()),
        name: name.into(),
        level,
        family: Some("Wolf".into()),
        talent: Some("Ferocity".into()),
        diet: vec!["Meat".into(), "Fish".into()],
    }
}

fn state(slots: [Option<StablePetSlot>; NUM_STABLE_SLOTS]) -> StableState {
    StableState {
        num_stable_slots: 1,
        next_slot_cost: 50_000,
        slots,
        has_live_pet: true,
    }
}

/// A hunter with a pet out and one stabled, one slot bought.
fn open(s: &mut UiScript) {
    s.set_stable(Some(state([
        Some(pet(7, "Rex", 41)),
        Some(pet(8, "Bruiser", 38)),
        None,
        None,
        None,
    ])));
}

#[test]
fn the_read_surface_answers_the_reference_calls() {
    let mut s = UiScript::new().unwrap();
    assert_eq!(s.eval::<i64>("return GetNumStableSlots()").unwrap(), 0);
    assert_eq!(s.eval::<i64>("return GetNumStablePets()").unwrap(), 0);
    assert_eq!(s.eval::<i64>("return GetNextStableSlotCost()").unwrap(), 0);
    assert_eq!(s.eval::<i64>("return GetSelectedStablePet()").unwrap(), -1);

    open(&mut s);
    assert_eq!(s.eval::<i64>("return GetNumStableSlots()").unwrap(), 1);
    assert_eq!(
        s.eval::<i64>("return GetNumStablePets()").unwrap(),
        1,
        "the stabled pets, not the current one"
    );
    assert_eq!(
        s.eval::<i64>("return GetNextStableSlotCost()").unwrap(),
        50_000
    );

    // `PetStable.lua:84`'s five-value destructuring, the talent tree last.
    assert_eq!(
        s.eval::<(String, String, i64, String, String)>(
            "local i, n, l, f, t = GetStablePetInfo(1) return i, n, l, f, t"
        )
        .unwrap(),
        (
            "Interface\\Icons\\Ability_Hunter_Pet_Wolf".into(),
            "Bruiser".into(),
            38,
            "Wolf".into(),
            "Ferocity".into()
        )
    );
    // Below 1, truncated, is the current pet (`0x5a1388`); a non-number raises.
    for i in ["0", "-3", "0.9", "1.9"] {
        let name = s.eval::<String>(&format!("return (select(2, GetStablePetInfo({i})))"));
        assert_eq!(
            name.unwrap(),
            if i == "1.9" { "Bruiser" } else { "Rex" },
            "{i}"
        );
    }
    let err = s.run("GetStablePetInfo()").unwrap_err().to_string();
    assert!(err.contains("Usage: GetStablePetInfo(index)"), "{err}");

    assert_eq!(
        s.eval::<(String, String)>("return GetStablePetFoodTypes(1)")
            .unwrap(),
        ("Meat".into(), "Fish".into())
    );
    assert!(s
        .eval::<bool>("return GetStablePetFoodTypes(2) == nil")
        .unwrap());
}

#[test]
fn a_miss_answers_three_values_and_a_family_without_a_tree_four() {
    let mut s = UiScript::new().unwrap();
    let mut no_tree = pet(9, "Imp", 20);
    no_tree.talent = None;
    // A row whose creature query has not landed has no family yet: the whole row misses.
    let pending = StablePetSlot {
        family: None,
        ..pet(10, "Pending", 30)
    };
    s.set_stable(Some(state([
        Some(pet(7, "Rex", 41)),
        Some(no_tree),
        Some(pending),
        None,
        None,
    ])));
    assert_eq!(s.arity("GetStablePetInfo(1)").unwrap(), 4);
    for call in [
        "GetStablePetInfo(3)",
        "GetStablePetInfo(4)",
        "GetStablePetInfo(9)",
    ] {
        assert_eq!(s.arity(call).unwrap(), 3, "{call} arity");
        // Still falsy for the stock window's `if ( GetStablePetInfo(i) )`.
        assert!(s.eval::<bool>(&format!("return not {call}")).unwrap());
        assert_eq!(
            s.eval::<i64>(&format!("local _, _, l = {call} return l"))
                .unwrap(),
            0,
            "{call} level is 0, not nil"
        );
    }
}

#[test]
fn every_plain_click_returns_truthy() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    for call in [
        "ClickStablePet(1)",
        "ClickStablePet(1)",
        "ClickStablePet(0)",
        "ClickStablePet(2)",
    ] {
        assert!(
            s.eval::<bool>(&format!("return {call} and true or false"))
                .unwrap(),
            "{call}"
        );
    }
    assert!(
        s.take_stable_intents().is_empty(),
        "selecting sends nothing"
    );
}

#[test]
fn the_selection_translates_back_to_slot_indices() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    s.eval::<()>("ClickStablePet(0)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedStablePet()").unwrap(), 0);
    s.eval::<()>("ClickStablePet(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetSelectedStablePet()").unwrap(), 1);
    s.eval::<()>("ClickStablePet(2)").unwrap();
    assert_eq!(
        s.eval::<i64>("return GetSelectedStablePet()").unwrap(),
        -1,
        "an empty slot selects nothing, not itself"
    );
}

#[test]
fn a_list_clears_the_selection_and_a_stale_pet_degrades() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    s.eval::<()>("ClickStablePet(1)").unwrap();
    assert_eq!(s.stable_selection(), 1);

    // Every list clears it (`0x4cadf8`).
    open(&mut s);
    assert_eq!(s.stable_selection(), -1, "a list clears the selection");
}

#[test]
fn every_drop_returns_falsy_even_when_it_sends() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    // A drop that sends.
    assert!(s
        .eval::<bool>("PickupStablePet(1) return ClickStablePet(0) == nil")
        .unwrap());
    assert_eq!(s.take_stable_intents(), vec![StableIntent::Swap(8)]);

    // A drop that sends nothing.
    assert!(s
        .eval::<bool>("PickupStablePet(1) return ClickStablePet(1) == nil")
        .unwrap());
    assert!(s.take_stable_intents().is_empty());
}

#[test]
fn dropping_onto_an_occupied_slot_swaps() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    s.eval::<()>("PickupStablePet(0) ClickStablePet(1)")
        .unwrap();
    assert_eq!(s.take_stable_intents(), vec![StableIntent::Swap(8)]);
}

#[test]
fn an_unpurchased_slot_takes_the_drop_and_sends_nothing() {
    let mut s = UiScript::new().unwrap();
    // Slot 1 bought and empty, slot 2 unbought.
    s.set_stable(Some(state([
        Some(pet(7, "Rex", 41)),
        None,
        None,
        None,
        None,
    ])));
    s.eval::<()>("PickupStablePet(0) ClickStablePet(1)")
        .unwrap();
    assert_eq!(s.take_stable_intents(), vec![StableIntent::Stable]);

    s.eval::<()>("PickupStablePet(0) ClickStablePet(2)")
        .unwrap();
    assert!(s.take_stable_intents().is_empty(), "slot 2 is not bought");
    // The drop completed: the cursor is empty, so the next click selects.
    assert!(s.eval::<bool>("return ClickStablePet(1) and true").unwrap());
    assert!(s.take_stable_intents().is_empty());
}

#[test]
fn the_fork_follows_the_live_pet_not_the_row() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    s.eval::<()>("PickupStablePet(1) ClickStablePet(0)")
        .unwrap();
    assert_eq!(s.take_stable_intents(), vec![StableIntent::Swap(8)]);

    // The same list, but the pet is dismissed: the row survives, the guid does not.
    let mut dismissed = state([
        Some(pet(7, "Rex", 41)),
        Some(pet(8, "Bruiser", 38)),
        None,
        None,
        None,
    ]);
    dismissed.has_live_pet = false;
    s.set_stable(Some(dismissed));
    s.eval::<()>("PickupStablePet(1) ClickStablePet(0)")
        .unwrap();
    assert_eq!(
        s.take_stable_intents(),
        vec![StableIntent::Unstable(8)],
        "a row without a live guid unstables"
    );
}

#[test]
fn stable_to_stable_is_a_no_op_and_empty_slots_do_not_grab() {
    let mut s = UiScript::new().unwrap();
    s.set_stable(Some(StableState {
        num_stable_slots: 2,
        slots: [None, Some(pet(8, "Bruiser", 38)), None, None, None],
        ..state([None, None, None, None, None])
    }));
    s.eval::<()>("PickupStablePet(1) ClickStablePet(2)")
        .unwrap();
    assert!(s.take_stable_intents().is_empty());

    // An empty slot never arms a grab, so the following click is a select.
    s.eval::<()>("PickupStablePet(2)").unwrap();
    assert!(s.eval::<bool>("return ClickStablePet(0) and true").unwrap());
    assert!(s.take_stable_intents().is_empty());
}

#[test]
fn buy_stable_slot_gates_locally_and_silently() {
    let mut s = UiScript::new().unwrap();
    s.set_money(1_000_000);
    open(&mut s);
    s.eval::<()>("BuyStableSlot()").unwrap();
    assert_eq!(s.take_stable_intents(), vec![StableIntent::BuySlot]);

    // Too poor.
    s.set_money(10);
    s.set_stable(Some(state([None, None, None, None, None])));
    s.eval::<()>("BuyStableSlot()").unwrap();
    assert!(s.take_stable_intents().is_empty(), "unaffordable is silent");

    // All four slots owned, with money restored so only the cap refuses.
    s.set_money(1_000_000);
    let mut full = state([None, None, None, None, None]);
    full.num_stable_slots = 4;
    s.set_stable(Some(full));
    s.eval::<()>("BuyStableSlot()").unwrap();
    assert!(s.take_stable_intents().is_empty(), "the cap is silent");
}

#[test]
fn the_unstable_binding_gates_where_the_drag_path_does_not() {
    let mut s = UiScript::new().unwrap();
    open(&mut s); // has_live_pet = true
    s.eval::<()>("UnstablePet(1)").unwrap();
    assert!(s.take_stable_intents().is_empty(), "a pet is out — silent");

    let mut dismissed = state([None, Some(pet(8, "Bruiser", 38)), None, None, None]);
    dismissed.has_live_pet = false;
    s.set_stable(Some(dismissed));
    s.eval::<()>("UnstablePet(0)").unwrap();
    assert!(s.take_stable_intents().is_empty(), "index 0 is rejected");
    s.eval::<()>("UnstablePet(1)").unwrap();
    assert_eq!(s.take_stable_intents(), vec![StableIntent::Unstable(8)]);
}

#[test]
fn close_drains_and_leaves_the_cursor_alone() {
    let mut s = UiScript::new().unwrap();
    open(&mut s);
    assert!(!s.take_stable_close());
    s.eval::<()>("ClosePetStables()").unwrap();
    assert!(s.take_stable_close());
    assert!(!s.take_stable_close(), "drain clears");
}

#[test]
fn the_paperdoll_setter_is_callable() {
    let s = UiScript::new().unwrap();
    s.eval::<()>("SetPetStablePaperdoll(nil)").unwrap();
}

#[test]
fn wrath_slots_three_and_four_read_select_and_pick_up() {
    let mut s = UiScript::new().unwrap();
    let mut full = state(std::array::from_fn(|i| {
        Some(pet(7 + i as u32, &format!("Pet{i}"), 40 + i as u32))
    }));
    full.num_stable_slots = 4;
    s.set_stable(Some(full));
    assert_eq!(s.eval::<i64>("return GetNumStableSlots()").unwrap(), 4);
    assert_eq!(s.eval::<i64>("return GetNumStablePets()").unwrap(), 4);
    for slot in [3, 4] {
        assert_eq!(
            s.eval::<(String, i64)>(&format!(
                "local _, name, level = GetStablePetInfo({slot}) return name, level"
            ))
            .unwrap(),
            (format!("Pet{slot}"), 40 + slot)
        );
        assert_eq!(
            s.eval::<(String, String)>(&format!("return GetStablePetFoodTypes({slot})"))
                .unwrap(),
            ("Meat".into(), "Fish".into())
        );
        s.eval::<()>(&format!("ClickStablePet({slot})")).unwrap();
        assert_eq!(s.stable_selection(), slot as i32);
        assert_eq!(
            s.eval::<i64>("return GetSelectedStablePet()").unwrap(),
            slot
        );
        assert!(s.take_stable_intents().is_empty());
        // Successful pickup drops this pet's number, not its slot index.
        s.eval::<()>(&format!("PickupStablePet({slot}) ClickStablePet(0)"))
            .unwrap();
        assert_eq!(
            s.take_stable_intents(),
            vec![StableIntent::Swap(7 + slot as u32)]
        );
    }
}

#[test]
fn fourth_slot_accepts_a_drop_only_when_bought() {
    let mut s = UiScript::new().unwrap();
    for bought in [3, 4] {
        let mut snapshot = state([Some(pet(7, "Rex", 41)), None, None, None, None]);
        snapshot.num_stable_slots = bought;
        s.set_stable(Some(snapshot));
        assert!(s
            .eval::<bool>("PickupStablePet(0) return ClickStablePet(4) == nil")
            .unwrap());
        let expected = if bought == 4 {
            vec![StableIntent::Stable]
        } else {
            vec![]
        };
        assert_eq!(s.take_stable_intents(), expected);
        // Both accepted and refused drops clear the held pet.
        assert!(s.eval::<bool>("return ClickStablePet(0) == 1").unwrap());
        assert!(s.take_stable_intents().is_empty());
    }
}

#[test]
fn buying_fourth_slot_is_allowed_but_fifth_is_refused() {
    let mut s = UiScript::new().unwrap();
    s.set_money(1_000_000);
    for bought in [3, 4, 5] {
        let mut snapshot = state([None, None, None, None, None]);
        snapshot.num_stable_slots = bought;
        s.set_stable(Some(snapshot));
        s.eval::<()>("BuyStableSlot()").unwrap();
        let expected = if bought == 3 {
            vec![StableIntent::BuySlot]
        } else {
            vec![]
        };
        assert_eq!(s.take_stable_intents(), expected, "{bought} already bought");
    }
}

#[test]
fn index_five_is_rejected_by_read_select_pickup_drop_and_unstable() {
    let mut s = UiScript::new().unwrap();
    let mut snapshot = state(std::array::from_fn(|i| Some(pet(7 + i as u32, "Pet", 40))));
    snapshot.num_stable_slots = 4;
    snapshot.has_live_pet = false;
    s.set_stable(Some(snapshot));
    assert_eq!(slot_index(4), Some(4));
    assert_eq!(slot_index(5), None);
    assert_eq!(slot_index(-1), None);
    assert_eq!(s.arity("GetStablePetInfo(5)").unwrap(), 3);
    assert!(s.eval::<bool>("return GetStablePetInfo(5) == nil").unwrap());
    assert_eq!(s.arity("GetStablePetFoodTypes(5)").unwrap(), 0);
    s.eval::<()>("ClickStablePet(4) ClickStablePet(5)").unwrap();
    assert_eq!(s.stable_selection(), -1);
    s.eval::<()>("PickupStablePet(5)").unwrap();
    assert!(s.eval::<bool>("return ClickStablePet(0) == 1").unwrap());
    assert!(s
        .eval::<bool>("PickupStablePet(4) return ClickStablePet(5) == nil")
        .unwrap());
    assert!(s.eval::<bool>("return ClickStablePet(0) == 1").unwrap());
    s.eval::<()>("UnstablePet(5)").unwrap();
    assert!(s.take_stable_intents().is_empty());
}
