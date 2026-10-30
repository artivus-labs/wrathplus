/// The production marker caches a decision, but its height input may arrive after the first
/// stamp or change with a display/scale update. No movement or descriptor delta is required.
#[test]
fn height_only_changes_revisit_the_cached_swim_decision() {
    use super::{SwimEvaluated, SwimMarkGate};
    use crate::app_core::state::CollisionHeight;
    use bevy::prelude::*;

    #[derive(Resource, Default)]
    struct Visits(usize);
    fn visit(q: Query<Entity, SwimMarkGate>, mut visits: ResMut<Visits>) {
        visits.0 += q.iter().count();
    }
    let mut app = App::new();
    app.init_resource::<Visits>().add_systems(Update, visit);
    let body = app
        .world_mut()
        .spawn((Transform::default(), SwimEvaluated))
        .id();
    app.update(); // consume the initial Transform change
    app.update();
    let before = app.world().resource::<Visits>().0;
    app.world_mut()
        .entity_mut(body)
        .insert(CollisionHeight(2.07));
    app.update();
    assert_eq!(
        app.world().resource::<Visits>().0,
        before + 1,
        "a late collision-height stamp must invalidate the cached default-height decision"
    );
    app.update();
    assert_eq!(
        app.world().resource::<Visits>().0,
        before + 1,
        "stable inputs stay cached"
    );
    app.world_mut().get_mut::<CollisionHeight>(body).unwrap().0 *= 2.0;
    app.update();
    assert_eq!(
        app.world().resource::<Visits>().0,
        before + 2,
        "height-only restamps must invalidate on both entry and exit"
    );
}
