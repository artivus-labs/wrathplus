//! Integration tests over the patch chain of a real 1.12.1 install.

use wrathplus_formats::{open_chain, Chain};

#[test]
fn reads_spell_dbc_from_vanilla_chain() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // Spell.dbc lives in patch.MPQ; reading by name must resolve through the chain.
    let bytes = chain
        .read_file("DBFilesClient/Spell.dbc")
        .expect("read DBFilesClient/Spell.dbc");

    assert_eq!(&bytes[..4], b"WDBC", "DBC files start with the WDBC magic");
    assert!(
        bytes.len() > 1_000_000,
        "Spell.dbc should be sizable, got {} bytes",
        bytes.len()
    );
}

#[test]
fn resolves_and_reads_across_archive_types() {
    let data = wrathplus_formats::wow_data_or_skip!();

    // A DBC, a UI BLP, and an M2 from a base archive with no listfile, found by name hash.
    let chain = open_chain(&data).expect("open vanilla patch chain");

    for path in [
        "DBFilesClient/Spell.dbc",
        "DBFilesClient/TaxiNodes.dbc",
        "Interface/Icons/Spell_Holy_ArcaneIntellect.blp",
        // The on-disk name: the `.mdx` -> `.m2` remap belongs to the loaders, not the reader.
        "Creature\\Kobold\\Kobold.m2",
    ] {
        assert!(chain.contains(path), "chain should resolve {path}");
        let bytes = chain
            .read(path)
            .unwrap_or_else(|e| panic!("chain read {path}: {e:#}"));
        assert!(!bytes.is_empty(), "{path} read empty");
    }
}

#[test]
fn culls_constant_zero_alpha_m2_batches() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // The embers' only batch is an 84-yd box with a constant 0.0 transparency weight, an emitter
    // anchor the reference never draws: it culls a batch at alpha <= 0.
    let embers = wrathplus_formats::load_m2_mesh(
        &mut chain,
        "world/kalimdor/orgrimmar/passivedoodads/orgrimmarbonfire/orgrimmarfloatingembers.m2",
    )
    .expect("mesh embers");
    assert!(
        embers.is_empty(),
        "the weight-0 emitter box must be culled, got {} batches",
        embers.len()
    );

    let lamppost = wrathplus_formats::load_m2_mesh(
        &mut chain,
        "world/azeroth/elwynn/passivedoodads/lamppost/lamppost.m2",
    )
    .expect("mesh lamppost");
    // Build 12340 keeps billboards on the joint palette, so the glow stays in its batch: no card
    // splits off, and none of the lamppost's four batches is culled.
    assert_eq!(lamppost.len(), 4, "a normal prop keeps all its batches");
}

#[test]
fn trigger_creature_models_carry_no_render_geometry() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // A trigger creature hides by its model drawing nothing: the stalker and the portal author no
    // vertices, and NoName's one batch has a constant 0.0 transparency track.
    for path in [
        "creature/invisiblestalker/invisiblestalker.m2",
        "creature/invisiblestalker/invisiblestalkernoname.m2",
        "creature/spells/creature_spellportal.m2",
    ] {
        let mesh = wrathplus_formats::load_m2_mesh(&mut chain, path)
            .unwrap_or_else(|e| panic!("mesh {path}: {e:#}"));
        assert!(
            mesh.is_empty(),
            "{path} is a trigger creature's model and must build no render geometry, got {} batches",
            mesh.len()
        );
    }

    let chicken = wrathplus_formats::load_m2_mesh(&mut chain, "creature/chicken/chicken.m2")
        .expect("mesh chicken");
    assert!(
        !chicken.is_empty(),
        "an ordinary creature model keeps its batches"
    );
}

#[test]
fn suppresses_white1_invisible_trap_placeholder() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // GameObject display 1287: a flat opaque quad on `WHITE1.BLP` that the reference draws at
    // per-instance alpha ~0, so the loader drops it.
    let trap = wrathplus_formats::load_m2_mesh(
        &mut chain,
        "world/generic/passivedoodads/traps/spellobject_invisibletrap.m2",
    )
    .expect("mesh trap");
    assert!(
        trap.is_empty(),
        "the WHITE1 invisible-trap placeholder must be suppressed, got {} batches",
        trap.len()
    );

    // A flat decal with a real texture: the fingerprint is opaque on `WHITE1`, not flatness.
    let mat = wrathplus_formats::load_m2_mesh(
        &mut chain,
        "world/azeroth/burningsteppes/passivedoodads/orcsleepmats/orcsleepmat01.m2",
    )
    .expect("mesh orc sleep mat");
    assert!(
        !mat.is_empty(),
        "a visible flat decal (real texture, AlphaTest) must be kept, not suppressed"
    );
}

#[test]
fn decodes_a_blp_icon_to_png() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let bytes = chain
        .read_file("Interface/Icons/Spell_Holy_ArcaneIntellect.blp")
        .expect("read spell icon BLP");

    // Keyed by pid: concurrent test runs would otherwise delete each other's file.
    let out = std::env::temp_dir().join(format!("wrathplus-fmt-icon-{}.png", std::process::id()));
    let (w, h) = wrathplus_formats::blp_to_png(&bytes, &out).expect("decode BLP -> PNG");

    assert_eq!((w, h), (64, 64), "spell icons are 64x64");
    let png = std::fs::read(&out).expect("read written PNG");
    assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10], "PNG magic");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn meshes_an_elwynn_terrain_tile() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let (tx, ty) = wrathplus_formats::find_tile_near(&mut chain, "Azeroth", -8840.56, 489.7)
        .expect("find an Elwynn tile");
    let tile =
        wrathplus_formats::load_tile_mesh(&mut chain, "Azeroth", tx, ty).expect("mesh the tile");

    assert!(!tile.chunks.is_empty(), "tile should have terrain chunks");
    assert!(
        tile.vertex_count() <= 256 * 145,
        "<= 256 chunks * (81 outer + 64 inner) = 145 verts/chunk (center-fan)"
    );
    // Rows of 9 outer and 8 inner vertices alternate, 17 to a pair.
    let mut hilltop_dev = 0.0f32;
    for c in &tile.chunks {
        assert_eq!(c.positions.len(), 145, "chunk = 81 outer + 64 inner verts");
        for r in 0..8usize {
            for col in 0..8usize {
                let tl = c.positions[r * 17 + col][2];
                let tr = c.positions[r * 17 + col + 1][2];
                let bl = c.positions[(r + 1) * 17 + col][2];
                let br = c.positions[(r + 1) * 17 + col + 1][2];
                let ctr = c.positions[r * 17 + 9 + col][2];
                let mean = (tl + tr + bl + br) * 0.25;
                hilltop_dev = hilltop_dev.max((ctr - mean).abs());
            }
        }
    }
    assert!(
        hilltop_dev > 1.0,
        "expected ≥1 cell on this Elwynn tile with center−corner deviation > 1 yd \
         (proves we read the authored MCVT inner heights, not just outer); got max {hilltop_dev:.2}",
    );

    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for chunk in &tile.chunks {
        assert_eq!(chunk.indices.len() % 3, 0, "triangle list per chunk");
        assert_eq!(chunk.positions.len(), chunk.uvs.len(), "one UV per vertex");
        assert!(
            chunk
                .indices
                .iter()
                .all(|&i| (i as usize) < chunk.positions.len()),
            "chunk indices reference valid vertices"
        );
        for p in &chunk.positions {
            for i in 0..3 {
                mn[i] = mn[i].min(p[i]);
                mx[i] = mx[i].max(p[i]);
            }
        }
    }

    // A tile is 533.33 yd square; a scrambled MCNK axis would blow the Z span.
    let span = |i: usize| mx[i] - mn[i];
    assert!(
        (500.0..540.0).contains(&span(0)) && (500.0..540.0).contains(&span(1)),
        "X/Y should each span ~one tile (533 yds); got X={:.1}, Y={:.1}",
        span(0),
        span(1)
    );
    assert!(
        mx[0] < 0.0,
        "tile 31_48 world X is negative; got max X={:.1}",
        mx[0]
    );
    assert!(
        span(2) < 600.0,
        "Z should be terrain heights; got span {:.1}",
        span(2)
    );

    assert!(
        tile.chunks.iter().any(|c| c
            .base_texture
            .as_deref()
            .is_some_and(|t| t.to_ascii_lowercase().ends_with(".blp"))),
        "expected a base-layer .blp texture on some chunk"
    );

    let multilayer = tile
        .chunks
        .iter()
        .find(|c| c.layer_textures.len() > 1)
        .expect("some chunk should have multiple texture layers");
    let alpha = multilayer
        .alpha_map
        .as_ref()
        .expect("a multi-layer chunk carries a packed alpha map");
    let size = wrathplus_formats::ALPHA_MAP_SIZE as usize;
    assert_eq!(alpha.len(), size * size * 4, "alpha map is RGBA size²");
    assert!(
        alpha
            .as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0),
        "a multi-layer chunk's alpha map should have some non-zero blend weight"
    );
    assert!(
        multilayer.layer_textures.len() <= 4,
        "at most 4 terrain layers per chunk"
    );
}

#[test]
fn terrain_chunks_carry_unit_upward_mcnr_normals() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let (tx, ty) = wrathplus_formats::find_tile_near(&mut chain, "Azeroth", -8840.56, 489.7)
        .expect("find an Elwynn tile");
    let tile =
        wrathplus_formats::load_tile_mesh(&mut chain, "Azeroth", tx, ty).expect("mesh the tile");

    let (mut checked, mut up) = (0usize, 0usize);
    for chunk in &tile.chunks {
        if chunk.normals.is_empty() {
            continue;
        }
        assert_eq!(
            chunk.normals.len(),
            chunk.positions.len(),
            "one MCNR normal per vertex"
        );
        for n in &chunk.normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!(
                (0.9..=1.1).contains(&len),
                "decoded normal should be ~unit, got len {len:.3} for {n:?}"
            );
            checked += 1;
            // WoW Z is up; a wrong X/Z/Y decode would put "up" in another component.
            if n[2] > 0.5 {
                up += 1;
            }
        }
    }
    assert!(checked > 0, "Elwynn tile should carry MCNR normals");
    assert!(
        up * 100 / checked >= 70,
        "most terrain normals should point up (WoW +Z); only {up}/{checked} did"
    );
}

/// Terrain winds CCW seen from above, so it draws backface-culled as the reference does: its chunk
/// pass never sets `EGxRs 0x14` and inherits `CULL_FACE = 1`. Exact on the XY lattice (`0x6b0e50`
/// takes X and Y from the grid, only Z from MCVT); against the authored MCNR normals it holds bar
/// outliers, 4 of 196368 pairs on this tile, hence the 0.01% ceiling.
#[test]
fn terrain_fans_wind_ccw_seen_from_above() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let (tx, ty) = wrathplus_formats::find_tile_near(&mut chain, "Azeroth", -8840.56, 489.7)
        .expect("find an Elwynn tile");
    let tile =
        wrathplus_formats::load_tile_mesh(&mut chain, "Azeroth", tx, ty).expect("mesh the tile");

    let (mut tris, mut min_nz) = (0usize, f32::MAX);
    let (mut pairs, mut disagreeing) = (0usize, 0usize);
    for chunk in &tile.chunks {
        let shading = (chunk.normals.len() == chunk.positions.len()).then_some(&chunk.normals);
        for t in chunk.indices.as_chunks::<3>().0 {
            let (i, j, k) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (a, b, c) = (chunk.positions[i], chunk.positions[j], chunk.positions[k]);
            let (u, v) = (
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            // The Z of u × v, positive for CCW seen from above.
            min_nz = min_nz.min(u[0] * v[1] - u[1] * v[0]);
            if let Some(normals) = shading {
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                for vert in [i, j, k] {
                    let s = normals[vert];
                    pairs += 1;
                    if n[0] * s[0] + n[1] * s[1] + n[2] * s[2] <= 0.0 {
                        disagreeing += 1;
                    }
                }
            }
            tris += 1;
        }
    }
    assert!(tris > 1000, "expected a meshed tile, got {tris} triangles");
    assert!(
        min_nz > 0.0,
        "every terrain triangle must face up (CCW from above in WoW space); \
         worst geometric-normal Z was {min_nz} over {tris} triangles"
    );
    assert!(pairs > 0, "Elwynn tile should carry MCNR normals");
    assert!(
        disagreeing * 10_000 <= pairs,
        "the emitted winding must agree with the authored MCNR normals bar a handful of outliers; \
         {disagreeing}/{pairs} disagreed (ceiling 0.01%) — a flipped fan turns ~all of them"
    );
}

#[test]
fn terrain_is_watertight_at_chunk_seams() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let (tx, ty) = wrathplus_formats::find_tile_near(&mut chain, "Azeroth", -8840.56, 489.7)
        .expect("find an Elwynn tile");
    let tile =
        wrathplus_formats::load_tile_mesh(&mut chain, "Azeroth", tx, ty).expect("mesh the tile");

    // 0.25-yd XY buckets group only coincident vertices: the grid step is 4.17 yd.
    use std::collections::HashMap;
    let mut buckets: HashMap<(i64, i64), Vec<[f32; 3]>> = HashMap::new();
    for c in &tile.chunks {
        for p in &c.positions {
            let key = ((p[0] * 4.0).round() as i64, (p[1] * 4.0).round() as i64);
            buckets.entry(key).or_default().push(*p);
        }
    }
    let mut shared = 0usize;
    for ps in buckets.values() {
        if ps.len() < 2 {
            continue;
        }
        shared += 1;
        for w in ps.windows(2) {
            assert_eq!(
                w[0][0], w[1][0],
                "shared-edge X must be bit-identical (no seam)"
            );
            assert_eq!(
                w[0][1], w[1][1],
                "shared-edge Y must be bit-identical (no seam)"
            );
            assert!(
                (w[0][2] - w[1][2]).abs() < 0.01,
                "shared-edge heights should match: {} vs {}",
                w[0][2],
                w[1][2]
            );
        }
    }
    assert!(
        shared > 0,
        "tile should have shared chunk-edge vertices to check"
    );
}

#[test]
fn loads_a_block_of_elwynn_tiles() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let tiles = wrathplus_formats::load_tiles_around(&mut chain, "Azeroth", -8840.56, 489.7, 1)
        .expect("load tile block");

    assert!(
        tiles.len() > 1,
        "radius 1 around Stormwind should load several contiguous tiles, got {}",
        tiles.len()
    );
    let mut coords: Vec<_> = tiles.iter().map(|(c, _)| *c).collect();
    coords.sort_unstable();
    coords.dedup();
    assert_eq!(coords.len(), tiles.len(), "tiles must be distinct");
}

#[test]
fn reads_and_decodes_a_terrain_texture() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let (tx, ty) = wrathplus_formats::find_tile_near(&mut chain, "Azeroth", -8840.56, 489.7)
        .expect("find a tile");
    let tile =
        wrathplus_formats::load_tile_mesh(&mut chain, "Azeroth", tx, ty).expect("mesh the tile");

    let texture = tile
        .chunks
        .iter()
        .find_map(|c| c.base_texture.clone())
        .expect("at least one chunk has a base texture");

    let (w, h, rgba) =
        wrathplus_formats::read_texture_rgba(&mut chain, &texture).expect("decode terrain texture");
    assert!(w > 0 && h > 0, "non-empty texture, got {w}x{h}");
    assert_eq!(rgba.len(), (w * h * 4) as usize, "RGBA8 pixel buffer");
}

#[test]
fn resolves_creature_models_from_dbcs() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let catalog = wrathplus_formats::load_creature_catalog(&mut chain)
        .expect("load creature display catalog");
    assert!(
        catalog.len() > 1000,
        "vanilla has thousands of creature displays, got {}",
        catalog.len()
    );

    // Not every `CreatureModelData` path ships; scale 0 is valid (many low ids are trigger NPCs).
    let (mut resolved, mut positive_scale, mut meshed) = (0u32, 0u32, false);
    for id in 1..6000u32 {
        let Some(m) = catalog.model(id) else { continue };
        resolved += 1;
        if m.scale > 0.0 {
            positive_scale += 1;
        }
        let p = m.model_path.to_ascii_lowercase();
        if !meshed && m.scale > 0.0 && p.contains("creature") && p.ends_with(".mdx") {
            if let Ok(subs) =
                wrathplus_formats::load_m2_mesh_skinned(&mut chain, &m.model_path, &m.textures)
            {
                meshed |= !subs.is_empty();
            }
        }
    }
    assert!(
        resolved > 500,
        "many display ids should resolve, got {resolved}"
    );
    assert!(
        positive_scale * 2 > resolved,
        "most resolved displays should have positive scale, got {positive_scale}/{resolved}"
    );
    assert!(
        meshed,
        "expected at least one resolved creature model to mesh"
    );
}

#[test]
fn resolves_gameobject_models_from_dbc() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let catalog = wrathplus_formats::load_gameobject_catalog(&mut chain)
        .expect("load gameobject display catalog");
    assert!(
        catalog.len() > 500,
        "vanilla has many GameObject displays, got {}",
        catalog.len()
    );

    let mut meshed = false;
    for id in 1..3000u32 {
        let Some(path) = catalog.model_path(id).map(str::to_string) else {
            continue;
        };
        let p = path.to_ascii_lowercase();
        assert!(
            p.ends_with(".mdx") || p.ends_with(".mdl") || p.ends_with(".wmo"),
            "unexpected GameObject model path: {path}"
        );
        if !meshed {
            if let Ok(subs) = wrathplus_formats::load_object_model(&mut chain, &path) {
                meshed |= !subs.is_empty();
            }
        }
    }
    assert!(meshed, "expected at least one GameObject model to mesh");
}

#[test]
fn loads_classic_models_with_cosmetic_chunks() {
    let data = wrathplus_formats::wow_data_or_skip!();

    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    // Their particle, light and texture-animation chunks are malformed; the geometry still loads.
    for path in [
        "Creature\\Kobold\\Kobold.mdx",
        "Creature\\Imp\\Imp.mdx",
        "World\\Azeroth\\Elwynn\\PassiveDoodads\\Campfire\\ElwynnCampfire.mdx",
    ] {
        let subs = wrathplus_formats::load_m2_mesh(&mut chain, path)
            .unwrap_or_else(|e| panic!("load {path}: {e:#}"));
        assert!(!subs.is_empty(), "{path} should produce submeshes");
    }
}

fn span3(ps: &[[f32; 3]]) -> [f32; 3] {
    if ps.is_empty() {
        return [0.0; 3];
    }
    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in ps {
        for i in 0..3 {
            mn[i] = mn[i].min(p[i]);
            mx[i] = mx[i].max(p[i]);
        }
    }
    [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]]
}

#[test]
fn decodes_m2_collision_hulls() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // Counts read off the raw bytes. The hull is a coarse trunk, far smaller than the canopy.
    let pine = wrathplus_formats::load_m2_collision_hull(
        &mut chain,
        "World\\Azeroth\\Elwynn\\PassiveDoodads\\Trees\\ElwynnTree01\\ElwynnPine01.mdx",
    )
    .expect("pine hull");
    assert_eq!(pine.triangle_count(), 6, "ElwynnPine01 hull = 6 tris");
    assert_eq!(pine.positions.len(), 5, "ElwynnPine01 hull = 5 verts");
    let s = span3(&pine.positions);
    assert!(
        (9.0..11.0).contains(&s[2]),
        "pine trunk hull ~10 yd tall, got Z span {:.2}",
        s[2]
    );
    assert!(
        s[0] < 4.0 && s[1] < 4.0,
        "pine trunk hull is thin (≪ canopy), got X={:.2} Y={:.2}",
        s[0],
        s[1]
    );
    assert!(
        pine.indices
            .iter()
            .all(|&i| (i as usize) < pine.positions.len()),
        "hull indices in range"
    );

    let canopy = wrathplus_formats::load_m2_collision_hull(
        &mut chain,
        "World\\Azeroth\\Elwynn\\PassiveDoodads\\Trees\\ElwynnTreeCanopy01.mdx",
    )
    .expect("canopy hull");
    assert_eq!(
        canopy.triangle_count(),
        52,
        "ElwynnTreeCanopy01 hull = 52 tris"
    );
    assert_eq!(
        canopy.positions.len(),
        30,
        "ElwynnTreeCanopy01 hull = 30 verts"
    );

    let bounds = wrathplus_formats::load_m2_bounds(
        &mut chain,
        "World\\Azeroth\\Elwynn\\PassiveDoodads\\Trees\\ElwynnTreeCanopy01.mdx",
    )
    .expect("canopy bounds");
    let render_x = bounds.bbox_max[0] - bounds.bbox_min[0];
    let hull_x = span3(&canopy.positions)[0];
    assert!(
        hull_x < render_x * 0.6,
        "collision hull (X span {hull_x:.1}) should be far tighter than the render bbox ({render_x:.1})"
    );
}

#[test]
fn filters_wmo_collidable_triangles_by_mopy() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // The reference's walking query skips a face iff `MOPY.flags & 0x04` (DETAIL): reject mask
    // `0x84`, tested at its box-query leaf `0x6bca50` (from `0x6b91c0`, `0x6bc1c0`).
    let inn = wrathplus_formats::load_wmo_collision_tris(
        &mut chain,
        "World\\wmo\\Azeroth\\Buildings\\GoldshireInn\\GoldshireInn.wmo",
    )
    .expect("goldshire inn collision");
    assert_eq!(inn.triangle_count(), 5114, "Goldshire Inn collidable tris");
    assert!(
        inn.indices
            .iter()
            .all(|&i| (i as usize) < inn.positions.len()),
        "collidable indices in range"
    );
    let s = span3(&inn.positions);
    assert!(
        (40.0..80.0).contains(&s[0])
            && (20.0..50.0).contains(&s[1])
            && (20.0..45.0).contains(&s[2]),
        "inn collidable bbox should be building-sized, got {s:?}"
    );

    let bridge = wrathplus_formats::load_wmo_collision_tris(
        &mut chain,
        "World\\wmo\\Azeroth\\Collidable Doodads\\Elwynn\\WideBridge\\ElwynnWideBridge.wmo",
    )
    .expect("elwynn wide bridge collision");
    assert!(
        bridge.triangle_count() > 0,
        "a bridge must carry collidable triangles (the 'walk under bridges' fix)"
    );
}

#[test]
fn derives_the_wmo_window_glass_law_from_momt() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    // The MOMT glass mechanisms: the exterior UNLIT drawer `0x6b4f10`, the interior WINDOW drawer
    // `0x6b5190` and the SIDN night ramp `0x6b4090`. The inn's mat 5 is flags 0x11 (UNLIT, SIDN)
    // in exterior groups; mat 21 is 0x28 (WINDOW) in interior groups, lit, never fullbright.
    let inn = wrathplus_formats::load_wmo(
        &mut chain,
        "World\\wmo\\Azeroth\\Buildings\\GoldshireInn\\GoldshireInn.wmo",
    )
    .expect("goldshire inn render submeshes");
    let tex_is = |sub: &wrathplus_formats::RenderSubmesh, name: &str| {
        sub.texture
            .as_deref()
            .is_some_and(|t| t.to_ascii_uppercase().ends_with(name))
    };
    let ext_panes: Vec<_> = inn
        .iter()
        .filter(|s| tex_is(s, "MM_ELWYNN_WND_EXT__01.BLP"))
        .collect();
    assert!(!ext_panes.is_empty(), "inn exterior window panes present");
    for s in &ext_panes {
        assert!(!s.interior, "the EXT pane sits in an exterior group");
        assert!(s.emissive, "UNLIT on an exterior group ⇒ unlit fullbright");
        assert_eq!(s.sidn, Some([203, 203, 203]), "the authored SIDN colour");
        assert!(!s.window, "no WINDOW bit on the exterior pane");
    }
    let int_panes: Vec<_> = inn
        .iter()
        .filter(|s| tex_is(s, "MM_ELWYNN_WND_INT__01.BLP"))
        .collect();
    assert!(!int_panes.is_empty(), "inn interior window panes present");
    for s in &int_panes {
        assert!(s.interior, "the INT pane sits in interior groups");
        assert!(
            !s.emissive,
            "the interior drawer ignores UNLIT — never fullbright"
        );
        assert!(s.window, "WINDOW (0x20) ⇒ the interior midpoint light");
        assert_eq!(s.sidn, None, "no SIDN flag on the interior pane");
        assert_eq!(
            s.wmo_batch,
            Some(wrathplus_formats::WmoBatchClass::Ext),
            "the inn's interior panes land in the EXT-in-group (lit) section"
        );
    }

    // SIDN is a BGRA `CImVector`: Shadowfang's glass is disk bytes 216,231,250,255.
    let sfk = wrathplus_formats::load_wmo(
        &mut chain,
        "World\\wmo\\Dungeon\\LD_ShadowFang\\LD_ShadowFang.wmo",
    )
    .expect("shadowfang render submeshes");
    assert!(
        sfk.iter().any(|s| s.sidn == Some([250, 231, 216])),
        "Shadowfang authors the warm-cream SIDN glass (BGRA 216,231,250 → RGB 250,231,216)"
    );
}

#[test]
fn reads_authentic_atmosphere_from_light_dbc() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let cat = wrathplus_formats::LightCatalog::load(&mut chain).expect("load Light DBCs");

    // Northshire resolves to Azeroth's global light 1, clear LightParams 12.
    let a = cat.sample_noon(0, [-8949.95, -132.49, 83.5]);

    assert_ne!(
        a.fog_end,
        wrathplus_formats::Atmosphere::DEFAULT.fog_end,
        "got the fallback atmosphere — Light bands didn't resolve"
    );

    // Decoded off the raw DBC bytes. Fog end is stored ×36 and scaled once at DBC load by
    // `0x6d6090`; the start fraction is never scaled.
    let to255 = |c: [f32; 3]| {
        [
            (c[0] * 255.0).round() as i32,
            (c[1] * 255.0).round() as i32,
            (c[2] * 255.0).round() as i32,
        ]
    };
    assert!(
        (a.fog_end - 500.0).abs() < 1.0,
        "clear fog_end should be 500 yd (raw 18000/36), got {}",
        a.fog_end
    );
    assert!(
        (a.fog_start_frac - 0.25).abs() < 0.001,
        "clear fog start fraction should be +0.25, got {}",
        a.fog_start_frac
    );
    assert_eq!(to255(a.sky[0]), [0, 31, 73], "SkyTop/zenith (int row 2)");
    assert_eq!(to255(a.sky[1]), [58, 162, 207], "SkyMiddle (int row 3)");
    assert_eq!(to255(a.sky[2]), [153, 220, 245], "SkyBand1 (int row 4)");
    assert_eq!(to255(a.sky[3]), [175, 218, 224], "SkyBand2 (int row 5)");
    assert_eq!(
        to255(a.sky[4]),
        [180, 180, 180],
        "SkySmog/horizon (int row 6)"
    );
    assert_eq!(to255(a.sun_color), [255, 247, 222], "sun (int row 9)");
    assert_eq!(to255(a.ambient), [104, 130, 154], "ambient (int row 1)");
    assert_eq!(to255(a.fog_color), [77, 120, 143], "fog color (int row 7)");
    for c in [a.fog_color, a.sun_color, a.ambient] {
        for ch in c {
            assert!(
                (0.0..=1.0).contains(&ch),
                "color channel out of range: {ch}"
            );
        }
    }
    assert!(
        a.fog_color.iter().sum::<f32>() > 0.2 && a.sun_color.iter().sum::<f32>() > 0.5,
        "daytime colors implausibly dark: fog {:?} sun {:?}",
        a.fog_color,
        a.sun_color
    );
}

/// Elwynn's storm param, LightParams 10, at noon. Its fog start fraction is -0.5 and the reference
/// never clamps it (`0x6cee6d`): the negative start is the constant ~33% near veil under rain.
#[test]
fn reads_elwynn_storm_fog_endpoints() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let cat = wrathplus_formats::LightCatalog::load(&mut chain).expect("load Light DBCs");

    // Northshire at noon, time 1440 of 2880, storm slot; its bands are single-stop.
    let s = cat.sample(0, [-8949.95, -132.49, 83.5], 1440, true);
    assert!(
        (s.fog_end - 277.8).abs() < 1.0,
        "storm fog_end should be ~278 yd (raw 10000/36), got {}",
        s.fog_end
    );
    assert!(
        (s.fog_start_frac - -0.5).abs() < 0.001,
        "storm fog start fraction should be −0.5 (negative ⇒ the near veil), got {}",
        s.fog_start_frac
    );
    let to255 = |c: [f32; 3]| {
        [
            (c[0] * 255.0).round() as i32,
            (c[1] * 255.0).round() as i32,
            (c[2] * 255.0).round() as i32,
        ]
    };
    assert_eq!(to255(s.fog_color), [82, 84, 82], "storm fog color (grey)");
    assert_eq!(
        to255(s.sun_diffuse),
        [101, 101, 101],
        "storm diffuse (flat grey)"
    );
    assert_eq!(to255(s.ambient), [78, 78, 95], "storm ambient (grey-blue)");
}

#[test]
fn reads_map_directories_from_map_dbc() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let cat = wrathplus_formats::load_map_catalog(&mut chain).expect("load Map.dbc");

    // 5875 has ~44 maps (2 continents, ~12 dungeons, BGs, dev maps).
    assert!(
        cat.len() >= 40,
        "expected ≥40 Map.dbc rows, got {}",
        cat.len()
    );
    assert_eq!(cat.directory(0), Some("Azeroth"));
    assert_eq!(cat.directory(1), Some("Kalimdor"));
    assert_eq!(cat.directory(36), Some("DeadminesInstance"));
}

#[test]
fn decodes_azeroth_wdl_against_apitrace_ground_truth() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    let wdl = wrathplus_formats::WdlFile::load(&mut chain, "Azeroth").expect("load Azeroth.wdl");

    // Counted off the file: MVER 18, 687 tiles present.
    assert_eq!(wdl.present_count(), 687, "Azeroth.wdl present-tile count");

    // From a reference frame capture: tile (34, 48) spans X -9066.7..-8533.3 and
    // Y -1600..-1066.7, with vertex heights 54..349.
    assert!(
        wdl.is_present(34, 48),
        "apitrace tile (34,48) must be present"
    );
    let mesh = wdl.tile_mesh(34, 48).expect("mesh for (34,48)");
    assert_eq!(mesh.positions.len(), 545, "545 verts (17x17 + 16x16)");
    assert_eq!(
        mesh.indices.len(),
        3072,
        "3072 indices (16x16 cells x 4 tris)"
    );

    let zs: Vec<f32> = mesh.positions.iter().map(|p| p[2]).collect();
    let zmin = zs.iter().copied().fold(f32::INFINITY, f32::min);
    let zmax = zs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(zmin, 54.0, "tile (34,48) min height (apitrace VB)");
    assert_eq!(zmax, 349.0, "tile (34,48) max height (apitrace VB)");

    let xs: Vec<f32> = mesh.positions.iter().map(|p| p[0]).collect();
    let ys: Vec<f32> = mesh.positions.iter().map(|p| p[1]).collect();
    let span = |v: &[f32]| {
        v.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - v.iter().copied().fold(f32::INFINITY, f32::min)
    };
    assert!(
        (span(&xs) - 533.333).abs() < 0.5,
        "X span ~533.33, got {}",
        span(&xs)
    );
    assert!(
        (span(&ys) - 533.333).abs() < 0.5,
        "Y span ~533.33, got {}",
        span(&ys)
    );
    // The max-X, max-Y corner is the tile origin, `wrathplus_wdt::tile_to_world(34, 48)`.
    let xmax = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let ymax = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!((xmax - (-8533.3)).abs() < 0.5, "tile origin X, got {xmax}");
    assert!((ymax - (-1066.7)).abs() < 0.5, "tile origin Y, got {ymax}");
}

/// Magma and slime submersion read the fixed global `LightParams` rows 7 and 6 (`0x6d2371`), which
/// no `Light.dbc` row references; they decode to the dense orange and green views.
#[test]
fn magma_and_slime_submersion_read_the_fixed_global_light_params() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let cat = wrathplus_formats::LightCatalog::load(&mut chain).expect("load Light DBCs");
    let to255 = |c: [f32; 3]| {
        [
            (c[0] * 255.0).round() as i32,
            (c[1] * 255.0).round() as i32,
            (c[2] * 255.0).round() as i32,
        ]
    };

    // Two continents with different clear atmospheres: a fixed row reads the same at both.
    let pins = [
        (0u32, [-7531.21f32, -1123.64, 172.58]), // Blackrock Mountain, in lava
        (1u32, [-5075.53f32, -2063.09, -50.10]), // Thousand Needles, in water
    ];
    for kind in [
        wrathplus_formats::Submersion::Magma,
        wrathplus_formats::Submersion::Slime,
    ] {
        let sampled: Vec<_> = pins
            .iter()
            .map(|(map, pos)| cat.sample_blended(*map, *pos, 1440, false, kind, false))
            .collect();
        assert_eq!(
            to255(sampled[0].fog_color),
            to255(sampled[1].fog_color),
            "{kind:?} is zone-INDEPENDENT: the fixed row must not vary by position"
        );
        assert_eq!(
            sampled[0].fog_end, sampled[1].fog_end,
            "{kind:?} fog end must not vary by position"
        );
    }

    // Row 7, the client's shortest fog: end 972/36 = 27 yd, start -2.0, so 67% at the eye.
    let magma = cat.sample_blended(
        0,
        pins[0].1,
        1440,
        false,
        wrathplus_formats::Submersion::Magma,
        false,
    );
    assert_eq!(
        to255(magma.fog_color),
        [200, 52, 0],
        "magma fog (row 7 int 7)"
    );
    assert_eq!(to255(magma.ambient), [255, 55, 0], "magma ambient");
    assert!(
        (magma.fog_end - 27.0).abs() < 0.01,
        "magma fog end 972/36 = 27 yd, got {}",
        magma.fog_end
    );
    assert!(
        (magma.fog_start_frac + 2.0).abs() < 1e-6,
        "magma start fraction −2.0, got {}",
        magma.fog_start_frac
    );

    // Row 6: end 1800/36 = 50 yd, start -1.0, so 50% at the eye.
    let slime = cat.sample_blended(
        0,
        pins[0].1,
        1440,
        false,
        wrathplus_formats::Submersion::Slime,
        false,
    );
    assert_eq!(
        to255(slime.fog_color),
        [0, 255, 0],
        "slime fog (row 6 int 7)"
    );
    assert_eq!(to255(slime.ambient), [0, 60, 0], "slime ambient");
    assert!(
        (slime.fog_end - 50.0).abs() < 0.01,
        "slime fog end 1800/36 = 50 yd, got {}",
        slime.fog_end
    );
    assert!(
        (slime.fog_start_frac + 1.0).abs() < 1e-6,
        "slime start fraction −1.0, got {}",
        slime.fog_start_frac
    );

    // Neither is the zone's water murk, LightParams 203 at the Thousand Needles pin.
    let water = cat.sample_blended(
        1,
        pins[1].1,
        1440,
        false,
        wrathplus_formats::Submersion::Water,
        false,
    );
    assert_ne!(
        to255(water.fog_color),
        to255(magma.fog_color),
        "lava must not inherit the zone's water murk"
    );
    assert_ne!(
        to255(water.fog_color),
        to255(slime.fog_color),
        "slime must not inherit the zone's water murk"
    );
    let dry = cat.sample_blended(
        1,
        pins[1].1,
        1440,
        false,
        wrathplus_formats::Submersion::Dry,
        false,
    );
    assert_ne!(
        to255(dry.fog_color),
        to255(water.fog_color),
        "the water pin's underwater slot must differ from its clear slot"
    );
}

/// The far band's tile window contains the camera's own tile: below the default view distance that
/// tile is all that draws the near horizon.
#[test]
fn the_wdl_window_contains_the_cameras_own_tile() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");
    let wdl = wrathplus_formats::WdlFile::load(&mut chain, "Kalimdor").expect("load Kalimdor.wdl");

    // Weazel's Crater, Thousand Needles.
    let (x, y) = (-5841.9_f32, -3802.4);
    let own = wrathplus_wdt::world_to_tile(x, y);
    assert_eq!(own, (39, 42), "the reported tile for Weazel's Crater");
    assert!(wdl.is_present(own.0, own.1), "the own tile is authored");

    let window = wdl.tiles_in_ring(x, y, 5);
    assert!(
        window.contains(&own),
        "the camera's own WDL tile must be in the window — without it the near horizon is a hole"
    );
    // A full (2r+1)² window, not a ring: every present tile within the radius.
    let expected = (-5i32..=5)
        .flat_map(|dy| (-5i32..=5).map(move |dx| (own.0 as i32 + dx, own.1 as i32 + dy)))
        .filter(|&(tx, ty)| (0..64).contains(&tx) && (0..64).contains(&ty))
        .filter(|&(tx, ty)| wdl.is_present(tx as u32, ty as u32))
        .count();
    assert_eq!(window.len(), expected, "every present tile in the window");
}

/// Manual install probe for the macro-catalog stall: the filtered listing must remain exactly
/// equivalent to filtering the full listing while resolving far fewer listfile names.
#[test]
#[ignore = "install timing probe; run explicitly with WOW_DATA"]
fn filtered_macro_icon_listing_matches_the_full_listing() {
    use std::time::Instant;

    const ICON_DIR: &str = "interface\\icons\\";
    let is_candidate = |name: &str| {
        let normalized = name.replace('/', "\\");
        let lower = normalized.to_ascii_lowercase();
        let Some(file) = lower.strip_prefix(ICON_DIR) else {
            return false;
        };
        !file.contains('\\')
            && (file.starts_with("spell_") || file.starts_with("ability_"))
            && (file.ends_with(".blp") || file.ends_with(".tga"))
    };

    let data = wrathplus_formats::wow_data_or_skip!();
    let profile = wrathplus_formats::DataProfile::from_env().expect("timing-probe profile");
    let chain = Chain::open_profile(&data, &profile).expect("open explicit-profile patch chain");
    let full_started = Instant::now();
    let mut full_resolved = 0usize;
    let full = chain
        .list_filtered(|_| {
            full_resolved += 1;
            true
        })
        .expect("full chain listing");
    let full_elapsed = full_started.elapsed();
    let expected: Vec<_> = full
        .iter()
        .filter(|entry| is_candidate(&entry.name))
        .map(|entry| (&entry.name, entry.size))
        .collect();
    let mut expected_icons: Vec<_> = expected
        .iter()
        .map(|(name, _)| {
            let normalized = name.replace('/', "\\");
            let stem = normalized[ICON_DIR.len()..].rsplit_once('.').unwrap().0;
            format!("Interface\\Icons\\{stem}")
        })
        .collect();
    expected_icons.sort_by_key(|name| name.to_ascii_lowercase());
    expected_icons.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

    let filtered_started = Instant::now();
    let mut filtered_resolved = 0usize;
    let filtered = chain
        .list_filtered(|name| {
            let keep = is_candidate(name);
            filtered_resolved += usize::from(keep);
            keep
        })
        .expect("filtered chain listing");
    let filtered_elapsed = filtered_started.elapsed();
    let actual: Vec<_> = filtered
        .iter()
        .map(|entry| (&entry.name, entry.size))
        .collect();

    assert_eq!(
        actual, expected,
        "filtering must not change names, sizes, or order"
    );
    let mut icon_chain =
        Chain::open_profile(&data, &profile).expect("reopen explicit-profile patch chain");
    let actual_icons = wrathplus_formats::load_macro_icons(&mut icon_chain).expect("macro icons");
    assert_eq!(
        actual_icons, expected_icons,
        "filtered and full-list catalogs differ"
    );
    eprintln!(
        "macro-list probe: full={} live names / {} resolution calls in {:?}; filtered={} live names / {} resolution calls in {:?}; {:.1}x fewer resolution calls (full then filtered, warm-cache-biased timing)",
        full.len(), full_resolved,
        full_elapsed,
        filtered.len(), filtered_resolved,
        filtered_elapsed,
        full_resolved as f64 / filtered_resolved.max(1) as f64
    );
}

/// A WMO root's MOSB skybox and the per-group `0x40000` flag that asks for it: the Stratholme city
/// root names one and 61 of 83 groups ask, the dungeon root names none. Four 1.12 roots name a
/// skybox no group asks for, so the gate is the flag, not the chunk.
#[test]
fn reads_the_skybox_and_its_per_group_gate() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = open_chain(&data).expect("open vanilla patch chain");

    let city = chain
        .read_file("World\\wmo\\Dungeon\\LD_Stratholme\\Stratholme_B.wmo")
        .expect("read the Stratholme city WMO root");
    let city = wrathplus_formats::parse_wmo_root(&city).expect("parse the city root");
    assert_eq!(
        city.skybox(),
        Some("environments\\stars\\stratholmeskybox.m2"),
        "the city root's MOSB names the painted sky (normalized .mdx -> .m2)"
    );
    let asking = city.group_infos().iter().filter(|g| g.show_skybox).count();
    assert_eq!(
        (asking, city.group_infos().len()),
        (61, 83),
        "the city's open streets ask for the skybox; its enclosed rooms don't"
    );

    let dungeon = chain
        .read_file("World\\wmo\\Dungeon\\LD_Stratholme\\Stratholme.wmo")
        .expect("read the Stratholme dungeon WMO root");
    let dungeon = wrathplus_formats::parse_wmo_root(&dungeon).expect("parse the dungeon root");
    assert_eq!(
        dungeon.skybox(),
        None,
        "the dungeon root's MOSB is the empty string — no skybox"
    );
    assert_eq!(
        dungeon
            .group_infos()
            .iter()
            .filter(|g| g.show_skybox)
            .count(),
        0,
        "and not one of its groups asks for one"
    );
}
