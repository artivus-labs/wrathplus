use super::*;

/// M2 blend 5 and 6 (Mod, Mod2x) multiply what is drawn (`0x70c190`) and fog to white and grey
/// (`0x70baf0`), as a model batch's do.
#[test]
fn modulate_effects_blend_and_fog_as_model_batches() {
    assert_eq!(EffectBlend::from(ParticleBlend::Mod), EffectBlend::Multiply);
    assert_eq!(EffectBlend::from(ParticleBlend::Mod2x), EffectBlend::Mod2x);
    assert_eq!(
        EffectFog::for_blend(0, ParticleBlend::Mod),
        EffectFog::White
    );
    assert_eq!(
        EffectFog::for_blend(0, ParticleBlend::Mod2x),
        EffectFog::Grey
    );
    assert_eq!(
        EffectFog::for_blend(0x8, ParticleBlend::Mod2x),
        EffectFog::Off
    );
}

/// M2 modes 2 to 4 fold into `ModelBlend::Blend`; flat `Spells\` ground quads are mode 4.
#[test]
fn additive_wins_over_the_folded_blend_enum() {
    for blend in [
        ModelBlend::Opaque,
        ModelBlend::AlphaTest,
        ModelBlend::Blend,
        ModelBlend::Mod,
        ModelBlend::Mod2x,
    ] {
        assert_eq!(
            EffectBlend::from_model(blend, true),
            EffectBlend::Add,
            "{blend:?} + additive must reach the pure-add state, not {:?}",
            EffectBlend::from_model(blend, false),
        );
    }
}

/// `model_render`'s mapping, with `AlphaTest` folded to `Alpha`.
#[test]
fn non_additive_keeps_the_material_paths_law() {
    assert_eq!(
        EffectBlend::from_model(ModelBlend::Opaque, false),
        EffectBlend::Opaque
    );
    assert_eq!(
        EffectBlend::from_model(ModelBlend::AlphaTest, false),
        EffectBlend::Alpha
    );
    assert_eq!(
        EffectBlend::from_model(ModelBlend::Blend, false),
        EffectBlend::Alpha
    );
    assert_eq!(
        EffectBlend::from_model(ModelBlend::Mod, false),
        EffectBlend::Multiply
    );
    assert_eq!(
        EffectBlend::from_model(ModelBlend::Mod2x, false),
        EffectBlend::Mod2x
    );
}

/// A writer without `.after(begin_effect_frame)` runs first when the clear carries a
/// dependency it lacks; the graph is `ParticlePlugin`'s clear and `EntitiesPlugin`'s beam.
mod write_order {
    use super::*;

    #[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
    struct Place;

    fn joint_palette() {}
    fn finalize_rigs() {}
    fn face_billboards() {}

    fn writer(mut quads: ResMut<EffectQuads>) {
        let start = quads.begin();
        for _ in 0..4 {
            quads.verts.push(EffectVertex {
                pos: [0.0; 3],
                uv: [0.0; 2],
                color: [1.0; 4],
            });
        }
        quads.commit_quads(
            start,
            EffectDrawSpec {
                cam: Entity::PLACEHOLDER,
                texture: AssetId::default(),
                blend: EffectBlend::Add,
                fog: EffectFog::Off,
                lighting: EffectLighting::None,
                anchor: Vec3::ZERO,
                bias: 0.0,
                raster_bias: 0,
                raster_slope: 0.0,
                cam_relative: false,
                no_depth_test: false,
                main_entity: Entity::PLACEHOLDER,
                light: None,
                clip: None,
            },
        );
    }

    /// `edged`: whether the writer declares `.after(begin_effect_frame)`.
    fn run(edged: bool) -> App {
        let mut app = App::new();
        app.init_resource::<EffectQuads>();
        app.add_systems(
            PostUpdate,
            (joint_palette, finalize_rigs, face_billboards)
                .chain()
                .in_set(Place),
        );
        let w = writer
            .in_set(Place)
            .after(joint_palette)
            .after(finalize_rigs);
        app.add_systems(
            PostUpdate,
            if edged {
                w.after(begin_effect_frame)
            } else {
                w
            },
        );
        app.add_systems(
            PostUpdate,
            begin_effect_frame
                .in_set(Place)
                .after(joint_palette)
                .after(finalize_rigs)
                // The dependency the writer lacks.
                .after(face_billboards),
        );
        app.add_systems(Last, clear_effect_frame_flag);
        // Two frames: the check only bites after the first `Last` reset.
        app.update();
        app.update();
        app
    }

    /// With the edge, the writer's draw survives the frame and reaches extract.
    #[test]
    fn the_edge_keeps_the_draw() {
        let app = run(true);
        let quads = app.world().resource::<EffectQuads>();
        assert_eq!(quads.draws.len(), 1, "the draw must survive to extract");
        assert_eq!(quads.verts.len(), 4);
    }

    /// Without the edge, debug builds diagnose the write before the frame clear.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "effect-stream write before `begin_effect_frame`")]
    fn without_the_edge_the_tripwire_names_it() {
        let _ = run(false);
    }

    /// Optimized builds omit the tripwire, but the missing edge still erases the draw.
    #[cfg(not(debug_assertions))]
    #[test]
    fn without_the_edge_the_clear_erases_the_draw() {
        let app = run(false);
        let quads = app.world().resource::<EffectQuads>();
        assert!(quads.draws.is_empty(), "the late clear erases the draw");
        assert!(quads.verts.is_empty(), "the late clear erases its vertices");
    }
}
