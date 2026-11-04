//! `wrathplus-assets`: the WoW 1.12.1 MPQ patch chain as an `mpq://` Bevy asset source backed by
//! [`wrathplus_formats::Chain`], and the per-format loaders (BLP, M2, WMO, ADT, WDT) on top of it.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, ErasedAssetReader, PathStream, Reader,
    VecReader,
};
use bevy::asset::AssetApp;
use bevy::prelude::*;
use wrathplus_formats::Chain;

pub mod column_grid;
pub mod coords;
pub mod materials;
pub mod minimap_grid;
mod spatial_cache;
pub mod trace;

mod anim_rng;
pub use anim_rng::AnimRng;
pub use spatial_cache::SpatialCache;
mod world_assets;
pub use world_assets::*;

mod model;
pub use model::{
    bone_target_id, merged_static_mesh_faded, submesh_to_skinned_mesh, submesh_to_static_mesh,
    AnimClip, BillboardInfo, ClipEvent, GlobalBone, GlobalSeqChannel, ModelAnimations,
    ModelAttachment, ModelJoint, ModelMarker, ModelSkeleton, ModelSubmesh, PoseBone, PoseClip,
    PoseNode, PoseSource, PoseTrack, ATTRIBUTE_WOW_FADE_SPHERE, ATTRIBUTE_WOW_JOINT_INDEX,
    ATTRIBUTE_WOW_JOINT_WEIGHT, ATTRIBUTE_WOW_MERGED_SLOT,
};
mod adt;
mod terrain;
mod wdt;
pub use adt::{chunk_to_mesh, chunks_to_mesh, AdtTile, ChunkShading, WrathAdtLoader};
pub use wdt::{WdtIndex, WdtIndexLoader};
mod blp;
pub use blp::{BlpImageLoader, BlpLoaderSettings, BlpVariant};
/// Whether this GPU takes WoW's stored DXT blocks, and the chain form that follows.
mod gpu_blp;
pub use gpu_blp::{bc_supported, for_upload, publish_bc_support, BlpGpuSupportPlugin, UploadChain};
mod tex_filter;
pub use tex_filter::{publish_tex_filter, tex_filter, TexFilterSetting, ANISO_RANGE};
mod m2;
pub use m2::{
    EmitterBillboard, M2Model, M2SequenceInfo, ModelEmitter, ModelLight, ModelRibbon,
    PortraitCamera, WrathM2ModelLoader,
};
mod wmo;
pub use wmo::{
    cap96, collision_tri_bounds, collision_tri_grids, floor168, footprint_tri_bounds,
    footprint_tri_grids, DoodadBase, WmoGroupNav, WmoLiquidState, WmoModel, WmoModelLoader,
};
pub use wrathplus_formats::{WmoPortalInfo, WmoPortalRef};

/// The asset-source id for MPQ-backed assets: load paths look like `mpq://World/Azeroth/foo.adt`.
pub const MPQ_SOURCE: &str = "mpq";

/// A Bevy [`AssetReader`] over the MPQ patch chain; every clone shares the one open [`Chain`].
#[derive(Clone)]
pub struct MpqAssetReader {
    chain: Arc<Chain>,
}

impl MpqAssetReader {
    /// Open the patch chain from a `Data` directory or a single `.MPQ`.
    pub fn open(data_dir: &Path) -> Result<Self> {
        Ok(Self {
            chain: Arc::new(Chain::open(data_dir)?),
        })
    }

    /// Open an explicitly selected client-data dialect.
    pub fn open_profile(data_dir: &Path, profile: &wrathplus_formats::DataProfile) -> Result<Self> {
        Ok(Self {
            chain: Arc::new(Chain::open_profile(data_dir, profile)?),
        })
    }
}

impl AssetReader for MpqAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let raw = path
            .to_str()
            .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()))?;
        // The sampler-mode marker (`texture_url`) names the asset, not the archive file.
        let stripped = strip_sampler_marker(raw);
        let internal = stripped.as_deref().unwrap_or(raw);
        if !self.chain.contains(internal) {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        }
        let bytes = self
            .chain
            .read(internal)
            .map_err(|e| AssetReaderError::Io(Arc::new(std::io::Error::other(format!("{e:#}")))))?;
        Ok(VecReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // MPQ assets carry no sidecar `.meta`; Bevy falls back to default meta on `NotFound`.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        // Loads are by explicit path; there is no directory listing.
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

/// The sampler-mode marker in an `mpq://` texture URL.
const SAMPLER_MARKER: char = '@';

/// The `mpq://` URL of a model texture at a sampler address mode. Bevy keys the sampler to the
/// `Image` and the `Image` to its path, so a mode other than repeat/repeat marks the path before
/// the extension (`…/leaves01@cc.blp`), which must stay `.blp` to pick the loader.
pub fn texture_url(internal: &str, wrap: (bool, bool)) -> String {
    let path = internal.replace('\\', "/").to_ascii_lowercase();
    if wrap == (true, true) {
        return format!("mpq://{path}");
    }
    let tag = match wrap {
        (true, false) => "rc",
        (false, true) => "cr",
        _ => "cc",
    };
    match path.rsplit_once('.') {
        Some((stem, ext)) => format!("mpq://{stem}{SAMPLER_MARKER}{tag}.{ext}"),
        None => format!("mpq://{path}{SAMPLER_MARKER}{tag}"),
    }
}

/// The `Map.dbc` [`MapCatalog`] (`mapId` to directory and `LoadingScreenID`) as a Bevy resource; a
/// newtype because the orphan rule forbids `Resource` on a foreign type.
#[derive(Resource)]
pub struct MapCatalogRes(pub wrathplus_formats::MapCatalog);

/// A model reference path (`.mdx`/`.mdl`, any case, backslashes) as its lowercase `mpq://…m2` URL:
/// the archive file is `.m2`, and one spelling per model keeps the handle dedup working.
pub fn m2_url(raw: &str) -> String {
    let p = raw.to_ascii_lowercase().replace('\\', "/");
    let stem = p
        .strip_suffix(".mdx")
        .or_else(|| p.strip_suffix(".mdl"))
        .or_else(|| p.strip_suffix(".m2"))
        .unwrap_or(&p);
    format!("mpq://{stem}.m2")
}

/// A WMO root path as its `mpq://…wmo` URL, lowercased for handle dedup.
pub fn wmo_url(raw: &str) -> String {
    format!("mpq://{}", raw.to_ascii_lowercase().replace('\\', "/"))
}

/// A creature skin variation's `mpq://` URL, `<model-dir>/<name>.blp`: the `Monster1/2/3` skins
/// live beside the model.
pub fn skin_url(model_dir: &str, name: &str) -> String {
    let dir = model_dir.replace('\\', "/").to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    if dir.is_empty() {
        format!("mpq://{name}.blp")
    } else {
        format!("mpq://{dir}/{name}.blp")
    }
}

/// The address mode encoded in an asset path by [`texture_url`]; repeat/repeat when unmarked.
pub fn sampler_mode_of(path: &str) -> (bool, bool) {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return (true, true);
    };
    match stem.rsplit_once(SAMPLER_MARKER) {
        Some((_, "rc")) => (true, false),
        Some((_, "cr")) => (false, true),
        Some((_, "cc")) => (false, false),
        _ => (true, true),
    }
}

/// The archive path under a [`texture_url`] marker; `None` when unmarked.
fn strip_sampler_marker(path: &str) -> Option<String> {
    let (stem, ext) = path.rsplit_once('.')?;
    let (base, tag) = stem.rsplit_once(SAMPLER_MARKER)?;
    matches!(tag, "rc" | "cr" | "cc").then(|| format!("{base}.{ext}"))
}

/// Immutable liquid metadata shared by WMO loaders, decoded once at source registration.
#[derive(Resource, Clone)]
struct WmoLiquidCatalogs(Arc<wrathplus_formats::wrath_liquid::LiquidCatalogs>);

/// The explicitly selected archive/schema dialect used by the loaders.
#[derive(Resource, Clone, Default)]
pub struct ClientDataProfile(pub wrathplus_formats::DataProfile);

/// Explicit boot policy shared by the asset source and the world's independent chain.
#[derive(Resource, Clone, Default)]
pub struct AssetOverridePolicy(pub wrathplus_formats::AssetOverrides);

/// Register the `mpq://` source on `app`, backed by the patch chain at `data_dir`. Call it before
/// `AssetPlugin` builds (before `DefaultPlugins`), which reads the sources.
pub fn register_mpq_source(app: &mut App, data_dir: &Path) -> Result<()> {
    register_mpq_source_profile(app, data_dir, &wrathplus_formats::DataProfile::default())
}

/// Register the explicitly selected archive dialect before Bevy's asset plugin builds.
pub fn register_mpq_source_profile(
    app: &mut App,
    data_dir: &Path,
    profile: &wrathplus_formats::DataProfile,
) -> Result<()> {
    register_mpq_source_overrides(
        app,
        data_dir,
        profile,
        wrathplus_formats::AssetOverrides::default(),
    )
}

/// Register a source with the same explicit policy used by direct world readers.
pub fn register_mpq_source_overrides(
    app: &mut App,
    data_dir: &Path,
    profile: &wrathplus_formats::DataProfile,
    overrides: wrathplus_formats::AssetOverrides,
) -> Result<()> {
    let mut chain = Chain::open_profile(data_dir, profile)?.with_overrides(overrides);
    // Decode once from the exact boot chain (including explicitly enabled overrides),
    // never from environment variables or a separately reopened archive chain.
    if matches!(profile, wrathplus_formats::DataProfile::Wrath { .. }) {
        let catalogs = wrathplus_formats::wrath_liquid::load_liquid_catalogs(&mut chain)?;
        app.insert_resource(WmoLiquidCatalogs(Arc::new(catalogs)));
    }
    let reader = MpqAssetReader {
        chain: Arc::new(chain),
    };
    app.insert_resource(ClientDataProfile(profile.clone()));
    app.register_asset_source(
        MPQ_SOURCE,
        AssetSourceBuilder::new(move || -> Box<dyn ErasedAssetReader> { Box::new(reader.clone()) }),
    );
    Ok(())
}

/// Register WrathPlus's asset loaders on `app`, after `AssetPlugin`, into its live `AssetServer`.
pub fn register_asset_loaders(app: &mut App) {
    app.init_asset::<M2Model>();
    app.init_asset::<WmoModel>();
    app.init_asset::<AdtTile>();
    app.init_asset::<WdtIndex>();
    // A plugin: whether the device takes DXT blocks is known only after `RenderPlugin::finish`.
    app.add_plugins(BlpGpuSupportPlugin);
    app.register_asset_loader(BlpImageLoader);
    let profile = app
        .world()
        .get_resource::<ClientDataProfile>()
        .cloned()
        .unwrap_or_default();
    app.register_asset_loader(WrathM2ModelLoader);
    app.register_asset_loader(WrathAdtLoader);
    let catalogs = app
        .world()
        .get_resource::<WmoLiquidCatalogs>()
        .map(|c| c.0.clone());
    app.register_asset_loader(WmoModelLoader {
        wrath: matches!(profile.0, wrathplus_formats::DataProfile::Wrath { .. }),
        catalogs,
    });
    app.register_asset_loader(WdtIndexLoader);
    // The materials' WGSL, compiled in; it too needs `AssetPlugin` first.
    materials::register_shaders(app);
}

// Installed-data pack replacement of a real model and texture (Experience, WS7).
#[cfg(test)]
mod override_real_data;

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::primitives::MeshAabb;
    use bevy::tasks::block_on;

    #[test]
    fn sampler_mode_rides_the_asset_path_and_round_trips() {
        let tex = "World\\KhazModan\\Ironforge\\PassiveDoodads\\Trees\\IronForgeleaves01.blp";
        let repeat = texture_url(tex, (true, true));
        assert_eq!(
            repeat,
            "mpq://world/khazmodan/ironforge/passivedoodads/trees/ironforgeleaves01.blp"
        );
        assert_eq!(sampler_mode_of(&repeat), (true, true));
        // Every other mode marks the stem, so `.blp` still picks the loader.
        for wrap in [(false, false), (true, false), (false, true)] {
            let url = texture_url(tex, wrap);
            assert!(url.ends_with(".blp"), "extension must survive: {url}");
            assert_ne!(url, repeat, "a marked mode is a distinct asset path");
            assert_eq!(sampler_mode_of(&url), wrap, "round-trip {wrap:?}");
            assert_eq!(
                strip_sampler_marker(url.strip_prefix("mpq://").unwrap()).as_deref(),
                Some("world/khazmodan/ironforge/passivedoodads/trees/ironforgeleaves01.blp"),
            );
        }
        // A bare path has nothing to strip, and an unrelated `@` is not a marker.
        assert_eq!(strip_sampler_marker("world/foo.blp"), None);
        assert_eq!(strip_sampler_marker("world/foo@bar.blp"), None);
        assert_eq!(sampler_mode_of("world/foo@bar.blp"), (true, true));
    }

    #[test]
    fn pack_texture_reaches_streamed_tile_and_doodad_consumers() {
        let root = std::env::temp_dir().join(format!("wp-pack-image-{}", std::process::id()));
        std::fs::create_dir_all(root.join("World")).unwrap();
        // One 4x4 DXT1 block, solid red, in a synthetic BLP2 envelope.
        let mut bytes = vec![0u8; 148 + 1024];
        bytes[..4].copy_from_slice(b"BLP2");
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes[8] = 2;
        bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&4u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&1172u32.to_le_bytes());
        bytes[84..88].copy_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 248, 0, 0, 0, 0, 0, 0]);
        std::fs::write(root.join("World/Doodad.blp"), bytes).unwrap();
        // A real ADT loader consumer, not merely another direct byte read.
        fn chunk(magic: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut bytes = magic.to_vec();
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(payload);
            bytes
        }
        let mut mcnk = vec![0u8; 128];
        mcnk[0x14..0x18].copy_from_slice(&128u32.to_le_bytes());
        mcnk[0x1c..0x20].copy_from_slice(&716u32.to_le_bytes());
        mcnk.extend(chunk(b"TVCM", &[0u8; 145 * 4]));
        mcnk.extend(chunk(b"YLCM", &[0u8; 16]));
        let mut tile = chunk(b"XETM", b"World/Doodad.blp\0");
        tile.extend(chunk(b"KNCM", &mcnk));
        // A build-12340 tile is `World/Maps/{map}/{map}_x_y.adt` beside its `{map}.wdt`.
        std::fs::create_dir_all(root.join("World/Maps/Test")).unwrap();
        std::fs::write(root.join("World/Maps/Test/Test_32_32.adt"), tile).unwrap();
        std::fs::write(
            root.join("World/Maps/Test/Test.wdt"),
            chunk(b"DHPM", &[0u8; 32]),
        )
        .unwrap();
        let policy = wrathplus_formats::AssetOverrides::from_roots([root.clone()]).unwrap();
        let reader = MpqAssetReader {
            chain: Arc::new(Chain::from_overrides(
                policy,
                wrathplus_formats::DataProfile::default(),
            )),
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.register_asset_source(
            MPQ_SOURCE,
            AssetSourceBuilder::new(move || -> Box<dyn ErasedAssetReader> {
                Box::new(reader.clone())
            }),
        );
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        register_asset_loaders(&mut app);
        app.init_asset::<Mesh>();
        let tile: Handle<AdtTile> = app
            .world()
            .resource::<AssetServer>()
            .load("mpq://World/Maps/Test/Test_32_32.adt");
        // Exactly the sampler-marked path that streamed M2/WMO doodads request.
        let handle: Handle<Image> = app
            .world()
            .resource::<AssetServer>()
            .load(texture_url("WORLD\\DOODAD.BLP", (false, false)));
        for _ in 0..500 {
            app.update();
            if app.world().resource::<Assets<Image>>().contains(&handle)
                && app.world().resource::<Assets<AdtTile>>().contains(&tile)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let tiles = app.world().resource::<Assets<AdtTile>>();
        let tile = tiles
            .get(&tile)
            .expect("pack-only tile streams through WrathAdtLoader");
        assert_eq!(
            tile.shading[0].layers[0], 1,
            "pack texture wins over fallback layer 0"
        );
        let images = app.world().resource::<Assets<Image>>();
        let layers = images
            .get(&tile.layer_array)
            .expect("tile authored layers upload");
        assert_eq!(layers.texture_descriptor.size.depth_or_array_layers, 2);
        // Layer zero is the stock green fallback; layer one is our red pack BLP.
        // With no authored _s variant, ADT packing clears alpha for the matte sheen mask.
        let layer_stride = layers.data.as_ref().unwrap().len() / 2;
        assert_eq!(
            &layers.data.as_ref().unwrap()[layer_stride..layer_stride + 4],
            &[255, 0, 0, 0]
        );
        let image = images
            .get(&handle)
            .expect("pack-only doodad texture uploads through AssetServer");
        assert_eq!(image.width(), 4);
        assert_eq!(image.height(), 4);
        assert_eq!(&image.data.as_ref().unwrap()[..4], &[255, 0, 0, 255]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mpq_reader_loads_real_client_bytes_through_the_assetreader() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let reader = MpqAssetReader::open(&data).expect("open mpq reader");

        let bytes = block_on(async {
            let mut r = AssetReader::read(&reader, Path::new("DBFilesClient/Spell.dbc"))
                .await
                .expect("read Spell.dbc via AssetReader");
            let mut buf = Vec::new();
            r.read_to_end(&mut buf).await.expect("drain reader");
            buf
        });
        assert_eq!(&bytes[..4], b"WDBC", "Spell.dbc starts with the WDBC magic");
        assert!(bytes.len() > 1_000_000, "Spell.dbc should be sizable");

        // A missing path is `NotFound`, which Bevy's meta fallback relies on.
        let missing = block_on(AssetReader::read(&reader, Path::new("does/not/exist.blp")));
        assert!(
            matches!(missing, Err(AssetReaderError::NotFound(_))),
            "missing path should be NotFound"
        );
    }

    #[test]
    fn loads_a_blp_image_through_the_full_mpq_pipeline() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_mpq_source(&mut app, &data).expect("register mpq source");
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        register_asset_loaders(&mut app);

        let handle: Handle<Image> = app
            .world()
            .resource::<AssetServer>()
            .load("mpq://Interface/Icons/Spell_Holy_ArcaneIntellect.blp");

        let mut got = None;
        // A generous ceiling: a parallel gate run starves the IO pool.
        for _ in 0..15_000 {
            app.update();
            if let Some(img) = app.world().resource::<Assets<Image>>().get(&handle) {
                got = Some((
                    img.width(),
                    img.height(),
                    img.texture_descriptor.mip_level_count,
                ));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (w, h, mips) =
            got.expect("the spell icon should load via mpq:// + BlpImageLoader within 300 ticks");
        assert_eq!((w, h), (64, 64), "spell icon is 64x64");
        assert!(
            mips >= 1,
            "authored mip levels should be present, got {mips}"
        );
    }

    /// The minimap streams its tiles with `load_with_settings(Sprite)`; the UI pass wants sRGB, and
    /// the `WorldArt` default would draw every tile about twice as bright.
    #[test]
    fn minimap_tile_settings_reach_the_async_loader() {
        use bevy::render::render_resource::TextureFormat;
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_mpq_source(&mut app, &data).expect("register mpq source");
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        register_asset_loaders(&mut app);

        // In isolation: a bare `load()` of the same path would share the handle and its settings.
        let mut chain = wrathplus_formats::Chain::open(&data).expect("open chain");
        let hash = wrathplus_formats::load_minimap_translate(&mut chain)
            .expect("md5translate.trs")
            .tile("Azeroth", 32, 48)
            .expect("Azeroth 32_48 has a tile")
            .to_string();
        let tile = format!("mpq://textures/Minimap/{hash}");
        let server = app.world().resource::<AssetServer>().clone();
        let sprite_h: Handle<Image> =
            server.load_with_settings(tile, |s: &mut BlpLoaderSettings| {
                s.variant = BlpVariant::Sprite;
            });

        let mut sprite_fmt = None;
        // A generous ceiling: a parallel gate run starves the IO pool.
        for _ in 0..15_000 {
            app.update();
            if let Some(img) = app.world().resource::<Assets<Image>>().get(&sprite_h) {
                sprite_fmt = Some(img.texture_descriptor.format);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        eprintln!("Sprite settings (isolated) -> {sprite_fmt:?}");
        assert_eq!(
            sprite_fmt,
            Some(TextureFormat::Rgba8UnormSrgb),
            "load_with_settings(Sprite) must produce an sRGB tile on the async mpq:// path"
        );
    }

    /// Ironforge's interior minimap tile counts per group, against `md5translate.trs`.
    #[test]
    fn ironforge_group_grid_matches_trs() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_mpq_source(&mut app, &data).expect("mpq");
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        register_asset_loaders(&mut app);
        let h: Handle<WmoModel> = app
            .world()
            .resource::<AssetServer>()
            .load("mpq://World/wmo/KhazModan/Cities/Ironforge/Ironforge.wmo");
        let mut model = None;
        // About 30 s, though a healthy run takes under one: a parallel gate run starves IO.
        for _ in 0..15_000 {
            app.update();
            if let Some(m) = app.world().resource::<Assets<WmoModel>>().get(&h) {
                model = Some(m.clone());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let m = model.expect("Ironforge loads");
        // (group, X tiles, Y tiles), read off the installed trs tile names.
        let mut chain = wrathplus_formats::Chain::open(&data).expect("open chain");
        let trs = wrathplus_formats::load_minimap_translate(&mut chain).expect("md5translate.trs");
        let tiles = |g: usize, axis: usize| {
            (0..8)
                .filter(|&n| {
                    let (x, y) = if axis == 0 { (n, 0) } else { (0, n) };
                    trs.get(&format!(
                        "wmo\\KhazModan\\Cities\\Ironforge\\ironforge_{g:03}_{x:02}_{y:02}.blp"
                    ))
                    .is_some()
                })
                .count()
        };
        let grid = |ext: f32| crate::minimap_grid::group_axis_grid(ext).0;
        for g in [1usize, 2, 10, 44, 66, 89] {
            let (ex_n, ey_n) = (tiles(g, 0) as u32, tiles(g, 1) as u32);
            let gn = m.group_nav.get(g).expect("group present");
            let nx = grid(gn.bbox_max[0] - gn.bbox_min[0]);
            let ny = grid(gn.bbox_max[1] - gn.bbox_min[1]);
            assert_eq!((nx, ny), (ex_n, ey_n), "group {g} grid");
        }
    }

    #[test]
    fn loads_an_m2_model_through_the_pipeline() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_mpq_source(&mut app, &data).expect("register mpq source");
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        // The M2 loader emits inverse bind poses, an `AnimationClip` and an `AnimationGraph`, which
        // `DefaultPlugins` would register.
        app.init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        app.init_asset::<bevy::animation::AnimationClip>();
        app.init_asset::<bevy::animation::graph::AnimationGraph>();
        register_asset_loaders(&mut app);

        // The campfire: a doodad with embedded textures.
        let handle: Handle<M2Model> = app
            .world()
            .resource::<AssetServer>()
            .load("mpq://World/Azeroth/Elwynn/PassiveDoodads/Campfire/ElwynnCampfire.m2");

        let mut info = None;
        for _ in 0..600 {
            app.update();
            if let Some(m) = app.world().resource::<Assets<M2Model>>().get(&handle) {
                let geometries: Vec<_> = m.submeshes.iter().map(|s| s.geometry.clone()).collect();
                let textured = m.submeshes.iter().filter(|s| s.texture.is_some()).count();
                info = Some((geometries, textured, m.bounds.is_some()));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (geometries, textured, has_bounds) =
            info.expect("the campfire M2 should load via mpq:// within 600 ticks");
        assert!(
            !geometries.is_empty(),
            "campfire should have render batches"
        );
        assert!(has_bounds, "M2 carries authored bounds");
        assert!(textured > 0, "campfire batches reference embedded textures");

        // The loader ships geometry; the spawn side inserts the static form's Aabb itself, as
        // `RENDER_WORLD` meshes race `calculate_bounds`.
        for g in &geometries {
            let mesh = submesh_to_static_mesh(g);
            assert!(mesh.count_vertices() > 0, "submesh has vertices");
            assert!(mesh.compute_aabb().is_some(), "static form yields an Aabb");
            let skinned = submesh_to_skinned_mesh(g);
            assert_eq!(
                skinned.count_vertices(),
                mesh.count_vertices(),
                "the skinned twin bakes the same geometry"
            );
        }
    }

    #[test]
    fn loads_a_wmo_model_through_the_pipeline() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_mpq_source(&mut app, &data).expect("register mpq source");
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        register_asset_loaders(&mut app);

        // The Goldshire Inn: a root and its group files.
        let handle: Handle<WmoModel> = app
            .world()
            .resource::<AssetServer>()
            .load("mpq://World/wmo/Azeroth/Buildings/GoldshireInn/GoldshireInn.wmo");

        let mut info = None;
        for _ in 0..600 {
            app.update();
            if let Some(m) = app.world().resource::<Assets<WmoModel>>().get(&handle) {
                let geometries: Vec<_> = m.submeshes.iter().map(|s| s.geometry.clone()).collect();
                let textured = m.submeshes.iter().filter(|s| s.texture.is_some()).count();
                info = Some((geometries, textured));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (geometries, textured) =
            info.expect("the Goldshire Inn WMO should load via mpq:// + WmoModelLoader");
        assert!(
            !geometries.is_empty(),
            "the inn has render batches across its groups"
        );
        assert!(textured > 0, "WMO batches reference textures");

        for g in &geometries {
            let mesh = submesh_to_static_mesh(g);
            assert!(mesh.count_vertices() > 0, "group submesh has vertices");
            assert!(mesh.compute_aabb().is_some(), "static form yields an Aabb");
        }
    }

    #[test]
    fn loads_an_adt_terrain_tile_through_the_pipeline() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let reader = wrathplus_formats::Chain::open(&data).expect("open chain");
        let mut url = None;
        'find: for tx in 28..36u32 {
            for ty in 46..52u32 {
                if reader.contains(&format!("World\\Maps\\Azeroth\\Azeroth_{tx}_{ty}.adt")) {
                    url = Some(format!("mpq://World/Maps/Azeroth/Azeroth_{tx}_{ty}.adt"));
                    break 'find;
                }
            }
        }
        let url = url.expect("an Azeroth ADT tile should exist near Elwynn");

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register_mpq_source(&mut app, &data).expect("register mpq source");
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        register_asset_loaders(&mut app);

        let handle: Handle<AdtTile> = app.world().resource::<AssetServer>().load(url);
        let mut info = None;
        for _ in 0..2000 {
            app.update();
            if let Some(t) = app.world().resource::<Assets<AdtTile>>().get(&handle) {
                info = Some((
                    t.chunks
                        .iter()
                        .zip(&t.shading)
                        .map(|(c, s)| (c.clone(), *s))
                        .collect::<Vec<_>>(),
                    t.shading.len(),
                    t.layer_array.clone(),
                    t.alpha_array.clone(),
                    t.shadow_array.clone(),
                    t.doodads.len(),
                ));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (cells, n_shading, layer_h, alpha_h, shadow_h, n_doodads) =
            info.expect("the ADT tile should load via mpq:// + WrathAdtLoader");

        // One mesh per drawn MCNK chunk, a 145-vertex 9×9 + 8×8 grid.
        assert_eq!(n_shading, cells.len(), "one ChunkShading per decoded chunk");
        let drawn: Vec<Mesh> = cells
            .iter()
            .filter_map(|(c, s)| chunk_to_mesh(c, s))
            .collect();
        assert!(
            (1..=256).contains(&drawn.len()),
            "a tile draws as its MCNK cells, got {} meshes",
            drawn.len()
        );
        for mesh in &drawn {
            assert_eq!(
                mesh.count_vertices(),
                145,
                "an MCNK cell is 9×9 + 8×8 vertices"
            );
            assert!(
                mesh.compute_aabb().is_some(),
                "a cell mesh must yield the Aabb the exterior cull fails open without"
            );
        }

        let images = app.world().resource::<Assets<Image>>();
        let layer = images.get(&layer_h).expect("layer array present");
        assert_eq!(
            layer.texture_descriptor.size.width, 256,
            "layer array packed at LAYER_TEX_SIZE"
        );
        assert!(
            layer.texture_descriptor.mip_level_count > 1,
            "layer array carries the authored mip chain"
        );
        assert!(images.get(&alpha_h).is_some(), "alpha array present");
        assert!(images.get(&shadow_h).is_some(), "shadow array present");
        // Some tiles have no placements, so the count is only printed.
        eprintln!(
            "ADT tile loaded: {} MCNK cells, {n_doodads} doodad placements",
            drawn.len()
        );
    }
}
