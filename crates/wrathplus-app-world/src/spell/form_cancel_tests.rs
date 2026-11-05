//! The predicted form cancel: `0x802f80`'s form arm, `0x71af70`'s read and `0x6da130`'s clear.

use super::*;
use wrathplus_protocol::types::Field;
use wrathplus_protocol::ObjectFields;

const BEAR: u8 = 5;
const BEAR_DISPLAY: u32 = 2281;
const NATIVE_DISPLAY: u32 = 55;

/// Our player in `form`, wearing `display`.
fn me(form: u8, display: u32) -> ObjectStore {
    ObjectStore(ObjectFields::from_pairs(&[
        (Field::UnitBytes2.index(), u32::from(form) << 24),
        (Field::UnitDisplayId.index(), display),
        (Field::UnitNativeDisplayId.index(), NATIVE_DISPLAY),
    ]))
}

/// A spell whose effect 1 applies `MOD_SHAPESHIFT` into `form`.
fn form_spell(form: i32) -> SpellDisplay {
    let mut d = SpellDisplay {
        shapeshift_form: u32::try_from(form).ok(),
        ..Default::default()
    };
    d.effect_apply_aura[1] = AURA_MOD_SHAPESHIFT;
    d.effect_misc_value[1] = form;
    d
}

fn forms() -> HashMap<u32, ShapeshiftForm> {
    HashMap::from([
        (
            u32::from(BEAR),
            ShapeshiftForm {
                creature_display: [BEAR_DISPLAY, 2289, 0, 0],
                ..Default::default()
            },
        ),
        // A warrior stance: `flags1 & 0x2`, not cancelable.
        (
            17,
            ShapeshiftForm {
                flags: 0x7,
                ..Default::default()
            },
        ),
    ])
}

#[test]
fn the_active_forms_cancel_is_predicted_and_reads_no_form() {
    let mut p = FormCancelPrediction::default();
    assert_eq!(p.form_byte(BEAR), BEAR);
    let me = me(BEAR, BEAR_DISPLAY);
    let bear = form_spell(i32::from(BEAR));
    assert_eq!(p.on_cancel(&bear, &me, &forms()), FormCancel::Predicted);
    assert!(p.predicted());
    assert_eq!(p.form_byte(BEAR), 0, "0x71af70 reads no form meanwhile");
    assert_eq!(
        p.display(),
        Some(NATIVE_DISPLAY),
        "the bear model swaps to the native"
    );
    assert!(p.take_announce(), "0x721cf0 fires UPDATE_SHAPESHIFT_FORM");
    assert!(!p.take_announce(), "once");
    // A second press sees no form: the arm does not run again.
    assert_eq!(p.on_cancel(&bear, &me, &forms()), FormCancel::Plain);
    assert!(!p.take_announce());
}

#[test]
fn only_the_current_forms_cancel_predicts() {
    let mut p = FormCancelPrediction::default();
    // Out of any form.
    let bear = form_spell(i32::from(BEAR));
    assert_eq!(p.on_cancel(&bear, &me(0, 1), &forms()), FormCancel::Plain);
    // A spell without MOD_SHAPESHIFT (a paladin aura's cancel).
    let aura = SpellDisplay::default();
    assert_eq!(
        p.on_cancel(&aura, &me(BEAR, BEAR_DISPLAY), &forms()),
        FormCancel::Plain
    );
    // Another cancelable form's spell.
    let mut other = forms();
    other.insert(1, ShapeshiftForm::default());
    let cat = form_spell(1);
    assert_eq!(
        p.on_cancel(&cat, &me(BEAR, BEAR_DISPLAY), &other),
        FormCancel::Plain
    );
    assert!(!p.predicted());
    assert!(!p.take_announce());
}

#[test]
fn a_missing_or_non_cancelable_form_row_sends_nothing() {
    let mut p = FormCancelPrediction::default();
    let stance = form_spell(17);
    assert_eq!(
        p.on_cancel(&stance, &me(17, 1), &forms()),
        FormCancel::Refused
    );
    let unknown = form_spell(99);
    assert_eq!(
        p.on_cancel(&unknown, &me(BEAR, 1), &forms()),
        FormCancel::Refused
    );
    assert!(!p.predicted());
}

#[test]
fn a_display_that_is_not_the_forms_stays() {
    let mut p = FormCancelPrediction::default();
    let bear = form_spell(i32::from(BEAR));
    // An illusion over the form: the display is none of the row's four.
    assert_eq!(
        p.on_cancel(&bear, &me(BEAR, 9999), &forms()),
        FormCancel::Predicted
    );
    assert_eq!(p.display(), None);
    assert_eq!(p.form_byte(BEAR), 0);
}

#[test]
fn the_form_bytes_move_clears_it_and_owes_the_event() {
    let mut p = FormCancelPrediction::default();
    let bear = form_spell(i32::from(BEAR));
    p.on_cancel(&bear, &me(BEAR, BEAR_DISPLAY), &forms());
    p.take_announce();
    p.observe(BEAR, false);
    assert!(p.predicted(), "the byte has not moved");
    assert!(!p.take_announce());
    p.observe(0, false);
    assert!(!p.predicted());
    assert_eq!(p.display(), None, "0x6da130 restores the form display");
    assert_eq!(p.form_byte(0), 0);
    assert!(p.take_announce(), "0x6da130 fires UPDATE_SHAPESHIFT_FORM");
    // Unpredicted, a move or a write owes nothing here: the stance feed's own diff fires.
    p.observe(BEAR, true);
    assert!(!p.take_announce());
}

#[test]
fn an_unchanged_write_of_the_form_word_clears_it() {
    let mut p = FormCancelPrediction::default();
    let bear = form_spell(i32::from(BEAR));
    p.on_cancel(&bear, &me(BEAR, BEAR_DISPLAY), &forms());
    p.take_announce();
    // `0x6da130` is always-fire: the server rewrote BYTES_2 with the same form.
    p.observe(BEAR, true);
    assert!(!p.predicted());
    assert_eq!(p.form_byte(BEAR), BEAR, "the form reads again");
    assert_eq!(p.display(), None);
    assert!(p.take_announce());
}

/// A predicting app with our player streamed in bear form, through the drain's merge.
fn predicting_app() -> (App, Entity) {
    let mut app = App::new();
    app.add_message::<crate::net::FieldChanged>()
        .add_message::<crate::net::FieldWritten>()
        .init_resource::<FormCancelPrediction>()
        .add_systems(Update, observe_form_byte);
    let created = ObjectFields::from_pairs(&[
        (Field::UnitBytes2.index(), u32::from(BEAR) << 24),
        (Field::UnitDisplayId.index(), BEAR_DISPLAY),
        (Field::UnitNativeDisplayId.index(), NATIVE_DISPLAY),
    ])
    .into_created(wrathplus_protocol::types::ObjectType::Player);
    let player = app
        .world_mut()
        .spawn((SelfPlayer, ObjectStore(created)))
        .id();
    let world = app.world_mut();
    let store = world.get::<ObjectStore>(player).unwrap().clone();
    let bear = form_spell(i32::from(BEAR));
    let mut p = world.resource_mut::<FormCancelPrediction>();
    assert_eq!(p.on_cancel(&bear, &store, &forms()), FormCancel::Predicted);
    p.take_announce();
    (app, player)
}

#[test]
fn a_same_value_bytes_2_update_clears_it() {
    let (mut app, player) = predicting_app();
    let same = ObjectFields::from_pairs(&[(Field::UnitBytes2.index(), u32::from(BEAR) << 24)]);
    crate::net::apply_fields_for_test(app.world_mut(), player, same);
    app.update();
    let mut p = app.world_mut().resource_mut::<FormCancelPrediction>();
    assert!(!p.predicted(), "0x6da130 is always-fire");
    assert!(p.take_announce());
}

#[test]
fn an_update_without_bytes_2_keeps_it() {
    let (mut app, player) = predicting_app();
    let stand = ObjectFields::from_pairs(&[
        (Field::UnitBytes1.index(), 1),
        (Field::UnitHealth.index(), 50),
    ]);
    crate::net::apply_fields_for_test(app.world_mut(), player, stand);
    app.update();
    let p = app.world().resource::<FormCancelPrediction>();
    assert!(p.predicted(), "the form word was not in the mask");
    assert_eq!(p.form_byte(BEAR), 0);
}

#[test]
fn a_re_create_of_our_player_clears_it() {
    let (mut app, player) = predicting_app();
    let recreate = ObjectFields::from_pairs(&[(Field::UnitBytes2.index(), u32::from(BEAR) << 24)])
        .into_created(wrathplus_protocol::types::ObjectType::Player);
    crate::net::apply_fields_for_test(app.world_mut(), player, recreate);
    app.update();
    assert!(!app.world().resource::<FormCancelPrediction>().predicted());
}
