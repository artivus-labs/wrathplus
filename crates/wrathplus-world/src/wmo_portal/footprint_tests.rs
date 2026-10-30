use super::*;
use wrathplus_assets::footprint_tri_bounds;
use wrathplus_formats::FootprintTris;

/// A model with nothing but footprint groups, all the sample reads.
fn bare_model() -> WmoModel {
    WmoModel {
        wmo_id: 0,
        submeshes: Vec::new(),
        submesh_group: Vec::new(),
        portal_vertices: Vec::new(),
        portal_infos: Vec::new(),
        portal_refs: Vec::new(),
        group_nav: Vec::new(),
        fogs: Vec::new(),
        skybox: None,
        group_collision_tris: Vec::new(),
        group_camera_only_tris: Vec::new(),
        group_collision_bounds: Vec::new(),
        group_collision_grids: Vec::new(),
        collision_bounds: None,
        collision: None,
        collision_camera: None,
        doodads: Vec::new(),
        doodad_sets: Vec::new(),
        lights: Vec::new(),
        group_bounds: Vec::new(),
        group_footprints: Vec::new(),
        material_ground_type: Vec::new(),
        material_diff_color: Vec::new(),
        group_footprint_bounds: Vec::new(),
        group_footprint_grids: Vec::new(),
        group_light_refs: Vec::new(),
        group_liquids: Vec::new(),
        wrath_liquid_states: None,
        doodad_base: Default::default(),
        doodad_owner: Default::default(),
        doodad_groups: Default::default(),
    }
}

/// Two single-face footprint groups: group 0 owns the floor under the probe (z = 0), group 1
/// sits far off in XY with a higher face (z = 5) that would shadow it through a broken cull.
fn two_group_footprints() -> Vec<Option<FootprintTris>> {
    let face = |offset: [f32; 3], material: u8| FootprintTris {
        positions: vec![
            [offset[0], offset[1], offset[2]],
            [offset[0] + 10.0, offset[1], offset[2]],
            [offset[0], offset[1] + 10.0, offset[2]],
        ],
        indices: vec![0, 1, 2],
        mocv: vec![[10, 20, 30]; 3],
        mopy_flags: vec![0],
        mopy_material: vec![material],
    };
    vec![
        Some(face([-5.0, -5.0, 0.0], 1)),
        Some(face([100.0, 100.0, 5.0], 2)),
    ]
}

/// A dungeon-scale footprint group: an `n × n` grid of floor quads at a sawtooth height, so a
/// column's answer depends on which face wins; 64 faces or more, so `ColumnGrid` indexes it.
fn slab_field(n: usize) -> FootprintTris {
    let (mut positions, mut indices, mut mocv, mut mopy_flags) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut mopy_material = Vec::new();
    for iy in 0..n {
        for ix in 0..n {
            let (x, y) = (ix as f32 * 4.0, iy as f32 * 4.0);
            // Sawtooth z: a mis-picked face changes the verdict.
            let z = ((ix + iy) % 3) as f32 * 0.5;
            let base = positions.len() as u16;
            positions.extend_from_slice(&[
                [x, y, z],
                [x + 4.0, y, z],
                [x + 4.0, y + 4.0, z],
                [x, y + 4.0, z],
            ]);
            let shade = ((ix * 7 + iy * 13) % 200) as u8;
            mocv.extend_from_slice(&[[shade, shade / 2, shade / 3]; 4]);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            mopy_flags.extend_from_slice(&[0, 0]);
            mopy_material.extend_from_slice(&[0, 0]);
        }
    }
    FootprintTris {
        positions,
        indices,
        mocv,
        mopy_flags,
        mopy_material,
    }
}

/// The material ray reads the hit face's material inside the group the arbitration chose; a
/// group with no face under the column is silent, the client's `-1`.
#[test]
fn material_ray_resolves_per_group_and_bounds_checks() {
    let mut model = bare_model();
    model.group_footprints = two_group_footprints();
    model.group_footprint_bounds = footprint_tri_bounds(&model.group_footprints);
    // A root MOMT: material 0 unauthored ("None"), 1 = Wood (TerrainType 4), 2 = Snow (3).
    model.material_ground_type = vec![10, 4, 3];
    assert_eq!(surface_terrain_sample(&model, 0, [0.0, 0.0, 2.0]), Some(4));
    // Group 1's only face is 100 yd away: silent, never group 0's face.
    assert_eq!(surface_terrain_sample(&model, 1, [0.0, 0.0, 2.0]), None);
    assert_eq!(
        surface_terrain_sample(&model, 1, [103.0, 103.0, 7.0]),
        Some(3)
    );
    // A material id past the root's MOMT is silent.
    model.material_ground_type = vec![10];
    assert_eq!(surface_terrain_sample(&model, 0, [0.0, 0.0, 2.0]), None);
}

/// The column index never changes a verdict, the sawtooth's exact-z ties included.
#[test]
fn footprint_column_index_is_exact() {
    let mut model = bare_model();
    model.group_footprints = vec![Some(slab_field(12))];
    model.group_footprint_bounds = footprint_tri_bounds(&model.group_footprints);
    let grids = wrathplus_assets::footprint_tri_grids(&model.group_footprints);
    assert!(grids[0].is_some(), "the field must actually be indexed");
    let mut probed_hits = 0;
    for gx in -2..=50 {
        for gy in -2..=50 {
            let probe = [gx as f32 * 1.1, gy as f32 * 0.9, 3.0];
            model.group_footprint_grids = Vec::new();
            let linear = footprint_sample(&model, probe);
            model.group_footprint_grids = grids.clone();
            let indexed = footprint_sample(&model, probe);
            assert_eq!(
                linear, indexed,
                "column {probe:?}: the index changed the footprint verdict"
            );
            probed_hits += usize::from(linear.is_some());
        }
    }
    assert!(
        probed_hits > 500,
        "the sweep must land on the field, got {probed_hits} hits"
    );
}

/// The broad phase is a pure skip: the verdict with face bounds equals the verdict with none.
#[test]
fn footprint_bounds_cull_is_exact() {
    let mut model = bare_model();
    model.group_footprints = two_group_footprints();
    // Over group 0's face; the unculled baseline first.
    let probe = [-1.0, -1.0, 2.0];
    let unculled = footprint_sample(&model, probe);
    assert_eq!(unculled, Some((0, [10, 20, 30], false)));
    model.group_footprint_bounds = footprint_tri_bounds(&model.group_footprints);
    assert_eq!(footprint_sample(&model, probe), unculled);
    // Between the groups: both agree on a miss.
    model.group_footprint_bounds = Vec::new();
    assert_eq!(footprint_sample(&model, [50.0, 50.0, 2.0]), None);
    model.group_footprint_bounds = footprint_tri_bounds(&model.group_footprints);
    assert_eq!(footprint_sample(&model, [50.0, 50.0, 2.0]), None);
    // Below every face: the min-z leg and the scan agree on a miss.
    model.group_footprint_bounds = Vec::new();
    assert_eq!(footprint_sample(&model, [-1.0, -1.0, -1.0]), None);
    model.group_footprint_bounds = footprint_tri_bounds(&model.group_footprints);
    assert_eq!(footprint_sample(&model, [-1.0, -1.0, -1.0]), None);
}

/// The containment attach's upward retry (`0x6a908d`): after a downward miss the nearest face
/// above answers, with the group, MOCV and MOPY it gives from below.
#[test]
fn the_upward_retry_finds_the_floor_a_sunk_object_sits_under() {
    let mut model = bare_model();
    model.group_footprints = two_group_footprints();
    model.group_footprint_bounds = footprint_tri_bounds(&model.group_footprints);

    // 2 cm under group 0's face at z = 0, as a portcullis spawned below its own slab sits.
    let sunk = [-1.0, -1.0, -0.02];
    assert_eq!(
        footprint_sample(&model, sunk),
        None,
        "the downward leg has nothing below it — this is the whole defect"
    );
    assert_eq!(
        footprint_sample_above(&model, sunk),
        Some((0, [10, 20, 30], false)),
        "the retry finds the slab it is sunk into, and reads the same bake"
    );
    // From above the legs swap roles: the retry must not become a second downward scan.
    let over = [-1.0, -1.0, 2.0];
    assert_eq!(
        footprint_sample(&model, over),
        Some((0, [10, 20, 30], false))
    );
    assert_eq!(
        footprint_sample_above(&model, over),
        None,
        "nothing above the probe, so the retry declines rather than re-finding the floor"
    );
    // The upward leg honours the broad phase: group 1's face (z = 5) is off this column in XY.
    assert_eq!(footprint_sample_above(&model, [-1.0, -1.0, 1.0]), None);
}

/// The derived bounds cover only vertices a face references: an orphan must not inflate them.
#[test]
fn footprint_bounds_ignore_unreferenced_vertices() {
    let mut fps = two_group_footprints();
    if let Some(fp) = fps[0].as_mut() {
        fp.positions.push([1000.0, 1000.0, 1000.0]); // orphan
    }
    let bounds = footprint_tri_bounds(&fps);
    let (min, max) = bounds[0].expect("group 0 has faces");
    assert!(max[0] <= 5.0 + 1e-6 && max[1] <= 5.0 + 1e-6);
    assert_eq!(min[2], 0.0);
}
