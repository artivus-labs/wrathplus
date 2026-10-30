use super::*;
use wrathplus_assets::{GlobalBone, GlobalSeqChannel};

fn clip(anim_id: u16) -> AnimClip {
    AnimClip {
        anim_id,
        seq_index: 0,
        node: AnimationNodeIndex::new(1),
        looping: true,
        duration: 2.0,
        move_speed: 0.0,
        blend_time: 0.0,
        bounds_center: Vec3::ZERO,
        bounds_radius: 0.0,
        bounds_min: Vec3::ZERO,
        bounds_max: Vec3::ZERO,
        events: Vec::new().into(),
        arm_nodes: None,
        upper_node: None,
        frequency: 0,
        replay: (0, 0),
        poses_bones: true,
    }
}

fn anims(clips: Vec<AnimClip>, first_seq: Option<usize>, gseq: bool) -> ModelAnimations {
    ModelAnimations {
        graph: Handle::default(),
        clips,
        playable_animation_lookup: Vec::new(),
        animation_lookup: Vec::new(),
        hand_close: [None, None],
        global_bones: if gseq {
            vec![GlobalBone {
                bone: 1,
                translation: None,
                rotation: None,
                scale: Some(GlobalSeqChannel {
                    period: 1.167,
                    keys: vec![(0.0, Vec3::ONE), (0.5, Vec3::splat(1.2))],
                }),
            }]
        } else {
            Vec::new()
        },
        first_seq,
        pose: Default::default(),
    }
}

/// One batch's per-sequence alpha, as the bake emits it: slot 0 hidden, slot 1 visible.
fn two_seq_alpha() -> std::sync::Arc<wrathplus_formats::AlphaAnim> {
    let hidden = wrathplus_formats::ScalarAnim {
        period: 0.0,
        step: true,
        wrap: true, // period 0: a constant has no clock
        gseq: false,
        keys: vec![(0.0, 0.0)],
    };
    std::sync::Arc::new(
        wrathplus_formats::AlphaAnim::new(vec![
            wrathplus_formats::AlphaSeq {
                color: None,
                weight: Some(hidden),
            },
            wrathplus_formats::AlphaSeq::default(),
        ])
        .expect("a hiding sequence is worth carrying"),
    )
}

fn seq_clip(anim_id: u16, seq_index: usize, node: usize) -> AnimClip {
    AnimClip {
        seq_index,
        node: AnimationNodeIndex::new(node),
        ..clip(anim_id)
    }
}

#[test]
fn hosted_mat_anim_follows_the_host_sequence() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_systems(Update, sample_mat_anim);

    // Two clips: graph node 1 is file sequence 0 (which hides the batch), node 2 is slot 1.
    let anims = anims(vec![seq_clip(0, 0, 1), seq_clip(1, 1, 2)], Some(0), false);
    let mut player = AnimationPlayer::default();
    player.play(AnimationNodeIndex::new(1)).repeat();
    let host = app.world_mut().spawn((player, anims)).id();
    let part = app
        .world_mut()
        .spawn(MatAnim::following(two_seq_alpha(), host))
        .id();

    app.update();
    assert_eq!(
        app.world().entity(part).get::<MatAnim>().unwrap().current,
        0.0,
        "playing sequence 0 ⇒ the batch is culled"
    );

    let world = app.world_mut();
    let mut entity = world.entity_mut(host);
    {
        let mut p = entity.get_mut::<AnimationPlayer>().unwrap();
        p.stop_all();
        p.play(AnimationNodeIndex::new(2)).repeat();
    }
    app.update();
    assert_eq!(
        app.world().entity(part).get::<MatAnim>().unwrap().current,
        1.0,
        "playing sequence 1 ⇒ the batch draws"
    );
}

/// With no player yet, slot 0 at t = 0, so the batch does not flash visible for a frame.
#[test]
fn hosted_mat_anim_without_a_player_reads_the_first_sequence() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_systems(Update, sample_mat_anim);
    let host = app.world_mut().spawn(Transform::default()).id();
    let part = app
        .world_mut()
        .spawn(MatAnim::following(two_seq_alpha(), host))
        .id();
    app.update();
    assert_eq!(
        app.world().entity(part).get::<MatAnim>().unwrap().current,
        0.0
    );
}

#[test]
fn pinned_mat_anim_reads_its_own_slot() {
    let a = two_seq_alpha();
    let doodad = MatAnim::new(a.clone(), 0.0, false);
    assert_eq!(doodad.current, 0.0, "slot 0 — the one-time load arm");
    assert!(!doodad.composes_unit_tag());
    let effect = MatAnim::driving_tag(a, 0.0, Some(1));
    assert_eq!(effect.current, 1.0, "the sequence the fx rig armed");
    assert!(effect.drives_tag);
    assert!(!effect.composes_unit_tag());
}

fn skeleton(joints: usize) -> ModelSkeleton {
    ModelSkeleton {
        joints: (0..joints)
            .map(|_| wrathplus_assets::ModelJoint {
                parent: -1,
                local_translation: Vec3::ZERO,
                billboard: None,
                parent_arm: None,
            })
            .collect(),
        spine_bone: None,
        head_bone: None,
    }
}

#[test]
fn classify_picks_the_measured_tiers() {
    // No ModelAnimations at all (a barrel): static.
    assert!(matches!(
        classify(&skeleton(3), None),
        DoodadAnimTier::Static
    ));
    // Boneless model: static even with parsed animations.
    let a = anims(vec![clip(0)], Some(0), true);
    assert!(matches!(
        classify(&skeleton(0), Some(&a)),
        DoodadAnimTier::Static
    ));
    // A motionless first sequence, no gseq (a posed tree): static.
    let a = anims(vec![clip(0)], None, false);
    assert!(matches!(
        classify(&skeleton(3), Some(&a)),
        DoodadAnimTier::Static
    ));
    // Gseq only (a candelabra glow pulse): the player-less tier.
    let a = anims(Vec::new(), None, true);
    assert!(matches!(
        classify(&skeleton(3), Some(&a)),
        DoodadAnimTier::GlobalSeqOnly
    ));
    // A moving first sequence (a flag / the torch flame bone): loop its clip.
    let a = anims(vec![clip(0)], Some(0), true);
    assert!(matches!(
        classify(&skeleton(3), Some(&a)),
        DoodadAnimTier::FirstSeq(c) if c.anim_id == 0
    ));
    // A clock-only model (clips but no `first_seq`: its animation lives in emitter tracks)
    // stays Static; the entity lane arms its clock off `idle_clip`.
    let a = anims(vec![clip(0)], None, false);
    assert!(matches!(
        classify(&skeleton(3), Some(&a)),
        DoodadAnimTier::Static
    ));
    // ...and one that also pulses on a global sequence takes the player-less tier, not a rig.
    let a = anims(vec![clip(0)], None, true);
    assert!(matches!(
        classify(&skeleton(3), Some(&a)),
        DoodadAnimTier::GlobalSeqOnly
    ));
}

/// `BlastedLandsLightningbolt01.M2`'s chain: two variations of animation id 0, weights 31129
/// and 1638, so slot 1, where its emitters key the whole burst, carries 5.0 %.
fn lightning_anims() -> ModelAnimations {
    let mut a = anims(vec![seq_clip(0, 0, 1), seq_clip(0, 1, 2)], Some(0), false);
    a.clips[0].frequency = 31129;
    a.clips[0].duration = 1.333;
    a.clips[1].frequency = 1638;
    a.clips[1].duration = 1.300;
    a
}

fn reroll_app() -> App {
    // No `TimePlugin`: its real clock would clobber the manual advances that step windows.
    let mut app = App::new();
    app.init_resource::<Time>();
    app.init_resource::<wrathplus_assets::AnimRng>();
    app.add_systems(Update, reroll_doodad_variation);
    app
}

fn lightning_host(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            DoodadAnimHost {
                meshes: Vec::new(),
                fade: crate::particles::EmitterFade::sphere(1.0, Vec3::ZERO),
                clip: None,
                armed_at: 0.0,
                window_hi: f32::NEG_INFINITY,
                anim_id: Some(0),
                active: true,
                parked_at: 0.0,
            },
            lightning_anims(),
            AnimationPlayer::default(),
        ))
        .id()
}

#[test]
fn a_placed_doodad_rerolls_its_variation_every_play_window() {
    let mut app = reroll_app();
    let host = lightning_host(&mut app);

    // Frame 1: born with an expired window, the holder's `variationIdx = -1` arm lands at once.
    app.update();
    let armed = |app: &App| {
        app.world()
            .entity(host)
            .get::<DoodadAnimHost>()
            .unwrap()
            .clip
    };
    assert!(armed(&app).is_some(), "the first frame arms");

    let mut seen = std::collections::HashMap::new();
    for _ in 0..400 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(1400));
        app.update();
        let h = app.world().entity(host).get::<DoodadAnimHost>().unwrap();
        let (node, duration) = h.clip.expect("a window always leaves something armed");
        // R = 1 for `replay = (0, 0)`, so the window is exactly the armed clip's own length.
        assert!(
            (h.window_hi - h.armed_at - duration).abs() < 1e-3,
            "window {} != clip duration {duration}",
            h.window_hi - h.armed_at
        );
        *seen.entry(node).or_insert(0u32) += 1;
    }

    let slot0 = seen.get(&AnimationNodeIndex::new(1)).copied().unwrap_or(0);
    let slot1 = seen.get(&AnimationNodeIndex::new(2)).copied().unwrap_or(0);
    assert_eq!(slot0 + slot1, 400, "every window arms one of the two");
    assert!(
        slot1 > 0,
        "the 5 % strike variation never came up in 400 windows — the bolt is stuck again"
    );
    assert!(slot0 > 0, "the 95 % silent variation never came up");
    // A sanity band, not a distribution test: this seed lands on 10 (~2.3σ low). Over 200 000
    // draws at two per window the rate is 4.9665 % against the authored 4.9988 %.
    assert!(
        (2..=60).contains(&slot1),
        "slot 1 came up {slot1}/400 — far enough off its authored 5 % to suspect the roll"
    );
}

/// Global sequences loop with no arming, so the gseq-only tier opens no window.
#[test]
fn a_gseq_only_host_never_rerolls() {
    let mut app = reroll_app();
    let host = app
        .world_mut()
        .spawn((
            DoodadAnimHost {
                meshes: Vec::new(),
                fade: crate::particles::EmitterFade::sphere(1.0, Vec3::ZERO),
                clip: None,
                armed_at: 0.0,
                window_hi: f32::NEG_INFINITY,
                anim_id: None,
                active: true,
                parked_at: 0.0,
            },
            anims(Vec::new(), None, true),
            AnimationPlayer::default(),
        ))
        .id();
    for _ in 0..8 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(1400));
        app.update();
    }
    let h = app.world().entity(host).get::<DoodadAnimHost>().unwrap();
    assert!(h.clip.is_none(), "nothing was ever armed");
    assert_eq!(h.window_hi, f32::NEG_INFINITY, "no window was ever opened");
}

/// The deviation on [`reroll_doodad_variation`]: an undrawn host cycles, its player untouched.
#[test]
fn an_undrawn_host_keeps_cycling() {
    let mut app = reroll_app();
    let host = lightning_host(&mut app);
    app.world_mut()
        .entity_mut(host)
        .get_mut::<DoodadAnimHost>()
        .unwrap()
        .active = false;

    app.update();
    let first = app
        .world()
        .entity(host)
        .get::<DoodadAnimHost>()
        .unwrap()
        .window_hi;
    for _ in 0..3 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(1400));
        app.update();
    }
    let h = app.world().entity(host).get::<DoodadAnimHost>().unwrap();
    assert!(
        h.window_hi > first,
        "an undrawn host still opened new windows"
    );
    assert!(
        h.clip.is_some(),
        "and still has something for the resume to arm"
    );
    assert_eq!(
        app.world()
            .entity(host)
            .get::<AnimationPlayer>()
            .unwrap()
            .playing_animations()
            .count(),
        0,
        "but its stopped player was left alone"
    );
}

mod gate;
