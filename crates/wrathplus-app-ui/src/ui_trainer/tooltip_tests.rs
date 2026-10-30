//! Direct Wrath trainer recipes through the real trainer, spell and item feeds and tooltip.
use super::*;
use crate::app_core::state::Spells;
use wrathplus_formats::{SpellDisplay, SPELL_ATTR_IS_TRADESKILL};

fn text(world: &World) -> String {
    world
        .non_send_resource::<UiScript>()
        .eval(
            "TT:SetOwner(B, 'ANCHOR_RIGHT'); TT:SetTrainerService(2); local t = {} \
         for i=1,TT:NumLines() do t[i]=getglobal('TTTextLeft'..i):GetText() or '' end \
         return table.concat(t, ' | ')",
        )
        .unwrap()
}

#[test]
fn trainer_recipe_product_uses_item_feed_after_late_metadata() {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx))
        .init_resource::<Items>()
        .init_resource::<PlayerActions>()
        .init_resource::<TrainerOpen>()
        .init_resource::<TrainerErrors>()
        .init_resource::<TrainerTooltipSubjects>()
        .init_resource::<NameCache>()
        .init_resource::<crate::net::GuidIndex>()
        .init_resource::<crate::spell::SpellModifiers>()
        .init_resource::<crate::ui_items::heirloom::ItemScalings>()
        .init_resource::<crate::net::wrath_inventory_capture::SocketTemplates>()
        .add_message::<FieldChanged>()
        .add_systems(
            Update,
            (
                feed_trainer,
                crate::ui_tooltip::spell_feed::feed_spell_tooltips,
                crate::ui_items::feed::feed_item_stats,
            )
                .chain(),
        );
    let recipe = SpellDisplay {
        name: "Fixture belt recipe".into(),
        attributes: SPELL_ATTR_IS_TRADESKILL,
        effects: [24, 0, 0],
        effect_item_type: [2857, 0, 0],
        reagents: [
            (2840, 10),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
        ],
        ..Default::default()
    };
    app.insert_resource(Spells {
        catalog: SpellCatalog::from_displays([(2666, recipe)].into()),
        ..Spells::empty_for_tests()
    })
    .insert_resource(SkillLines {
        catalog: SkillLineCatalog::from_spell_lines([]),
    });
    let script = UiScript::new().unwrap();
    script
        .run(
            "TRADESKILL_SERVICE_LEARN='Recipes'; ARMOR_TEMPLATE='%d Armor'; \
        ITEM_MOD_STRENGTH='%c%d Strength'; \
        B=CreateFrame('Button','B'); B:SetPoint('CENTER',0,0); \
        TT=CreateFrame('GameTooltip','TT')",
        )
        .unwrap();
    app.world_mut().insert_non_send_resource(script);
    let wire = TrainerSpell {
        spell: 2666,
        state: 0,
        cost: 200,
        req_level: 13,
        req_skill: 164,
        req_skill_value: 70,
        can_learn_primary_prof: false,
        is_primary_prof_first_rank: false,
        wrath_point_cost: None,
        req_spells: [0; 3],
    };
    app.world_mut()
        .resource_mut::<TrainerOpen>()
        .open(4258, 2, vec![wire], String::new());
    app.world_mut()
        .resource_mut::<Items>()
        .insert_template(2840, Some(crate::items::test_template("Fixture copper")));
    app.update();
    let cold = text(app.world());
    assert!(cold.contains("Reagents: Fixture copper (10)"), "{cold}");
    assert!(!cold.contains("Armor"), "no fabricated stats: {cold}");
    app.update();
    assert_eq!(
        text(app.world()),
        cold,
        "still missing metadata: stable spell head"
    );
    let queries: Vec<_> = rx
        .try_iter()
        .filter_map(|c| match c {
            ClientCommand::ItemQuery { entry: 2857, guid } => Some(guid),
            _ => None,
        })
        .collect();
    assert_eq!(queries, [0], "one template-only product query");
    let mut item = crate::items::test_template("Fixture crafted belt");
    item.armor = 123; // Synthetic values: never assertions about installed Runed Copper Belt stats.
    item.stats = vec![(4, 7)];
    app.world_mut()
        .resource_mut::<Items>()
        .insert_template(2857, Some(item));
    app.update();
    let warm = text(app.world());
    for line in [
        "Reagents: Fixture copper (10)",
        "Fixture crafted belt",
        "123 Armor",
        "+7 Strength",
    ] {
        assert!(warm.contains(line), "{line}: {warm}");
    }
    let s = app.world().non_send_resource::<UiScript>();
    assert_eq!(
        s.eval::<u32>("return GetTrainerServiceCost(2)").unwrap(),
        200
    );
    assert_eq!(
        s.eval::<String>("local _,_,c=GetTrainerServiceInfo(2); return c")
            .unwrap(),
        "available"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn direct_recipe_output_resolution_is_bounded() {
    for effect in [24, 59, 157] {
        for (trade, item, want_item) in [(true, 2857, true), (false, 2857, false), (true, 0, false)]
        {
            let d = SpellDisplay {
                attributes: if trade { SPELL_ATTR_IS_TRADESKILL } else { 0 },
                effects: [effect, 0, 0],
                effect_item_type: [item, 777, 888],
                ..Default::default()
            };
            let cat = SpellCatalog::from_displays([(2666, d)].into());
            assert_eq!(
                matches!(
                    service_tooltip(2666, &cat),
                    TrainerTooltip::Recipe { item_id: 2857, .. }
                ),
                want_item
            );
        }
    }
}
