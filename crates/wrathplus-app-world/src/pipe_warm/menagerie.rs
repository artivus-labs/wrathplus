//! The menagerie, `pipe_warm`'s warm set: one tiny rig per reachable pipeline variant, and the
//! lane-coverage tests that keep a new lane from shipping unwarmed.

use bevy::camera::primitives::MeshAabb;
use bevy::ecs::system::SystemParam;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::render::render_resource::Buffer;
use wrathplus_formats::{FogPolicy, ModelBlend};

use wrathplus_assets::materials::{LiquidMaterial, WowModelMaterial};
use wrathplus_world::clouds::CloudMaterial;
use wrathplus_world::model_render::{
    far_twin_of, model_material, zfill_material, MaterialCache, ShadeSel,
};
use wrathplus_world::sky::SkyMaterial;
use wrathplus_world::sun::{CelestialMaterial, StarMaterial};

use super::WarmRig;

#[path = "warm_meshes.rs"]
mod warm_meshes;
use warm_meshes::{warm_liquid_mesh, warm_pos_uv_mesh, warm_quad};

// --------------------------------------------------------------------------------------------
// The warm set: the model lane (4 vertex layouts by the blend and depth-flag families), its
// shard-rung and far-side-of-water twins, and the sky and water lanes. Each rig is 1 cm on a
// camera that renders under the cover, so its draw queues its pipeline through the production
// specialize path. A variant this misses shows as the tripwire's "compiled LIVE" warn.

/// The lanes' material stores, bundled for [`run_warm_pass`]'s arity. The sky and water stores
/// are complete at `Startup`, so iterating them is the reachable set; the WMO skybox is a cross of
/// `WowModelMaterial` keys, warmed with the model cross.
#[derive(SystemParam)]
pub(super) struct WarmLanes<'w> {
    celestial: ResMut<'w, Assets<CelestialMaterial>>,
    stars: ResMut<'w, Assets<StarMaterial>>,
    clouds: ResMut<'w, Assets<CloudMaterial>>,
    sky: ResMut<'w, Assets<SkyMaterial>>,
    liquid: ResMut<'w, Assets<LiquidMaterial>>,
    /// For representatives of the on-demand nameplate and raid-mark materials.
    standard: ResMut<'w, Assets<StandardMaterial>>,
    /// The fallback cube: production mesh + materials, drawn while a model streams.
    cubes: Option<Res<'w, crate::entities::CubeAssets>>,
    /// For the twin booth's render target and the stand-in textures.
    pub(super) images: ResMut<'w, Assets<Image>>,
    /// For the minimap interior composite's tile material, through its production builder.
    ui_quads: ResMut<'w, Assets<crate::overlay::UiQuadMaterial>>,
}

/// A portrait booth camera and its layer. Booths run `Msaa::Off`, so each model pipeline has a
/// samples=1 twin per projection class: a real booth's Perspective placeholder, and the custom
/// projection its first bake installs, which the twin booth
/// ([`crate::portrait::spawn_warm_booth`]) carries.
pub(super) type BoothCamQuery<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static bevy::camera::visibility::RenderLayers),
    With<crate::portrait::BoothCam>,
>;

/// Spawn one tiny quad per reachable pipeline variant and return the count. Materials come from
/// the production builders or live stores and meshes from the production builders, so neither
/// can drift from the real spawn paths.
pub(super) fn spawn_menagerie(
    commands: &mut Commands,
    cam: Entity,
    booth: Option<(Entity, &bevy::camera::visibility::RenderLayers)>,
    warm_booth: &(Entity, bevy::camera::visibility::RenderLayers),
    warm_ortho: &(Entity, bevy::camera::visibility::RenderLayers),
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WowModelMaterial>,
    lanes: &mut WarmLanes,
    cache: &mut MaterialCache,
    light: &Buffer,
) -> usize {
    // The model lane's four layouts (strides 40/56/64/80, every model mesh carrying a second UV
    // set): static and skinned, plain and vertex-coloured. Statics are render-world-only, so
    // their Aabb is inserted explicitly.
    let mut layouts: Vec<(Handle<Mesh>, Option<bevy::camera::primitives::Aabb>, bool)> = Vec::new();
    for colors in [false, true] {
        let stat = wrathplus_assets::submesh_to_static_mesh(&warm_quad(colors, false));
        let aabb = stat.compute_aabb();
        layouts.push((meshes.add(stat), aabb, false));
        let skin = wrathplus_assets::submesh_to_skinned_mesh(&warm_quad(colors, true));
        layouts.push((meshes.add(skin), None, true));
    }

    // The full cross is deliberate: every branch is authorable in an M2 or WMO (blends, the
    // 0x10/0x08 depth flags, sidedness).
    let mut mats: Vec<Handle<WowModelMaterial>> = Vec::new();
    // The sky lane stays apart: static layout only, never far-twinned or on a booth.
    let mut sky_mats: Vec<Handle<WowModelMaterial>> = Vec::new();
    for two_sided in [false, true] {
        for blend in [
            ModelBlend::Opaque,
            ModelBlend::AlphaTest,
            ModelBlend::Blend,
            ModelBlend::Mod,
            ModelBlend::Mod2x,
        ] {
            for no_depth_write in [false, true] {
                for no_depth_test in [false, true] {
                    mats.push(model_material(
                        cache,
                        materials,
                        None,
                        blend,
                        two_sided,
                        false,
                        false,
                        false,
                        false,
                        false,
                        no_depth_write,
                        no_depth_test,
                        FogPolicy::Scene,
                        // Not a `WowModelKey` axis: it swaps a sampled UV, not pipeline state.
                        false,
                        ShadeSel::Lit,
                        0,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                        false, // the world lane
                        light,
                        // The shared batch material; a per-placement clone has the same
                        // pipeline.
                        None,
                    ));
                }
            }
        }
        // The additive glow-card blend and the distance-fade blend, each over the full
        // depth-flag cross: the builders forward the batch's 0x10/0x08 flags into both.
        for additive_not_fade in [true, false] {
            for no_depth_write in [false, true] {
                for no_depth_test in [false, true] {
                    mats.push(model_material(
                        cache,
                        materials,
                        None,
                        ModelBlend::Blend,
                        two_sided,
                        false,
                        false,
                        false,
                        additive_not_fade,
                        !additive_not_fade,
                        no_depth_write,
                        no_depth_test,
                        FogPolicy::Scene,
                        // Not a `WowModelKey` axis: it swaps a sampled UV, not pipeline state.
                        false,
                        ShadeSel::Lit,
                        0,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                        false, // the world lane
                        light,
                        None,
                    ));
                }
            }
        }
        // The depth-prime twin (colour writes masked off), plain and cutout.
        for cutout in [false, true] {
            mats.push(zfill_material(
                cache, materials, None, two_sided, cutout, light,
            ));
        }
        // The WMO-skybox lane: `sky_depth` is a `WowModelKey` axis (the forced-far-depth branch).
        // Built as `M2BatchMaterials::skybox` builds it: depth-write off, depth-test on, every
        // blend and sidedness.
        for blend in [
            ModelBlend::Opaque,
            ModelBlend::AlphaTest,
            ModelBlend::Blend,
            ModelBlend::Mod,
            ModelBlend::Mod2x,
        ] {
            for additive in [false, true] {
                sky_mats.push(model_material(
                    cache,
                    materials,
                    None,
                    blend,
                    two_sided,
                    false,
                    false,
                    true, // every shipped skybox batch is unlit
                    additive,
                    false,
                    true,  // depth-write off
                    false, // depth-test on
                    FogPolicy::Off,
                    false,
                    ShadeSel::Lit,
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                    false,
                    true, // the sky lane
                    light,
                    None, // the shared lane
                ));
            }
        }
    }
    // The ground-clutter lane, both sidednesses and all three alpha modes its builder maps to
    // (Opaque, Mask, Blend). The pipeline sees only key bits, so `clutter_fade` is armed on a
    // copy of the plain material, leaving the dedup cache's entry untouched.
    for two_sided in [false, true] {
        for blend in [ModelBlend::Opaque, ModelBlend::AlphaTest, ModelBlend::Blend] {
            let plain = model_material(
                cache,
                materials,
                None,
                blend,
                two_sided,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                FogPolicy::Scene,
                // Not a `WowModelKey` axis: it swaps a sampled UV, not pipeline state.
                false,
                ShadeSel::Lit,
                0,
                None,
                None,
                None,
                None,
                None,
                false,
                false, // the world lane
                light,
                None, // the shared lane
            );
            // `model_render::lazy` parks a built material until something binds it.
            wrathplus_world::model_render::lazy::realize_all(materials);
            if let Some(m) = materials.get(&plain) {
                let mut m = m.clone();
                m.extension.clutter_fade = Vec4::new(52.5, 70.0, 0.0, 1.0);
                let clutter = materials.add(m);
                mats.push(clutter);
            }
        }
    }

    // The shard-rung rows: a model particle's `depth_bias` carries its owner-last rung, a key
    // axis, so the runtime stamps only the closed bucket set and this compiles it. Every 1.12.1
    // shard batch is Blend, either sidedness, either additive, no depth write (`wrathplus-extract
    // shardcensus`); built as bias-stamped copies as the runtime does.
    let mut shard_mats: Vec<Handle<WowModelMaterial>> = Vec::new();
    for &bucket in wrathplus_formats::OWNER_RUNG_BUCKETS.iter() {
        for two_sided in [false, true] {
            for additive in [false, true] {
                for no_depth_write in [false, true] {
                    let h = model_material(
                        cache,
                        materials,
                        None,
                        ModelBlend::Blend,
                        two_sided,
                        false,
                        false,
                        false,
                        additive,
                        false,
                        no_depth_write,
                        false,
                        FogPolicy::Scene,
                        // Not a `WowModelKey` axis: it swaps a sampled UV, not pipeline state.
                        false,
                        ShadeSel::Lit,
                        0,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                        false, // the world lane
                        light,
                        None, // the shared lane
                    );
                    wrathplus_world::model_render::lazy::realize_all(materials);
                    if let Some(m) = materials.get(&h) {
                        let mut m = m.clone();
                        m.base.depth_bias = bucket;
                        shard_mats.push(materials.add(m));
                    }
                }
            }
        }
    }

    // The far-side-of-water twins: `classify_water_side` swaps a transparent material for its
    // `far_twin_of`, a distinct pipeline key the cache never dedups against the near one.
    let far_mats = far_twins_of(materials, &mats);
    let far_shard_mats = far_twins_of(materials, &shard_mats);

    let mut count = 0;
    // The main cross and its far twins ride every layout, on the world camera, one real booth and
    // the twin booth; the orthographic twin (the UI model tile camera) takes the main cross only,
    // as no water classify runs over a tile. Shard rows ride the static layouts on the world
    // camera only: shard models are static and never on a booth.
    for (mesh, aabb, skinned) in &layouts {
        for mat in mats.iter().chain(far_mats.iter()) {
            spawn_model_rig(commands, cam, None, mesh, aabb, *skinned, mat);
            count += 1;
            if let Some((booth_cam, layers)) = booth {
                spawn_model_rig(
                    commands,
                    booth_cam,
                    Some(layers.clone()),
                    mesh,
                    aabb,
                    *skinned,
                    mat,
                );
                count += 1;
            }
            spawn_model_rig(
                commands,
                warm_booth.0,
                Some(warm_booth.1.clone()),
                mesh,
                aabb,
                *skinned,
                mat,
            );
            count += 1;
        }
        for mat in &mats {
            spawn_model_rig(
                commands,
                warm_ortho.0,
                Some(warm_ortho.1.clone()),
                mesh,
                aabb,
                *skinned,
                mat,
            );
            count += 1;
        }
        if !*skinned {
            for mat in shard_mats.iter().chain(far_shard_mats.iter()) {
                spawn_model_rig(commands, cam, None, mesh, aabb, false, mat);
                count += 1;
            }
        }
    }

    // The merged-blob layouts (`WOW_STATIC_MERGE`): the static layouts plus the baked fade
    // sphere and the interior-prop probe slot, strides 48/52 and 64/68, keyed by the
    // `WOW_MERGED_FADE`/`WOW_MERGED_SLOT` defs. World camera only: a blob never far-twins or
    // reaches a booth.
    for colors in [false, true] {
        for slot in [false, true] {
            let part = std::sync::Arc::new(warm_quad(colors, false));
            let (mesh, mn, mx, _center) = wrathplus_assets::merged_static_mesh_faded(
                &[(part, Transform::IDENTITY)],
                &[Vec4::new(0.0, 0.0, 0.0, 1.0)],
                slot.then_some(&[0u32][..]),
            );
            let aabb = Some(bevy::camera::primitives::Aabb::from_min_max(mn, mx));
            let mesh = meshes.add(mesh);
            for mat in &mats {
                spawn_model_rig(commands, cam, None, &mesh, &aabb, false, mat);
                count += 1;
            }
        }
    }

    // The skybox rows and the sky and water lanes draw with no second UV set: the static plain
    // and colour layouts without it.
    let [plain, colours] = [0, 2].map(|i| {
        let mut m = meshes.get(&layouts[i].0).expect("added above").clone();
        m.remove_attribute(Mesh::ATTRIBUTE_UV_1);
        (meshes.add(m), layouts[i].1)
    });
    // The WMO-skybox rows: `skybox::build_skybox` inserts POSITION + NORMAL + UV_0, exactly
    // `plain`, on the world camera only.
    {
        let (plain_mesh, plain_aabb) = &plain;
        for mat in &sky_mats {
            spawn_model_rig(commands, cam, None, plain_mesh, plain_aabb, false, mat);
            count += 1;
        }
    }

    // The sky and water lanes: every material in these stores gets a rig with its production
    // layout, without the model lane's second UV set.
    let (plain_mesh, plain_aabb) = plain;
    let (colours_mesh, colours_aabb) = colours;
    let posuv = meshes.add(warm_pos_uv_mesh());
    let liquid_mesh = meshes.add(warm_liquid_mesh(false));
    let liquid_color_mesh = meshes.add(warm_liquid_mesh(true));
    // Celestial discs and glares (`sun::setup` quads: position, normal, UV).
    for mat in lane_handles(&mut lanes.celestial) {
        spawn_lane_rig(
            commands,
            cam,
            None,
            &plain_mesh,
            plain_aabb.as_ref(),
            mat,
            &mut count,
        );
    }
    // Stars: `Stars.m2` patches carry position and UV; the assetless fallback adds normals.
    for mat in lane_handles(&mut lanes.stars) {
        spawn_lane_rig(commands, cam, None, &posuv, None, mat.clone(), &mut count);
        spawn_lane_rig(
            commands,
            cam,
            None,
            &plain_mesh,
            plain_aabb.as_ref(),
            mat,
            &mut count,
        );
    }
    // The cloud dome (position, normal, UV, colour) and the gradient dome (without colour).
    for mat in lane_handles(&mut lanes.clouds) {
        spawn_lane_rig(
            commands,
            cam,
            None,
            &colours_mesh,
            colours_aabb.as_ref(),
            mat,
            &mut count,
        );
    }
    for mat in lane_handles(&mut lanes.sky) {
        spawn_lane_rig(
            commands,
            cam,
            None,
            &plain_mesh,
            plain_aabb.as_ref(),
            mat,
            &mut count,
        );
    }
    // Liquid: every material on both grid layouts. An interior WMO pool bakes `MOMT.diffColor`
    // into `ATTRIBUTE_COLOR`, read behind `#ifdef VERTEX_COLORS` (`liquid.wgsl:275`), a separate
    // pipeline.
    for mat in lane_handles(&mut lanes.liquid) {
        spawn_lane_rig(
            commands,
            cam,
            None,
            &liquid_mesh,
            None,
            mat.clone(),
            &mut count,
        );
        spawn_lane_rig(
            commands,
            cam,
            None,
            &liquid_color_mesh,
            None,
            mat,
            &mut count,
        );
    }
    // The plain-`StandardMaterial` lanes. The fallback cube, drawn while a model streams, goes on
    // the world camera and the booths but not the tile camera, as `ui_models` has no cube. The
    // nameplate and raid-mark materials are representatives with their builders' key fields.
    if let Some(cubes) = lanes.cubes.as_ref() {
        let (cube_mesh, cube_mats) = cubes.warm_parts();
        for mat in cube_mats {
            spawn_lane_rig(
                commands,
                cam,
                None,
                &cube_mesh,
                None,
                mat.clone(),
                &mut count,
            );
            if let Some((booth_cam, layers)) = booth {
                spawn_lane_rig(
                    commands,
                    booth_cam,
                    Some(layers.clone()),
                    &cube_mesh,
                    None,
                    mat.clone(),
                    &mut count,
                );
            }
            spawn_lane_rig(
                commands,
                warm_booth.0,
                Some(warm_booth.1.clone()),
                &cube_mesh,
                None,
                mat,
                &mut count,
            );
        }
    }
    let plate = lanes.standard.add(StandardMaterial {
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        depth_bias: wrathplus_world::sky_order::Rung::NAMEPLATE,
        ..default()
    });
    spawn_lane_rig(
        commands,
        cam,
        None,
        &plain_mesh,
        plain_aabb.as_ref(),
        plate,
        &mut count,
    );
    let mark = lanes.standard.add(StandardMaterial {
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    spawn_lane_rig(
        commands,
        cam,
        None,
        &plain_mesh,
        plain_aabb.as_ref(),
        mark,
        &mut count,
    );

    // The minimap interior composite's tile quad: a `Material2d` pipeline is keyed on the mesh
    // layout too, and its `Rectangle` differs from the HUD batch mesh. The composite's camera is
    // inactive outside a WMO interior, but its view key equals the player-UI camera's, so this
    // draws through that one.
    //
    // A real stand-in texture: an unresolved `#[texture]` handle returns `RetryNextUpdate`, and a
    // rig that never prepares warms nothing.
    let tile_tex = lanes.images.add(Image::default());
    commands.spawn((
        Mesh2d(meshes.add(crate::overlay::tile_quad_mesh())),
        MeshMaterial2d(
            lanes
                .ui_quads
                .add(crate::overlay::UiQuadMaterial::interior_tile(
                    tile_tex,
                    crate::minimap::INTERIOR_TILE_ALPHA_REF,
                )),
        ),
        // Visible, unlike the paced model rigs: one pipeline, drawn at a negligible scale.
        Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::splat(0.001)),
        crate::overlay::ui_render_layers(),
        WarmRig,
    ));
    count += 1;

    count
}

/// The far twins of every transparent material in `src`, by `classify_water_side`'s own
/// predicate and builder.
fn far_twins_of(
    materials: &mut Assets<WowModelMaterial>,
    src: &[Handle<WowModelMaterial>],
) -> Vec<Handle<WowModelMaterial>> {
    wrathplus_world::model_render::lazy::realize_all(materials);
    let twins: Vec<WowModelMaterial> = src
        .iter()
        .filter_map(|h| materials.get(h))
        .filter(|m| matches!(m.base.alpha_mode, AlphaMode::Blend))
        .map(far_twin_of)
        .collect();
    twins.into_iter().map(|m| materials.add(m)).collect()
}

/// Strong handles to every material in a lane's store.
fn lane_handles<M: Material>(assets: &mut Assets<M>) -> Vec<Handle<M>> {
    let ids: Vec<AssetId<M>> = assets.iter().map(|(id, _)| id).collect();
    ids.into_iter()
        .filter_map(|id| assets.get_strong_handle(id))
        .collect()
}

/// One model-lane rig, 1 cm in front of `cam`; `layers` puts it on a booth camera's layer.
fn spawn_model_rig(
    commands: &mut Commands,
    cam: Entity,
    layers: Option<bevy::camera::visibility::RenderLayers>,
    mesh: &Handle<Mesh>,
    aabb: &Option<bevy::camera::primitives::Aabb>,
    skinned: bool,
    mat: &Handle<WowModelMaterial>,
) {
    let tag = if skinned {
        MeshTag(wrathplus_world::mesh_tag::rig_bits(0) | wrathplus_world::mesh_tag::alpha_bits(1.0))
    } else {
        MeshTag(wrathplus_world::mesh_tag::alpha_bits(1.0))
    };
    let mut e = commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(mat.clone()),
        Transform::from_xyz(0.0, 0.0, -0.5).with_scale(Vec3::splat(0.01)),
        tag,
        WarmRig,
        // Hidden at spawn, so it queues no pipeline until `super::reveal_slice` reveals it.
        Visibility::Hidden,
        ChildOf(cam),
    ));
    if let Some(aabb) = aabb {
        e.insert(*aabb);
    }
    if let Some(layers) = layers {
        e.insert(layers);
    }
}

/// One sky, water or standard-lane rig: the same rig without a `MeshTag`, as in production.
fn spawn_lane_rig<M: Material>(
    commands: &mut Commands,
    cam: Entity,
    layers: Option<bevy::camera::visibility::RenderLayers>,
    mesh: &Handle<Mesh>,
    aabb: Option<&bevy::camera::primitives::Aabb>,
    mat: Handle<M>,
    count: &mut usize,
) {
    let mut e = commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(mat),
        Transform::from_xyz(0.0, 0.0, -0.5).with_scale(Vec3::splat(0.01)),
        WarmRig,
        // Hidden at spawn, revealed a slice at a time.
        Visibility::Hidden,
        ChildOf(cam),
    ));
    if let Some(aabb) = aabb {
        e.insert(*aabb);
    }
    if let Some(layers) = layers {
        e.insert(layers);
    }
    *count += 1;
}

#[cfg(test)]
mod tests {
    /// Every hand-rolled `Specialized*Pipeline` impl's type must be named in the pipe_warm
    /// module, comments included.
    #[test]
    fn every_custom_pipeline_lane_has_a_warm_contributor() {
        // Lanes whose one pipeline compiles covered by construction:
        // - UiGammaPipeline (`ui_gamma`): one variant keyed on the swapchain format, specialised
        //   on the first frame, pre-world, and the format never changes.
        // `FfxCombinePipeline` compiles covered the same way (first frame, pre-world, a bake's
        // fixed format; a world view's pair every frame in `prepare_textures`); this mention of
        // its name is what passes it here.
        let exempt = ["UiGammaPipeline"];
        let own_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let warm_src = std::fs::read_to_string(own_src.join("pipe_warm/mod.rs")).unwrap()
            + &std::fs::read_to_string(own_src.join("pipe_warm/menagerie.rs")).unwrap();
        let mut missing = Vec::new();
        let files: Vec<_> = workspace_src_roots()
            .iter()
            .flat_map(|r| walk_rs(r))
            .collect();
        for (path, text) in files {
            for needle in [
                "impl SpecializedRenderPipeline for ",
                "impl SpecializedMeshPipeline for ",
                "impl SpecializedComputePipeline for ",
            ] {
                for (i, _) in text.match_indices(needle) {
                    let rest = &text[i + needle.len()..];
                    let ty: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !exempt.contains(&ty.as_str()) && !warm_src.contains(&ty) {
                        missing.push(format!("{ty} (impl in {})", path.display()));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "custom pipeline lanes with no pipe_warm contributor: {missing:?} — a lane the \
             menagerie can't see compiles its pipelines live on first draw, a render-thread \
             stall"
        );
    }

    /// The scan roots: every crate's `src` in the workspace, since most lanes live in
    /// `wrathplus-world`.
    fn workspace_src_roots() -> Vec<std::path::PathBuf> {
        // `crates/wrathplus-app` -> `crates`, then every crate's `src` under it.
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CARGO_MANIFEST_DIR always has a parent")
            .to_path_buf();
        let mut roots: Vec<std::path::PathBuf> = std::fs::read_dir(&crates)
            .expect("the crates/ directory is always readable from a test")
            .filter_map(|e| {
                let p = e.ok()?.path().join("src");
                p.is_dir().then_some(p)
            })
            .collect();
        roots.sort();
        assert!(
            roots.len() > 1,
            "the lane scans must walk the whole workspace — a single root is the bug this \
             function exists to prevent (only {roots:?} found under {})",
            crates.display(),
        );
        roots
    }

    /// Every `.rs` file under `src`, read.
    fn walk_rs(src_root: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
        let mut out = Vec::new();
        let mut stack = vec![src_root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((path, text));
            }
        }
        out
    }

    /// Every registered 3-D, 2-D and UI material lane must be named in the pipe_warm module or
    /// exempt with a reason; `watch_pipelines` catches per-variant drift inside a lane.
    #[test]
    fn every_material_lane_has_a_warm_contributor() {
        // Lanes that never need the menagerie:
        // - TerrainMaterial / WdlMaterial: always drawn under the entry cover, no variant axis.
        // - AddUiMaterial: glue screens only, and every pre-world frame counts as covered.
        let families: [(&str, &[&str]); 3] = [
            ("MaterialPlugin::<", &["TerrainMaterial", "WdlMaterial"]),
            ("Material2dPlugin::<", &[]),
            ("UiMaterialPlugin::<", &["AddUiMaterial"]),
        ];
        let own_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // Anywhere in the pipe_warm module folder.
        let warm_src = std::fs::read_to_string(own_src.join("pipe_warm/mod.rs")).unwrap()
            + &std::fs::read_to_string(own_src.join("pipe_warm/menagerie.rs")).unwrap();
        let mut missing = Vec::new();
        let files: Vec<_> = workspace_src_roots()
            .iter()
            .flat_map(|r| walk_rs(r))
            .collect();
        for (path, text) in files {
            for (needle, exempt) in families {
                for (i, _) in text.match_indices(needle) {
                    // A preceding ident char means a longer family's name, such as
                    // `UiMaterialPlugin`, which has its own row.
                    if i > 0 && (text.as_bytes()[i - 1].is_ascii_alphanumeric()) {
                        continue;
                    }
                    let rest = &text[i + needle.len()..];
                    let Some(end) = rest.find('>') else { continue };
                    // A registration may wrap (`UiMaterialPlugin::<\n    AddUiMaterial,\n>`):
                    // strip the path, any trailing turbofish comma, and surrounding whitespace.
                    let ty = rest[..end]
                        .rsplit("::")
                        .next()
                        .unwrap()
                        .trim()
                        .trim_end_matches(',');
                    if exempt.contains(&ty) || warm_src.contains(ty) {
                        continue;
                    }
                    missing.push(format!("{ty} (registered in {})", path.display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "material lanes with no pipe_warm contributor: {missing:?} — every registered \
             lane's pipelines compile behind the loading cover, or its first sight is a live \
             render-thread stall"
        );
    }
}
