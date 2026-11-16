//! The Wrath M2 render adaptation, over synthetic and installed models.

use super::*;
use wrathplus_m2::wrath::{WrathTrack, WrathTrackKey};

fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(bytes: &mut [u8], at: usize, value: f32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_array(bytes: &mut [u8], at: usize, count: u32, offset: u32) {
    put_u32(bytes, at, count);
    put_u32(bytes, at + 4, offset);
}

fn push_vec3(bytes: &mut Vec<u8>, value: [f32; 3]) {
    for component in value {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
}

fn synthetic_m2() -> Vec<u8> {
    let mut bytes = vec![0; 0x130];
    bytes[..4].copy_from_slice(b"MD20");
    put_u32(&mut bytes, 4, 264);
    put_u32(&mut bytes, 0x44, 1);

    let animation_offset = bytes.len() as u32;
    for (id, duration) in [(0_u16, 1000_u32), (4, 500)] {
        let mut sequence = vec![0; 64];
        put_u16(&mut sequence, 0, id);
        put_u32(&mut sequence, 4, duration);
        put_u32(&mut sequence, 12, 0x20);
        put_u16(&mut sequence, 16, 1);
        put_u32(&mut sequence, 20, 1);
        put_u32(&mut sequence, 24, 1);
        put_u32(&mut sequence, 28, 100);
        put_f32(&mut sequence, 44, 1.0);
        put_f32(&mut sequence, 48, 1.0);
        put_f32(&mut sequence, 52, 2.0);
        put_f32(&mut sequence, 56, 2.0);
        bytes.extend(sequence);
    }
    put_array(&mut bytes, 0x1c, 2, animation_offset);

    let vertex_offset = bytes.len() as u32;
    for (position, uv) in [
        ([0.0, 0.0, 0.0], [0.0, 0.0]),
        ([1.0, 0.0, 0.0], [1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 1.0]),
    ] {
        let mut vertex = vec![0; 48];
        for (axis, component) in position.into_iter().enumerate() {
            put_f32(&mut vertex, axis * 4, component);
        }
        vertex[12] = u8::MAX;
        put_f32(&mut vertex, 28, 1.0);
        put_f32(&mut vertex, 32, uv[0]);
        put_f32(&mut vertex, 36, uv[1]);
        bytes.extend(vertex);
    }
    put_array(&mut bytes, 0x3c, 3, vertex_offset);

    let material_offset = bytes.len() as u32;
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    put_array(&mut bytes, 0x70, 1, material_offset);

    // Two independent nested arrays both start at timestamp zero. Flattening them into a
    // vanilla absolute timeline would make sequence 1 read sequence 0's values.
    let mut time_offsets = Vec::new();
    let mut value_offsets = Vec::new();
    for (duration, values) in [
        (1000_u32, [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]),
        (500_u32, [[10.0, 0.0, 0.0], [20.0, 0.0, 0.0]]),
    ] {
        time_offsets.push(bytes.len() as u32);
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&duration.to_le_bytes());
        value_offsets.push(bytes.len() as u32);
        push_vec3(&mut bytes, values[0]);
        push_vec3(&mut bytes, values[1]);
    }
    let times_outer = bytes.len() as u32;
    for offset in &time_offsets {
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
    }
    let values_outer = bytes.len() as u32;
    for offset in &value_offsets {
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
    }
    let bone_offset = bytes.len() as u32;
    let mut bone = vec![0; 88];
    put_u16(&mut bone, 8, u16::MAX);
    put_u16(&mut bone, 16, 1);
    put_u16(&mut bone, 18, u16::MAX);
    put_array(&mut bone, 20, 2, times_outer);
    put_array(&mut bone, 28, 2, values_outer);
    put_u16(&mut bone, 38, u16::MAX);
    put_u16(&mut bone, 58, u16::MAX);
    bytes.extend(bone);
    put_array(&mut bytes, 0x2c, 1, bone_offset);
    bytes
}

#[test]
fn unsupported_ribbon_does_not_drop_model_or_supported_sibling() {
    let mut bytes = synthetic_m2();
    let offset = bytes.len();
    bytes.resize(offset + 3 * 176, 0);
    put_array(&mut bytes, 0x120, 3, offset as u32);
    for i in 0..3 {
        put_u32(&mut bytes, offset + i * 176, i as u32);
        put_u16(&mut bytes, offset + i * 176 + 154, u16::MAX);
    }
    // Second ribbon authors a cubic appearance track.
    put_u16(&mut bytes, offset + 176 + 36, 2);
    // Third authors animated visibility on a global clock.
    let track = offset + 2 * 176 + 152;
    put_u16(&mut bytes, track + 2, 0);
    let descriptors = bytes.len();
    bytes.resize(descriptors + 26, 0);
    put_array(&mut bytes, track + 4, 1, descriptors as u32);
    put_array(&mut bytes, track + 12, 1, (descriptors + 8) as u32);
    put_array(&mut bytes, descriptors, 2, (descriptors + 16) as u32);
    put_array(&mut bytes, descriptors + 8, 2, (descriptors + 24) as u32);
    bytes[descriptors + 25] = 1;
    let model = parse_wrath_render_model(&bytes, &synthetic_skin(0), "", &[]).unwrap();
    assert!(!model.submeshes.is_empty(), "geometry survives");
    assert_eq!(model.ribbons.len(), 1, "only unsupported ribbons dropped");
}

fn synthetic_skin(texture_count: u16) -> Vec<u8> {
    let mut bytes = vec![0; 48];
    bytes[..4].copy_from_slice(b"SKIN");
    let indices = bytes.len() as u32;
    for index in [0_u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    let triangles = bytes.len() as u32;
    for index in [0_u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    let section_offset = bytes.len() as u32;
    let mut section = vec![0; 48];
    put_u16(&mut section, 0, 101);
    put_u16(&mut section, 10, 3);
    bytes.extend(section);
    let batch_offset = bytes.len() as u32;
    let mut batch = vec![0; 24];
    put_u16(&mut batch, 14, texture_count);
    bytes.extend(batch);
    put_array(&mut bytes, 4, 3, indices);
    put_array(&mut bytes, 12, 3, triangles);
    put_array(&mut bytes, 28, 1, section_offset);
    put_array(&mut bytes, 36, 1, batch_offset);
    bytes
}

#[test]
fn adapter_keeps_nested_sequence_tracks_and_builds_render_geometry() {
    let parsed = parse_wrath_render_model(
        &synthetic_m2(),
        &synthetic_skin(0),
        "Character\\Synthetic",
        &[],
    )
    .expect("adapt synthetic Wrath model");
    assert_eq!(parsed.submeshes.len(), 1);
    assert_eq!(parsed.submeshes[0].positions.len(), 3);
    assert_eq!(parsed.submeshes[0].indices, [0, 1, 2]);
    assert_eq!(parsed.submeshes[0].geoset_id, 101);
    assert_eq!(parsed.skeleton.bones.len(), 1);
    assert_eq!(parsed.animations.len(), 2);
    assert_eq!(
        parsed.animations[0].bones[0].translation[1],
        (1.0, [2.0, 0.0, 0.0])
    );
    assert_eq!(
        parsed.animations[1].bones[0].translation[0],
        (0.0, [10.0, 0.0, 0.0])
    );
    assert_eq!(
        parsed.animations[1].bones[0].translation[1],
        (0.5, [20.0, 0.0, 0.0])
    );
    assert_eq!(parsed.animation_interpolation[1][0].translation, 1);
    assert!(parsed
        .animations
        .iter()
        .all(|animation| animation.start_ms == 0));
}

#[test]
fn nested_empty_sequence_does_not_borrow_following_sequence_key() {
    let track = M2Track {
        interp: 1,
        gseq: u16::MAX,
        // This is the v264 parser's representation of an empty first inner array followed
        // by a one-key second inner array.
        ranges: vec![(0, 0), (0, 0)],
        keys: vec![(250, [7.0, 8.0, 9.0])],
    };

    assert!(sequence_keys(&track, 0).unwrap().is_empty());
    assert_eq!(
        sequence_keys(&track, 1).unwrap(),
        vec![(0.25, [7.0, 8.0, 9.0])]
    );
}

#[test]
fn global_step_track_with_multiple_keys_fails_closed() {
    let track = M2Track {
        interp: 0,
        gseq: 0,
        ranges: vec![(0, 1)],
        keys: vec![(0, [0.0, 0.0, 0.0]), (500, [1.0, 0.0, 0.0])],
    };

    let error = global_sequence_channel(&track, &[1000])
        .expect_err("step interpolation cannot be represented by the runtime lerp sampler");
    assert!(
        error
            .to_string()
            .contains("unsupported Wrath global bone interpolation 0"),
        "{error:#}"
    );
}

#[test]
fn adapter_multitexture_retains_both_stages_and_geometry() {
    let mut bytes = synthetic_m2();
    let textures = bytes.len() as u32;
    for texture_type in [11_u32, 12] {
        let mut record = vec![0; 16];
        put_u32(&mut record, 0, texture_type);
        bytes.extend(record);
    }
    put_array(&mut bytes, 0x50, 2, textures);
    let lookup = bytes.len() as u32;
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    put_array(&mut bytes, 0x80, 2, lookup);
    let coord = bytes.len() as u32;
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    put_array(&mut bytes, 0x88, 2, coord);
    let skins = [Some("Base".into()), Some("Layer".into())];
    let model = parse_wrath_render_model(&bytes, &synthetic_skin(2), "Synthetic", &skins)
        .expect("combined stages admit geometry");
    assert_eq!(model.submeshes.len(), 1);
    assert_eq!(
        model.submeshes[0].texture.as_deref(),
        Some("Synthetic\\Base.blp")
    );
    assert_eq!(model.submeshes[0].skin_slot, Some(0));
    let stage = model.submeshes[0].stage2.as_ref().unwrap();
    assert_eq!(stage.texture.as_deref(), Some("Synthetic\\Layer.blp"));
    assert_eq!((stage.skin_slot, stage.coord, stage.op), (Some(1), 1, 0));
    assert_eq!(model.submeshes[0].indices, [0, 1, 2]);
    assert_eq!(model.submeshes[0].positions.len(), 3);
}

#[test]
fn adapter_multitexture_fallback_still_rejects_missing_stage_zero() {
    let error = parse_wrath_render_model(&synthetic_m2(), &synthetic_skin(2), "", &[])
        .err()
        .expect("a missing base texture is malformed, not a stage-zero fallback");
    assert!(
        error.to_string().contains("texture lookup is missing"),
        "{error:#}"
    );
}

#[test]
fn camera_sampling_selects_the_requested_nested_sequence_and_keeps_cubic_tangents() {
    let key = |timestamp_ms, value, in_tangent, out_tangent| WrathTrackKey {
        timestamp_ms,
        value,
        in_tangent,
        out_tangent,
    };
    let positions = WrathTrack {
        interpolation: 2,
        global_sequence: u16::MAX,
        sequences: vec![
            vec![key(0, [1.0, 0.0, 0.0], None, None)],
            vec![
                key(
                    0,
                    [10.0, 0.0, 0.0],
                    Some([10.0, 0.0, 0.0]),
                    Some([12.0, 0.0, 0.0]),
                ),
                key(
                    1000,
                    [20.0, 0.0, 0.0],
                    Some([18.0, 0.0, 0.0]),
                    Some([20.0, 0.0, 0.0]),
                ),
            ],
        ],
    };
    let empty_vec3 = WrathTrack {
        interpolation: 1,
        global_sequence: u16::MAX,
        sequences: vec![Vec::new(), Vec::new()],
    };
    let empty_scalar = WrathTrack {
        interpolation: 1,
        global_sequence: u16::MAX,
        sequences: vec![Vec::new(), Vec::new()],
    };
    let camera = WrathCamera {
        camera_type: 0,
        fov: 0.75,
        far_clip: 100.0,
        near_clip: 0.5,
        positions,
        position_base: [1.0, 2.0, 3.0],
        target: empty_vec3,
        target_base: [4.0, 5.0, 6.0],
        roll: empty_scalar,
    };
    // Mode 2 is Bézier (12340 `0x716c41`): ⅛·10 + ⅜·12 + ⅜·18 + ⅛·20 = 15, plus the base.
    let pose = sample_wrath_camera(&camera, 1, 500, 0).expect("sample camera");
    assert!((pose.position[0] - 16.0).abs() < 1e-5, "{pose:?}");
    assert_eq!(pose.position[1..], [2.0, 3.0]);
    assert_eq!(pose.target, [4.0, 5.0, 6.0]);
    // Mode 3 is Hermite (`0x716b9e`): the same keys' tangents read as velocities,
    // ½·10 + ⅛·12 + ½·20 − ⅛·18 = 14.25, plus the base.
    let mut hermite = camera.clone();
    hermite.positions.interpolation = 3;
    let pose = sample_wrath_camera(&hermite, 1, 500, 0).expect("sample camera");
    assert!((pose.position[0] - 15.25).abs() < 1e-5, "{pose:?}");
}

/// Installed models the two-stage adapter used to refuse whole: a helm whose sheen stage draws
/// its own object skin, a shield whose stage-2 lookup is 2, and a fist weapon with a Mod2x first
/// stage. 12340's vertex programs read a two-stage batch's coordinates off its environment bits
/// alone (`Diffuse_T1_T2`, `Diffuse_Env_T2`), and `Combiners_Mod_Mod2x` serves a Mod2x first stage.
#[test]
fn installed_two_stage_gear_adapts() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::Chain::open_wrath(&data, "enUS").expect("chain");
    let mut adapt = |name: &str| {
        let m2 = chain.read_file(name).expect(name);
        let skin = chain
            .read_file(&format!("{}00.skin", &name[..name.len() - 3]))
            .expect("skin 00");
        parse_wrath_render_model(&m2, &skin, "", &[]).expect(name)
    };
    let stages = |name: &str, m: &WrathRenderModel| -> Vec<crate::models::ModelTextureStage> {
        let s: Vec<_> = m
            .submeshes
            .iter()
            .filter_map(|s| s.stage2.clone())
            .collect();
        assert!(!s.is_empty(), "{name}: a second stage");
        s
    };

    let helm = "ITEM\\OBJECTCOMPONENTS\\HEAD\\Helm_Mail_Northrend_B_02_BeF.M2";
    let m = adapt(helm);
    assert!(stages(helm, &m)
        .iter()
        .any(|s| s.char_slot == Some(CharSkinSlot::Object) && s.coord > 2));

    let shield = "ITEM\\OBJECTCOMPONENTS\\Shield\\Shield_Healer_PVP_C_01.M2";
    let m = adapt(shield);
    assert!(
        stages(shield, &m)
            .iter()
            .all(|s| s.coord <= 1 || s.coord > 2),
        "a non-environment stage 2 reads UV0 or UV1, never lookup 2"
    );

    let fist = "ITEM\\OBJECTCOMPONENTS\\WEAPON\\Hand_1H_OutlandRaid_D_05Left.m2";
    let m = adapt(fist);
    assert!(stages(fist, &m)
        .iter()
        .any(|s| s.op == 4 && !s.first_opaque));

    // The one installed Add-over-Mod pair: the whole Combiners_Add_Mod program, op 8.
    let cell = "World\\EXPANSION01\\DOODADS\\TEMPESTKEEP\\PASSIVEDOODADS\\PRISONCELLGLASS\\\
                TK_Prison_Cell_Illuminated.m2";
    let m = adapt(cell);
    assert!(stages(cell, &m).iter().any(|s| s.op == 8));
}
