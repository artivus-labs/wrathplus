//! Tests for the trainer tree: the per-type row orders, the filter, the collapse and the intents.

use super::*;
use crate::script::UiScript;

fn svc(
    spell_id: u32,
    name: &str,
    cat: TrainerServiceCategory,
    group_key: u32,
    group_name: &str,
) -> TrainerService {
    TrainerService {
        spell_id,
        tooltip: TrainerTooltip::Spell {
            spell_id,
            alt_caster: false,
        },
        name: Some(name.into()),
        subtext: Some("Rank 1".into()),
        texture: Some(format!("Interface\\Icons\\Spell_{spell_id}")),
        description: format!("Teaches {name}."),
        cost: 100,
        prof_first_rank: false,
        category: cat,
        level_req: 10,
        skill_req: None,
        ability_reqs: vec![],
        is_trade_skill: false,
        group_key,
        group_name: group_name.into(),
        item_link: None,
        skill_line: None,
    }
}

/// A class trainer whose tree is `[H:Arms, Heroic Strike, Cleave, H:Fury, Bloodrage]`, the three
/// services available, used and unavailable.
pub(super) fn trainer() -> TrainerState {
    let mut hs = svc(
        78,
        "Heroic Strike",
        TrainerServiceCategory::Available,
        26,
        "Arms",
    );
    hs.level_req = 1;
    let mut cl = svc(284, "Cleave", TrainerServiceCategory::Used, 26, "Arms");
    cl.level_req = 20;
    let br = svc(
        285,
        "Bloodrage",
        TrainerServiceCategory::Unavailable,
        256,
        "Fury",
    );
    TrainerState {
        greeting: "What would you like to learn?".into(),
        trainer_type: 0,
        groups: Vec::new(),
        services: vec![hs, cl, br],
    }
}

/// A VM whose `serviceTypeFilter` shows every state; the CVar's default 3 hides known services.
fn every_state_shown() -> UiScript {
    let s = UiScript::new().unwrap();
    s.run("SetTrainerServiceTypeFilter('all', 1)").unwrap();
    s
}

/// Every visible row as `(name, serviceType)`.
fn visible(s: &mut UiScript) -> Vec<(String, String)> {
    let n = s.eval::<i64>("return GetNumTrainerServices()").unwrap();
    (1..=n)
        .map(|i| {
            s.eval::<(String, String)>(&format!(
                "local n,_,t = GetTrainerServiceInfo({i}) return n,t"
            ))
            .unwrap()
        })
        .collect()
}

#[test]
fn tree_interleaves_headers_and_ordered_services() {
    let mut s = every_state_shown();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 0);
    assert!(s
        .eval::<bool>("return GetTrainerServiceInfo(1) == nil")
        .unwrap());

    s.set_trainer(Some(trainer()));
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 5);

    let (hn, hsub, ht, hexp) = s
        .eval::<(String, Option<String>, String, Option<i64>)>(
            "local n,s,t,e = GetTrainerServiceInfo(1) return n,s,t,e",
        )
        .unwrap();
    assert_eq!((hn.as_str(), ht.as_str()), ("Arms", "header"));
    // A header's subtext is "" (`0x594fd4`).
    assert_eq!((hsub, hexp), (Some(String::new()), Some(1)));

    let info = |s: &mut UiScript, i: i64| {
        s.eval::<(String, String)>(&format!(
            "local n,_,t = GetTrainerServiceInfo({i}) return n,t"
        ))
        .unwrap()
    };
    assert_eq!(
        info(&mut s, 2),
        ("Heroic Strike".into(), "available".into())
    );
    assert_eq!(info(&mut s, 3), ("Cleave".into(), "used".into()));
    assert_eq!(info(&mut s, 4), ("Fury".into(), "header".into()));
    assert_eq!(info(&mut s, 5), ("Bloodrage".into(), "unavailable".into()));
}

#[test]
fn service_getters_read_the_row_at_a_visible_index() {
    let mut s = UiScript::new().unwrap();
    let mut t = trainer();
    // Heroic Strike, row 2, gets a full gate set.
    t.services[0].skill_req = Some(TrainerSkillReq {
        name: "Blacksmithing".into(),
        rank: 100,
        met: true,
    });
    t.services[0].ability_reqs = vec![TrainerAbilityReq {
        name: "Apprentice".into(),
        met: false,
    }];
    s.set_trainer(Some(t));

    assert_eq!(
        s.eval::<(i64, i64, i64)>("return GetTrainerServiceCost(2)")
            .unwrap(),
        (100, 0, 0)
    );
    assert_eq!(
        s.eval::<i64>("return GetTrainerServiceLevelReq(2)")
            .unwrap(),
        1
    );
    assert_eq!(
        s.eval::<String>("return GetTrainerServiceDescription(2)")
            .unwrap(),
        "Teaches Heroic Strike."
    );
    let (skill, rank, has) = s
        .eval::<(String, i64, i64)>("return GetTrainerServiceSkillReq(2)")
        .unwrap();
    assert_eq!((skill.as_str(), rank, has), ("Blacksmithing", 100, 1));
    assert!(s
        .eval::<bool>(
            "local n,h = GetTrainerServiceAbilityReq(2,1) return n=='Apprentice' and h==nil"
        )
        .unwrap());
    assert!(s
        .eval::<bool>("local l,p = IsTrainerServiceLearnSpell(2) return l==1 and p==nil")
        .unwrap());

    // Row 1 is a header, so the getters answer their defaults.
    assert_eq!(
        s.eval::<i64>("return GetTrainerServiceLevelReq(1)")
            .unwrap(),
        0
    );
    assert!(s
        .eval::<bool>("return GetTrainerServiceIcon(1) == nil")
        .unwrap());
    assert!(s
        .eval::<bool>("return GetTrainerServiceSkillReq(1) == nil")
        .unwrap());
}

/// The reference's filter hide exempts no header (`0x4d8528`, `0x4d8535`).
#[test]
fn state_filter_takes_a_groups_header_with_its_last_service() {
    let mut s = every_state_shown();
    s.set_trainer(Some(trainer()));
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 5);

    s.run("SetTrainerServiceTypeFilter('used', 0)").unwrap();
    assert_eq!(
        visible(&mut s)
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>(),
        ["header", "available", "header", "unavailable"]
    );

    s.run("SetTrainerServiceTypeFilter('available', 0)")
        .unwrap();
    assert_eq!(
        visible(&mut s),
        [
            ("Fury".to_string(), "header".to_string()),
            ("Bloodrage".to_string(), "unavailable".to_string()),
        ]
    );

    s.run("SetTrainerServiceTypeFilter('unavailable', 0)")
        .unwrap();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 0);

    // The rows remain: the getters bound on the total `ds:0xb73a10` (`0x4d89b0`), not the visible
    // count `ds:0xb73a18`.
    assert_eq!(
        s.eval::<String>("return (GetTrainerServiceInfo(1))")
            .unwrap(),
        "Arms",
        "row 1 is still the Arms header, now in the hidden tail"
    );
    // The list seated the first available service (12340 `0x594ba0`), now hidden with its group.
    assert!(
        s.eval::<bool>("return GetTrainerSelectionIndex() > GetNumTrainerServices()")
            .unwrap(),
        "the hidden selection reads past the visible count, a different question from row 1"
    );
}

/// The reference's collapse test exempts headers (`0x4d853d`), where the filter's does not.
#[test]
fn collapse_keeps_the_header_the_filter_would_remove() {
    let mut s = UiScript::new().unwrap();
    s.set_trainer(Some(trainer()));
    s.run("CollapseTrainerSkillLine(1)").unwrap(); // fold Arms
    assert_eq!(
        visible(&mut s),
        [
            ("Arms".to_string(), "header".to_string()),
            ("Fury".to_string(), "header".to_string()),
            ("Bloodrage".to_string(), "unavailable".to_string()),
        ]
    );
    s.run("CollapseTrainerSkillLine(0)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 2);
}

#[test]
fn collapse_by_header_index_and_collapse_all() {
    let mut s = every_state_shown();
    s.set_trainer(Some(trainer()));

    s.run("CollapseTrainerSkillLine(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 3);
    assert!(s
        .eval::<bool>("local _,_,t,e = GetTrainerServiceInfo(1) return t=='header' and e==nil")
        .unwrap());
    assert_eq!(
        s.eval::<String>("local n = GetTrainerServiceInfo(2) return n")
            .unwrap(),
        "Fury",
        "Arms' services are folded; Fury's header is now row 2"
    );

    s.run("ExpandTrainerSkillLine(1)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 5);

    s.run("CollapseTrainerSkillLine(0)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 2);
    s.run("ExpandTrainerSkillLine(0)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 5);
}

#[test]
fn collapse_survives_a_content_update_and_resets_on_close() {
    let mut s = every_state_shown();
    s.set_trainer(Some(trainer()));
    s.run("CollapseTrainerSkillLine(1)").unwrap(); // fold Arms
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 3);

    // A content update over the same groups keeps the fold.
    s.set_trainer(Some(trainer()));
    assert_eq!(
        s.eval::<i64>("return GetNumTrainerServices()").unwrap(),
        3,
        "Arms stays collapsed across a content update"
    );

    s.set_trainer(None);
    s.set_trainer(Some(trainer()));
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 5);
}

/// The reference's buy refuses any state byte but 0 (`0x4d89d0`, `0x4d89e5`).
#[test]
fn buy_queues_an_available_services_spell_id_and_refuses_every_other_row() {
    let mut s = UiScript::new().unwrap();
    s.set_trainer(Some(trainer()));
    s.run("BuyTrainerService(2)").unwrap();
    assert_eq!(s.take_trainer_buys(), vec![78]);
    assert!(s.take_trainer_buys().is_empty(), "drained");

    s.run("BuyTrainerService(1)").unwrap();
    assert!(s.take_trainer_buys().is_empty(), "a header is not buyable");

    s.run("BuyTrainerService(3)").unwrap();
    s.run("BuyTrainerService(5)").unwrap();
    assert!(
        s.take_trainer_buys().is_empty(),
        "only state byte 0 — available — reaches the wire"
    );
}

/// `IsTradeskillTrainer` tests type 2 (`0x4d8ea0`), `IsTalentTrainer` type 1 (`0x4d8ed0`).
#[test]
fn tradeskill_and_talent_flags() {
    let mut s = UiScript::new().unwrap();
    assert!(s
        .eval::<bool>("return IsTradeskillTrainer() == nil")
        .unwrap());

    s.set_trainer(Some(trainer()));
    assert!(s
        .eval::<bool>("return IsTradeskillTrainer() == nil and IsTalentTrainer() == nil")
        .unwrap());

    // One "Recipes" group, so its service is row 2.
    let mut recipe = svc(
        2743,
        "Copper Chain Pants",
        TrainerServiceCategory::Unavailable,
        2,
        "Recipes",
    );
    recipe.is_trade_skill = true;
    s.set_trainer(Some(TrainerState {
        greeting: String::new(),
        trainer_type: 2,
        groups: Vec::new(),
        services: vec![recipe],
    }));
    assert!(s
        .eval::<bool>("return IsTradeskillTrainer() == 1 and IsTalentTrainer() == nil")
        .unwrap());
    assert!(s
        .eval::<bool>("return IsTrainerServiceTradeSkill(2) == 1")
        .unwrap());

    s.set_trainer(Some(TrainerState {
        greeting: String::new(),
        trainer_type: 1,
        groups: Vec::new(),
        services: vec![svc(
            33,
            "Riding",
            TrainerServiceCategory::Available,
            762,
            "Riding",
        )],
    }));
    assert!(s
        .eval::<bool>("return IsTalentTrainer() == 1 and IsTradeskillTrainer() == nil")
        .unwrap());
}

#[test]
fn unresolved_skill_line_is_dropped() {
    let mut s = every_state_shown();
    let mut t = trainer();
    t.services.push(svc(
        999,
        "Orphan Spell",
        TrainerServiceCategory::Available,
        0,
        "",
    ));
    s.set_trainer(Some(t));
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 5);
}

/// The selection is a record id (12340 `0xc0e49c`), scanned for across the whole array
/// (`0x594170`), so a hidden selection reads past the visible count.
#[test]
fn the_selection_follows_its_service_and_lands_in_the_tail_when_it_is_hidden() {
    let mut s = every_state_shown();
    s.set_trainer(Some(trainer()));
    let sel = |s: &mut UiScript| {
        s.eval::<Option<i64>>("return GetTrainerSelectionIndex()")
            .unwrap()
            .unwrap_or(0)
    };
    let shown = |s: &mut UiScript| s.eval::<i64>("return GetNumTrainerServices()").unwrap();
    let at = |s: &mut UiScript, i: i64| {
        s.eval::<String>(&format!("return (GetTrainerServiceInfo({i})) or ''"))
            .unwrap()
    };

    s.run("SelectTrainerService(5)").unwrap();
    assert_eq!(sel(&mut s), 5);
    assert_eq!(at(&mut s, 5), "Bloodrage");

    // Hiding Cleave moves Bloodrage up to row 4.
    s.run("SetTrainerServiceTypeFilter('used', 0)").unwrap();
    assert_eq!(at(&mut s, 4), "Bloodrage");
    assert_eq!(
        sel(&mut s),
        4,
        "it followed its service, it did not stay put"
    );

    // Row 3 is now the Fury header.
    s.run("CollapseTrainerSkillLine(3)").unwrap();
    let folded = sel(&mut s);
    assert!(
        folded > shown(&mut s),
        "a folded-away selection reads past the visible count, not as row {folded} of {}",
        shown(&mut s)
    );
    s.run("ExpandTrainerSkillLine(3)").unwrap();
    assert_eq!(sel(&mut s), 4, "and back on screen when the group unfolds");

    // Learned, it comes back gray and the filter hides it.
    let mut learned = trainer();
    learned.services[2].category = TrainerServiceCategory::Used;
    s.set_trainer(Some(learned));
    assert!(
        sel(&mut s) > shown(&mut s),
        "the learned spell is off screen, not at some live row"
    );

    let mut shorter = trainer();
    shorter.services.remove(2);
    s.set_trainer(Some(shorter));
    assert_eq!(sel(&mut s), 0);

    s.set_trainer(Some(trainer()));
    s.run("SelectTrainerService(1)").unwrap();
    assert_eq!(sel(&mut s), 0, "row 1 is the Arms header");
}

#[test]
fn clearing_empties_and_resets_selection() {
    let mut s = UiScript::new().unwrap();
    s.set_trainer(Some(trainer()));
    s.run("SelectTrainerService(2)").unwrap();
    s.set_trainer(None);
    assert_eq!(s.eval::<i64>("return GetNumTrainerServices()").unwrap(), 0);
    assert!(s
        .eval::<bool>("return GetTrainerSelectionIndex() == nil")
        .unwrap());
}

/// Creature 957's 19 vmangos `npc_trainer` rows, in the order the reference's builder `0x4d7560`
/// and finalizer `0x4d8410` give them at type 2: the `SKILL_STEP` learn row first under its own
/// header, `reqLevel` inert, the recipes by `reqSkillValue` then name.
#[test]
fn tradeskill_trainer_matches_the_emulated_reference_order() {
    // (wire spell, name, reqSkillValue), reversed below so the order must come from the sort.
    let recipes: &[(u32, &str, u32)] = &[
        (2743, "Copper Chain Pants", 1),
        (2754, "Copper Mace", 15),
        (2755, "Copper Axe", 20),
        (3340, "Copper Chain Boots", 20),
        (2756, "Copper Shortsword", 25),
        (3341, "Rough Grinding Stone", 25),
        (9984, "Copper Claymore", 30),
        (8881, "Copper Dagger", 30),
        (2744, "Copper Battle Axe", 35),
        (3299, "Copper Chain Belt", 35),
        (3342, "Runed Copper Gauntlets", 40),
        (3343, "Runed Copper Pants", 45),
        (2746, "Coarse Sharpening Stone", 65),
        (7409, "Coarse Weightstone", 65),
        (3118, "Heavy Copper Maul", 65),
        (3300, "Runed Copper Belt", 70),
        (2747, "Thick War Axe", 70),
        (3344, "Coarse Grinding Stone", 75),
    ];
    let mut services: Vec<TrainerService> = recipes
        .iter()
        .map(|&(id, name, req)| {
            // Group key 2, `TRADESKILL_SERVICE_LEARN`: the wire spell has no `SKILL_STEP` effect.
            let mut s = svc(id, name, TrainerServiceCategory::Unavailable, 2, "Recipes");
            s.subtext = None;
            s.level_req = 0;
            s.skill_req = Some(TrainerSkillReq {
                name: "Blacksmithing".into(),
                rank: req,
                met: false,
            });
            s
        })
        .collect();
    services.reverse();
    // Group key 1, `TRADESKILL_SERVICE_STEP`: spell 2020 has effect 44, `SKILL_STEP`.
    let mut learn = svc(
        2020,
        "Apprentice Blacksmith",
        TrainerServiceCategory::Available,
        1,
        "Development Skills",
    );
    learn.subtext = None;
    learn.level_req = 5; // inert at type 2
    learn.prof_first_rank = true;
    services.insert(services.len() / 2, learn);

    let mut s = UiScript::new().unwrap();
    s.set_trainer(Some(TrainerState {
        greeting: "Care to learn how to turn the ore that you find into weapons?".into(),
        trainer_type: 2,
        groups: Vec::new(),
        services,
    }));

    let rows = visible(&mut s);
    let expected: Vec<(&str, &str)> = vec![
        ("Development Skills", "header"),
        ("Apprentice Blacksmith", "available"),
        ("Recipes", "header"),
        ("Copper Chain Pants", "unavailable"),
        ("Copper Mace", "unavailable"),
        ("Copper Axe", "unavailable"),
        ("Copper Chain Boots", "unavailable"),
        ("Copper Shortsword", "unavailable"),
        ("Rough Grinding Stone", "unavailable"),
        ("Copper Claymore", "unavailable"),
        ("Copper Dagger", "unavailable"),
        ("Copper Battle Axe", "unavailable"),
        ("Copper Chain Belt", "unavailable"),
        ("Runed Copper Gauntlets", "unavailable"),
        ("Runed Copper Pants", "unavailable"),
        ("Coarse Sharpening Stone", "unavailable"),
        ("Coarse Weightstone", "unavailable"),
        ("Heavy Copper Maul", "unavailable"),
        ("Runed Copper Belt", "unavailable"),
        ("Thick War Axe", "unavailable"),
        ("Coarse Grinding Stone", "unavailable"),
    ];
    let got: Vec<(&str, &str)> = rows.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    assert_eq!(got, expected);
}

/// 12340's group order (`0x5941b0`): a group whose every service costs a profession point sorts
/// after the rest, which go by name; one service without the cost clears the group's flag.
#[test]
fn a_group_of_profession_first_ranks_sorts_after_the_rest() {
    let first_rank = |id: u32, name: &str, key: u32, group: &str| TrainerService {
        prof_first_rank: true,
        ..svc(id, name, TrainerServiceCategory::Available, key, group)
    };
    let services = vec![
        first_rank(2259, "Alchemy", 171, "Alchemy"),
        TrainerService {
            description: String::new(),
            ..svc(3, "Zap", TrainerServiceCategory::Available, 900, "Zeta")
        },
        first_rank(2018, "Blacksmithing", 164, "Blacksmithing"),
        svc(
            4,
            "Mix",
            TrainerServiceCategory::Available,
            164,
            "Blacksmithing",
        ),
    ];
    let mut s = every_state_shown();
    s.set_trainer(Some(TrainerState {
        greeting: String::new(),
        trainer_type: 0,
        groups: Vec::new(),
        services,
    }));
    let got: Vec<String> = visible(&mut s).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        got,
        [
            "Blacksmithing",
            "Blacksmithing",
            "Mix",
            "Zeta",
            "Zap",
            "Alchemy",
            "Alchemy"
        ]
    );
    // `005959d0` answers nil for a service with no description, never "".
    assert!(s
        .eval::<bool>("return GetTrainerServiceDescription(5) == nil")
        .unwrap());
}

/// 12340's builder has no known-services group and one row order (`0x5947d0`) for every type
/// but tradeskill: a mount trainer sorts as a class trainer does.
#[test]
fn a_mount_trainer_orders_rows_as_a_class_trainer() {
    let one = |id: u32, name: &str, cat: TrainerServiceCategory| svc(id, name, cat, 762, "Riding");
    let services = vec![
        one(3, "Cee", TrainerServiceCategory::Available),
        one(1, "Aye", TrainerServiceCategory::Used),
        one(2, "Bee", TrainerServiceCategory::Unavailable),
    ];
    for trainer_type in [1, 0] {
        let mut s = every_state_shown();
        s.set_trainer(Some(TrainerState {
            greeting: String::new(),
            trainer_type,
            groups: Vec::new(),
            services: services.clone(),
        }));
        let got: Vec<String> = visible(&mut s).into_iter().map(|(n, _)| n).collect();
        assert_eq!(got, ["Riding", "Aye", "Bee", "Cee"], "type {trainer_type}");
    }
}

/// `0x76ec80`'s collation: case and accents fold (`Élan` sits with `Elan`), ligatures expand
/// (`Æther` = `AEther`), and `_` sorts after the letters, as the upper-casing puts it.
#[test]
fn names_collate_as_the_clients_folding_compare() {
    use std::cmp::Ordering::*;
    assert_eq!(
        super::collate("élan", "Elan"),
        Greater,
        "equal fold; the raw tie"
    );
    assert_eq!(super::collate("Élan", "Elk"), Less);
    assert_eq!(super::collate("Æther", "AEtherz"), Less);
    assert_eq!(super::collate("Æther", "Afar"), Less);
    assert_eq!(super::collate("_x", "Zed"), Greater);
    assert_eq!(super::collate("apple", "Banana"), Less);
    assert_eq!(
        super::collate("Straße", "Strasse"),
        Greater,
        "ß = SS; the raw tie"
    );
}

/// 12340's shapes: a service row's fourth return is 1 (`0x59512d`); the fold verbs raise on a row
/// that is no header (`0xa14340`) and take `<= 0` as all; the skill line is the spell's own.
#[test]
fn service_rows_folds_and_the_skill_line_follow_12340() {
    let mut s = UiScript::new().unwrap();
    let mut t = trainer();
    t.services[0].skill_line = Some("Alchemy".into());
    s.set_trainer(Some(t));
    assert_eq!(
        s.eval::<Option<i64>>("return select(4, GetTrainerServiceInfo(2))")
            .unwrap(),
        Some(1)
    );
    let line = |s: &UiScript, i: u32| -> Option<String> {
        s.eval(&format!("return GetTrainerServiceSkillLine({i})"))
            .unwrap()
    };
    assert_eq!(
        line(&s, 2).as_deref(),
        Some("Alchemy"),
        "the spell's line, not the header"
    );
    assert_eq!(line(&s, 3), None, "no line resolved: nil");
    for (verb, text) in [
        (
            "CollapseTrainerSkillLine",
            "Bad skill line in CollapseTrainerSkillLine",
        ),
        (
            "ExpandTrainerSkillLine",
            "Bad skill line in ExpandTrainerSkillLine",
        ),
    ] {
        let err = s.run(&format!("{verb}(2)")).unwrap_err().to_string();
        assert!(err.contains(text), "{err}");
    }
    s.run("CollapseTrainerSkillLine(0) ExpandTrainerSkillLine(-1)")
        .unwrap();
    let err = s
        .run("GetTrainerServiceSkillLine()")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Usage: GetTrainerServiceSkillLine(index)"),
        "{err}"
    );
}
