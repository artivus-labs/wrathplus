use super::replace_fog_policy;
use wrathplus_formats::FogPolicy;

#[test]
fn replace_fog_policy_preserves_pipeline_markers() {
    // Every marker bit (0-3, 7-8) set, fog policy Grey (3) in bits 4-6.
    let z = f32::from(0b1_1000_1111u16 | (3 << 4));
    let out = replace_fog_policy(z, FogPolicy::Off) as u16;
    assert_eq!(out & 0b1_1000_1111, 0b1_1000_1111, "markers preserved");
    assert_eq!((out >> 4) & 7, FogPolicy::Off as u16, "fog swapped");
    // No stray bits invented.
    let out2 = replace_fog_policy(f32::from(0u16), FogPolicy::Grey) as u16;
    assert_eq!(out2, (FogPolicy::Grey as u16) << 4);
}

/// Too small and coplanar layers strobe; too large and effects interleave into their owner.
#[test]
fn batch_order_sort_eps_sits_between_f32_noise_and_the_effect_rung() {
    // The capped product is the deepest any batch can bias; WMO roots index past 2000.
    let deepest =
        (super::BATCH_ORDER_SORT_EPS * f32::from(u16::MAX)).min(super::BATCH_ORDER_SORT_CAP);
    assert!(
        deepest < wrathplus_formats::owner_last_rung(0.0),
        "a model's last batch must still sort before its own effects' rung"
    );
    let far_noise = 500.0_f32 * f32::EPSILON;
    assert!(
        super::BATCH_ORDER_SORT_EPS > 8.0 * far_noise,
        "one order step must dominate f32 noise on a far sort distance"
    );
    // The pipeline key truncates the whole band to 0 for any u16 index.
    assert_eq!(deepest as i32, 0);
    const {
        assert!(super::BATCH_ORDER_SORT_CAP < 1.0);
        // A capped batch's far twin still truncates onto the far band's one key integer.
        assert!(super::BATCH_ORDER_SORT_CAP < super::FAR_KEY_PULL);
    }
    assert_eq!(
        super::far_sort_bias(super::BATCH_ORDER_SORT_CAP) as i32,
        super::far_sort_bias(0.0) as i32,
        "the far band stays one pipeline key across the whole capped eps range"
    );
}

#[test]
fn the_skybox_band_orders_its_batches_on_one_pipeline_key() {
    // `CavernsOfTimeSky.m2` is 21 batches; `order` is the index + 1.
    let band: Vec<f32> = (0..=21).map(super::skybox_sort_bias).collect();

    // 1. Strictly increasing: at −6e4 the ordinary 1e-3 eps would round onto one f32.
    for pair in band.windows(2) {
        assert!(
            pair[1] > pair[0],
            "skybox batch order does not resolve at the rung's magnitude: {pair:?} — \
             SKYBOX_ORDER_EPS is under the f32 ulp there"
        );
    }

    // 2. One pipeline key for the whole band (`depth_bias as i32` is a key axis), for any
    //    u16 order: the cap holds it.
    let key = band[0] as i32;
    for order in [0u16, 1, 21, 57, 58, 1000, u16::MAX] {
        assert_eq!(
            super::skybox_sort_bias(order) as i32,
            key,
            "batch order {order} mints a second skybox pipeline"
        );
    }

    // 3. Under every world transparent, even with the skybox's shell radius (~94 yd for
    //    Caverns of Time) on its rung and a world draw at the far plane.
    const FAR_PLANE: f32 = 3000.0;
    const SHELL: f32 = 128.0;
    let deepest_world = crate::sky_order::FAR_SIDE_BIAS - FAR_PLANE - super::FAR_KEY_PULL;
    assert!(
        band.last().unwrap() + SHELL < deepest_world,
        "a skybox can sort in front of a world transparent — the painted sky would paint \
         over it (sky_order::WMO_SKYBOX_BIAS)"
    );

    // 4. The rung stays within ten times the world band: margin, not drift.
    const {
        assert!(crate::sky_order::WMO_SKYBOX_BIAS > 10.0 * crate::sky_order::FAR_SIDE_BIAS);
    }
}

#[test]
fn far_twin_keeps_the_source_identity_one_rung_down() {
    assert_eq!(
        super::far_sort_bias(super::ZFILL_SORT_BIAS),
        crate::sky_order::FAR_SIDE_BIAS + super::ZFILL_SORT_BIAS - super::FAR_KEY_PULL,
        "one rung down from wherever the source sat (less the key-collapse pull)"
    );
    assert!(
        super::far_sort_bias(super::ZFILL_SORT_BIAS)
            < super::far_sort_bias(super::BATCH_ORDER_SORT_EPS * 512.0),
        "a far zfill twin still primes before its model's far colour batches"
    );
    // The whole batch-eps band truncates to one pipeline-key integer; the zfill twin keeps
    // its own one bracket down.
    assert_eq!(
        super::far_sort_bias(0.0) as i32,
        super::far_sort_bias(super::BATCH_ORDER_SORT_EPS * 512.0) as i32,
        "batch-order far twins may never mint a second pipeline"
    );
    const {
        assert!(super::FAR_KEY_PULL > super::BATCH_ORDER_SORT_EPS * 512.0);
    }
    assert_eq!(
        super::far_sort_bias(super::ZFILL_SORT_BIAS) as i32,
        crate::sky_order::FAR_SIDE_BIAS as i32 + super::ZFILL_SORT_BIAS as i32,
        "the zfill far key stays put — the pull is sub-integer"
    );
    let src = super::ZFILL_MARKER | super::TWIN_CUTOUT_MARKER | (3 << 4);
    let z = super::far_markers(f32::from(src)) as u16;
    assert_ne!(z & super::FAR_SIDE_MARKER, 0, "the sort-only pipeline bit");
    assert_eq!(
        z & !super::FAR_SIDE_MARKER,
        src,
        "every other marker preserved"
    );
    // The marker word stays f32-exact with every bit up to `FAR_SIDE_MARKER` set.
    let all = src | super::FAR_SIDE_MARKER | 0x0f;
    assert_eq!(f32::from(all) as u16, all);
}

#[test]
fn unified_fallback_ambient_preserves_presence_and_gamma_rgb() {
    use bevy::prelude::Vec4;
    assert_eq!(super::unified_ambient(None), Vec4::ZERO);
    assert_eq!(super::unified_ambient(Some([0; 3])), Vec4::W);
    assert_eq!(super::unified_ambient(Some([255; 3])), Vec4::ONE);
    assert_eq!(
        super::unified_ambient(Some([17, 91, 203])),
        Vec4::new(17.0 / 255.0, 91.0 / 255.0, 203.0 / 255.0, 1.0)
    );
}

#[test]
fn unified_fallback_material_key_distinguishes_none_black_and_rgb() {
    use super::*;
    let key = |wmo_unified| MatKey {
        light: bevy::render::render_resource::BufferId::new(),
        texture: None,
        texture2: None,
        stage2: [0; 4],
        stage2_owner: 0,
        light_sphere: [0; 4],
        blend: ModelBlend::Opaque,
        two_sided: false,
        is_wmo: true,
        is_interior: true,
        is_emissive: false,
        is_additive: false,
        fade_variant: false,
        no_depth_write: false,
        no_depth_test: false,
        fog_policy: FogPolicy::Scene,
        env_map: false,
        shade: ShadeSel::Matte,
        batch_order: 0,
        uv_anim: None,
        rgb_anim: None,
        wmo_class: Some(WmoBatchClass::Int),
        sidn: Some([10, 20, 30]),
        wmo_unified,
        window: true,
        zfill: false,
        sky_depth: false,
        instance: None,
    };
    let plain = key(None);
    let mut black = key(Some([0; 3]));
    let mut colored = key(Some([17, 91, 203]));
    // All other key fields, including buffer identity, agree.
    black.light = plain.light;
    colored.light = plain.light;
    assert!(plain != black);
    assert!(black != colored);
    let mut cache = MaterialCache::default();
    let plain_handle = Handle::default();
    cache.insert(plain, plain_handle);
    assert!(cache.fetch(&black).is_none());
    assert!(cache.fetch(&colored).is_none());
}
