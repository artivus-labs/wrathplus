use super::*;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;

/// A 1 yd cube, the skinned twin's bind-pose bound.
fn tiny_mesh() -> Mesh {
    Mesh::from(bevy::math::primitives::Cuboid::new(1.0, 1.0, 1.0))
}

/// The authored all-animation bound: a bird's 67 yd circuit around that 1 yd body.
fn authored() -> Aabb {
    Aabb::from_min_max(Vec3::new(-4.2, 8.8, -30.5), Vec3::new(13.6, 16.0, 36.6))
}

/// Bevy's bounds system over the authored bound, right after a promote-style `Mesh3d` swap.
fn swapped(guard: bool) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins((bevy::MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.add_systems(Update, bevy::camera::visibility::calculate_bounds);
    let stat = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(tiny_mesh());
    let skinned = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(tiny_mesh());
    let e = app.world_mut().spawn((Mesh3d(stat), authored())).id();
    if guard {
        app.world_mut().entity_mut(e).insert(NoAutoAabb);
    }
    app.update();
    app.world_mut().entity_mut(e).get_mut::<Mesh3d>().unwrap().0 = skinned;
    app.update();
    (app, e)
}

fn bound(app: &App, e: Entity) -> Aabb {
    *app.world().entity(e).get::<Aabb>().expect("a bound")
}

#[test]
fn an_unguarded_bound_is_clobbered_by_the_mesh_swap() {
    let (app, e) = swapped(false);
    let b = bound(&app, e);
    assert!(
        b.half_extents.max_element() < 1.0,
        "expected the 1 yd cube's own bound, got half-extents {:?} — if this now keeps the \
         authored box, Bevy changed `calculate_bounds` and `NoAutoAabb` may be unnecessary",
        b.half_extents
    );
}

#[test]
fn a_guarded_bound_survives_the_mesh_swap() {
    let (app, e) = swapped(true);
    let b = bound(&app, e);
    assert!(
        (Vec3::from(b.min()) - Vec3::from(authored().min())).length() < 1e-3
            && (Vec3::from(b.max()) - Vec3::from(authored().max())).length() < 1e-3,
        "the authored all-animation bound must survive the promote, got {:?}..{:?}",
        b.min(),
        b.max()
    );
}
