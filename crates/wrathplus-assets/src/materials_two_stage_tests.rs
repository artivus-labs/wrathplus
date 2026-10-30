//! The Wrath second texture stage: the sphere map, the shader ABI and the combiner programs.

// CPU twin of the Wrath-only environment stage in wow_model.wgsl.
fn sphere_uv(p: bevy::math::Vec3, n: bevy::math::Vec3) -> bevy::math::Vec2 {
    let i = p.normalize();
    let n = n.normalize();
    let r = i - 2.0 * i.dot(n) * n;
    bevy::math::Vec2::splat(0.5) + 0.5 * r.truncate() / (r + bevy::math::Vec3::Z).length()
}

#[test]
fn wrath_stage_two_sphere_map_known_reflections() {
    use bevy::math::{Vec2, Vec3};
    // Visible points have negative view Z. Eye-to-vertex I=-Z reflected
    // about a camera-facing +Z normal returns +Z: finite sphere centre.
    let head_on = sphere_uv(-Vec3::Z, Vec3::Z);
    assert!(head_on.is_finite());
    assert_eq!(head_on, Vec2::splat(0.5));
    let q = std::f32::consts::FRAC_1_SQRT_2;
    for (normal, expected) in [
        (Vec3::new(q, 0.0, q), Vec2::new(0.5 + 0.5 * q, 0.5)),
        (Vec3::new(-q, 0.0, q), Vec2::new(0.5 - 0.5 * q, 0.5)),
        (Vec3::new(0.0, q, q), Vec2::new(0.5, 0.5 + 0.5 * q)),
    ] {
        assert!(sphere_uv(-Vec3::Z, normal).abs_diff_eq(expected, 1e-6));
        assert!(sphere_uv(-3.0 * Vec3::Z, 2.0 * normal).abs_diff_eq(expected, 1e-6));
    }
    // R near -Z approaches the rim; exactly -Z is the native singularity.
    let grazing = sphere_uv(-Vec3::Z, Vec3::new(1.0, 0.0, 0.01));
    assert!(grazing.is_finite());
    assert!(grazing.x > 0.99 && grazing.x <= 1.0);
    assert_eq!(grazing.y, 0.5);
    let source = include_str!("shaders/wow_model.wgsl");
    assert!(source.contains("reflect(normalize(p_view), n_view)"));
    assert!(source.contains("length(refl + vec3<f32>(0.0, 0.0, 1.0))"));
    assert!(source.contains("if (m.stage2.x > 0.5 && m.stage2.z > 2.0)"));
    // UV0's Vanilla expression is intentionally unchanged.
    assert!(source.contains("out.uv = refl.xy * 0.5 + vec2<f32>(0.5, 0.5)"));
}

/// The second stage's Add and AddNA against the client's own programs
/// (`Shaders\Pixel\arbfp1\Combiners_*.bls`): `t0·primary + t1`, the second texel added as it
/// is; Add sums the alphas, AddNA keeps the first stage's.
#[test]
fn stage_two_add_matches_the_installed_combiner_programs() {
    let forward = include_str!("shaders/wow_model.wgsl");
    assert!(forward.contains("stage_add = t.rgb;"));
    assert!(
        !forward.contains("stage_add *="),
        "the added texel takes no tint and no primary colour"
    );
    let shadow = include_str!("shaders/wow_shadow.wgsl");
    assert!(shadow.contains("if (op == 3u) {\n            alpha += a1;"));
    // Combiners_Add_Mod (op 8): `t1·(primary + t0)`, the t0·t1 term outside the lit primary.
    assert!(forward.contains("stage_add = t0.rgb * t.rgb;"));
    assert!(forward.contains("base_color = vec4<f32>(p.rgb * t.rgb, t.a * (p.a + t0.a));"));
    assert!(shadow.contains("alpha += t0a; // Combiners_Add_Mod"));

    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::Chain::open_wrath(&data, "enUS").expect("chain");
    for (program, lines) in [
        (
            "Combiners_Opaque_Add",
            [
                "MAD result.color.xyz, R1, fragment.color.primary, R0;",
                "ADD result.color.w, fragment.color.primary, R0;",
            ],
        ),
        (
            "Combiners_Opaque_AddNA",
            [
                "MAD result.color.xyz, R0, fragment.color.primary, R1;",
                "MOV result.color.w, fragment.color.primary;",
            ],
        ),
        (
            "Combiners_Mod_Add",
            [
                "TEX R1, fragment.texcoord[1], texture[1], 2D;",
                "MAD result.color, R0, fragment.color.primary, R1;",
            ],
        ),
        (
            "Combiners_Add_Mod",
            [
                "MAD result.color.xyz, R0, fragment.color.primary, R1;",
                "MUL result.color.w, R0, R1;",
            ],
        ),
        (
            "Combiners_Mod_AddNA",
            [
                "MAD result.color.xyz, R0, fragment.color.primary, R1;",
                "MUL result.color.w, R0, fragment.color.primary;",
            ],
        ),
    ] {
        let bytes = chain
            .read_file(&format!("Shaders\\Pixel\\arbfp1\\{program}.bls"))
            .expect(program);
        let text = String::from_utf8_lossy(&bytes);
        for line in lines {
            assert!(text.contains(line), "{program}: {line}");
        }
    }
}

#[test]
fn both_shader_parameter_layouts_match_rust_stage_order() {
    let forward = include_str!("shaders/wow_model.wgsl");
    let shadow = include_str!("shaders/wow_shadow.wgsl");
    let fields = [
        "clutter_fade",
        "model_flags",
        "sun_scale",
        "tint",
        "sidn",
        "wmo_unified",
        "anim_slots",
        "stage2",
        "stage2_anim",
        "stage2_seed",
        "light_sphere",
    ];
    for source in [forward, shadow] {
        let abi = source
            .split("struct ModelParams {")
            .nth(1)
            .unwrap()
            .split("};")
            .next()
            .unwrap();
        let mut last = 0;
        for field in fields {
            let at = abi.find(&format!("{field}: vec4<f32>")).unwrap();
            assert!(at >= last);
            last = at;
        }
        assert_eq!(abi.matches(": vec4<f32>").count(), 11);
        assert!(source.contains("@binding(101) var second_texture"));
        assert!(source.contains("@binding(102) var second_sampler"));
    }
    assert!(shadow.contains("m.stage2.x > 0.5 && m.stage2.z > 2.0) { discard; }"));
}

// Native texture-derived coverage remains distinct from vertex/material opacity.
fn coverage(opaque: bool, op: u32, a0: f32, a1: f32) -> f32 {
    let first = if opaque { 1.0 } else { a0 };
    match op {
        0 | 6 | 7 => first,
        3 if opaque => 1.0,
        3 => first + a1,
        4 => first * a1 * 2.0,
        _ => first * a1,
    }
}
#[test]
fn combined_coverage_keeps_zero_opacity_zero_and_cutout_unfaded() {
    for opaque in [false, true] {
        for op in 0..8 {
            for a0 in [0.0, 0.5, 1.0] {
                for a1 in [0.0, 0.5, 1.0] {
                    let tex = coverage(opaque, op, a0, a1);
                    for opacity in [0.0, 0.25, 1.0] {
                        let final_alpha = opacity * tex;
                        if opacity == 0.0 {
                            assert_eq!(final_alpha, 0.0);
                        }
                    }
                    if opaque && matches!(op, 0 | 3 | 6 | 7) {
                        assert_eq!(tex, 1.0);
                    }
                }
            }
        }
    }
    let cutoff = super::VANILLA_ALPHA_KEY_REF;
    assert!(coverage(false, 1, cutoff, 1.0) >= cutoff);
    assert!(coverage(false, 1, cutoff - 0.0001, 1.0) < cutoff);
    assert_eq!(coverage(false, 3, 0.5, 0.5), 1.0);
    assert_eq!(coverage(false, 6, 0.5, 0.0), 0.5);
}
