use super::testkit::{batch, batch_of, object, tri};
use super::*;

#[test]
fn a_collected_region_is_undrawn_until_it_publishes() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    assert_eq!(
        gx.undrawn_regions(),
        0,
        "nothing collected, nothing pending"
    );
    assert!(gx.divert(batch(&g, Vec3::ZERO, None, ModelBlend::Opaque)));
    assert_eq!(gx.undrawn_regions(), 1, "diverted, unbaked — a hole");
    // Publishing a region clears `dirty`.
    let cell = *gx.cells.keys().next().unwrap();
    gx.cells.get_mut(&cell).unwrap().dirty = false;
    assert_eq!(gx.undrawn_regions(), 0, "baked — on screen");
}

#[test]
fn the_divert_declines_the_excluded_families() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let mut b = batch(&g, Vec3::ZERO, None, ModelBlend::Opaque);
    b.env_map = true;
    assert!(!gx.divert(b));
    let mut b = batch(&g, Vec3::ZERO, None, ModelBlend::Opaque);
    b.no_depth_write = true;
    assert!(!gx.divert(b));
    let mut b = batch(&g, Vec3::ZERO, None, ModelBlend::Opaque);
    b.shade = ShadeSel::Rig;
    assert!(!gx.divert(b), "a glue-booth rig material never diverts");
    assert_eq!(gx.declined, [1, 1, 1, 0, 0]);
    let mut b = batch(&g, Vec3::ZERO, None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    assert!(
        gx.divert(b),
        "an ADT map doodad is Matte and still belongs here"
    );
    assert!(gx.divert(batch(&g, Vec3::ZERO, None, ModelBlend::Opaque)));
    assert_eq!(gx.cells.len(), 1);
}

#[test]
fn a_fader_divert_registers_its_placement_seed() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let mk_seed = || GxFadeSeed {
        radius: 0.4,
        local_center: Vec3::ZERO,
        stat_mesh: Handle::default(),
        aabb: None,
        cutout: Handle::default(),
        blend: Handle::default(),
    };
    // One placement, two batches, one seed; a third, never-fade batch stays bare.
    let fence = object(77);
    let mut a = batch_of(
        &fence,
        &g,
        Vec3::new(5.0, 0.0, 5.0),
        None,
        ModelBlend::Opaque,
    );
    a.fade = Some(mk_seed());
    assert!(gx.divert(a));
    let mut b = batch_of(
        &fence,
        &g,
        Vec3::new(5.0, 0.0, 5.0),
        None,
        ModelBlend::AlphaTest,
    );
    b.fade = Some(mk_seed());
    assert!(gx.divert(b));
    assert!(gx.divert(batch(
        &g,
        Vec3::new(6.0, 0.0, 6.0),
        None,
        ModelBlend::Opaque
    )));
    let cell = &gx.cells[&(0, 0)];
    assert_eq!(cell.faders.len(), 1, "one placement, one exile unit");
    let f = &cell.faders[&77];
    assert_eq!(f.batches.len(), 2);
    // Radius 0.4 takes the fade table's 40.4..50.4 band.
    assert!((f.near - 40.4).abs() < 1e-4 && (f.far - 50.4).abs() < 1e-4);
    assert!(matches!(f.state, FaderState::Steady));
    assert_eq!(cell.ring, (f.near, f.far));
    assert_eq!(
        cell.items.iter().filter(|i| i.fader == Some(77)).count(),
        2,
        "both fader items carry the uid; the never-fade item stays bare"
    );
}

/// Props land in their instance's region, not a cell, with slot and Matte bit on the item.
#[test]
fn a_prop_divert_dedups_referrer_sets() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let instance = Entity::PLACEHOLDER;
    let rooms_a: Arc<[u16]> = Arc::from([3u16, 5].as_slice());
    let rooms_b: Arc<[u16]> = Arc::from([9u16].as_slice());
    let mk = |groups: &Arc<[u16]>, slot| GxPropBatch {
        instance,
        groups: Arc::clone(groups),
        slot,
    };
    let mut b = batch(&g, Vec3::ZERO, None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.prop = Some(mk(&rooms_a, Some(11)));
    assert!(gx.divert(b));
    let mut b = batch(&g, Vec3::ONE, None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.prop = Some(mk(&rooms_a, Some(12)));
    assert!(gx.divert(b));
    let mut b = batch(&g, Vec3::ONE, None, ModelBlend::Opaque);
    b.shade = ShadeSel::Shaded;
    b.prop = Some(mk(&rooms_b, None));
    assert!(gx.divert(b));
    assert!(gx.cells.is_empty() && gx.wmos.is_empty());
    let region = &gx.props[&instance];
    assert_eq!(region.sets.len(), 2, "same rooms share a set");
    assert_eq!(region.items.len(), 3);
    let sets: Vec<u16> = region
        .items
        .iter()
        .map(|i| i.prop.as_ref().unwrap().set)
        .collect();
    assert_eq!(sets, vec![0, 0, 1]);
    assert_eq!(region.items[0].prop.as_ref().unwrap().slot, Some(11));
    assert!(region.items[0].matte, "Matte is the prop lane's own bit");
    assert!(!region.items[2].matte, "Shaded stays the 0.5 family");
    gx.clear();
    assert!(gx.props.is_empty() && gx.world.props.is_empty());
}

#[test]
fn a_dead_owner_queues_its_exiles() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let doomed = object(9);
    let mut a = batch_of(&doomed, &g, Vec3::ZERO, None, ModelBlend::Opaque);
    a.owner = (1, 1);
    a.fade = Some(GxFadeSeed {
        radius: 0.4,
        local_center: Vec3::ZERO,
        stat_mesh: Handle::default(),
        aabb: None,
        cutout: Handle::default(),
        blend: Handle::default(),
    });
    assert!(gx.divert(a));
    let ghost = Entity::PLACEHOLDER;
    gx.cells
        .get_mut(&(0, 0))
        .unwrap()
        .faders
        .get_mut(&9)
        .unwrap()
        .state = FaderState::Exiled {
        ents: vec![ghost],
        armed: true,
    };
    gx.release_owner((1, 1));
    let cell = &gx.cells[&(0, 0)];
    assert!(cell.faders.is_empty());
    assert_eq!(gx.pending_despawn, vec![ghost]);
}

#[test]
fn a_dead_owner_leaves_the_cells() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let mut a = batch(&g, Vec3::ZERO, None, ModelBlend::Opaque);
    a.owner = (1, 1);
    assert!(gx.divert(a));
    let mut b = batch(&g, Vec3::ONE, None, ModelBlend::Opaque);
    b.owner = (2, 2);
    assert!(gx.divert(b));
    gx.frame += IDLE_FRAMES; // silence any later-change guard
    gx.release_owner((1, 1));
    let state = &gx.cells[&(0, 0)];
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].owner, (2, 2));
    assert!(state.dirty, "the survivor cell re-bakes");
    gx.clear();
    assert!(gx.cells.is_empty() && gx.world.cells.is_empty());
}
