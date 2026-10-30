use super::super::testkit::{batch, batch_of, object, tri};
use super::super::GxWmoBatch;
use super::*;
use crate::model_render::ShadeSel;
use std::sync::Arc;
use wrathplus_formats::{ModelBlend, RenderSubmesh, WmoBatchClass};

#[test]
fn the_decline_census_prints_once_a_burst_settles() {
    let mut gx = StaticGx::default();
    let mut frame = 0u32;
    let tick = |gx: &mut StaticGx, frame: &mut u32| {
        *frame += 1;
        census_declines(gx, *frame);
    };
    for _ in 0..10 {
        gx.declined[3] += 1;
        tick(&mut gx, &mut frame);
    }
    assert_eq!(gx.declined_logged, [0; 5], "a moving count stays quiet");
    for _ in 0..CENSUS_SETTLE_FRAMES {
        tick(&mut gx, &mut frame);
    }
    assert_eq!(gx.declined_logged[3], 10, "the settled total reports");
    let logged = gx.declined_logged;
    for _ in 0..CENSUS_SETTLE_FRAMES * 2 {
        tick(&mut gx, &mut frame);
    }
    assert_eq!(gx.declined_logged, logged, "…once, not every frame after");
    for _ in 0..CENSUS_MAX_HOLD_FRAMES + 1 {
        gx.declined[3] += 1;
        tick(&mut gx, &mut frame);
    }
    assert!(
        gx.declined_logged[3] > 10,
        "a count that never settles still reports"
    );
}

#[test]
fn a_map_change_resets_the_census() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    assert!(gx.divert(batch(&g, Vec3::ZERO, None, ModelBlend::Opaque)));
    gx.tally_prop_declined(false);
    assert_eq!((gx.accepted, gx.declined[3]), (1, 1));
    gx.clear();
    assert_eq!((gx.accepted, gx.declined[3]), (0, 0), "a fresh world");
}

#[test]
fn the_bake_waits_short_first_and_long_after() {
    let mut state = super::super::GxCell::default();
    assert!(
        !bake_due(&state, false, 100, false),
        "clean cells are never due"
    );
    state.dirty = true;
    state.dirty_since = 0;
    state.last_change = 0;
    assert!(!bake_due(&state, false, IDLE_FRAMES - 1, false));
    assert!(
        bake_due(&state, false, IDLE_FRAMES, false),
        "first bake: short window"
    );
    assert!(
        !bake_due(&state, true, IDLE_FRAMES, false),
        "published: short is not enough"
    );
    assert!(!bake_due(&state, true, REBAKE_FRAMES - 1, false));
    assert!(
        bake_due(&state, true, REBAKE_FRAMES, false),
        "published: long window"
    );
    state.last_change = IDLE_FRAMES;
    assert!(
        !bake_due(&state, false, IDLE_FRAMES, false),
        "still quiet-timing"
    );
    assert!(
        bake_due(&state, false, IDLE_FRAMES, true),
        "flush_now: due now"
    );
    assert!(bake_due(&state, true, IDLE_FRAMES, true), "…published too");
    state.dirty = false;
    assert!(
        !bake_due(&state, false, IDLE_FRAMES, true),
        "flush_now still bakes nothing that is not dirty"
    );
    state.dirty = true;
    state.last_change = 0;
    // Changed this very frame, never quiet: the age cap still bakes it.
    state.last_change = MAX_DIRTY_FRAMES;
    assert!(bake_due(&state, true, MAX_DIRTY_FRAMES, false));
}

#[test]
fn a_quiet_cell_bakes_sorted_contiguous_draws() {
    let mut gx = StaticGx::default();
    let g = tri([10.0, 0.0, 10.0]);
    // A cutout item pushed first must sort after the two opaque items.
    let mut cut = batch(&g, Vec3::new(5.0, 0.0, 5.0), None, ModelBlend::AlphaTest);
    cut.unlit = true;
    assert!(gx.divert(cut));
    assert!(gx.divert(batch(
        &g,
        Vec3::new(6.0, 0.0, 6.0),
        None,
        ModelBlend::Opaque
    )));
    assert!(gx.divert(batch(
        &g,
        Vec3::new(7.0, 0.0, 7.0),
        None,
        ModelBlend::Opaque
    )));
    let mut meshes = Assets::<Mesh>::default();
    let state = gx.cells.get_mut(&(0, 0)).unwrap();
    state
        .items
        .sort_by_key(|i| ((u8::from(i.cutout) << 1) | u8::from(i.two_sided), i.texture));
    let baked = bake_cell(&state.items, &mut meshes);
    assert_eq!(baked.draws.len(), 3);
    assert!(!baked.draws[0].cutout && !baked.draws[1].cutout);
    assert!(baked.draws[2].cutout, "the cutout item sorted last");
    assert_eq!(baked.draws[0].index_range, 0..3);
    assert_eq!(baked.draws[1].index_range, 3..6);
    assert_eq!(baked.draws[2].index_range, 6..9);
    let mesh = meshes.get(&baked.mesh).unwrap();
    let Some(bevy::mesh::VertexAttributeValues::Uint32(words)) = mesh.attribute(ATTRIBUTE_GX_WORD)
    else {
        panic!("gx word attribute missing")
    };
    // The low bits carry the bake-order index; the cutout item (index 2) carries UNLIT.
    assert_eq!(words[0] & 0xffff, 0);
    assert_eq!(words[3] & 0xffff, 1);
    assert_eq!(words[6] & 0xffff, 2);
    assert_ne!(words[6] & WORD_UNLIT, 0);
    assert_eq!(words[0] & WORD_UNLIT, 0);
    assert!(Vec3::from(baked.aabb.center).length() < 1e-4);
    assert!(baked.origin.length() > 1.0);
}

#[test]
fn the_remap_names_each_faders_post_sort_items() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let seed = || super::super::GxFadeSeed {
        radius: 0.4,
        local_center: Vec3::ZERO,
        stat_mesh: Handle::default(),
        aabb: None,
        cutout: Handle::default(),
        blend: Handle::default(),
    };
    let (p1, p2) = (object(1), object(2));
    // Placement 1: a cutout batch (sorts last) and an opaque batch (sorts first)…
    let mut b = batch_of(
        &p1,
        &g,
        Vec3::new(1.0, 0.0, 1.0),
        None,
        ModelBlend::AlphaTest,
    );
    b.fade = Some(seed());
    assert!(gx.divert(b));
    let mut b = batch_of(&p1, &g, Vec3::new(1.0, 0.0, 1.0), None, ModelBlend::Opaque);
    b.fade = Some(seed());
    assert!(gx.divert(b));
    // …a never-fade opaque batch between them, and placement 2's opaque batch.
    assert!(gx.divert(batch(
        &g,
        Vec3::new(2.0, 0.0, 2.0),
        None,
        ModelBlend::Opaque
    )));
    let mut b = batch_of(&p2, &g, Vec3::new(3.0, 0.0, 3.0), None, ModelBlend::Opaque);
    b.fade = Some(seed());
    assert!(gx.divert(b));
    let state = gx.cells.get_mut(&(0, 0)).unwrap();
    state
        .items
        .sort_by_key(|i| ((u8::from(i.cutout) << 1) | u8::from(i.two_sided), i.texture));
    remap_fader_items(state);
    assert!(state.bits_stale, "the same-frame scan rebuilds the bitmap");
    let f1 = &state.faders[&1].items;
    let f2 = &state.faders[&2].items;
    assert_eq!(f1.len(), 2);
    assert!(f1.contains(&3), "the cutout batch sorted to the tail");
    assert_eq!(f2.len(), 1);
    assert!(!f2.iter().any(|i| f1.contains(i)), "no shared kill targets");
    let claimed: usize = f1.len() + f2.len();
    assert_eq!(state.items.len() - claimed, 1);
}

#[test]
fn a_wmo_batch_diverts_by_instance_and_bakes_group_ranges() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let instance = Entity::PLACEHOLDER;
    let wmo = |group: u16, class: Option<WmoBatchClass>, interior: bool| GxWmoBatch {
        instance,
        group,
        interior,
        class,
        sidn: Some([10, 20, 30]),
        wmo_unified: match (interior, class) {
            (true, Some(WmoBatchClass::Int)) => Some([0; 3]),
            (true, Some(WmoBatchClass::Trans)) => Some([17, 91, 203]),
            _ => None,
        },
        window: true,
        batch_order: group + 1,
    };
    // An INT batch of group 2, pushed first…
    let mut b = batch(&g, Vec3::new(1.0, 0.0, 1.0), None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.wmo = Some(wmo(2, Some(WmoBatchClass::Int), true));
    assert!(gx.divert(b));
    // …a TRANS batch of group 1, same bucket and texture, which the sort must bring first…
    let mut b = batch(&g, Vec3::new(2.0, 0.0, 2.0), None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.wmo = Some(wmo(1, Some(WmoBatchClass::Trans), true));
    assert!(gx.divert(b));
    // …and an exterior-law batch of group 2, which sorts beside the first.
    let mut b = batch(&g, Vec3::new(3.0, 0.0, 3.0), None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.wmo = Some(wmo(2, None, false));
    assert!(gx.divert(b));
    assert!(gx.cells.is_empty(), "WMO items never land in cells");
    let state = gx.wmos.get_mut(&instance).expect("the instance's region");
    state.items.sort_by_key(|i| {
        (
            (u8::from(i.cutout) << 1) | u8::from(i.two_sided),
            i.texture,
            i.wmo.as_ref().map_or(0, |w| w.group),
        )
    });
    let mut meshes = Assets::<Mesh>::default();
    let baked = bake_cell(&state.items, &mut meshes);
    assert_eq!(
        baked.draws.iter().map(|d| d.group).collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(2)]
    );
    assert_eq!(baked.draws[0].index_range, 0..3);
    assert_eq!(baked.draws[2].index_range, 6..9);
    let mut groups: Vec<u16> = baked.groups.iter().map(|(g, _)| *g).collect();
    groups.sort_unstable();
    assert_eq!(groups, vec![1, 2]);
    assert_eq!(baked.draws[0].order, 2); // group 1's batch_order = group + 1
    assert_eq!(baked.draws[0].sidn, [10, 20, 30]);
    let mesh = meshes.get(&baked.mesh).unwrap();
    let Some(bevy::mesh::VertexAttributeValues::Uint32(words)) = mesh.attribute(ATTRIBUTE_GX_WORD)
    else {
        panic!("gx word attribute missing")
    };
    let w_trans = words[baked.draws[0].vertex_range.start as usize];
    let w_int = words[baked.draws[1].vertex_range.start as usize];
    let w_ext = words[baked.draws[2].vertex_range.start as usize];
    for w in [w_trans, w_int, w_ext] {
        assert_ne!(w & WORD_WMO, 0);
        assert_ne!(w & WORD_WINDOW, 0);
        assert_eq!(w & WORD_HAS_VC, 0, "the fixture authors no colours");
    }
    assert_eq!(baked.draws[0].wmo_unified, Some([17, 91, 203]));
    assert_eq!(baked.draws[1].wmo_unified, Some([0; 3]));
    assert_eq!(baked.draws[2].wmo_unified, None);
    assert_ne!(w_trans & WORD_UNIFIED, 0);
    assert_ne!(w_int & WORD_UNIFIED, 0, "black ambient is still unified");
    assert_eq!(w_ext & WORD_UNIFIED, 0);
    assert_ne!(w_trans & WORD_CLASS_TRANS, 0);
    assert_ne!(w_int & WORD_CLASS_INT, 0);
    assert_eq!(w_ext & (WORD_CLASS_INT | WORD_CLASS_TRANS), 0);
    assert_ne!(w_trans & WORD_INTERIOR, 0);
    assert_eq!(w_ext & WORD_INTERIOR, 0);
    let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
        mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("gx colour attribute missing")
    };
    assert_eq!(colors[0], [1.0, 1.0, 1.0, 1.0]);
    gx.clear();
    assert!(gx.wmos.is_empty());
}

#[test]
fn a_prop_region_bakes_set_ranges_and_slots() {
    let mut gx = StaticGx::default();
    let g = tri([0.0; 3]);
    let instance = Entity::PLACEHOLDER;
    let rooms_a: Arc<[u16]> = Arc::from([3u16].as_slice());
    let rooms_b: Arc<[u16]> = Arc::from([5u16, 8].as_slice());
    let mk = |groups: &Arc<[u16]>, slot| super::super::GxPropBatch {
        instance,
        groups: Arc::clone(groups),
        slot,
    };
    // Set B arrives first, so the dedup gives B index 0 and A index 1.
    let mut b = batch(&g, Vec3::new(1.0, 0.0, 1.0), None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.prop = Some(mk(&rooms_b, Some(42)));
    assert!(gx.divert(b));
    let mut b = batch(&g, Vec3::new(2.0, 0.0, 2.0), None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.prop = Some(mk(&rooms_a, None));
    assert!(gx.divert(b));
    let mut b = batch(&g, Vec3::new(3.0, 0.0, 3.0), None, ModelBlend::Opaque);
    b.shade = ShadeSel::Matte;
    b.prop = Some(mk(&rooms_b, Some(43)));
    assert!(gx.divert(b));
    let state = gx.props.get_mut(&instance).expect("the instance's region");
    state.items.sort_by_key(|i| {
        (
            (u8::from(i.cutout) << 1) | u8::from(i.two_sided),
            i.texture,
            i.prop.as_ref().map_or(0, |p| p.set),
        )
    });
    let mut meshes = Assets::<Mesh>::default();
    let baked = bake_cell(&state.items, &mut meshes);
    assert_eq!(
        baked.draws.iter().map(|d| d.group).collect::<Vec<_>>(),
        vec![Some(0), Some(0), Some(1)]
    );
    assert_eq!(
        baked.draws.iter().map(|d| d.slot).collect::<Vec<_>>(),
        vec![42, 43, 0]
    );
    let mut sets: Vec<u16> = baked.groups.iter().map(|(s, _)| *s).collect();
    sets.sort_unstable();
    assert_eq!(sets, vec![0, 1]);
    // Slotted: INTERIOR without WMO. Slot-less: the exterior law with MATTE.
    let mesh = meshes.get(&baked.mesh).unwrap();
    let Some(bevy::mesh::VertexAttributeValues::Uint32(words)) = mesh.attribute(ATTRIBUTE_GX_WORD)
    else {
        panic!("gx word attribute missing")
    };
    let w_int = words[baked.draws[0].vertex_range.start as usize];
    let w_ext = words[baked.draws[2].vertex_range.start as usize];
    assert_ne!(w_int & WORD_INTERIOR, 0);
    assert_eq!(w_int & WORD_WMO, 0, "a prop item never takes the WMO lane");
    assert_eq!(w_ext & WORD_INTERIOR, 0);
    assert_ne!(w_ext & WORD_MATTE, 0);
}

/// Raw, as `model.rs` inserts `vertex_colors` on the entity mesh.
#[test]
fn authored_vertex_colours_bake_raw_and_set_the_bit() {
    let mut gx = StaticGx::default();
    let g = Arc::new(RenderSubmesh {
        positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uvs: vec![[0.0, 0.0]; 3],
        indices: vec![0, 1, 2],
        vertex_colors: vec![[0.25, 0.5, 0.75, 0.5]; 3],
        ..Default::default()
    });
    assert!(gx.divert(batch(&g, Vec3::ZERO, None, ModelBlend::Opaque)));
    let mut meshes = Assets::<Mesh>::default();
    let baked = bake_cell(&gx.cells[&(0, 0)].items, &mut meshes);
    let mesh = meshes.get(&baked.mesh).unwrap();
    let Some(bevy::mesh::VertexAttributeValues::Uint32(words)) = mesh.attribute(ATTRIBUTE_GX_WORD)
    else {
        panic!("gx word attribute missing")
    };
    assert_ne!(words[0] & WORD_HAS_VC, 0);
    assert_eq!(words[0] & WORD_WMO, 0, "a cell item takes no WMO lane");
    let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
        mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("gx colour attribute missing")
    };
    assert_eq!(colors[1], [0.25, 0.5, 0.75, 0.5]);
}
