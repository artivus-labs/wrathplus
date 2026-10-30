//! The doodad animation host: placed M2 doodads (ADT MDDF and WMO MODD props) animate.
//!
//! The reference arms a doodad at load (bone 0, animation id 0, `linkFlag=1`) and re-arms it every
//! play-window with a fresh weighted variation: the watchdog `0x719370` fires on `now ≥ windowHi`
//! and runs the callback `0x6951b0` that `0x695100` installed at `[model+0x70]`, which re-arms
//! with `variationIdx = -1`. Global sequences loop with no arming. The walk `0x7074b0` advances
//! only the models the doodad drain `0x683f80` links each frame from the drawn and faded set, and
//! sampling is clock-indexed (`cursor = clock − startOffset`), so pausing drifts nothing.

use bevy::animation::graph::AnimationNodeIndex;
use bevy::app::AnimationSystems;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;

use wrathplus_assets::{AnimClip, M2Model, ModelAnimations, ModelSkeleton};

use crate::rig_anim::{AnimParked, GlobalSeqDrive, RigPose};
use crate::vis_chain::VisChainOnly;

mod lazy;
mod mat_anim;
pub(crate) use lazy::{LazyRig, SkinnedTwin};
use mat_anim::tick_anim_materials;
pub use mat_anim::{
    playing_seq, register_entity_uv, register_entity_uv_stage2, register_fx_uv, register_tint,
    sample_mat_anim, AnimMatPart, MatAnim, TintAnimMaterials, TintLoop, UvAnimMaterials, UvLoops,
};
pub(crate) use mat_anim::{register_uv, UvLoop};

/// What a placed doodad model animates, decided per model at spawn.
pub enum DoodadAnimTier<'a> {
    /// No animated bone channel: the static path (most doodads: trees, fences, rocks).
    Static,
    /// Only global-sequence channels (a candelabra's glow): a [`GlobalSeqDrive`], no player.
    GlobalSeqOnly,
    /// The loader-idle seed moves bones (flags, windmills, torch flames): an `AnimationPlayer`
    /// looping this clip. The seed is animation id 0 resolved through the model's
    /// `playableAnimationLookup`, not the file-order first sequence the name suggests.
    FirstSeq(&'a AnimClip),
}

/// The animation-event tags that make a placed doodad sound: `$DSL` (a looping emitter), `$DSO` (a
/// positioned one-shot), `$DSE` (the stop that releases a `$DSL`) and `$SND`, the markers the
/// reference's doodad event callback `0x6951e0` routes on.
pub(crate) const SOUND_EVENT_TAGS: [&[u8; 4]; 4] = [b"$DSL", b"$DSO", b"$DSE", b"$SND"];

/// Whether the model's arm chain (the loader-idle seed and its variations, the only sequences a
/// placed doodad plays) carries a sound-event marker. Kept out of [`classify`], which answers what
/// the model renders: the reference arms every placed doodad (`0x695100` → `0x7121a0`), and its
/// event track runs on that clock even when no bone moves (`KalidarStreetLamp01.m2`'s `$DSL`).
pub(crate) fn arms_for_sound(anims: &ModelAnimations) -> bool {
    let Some(idle) = anims.idle_clip() else {
        return false;
    };
    anims
        .clips
        .iter()
        .filter(|c| c.anim_id == idle.anim_id)
        .any(|c| {
            c.events
                .iter()
                .any(|e| SOUND_EVENT_TAGS.contains(&&e.ident))
        })
}

/// Classify a placed model; a boneless one is `Static`, with no joints for a skin to index.
pub fn classify<'a>(
    skeleton: &ModelSkeleton,
    animations: Option<&'a ModelAnimations>,
) -> DoodadAnimTier<'a> {
    let Some(anims) = animations else {
        return DoodadAnimTier::Static;
    };
    if skeleton.joints.is_empty() {
        return DoodadAnimTier::Static;
    }
    // `first_seq` is the content gate, not the arm's identity (`idle_seq`): a seed that holds bind
    // pose renders identically to the static mesh.
    match anims.first_seq.and_then(|i| anims.clips.get(i)) {
        Some(clip) => DoodadAnimTier::FirstSeq(clip),
        None if !anims.global_bones.is_empty() => DoodadAnimTier::GlobalSeqOnly,
        None => DoodadAnimTier::Static,
    }
}

/// What [`spawn_anim_host`] set up for one placement; the [`RigPose`] rides here by value until
/// [`Self::finish`], so anchors ([`Self::anchor`]) are minted only for bones something rides.
pub struct AnimHostSpawn {
    pub root: Entity,
    /// The pose buffer, inserted on `root` by [`Self::finish`] once every anchor is minted.
    pose: RigPose,
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    /// The armed clip's node and duration, `None` on the gseq-only tier.
    pub(crate) clip: Option<(AnimationNodeIndex, f32)>,
    /// The file sequence slot the load arm seeded ([`AnimClip::seq_index`]); the first re-roll
    /// replaces it, as the holder's `0x695100` arm does, so the current slot is read off the host's
    /// player ([`crate::particles::EmitClock::Host`]). `None` on the gseq-only tier.
    pub(crate) seq: Option<usize>,
    /// The animation id the arm rolls variations over; `None` on the gseq-only tier.
    pub(crate) anim_id: Option<u16>,
}

impl AnimHostSpawn {
    /// The anchor entity for `bone`, spawned on first demand and shared after
    /// ([`RigPose::anchor_for`]); `None` for a bone outside the skeleton.
    pub fn anchor(&mut self, commands: &mut Commands, bone: u16) -> Option<Entity> {
        self.pose.anchor_for(commands, self.root, bone)
    }

    /// The skeleton's bone count, the palette allocation size.
    pub fn bones(&self) -> u32 {
        self.pose.locals.len() as u32
    }

    /// Attach the pose buffer once the anchors are minted; returns the `(bone, anchor)` registry.
    pub fn finish(self, commands: &mut Commands) -> Vec<(u16, Entity)> {
        let anchors = self.pose.anchors.clone();
        commands.entity(self.root).insert(self.pose);
        anchors
    }
}

/// Whether [`spawn_anim_host`] would rig this model, by the same tier and capture gates, so skinned
/// twins are built only for placed models that draw them.
pub fn wants_rig(m: &M2Model) -> bool {
    !crate::dev_state::deterministic_run()
        && !matches!(
            classify(&m.skeleton, m.animations.as_ref()),
            DoodadAnimTier::Static
        )
}

/// Spawn the animation host for one placed M2 that animates: a root at the placement transform
/// with the collapsed [`RigPose`] and, per tier, an `AnimationPlayer` looping the loader-idle seed
/// (the reference's load arm `0x70ebd0` → `0x7121a0(bone 0, animation id 0, linkFlag=1)`) and/or
/// a [`GlobalSeqDrive`]. `None` keeps the static path.
pub fn spawn_anim_host(
    commands: &mut Commands,
    m: &M2Model,
    transform: Transform,
) -> Option<AnimHostSpawn> {
    // Captures keep every doodad static, so their frames are deterministic.
    if crate::dev_state::deterministic_run() {
        return None;
    }
    let tier = classify(&m.skeleton, m.animations.as_ref());
    // The sound arm, orthogonal to the tier: a bind-posed model still runs its event track, as in
    // the reference, so it gets a host with the arm bookkeeping but no `AnimationPlayer`.
    let sound_arm = m.animations.as_ref().is_some_and(arms_for_sound);
    if matches!(tier, DoodadAnimTier::Static) && !sound_arm {
        return None;
    }
    let anims = m
        .animations
        .as_ref()
        .expect("animated tier or sound arm ⇒ animations");
    let root = commands
        .spawn((transform, Visibility::default()))
        .vis_chain_only()
        .id();
    // The caller allocates the palette slot: lazily on the terrain-stream lane (`LazyRig`), eagerly
    // for a gate-less caller such as quest markers, whose host would otherwise never skin.
    let pose = RigPose::new(root, &m.skeleton);
    let mut clip_info = None;
    let mut armed_seq = None;
    let mut arm_id = None;
    if let DoodadAnimTier::FirstSeq(head) = tier {
        // The loader's var-0 seed only (`0x70ebd0`). The effective arm is the holder's
        // `variationIdx = -1` op4 (`0x695100`), here `reroll_doodad_variation`'s first pass, the
        // same frame, since the host is born with an expired window.
        let mut player = AnimationPlayer::default();
        player.play(head.node).repeat();
        commands.entity(root).insert((
            player,
            AnimationGraphHandle(anims.graph.clone()),
            // For the re-roll and the per-sequence consumers resolving off this host.
            anims.clone(),
        ));
        clip_info = Some((head.node, head.duration));
        armed_seq = Some(head.seq_index);
        arm_id = Some(head.anim_id);
    } else if sound_arm {
        // The clock-only arm: the re-roll's query needs `ModelAnimations`; nothing here poses.
        let head = anims.idle_clip().expect("sound arm ⇒ an idle clip");
        clip_info = Some((head.node, head.duration));
        arm_id = Some(head.anim_id);
        commands.entity(root).insert(anims.clone());
        // `armed_seq` stays `None`: it alone moves the placement's emitters onto `EmitClock::Host`,
        // which reads a player this host lacks. The sound clock reads `clip` and `armed_at`.
    }
    if let Some(drive) = GlobalSeqDrive::new_rig(&anims.global_bones, m.skeleton.joints.len()) {
        commands.entity(root).insert(drive);
    }
    Some(AnimHostSpawn {
        root,
        pose,
        inverse_bindposes: m.inverse_bindposes.clone(),
        clip: clip_info,
        seq: armed_seq,
        anim_id: arm_id,
    })
}

/// Animation bounds for an entity-parented WMO prop, in its parent's local frame.
/// The placement is already included in `center` and `radius`; only the parent's live world
/// transform is applied by the gate. Terrain hosts omit this component and keep world spheres.
#[derive(Component)]
pub struct ParentSpaceAnimBound {
    pub parent: Entity,
    pub center: Vec3,
    pub radius: f32,
}

/// The anim root of one animated placement: the draw gate's state and what to re-arm on resume.
/// It despawns with its tile, its bone anchors with it.
#[derive(Component)]
pub struct DoodadAnimHost {
    /// The placement's skinned submeshes; animation runs while any of them is drawn.
    pub(crate) meshes: Vec<Entity>,
    /// The draw-set gate of a meshless (particles-only) host: the [`crate::particles::EmitterFade`]
    /// its emitters, ribbons and lights were built with, so they freeze together. It carries the
    /// prop's building (instance and room), without which a sealed room would park its own props.
    pub(crate) fade: crate::particles::EmitterFade,
    /// The armed clip's graph node and duration (secs); `None` on the gseq-only tier.
    pub(crate) clip: Option<(AnimationNodeIndex, f32)>,
    /// `Time::elapsed_secs` at the current arm, the player's clock origin: a resume seeks to
    /// `(now − armed_at) mod duration`. The [`GlobalSeqDrive`] keeps its own attach anchor instead.
    pub(crate) armed_at: f32,
    /// The end of the armed play-window on the shared clock, the reference's `windowHi`
    /// (`[model+0xac]`); `now ≥ window_hi` re-rolls. Born at `NEG_INFINITY`, so frame one arms.
    pub(crate) window_hi: f32,
    /// The animation id to re-roll over; `None` on the gseq-only tier, which has no window.
    pub(crate) anim_id: Option<u16>,
    /// Whether the host was ticking last frame (edge-triggered pause/resume).
    pub active: bool,
    /// `Time::elapsed_secs` when `active` last went false: the reaper demotes the longest-parked.
    pub(crate) parked_at: f32,
}

impl DoodadAnimHost {
    /// The armed clip's `(node, seek)` on the shared clock, `(now − armed_at) mod duration`. It
    /// answers while parked, but the reference fires no events for a doodad culled out of its
    /// scene list (`[CM2Scene+0x20]`), so a consumer reads [`DoodadAnimHost::active`] beside it.
    pub fn arm_clock(&self, now: f32) -> Option<(AnimationNodeIndex, f32)> {
        let (node, duration) = self.clip?;
        // A zero-length clip has no phase to compute; it sits at its own t = 0.
        let seek = if duration > 0.0 {
            (now - self.armed_at).rem_euclid(duration)
        } else {
            0.0
        };
        Some((node, seek))
    }
}

/// The doodad's self-sustaining re-arm: when the armed play-window ends, roll a fresh
/// frequency-weighted variation of the same animation id, snap to it (`blendFlag = 0` at
/// `0x6951c8`, no cross-fade) and write the next window. An undrawn host only updates
/// [`DoodadAnimHost::clip`], for [`gate_doodad_anim`]'s resume to arm.
///
/// Deviation: this runs for every resident host, where the reference stops a doodad culled out
/// of its scene list (`[CM2Scene+0x20]`) and re-arms it at once on re-link (`0x7074b0`,
/// `0x683f80`), because gating it so would re-roll the whole field on the frame the camera turns
/// back.
fn reroll_doodad_variation(
    time: Res<Time>,
    mut rng: ResMut<wrathplus_assets::AnimRng>,
    mut hosts: Query<(
        &mut DoodadAnimHost,
        &ModelAnimations,
        Option<&mut AnimationPlayer>,
    )>,
) {
    let now = time.elapsed_secs();
    for (mut host, anims, player) in &mut hosts {
        let Some(anim_id) = host.anim_id else {
            continue; // gseq-only: never armed
        };
        if now < host.window_hi {
            continue;
        }
        let Some(clip) = anims.pick_variation(anim_id, rng.draw()) else {
            // No chain for this id: stop asking.
            host.anim_id = None;
            continue;
        };
        let (node, duration) = (clip.node, clip.duration);
        let replay = rng.replay_count(clip.replay);
        host.armed_at = now;
        // `span · R`, one loop for `replay = (0, 0)`; a zero-duration clip gets a minimum window.
        host.window_hi = now + (duration * replay as f32).max(f32::EPSILON);
        host.clip = Some((node, duration));
        if host.active {
            if let Some(mut p) = player {
                p.stop_all(); // the snap
                p.play(node).repeat();
            }
        }
    }
}

/// The draw gate: stop a doodad's animation while it is not drawn, and on resume seek the player
/// to the arm's shared-clock phase. Runs before [`AnimationSystems`], so the seek lands the same
/// frame.
#[allow(clippy::type_complexity)] // one Bevy system's full input set
fn gate_doodad_anim(
    time: Res<Time>,
    mut hosts: Query<(
        Entity,
        &mut DoodadAnimHost,
        Option<&ParentSpaceAnimBound>,
        Option<&lazy::LazyRig>,
        Option<&RigPose>,
        Has<crate::rig_palette::RigSkin>,
        Option<&mut AnimationPlayer>,
        Option<&mut GlobalSeqDrive>,
    )>,
    vis: Query<&Visibility>,
    cam: Query<
        (
            Ref<GlobalTransform>,
            &bevy::camera::primitives::Frustum,
            Ref<bevy::camera::Projection>,
            // The seat's own write: `GlobalTransform` propagates after this system, so on a
            // teleport frame only the local shows the camera moved.
            Option<Ref<Transform>>,
        ),
        With<crate::view::WorldCamera>,
    >,
    // With no part's `Visibility` changed and the camera still, every verdict is last frame's.
    changed_vis: Query<(), (Changed<Visibility>, With<crate::model_render::ModelPart>)>,
    // The draw-set inputs (far clip, exterior-window gate, camera room, portal instances), the
    // same `SystemParam` the particle and ribbon sims read, so a placement's riders agree.
    scene: crate::particles::sim::SceneGates,
    // The lazy-rig lane's wake half: a drawn host without a slot promotes here.
    mut palettes: ResMut<crate::rig_palette::RigPalettes>,
    ibps: Res<Assets<SkinnedMeshInverseBindposes>>,
    worlds: Query<&GlobalTransform>,
    mut twin_parts: lazy::TwinParts,
    mut commands: Commands,
    mut logged: Local<bool>,
) {
    // One log line per session, the first frame any host exists.
    if !*logged && !hosts.is_empty() {
        *logged = true;
        info!("doodad anim: first host armed");
    }
    let now = time.elapsed_secs();
    let world_cam = cam.single().ok();
    let (farclip, exterior_gate, camera_instance) =
        scene.scene(world_cam.as_ref().map(|(tf, _, proj, _)| (&**tf, &**proj)));
    let verdicts_still = world_cam.as_ref().is_some_and(|(tf, _, proj, local)| {
        !tf.is_changed() && !proj.is_changed() && !local.as_ref().is_some_and(|l| l.is_changed())
    }) && !scene.changed()
        && changed_vis.is_empty();
    // Static hosts retain the assembler's already-world-space sphere.
    let in_frustum = |frustum: &bevy::camera::primitives::Frustum,
                      fade: &crate::particles::EmitterFade| {
        frustum.intersects_sphere(
            &bevy::camera::primitives::Sphere {
                center: fade.center.into(),
                radius: fade.radius,
            },
            false,
        )
    };
    for (entity, mut host, parent_bound, lazy, pose, has_rig, player, drive) in &mut hosts {
        // Mover bounds are rejudged even with a fixed camera and unchanged part visibility.
        // Resolve the parent, not the host: the bound already includes the prop placement.
        let moving_fade = parent_bound.and_then(|bound| {
            let parent = worlds.get(bound.parent).ok()?;
            let mut fade = host.fade.clone();
            fade.center = parent.transform_point(bound.center);
            fade.radius = bound.radius * parent.compute_transform().scale.abs().max_element();
            Some(fade)
        });
        let fade = moving_fade.as_ref().unwrap_or(&host.fade);
        // A host born this frame has no verdict to reuse (`active` starts false).
        let drawn = if parent_bound.is_none() && verdicts_still && !host.is_added() {
            host.active
        } else if parent_bound.is_some() && moving_fade.is_none() {
            false // a lost parent cannot supply a world-space bound
        } else if host.meshes.is_empty() {
            // Meshless: the reference ticks any model in the draw set, and a 0-batch model is
            // admitted on its fade sphere like its emitters, through the same `in_draw_set`.
            world_cam.as_ref().is_some_and(|(cam_tf, frustum, _, _)| {
                fade.in_draw_set(
                    cam_tf.translation(),
                    Vec3::from(cam_tf.forward()),
                    farclip,
                    in_frustum(frustum, fade),
                    fade.exterior_admitted(&exterior_gate, camera_instance),
                    scene.room_admits(fade),
                )
            })
        } else {
            // Meshed: the fade and cull authorities' `Hidden` carries their verdicts, and the
            // frustum term is the one Bevy's culling (`ViewVisibility`) never writes here; the
            // reference animates only the worklist, which passes the 6-plane test (`0x683700`).
            host.meshes
                .iter()
                .any(|&e| vis.get(e).is_ok_and(|v| *v != Visibility::Hidden))
                && world_cam
                    .as_ref()
                    .is_none_or(|(_, frustum, _, _)| in_frustum(frustum, fade))
        };
        // The lazy-rig promote, retried every frame the host stays drawn so a full table is only
        // a delay. It needs a second drawn frame (`host.active`): on its first, a spawned part's
        // `Visibility` is still the `Inherited` default the fade and cull authorities have not
        // judged, so every far doodad of a new tile reads drawn.
        if drawn && host.active && !has_rig {
            if let Some(lazy) = lazy {
                lazy::promote_lazy_rig(
                    &mut commands,
                    &mut palettes,
                    &ibps,
                    &worlds,
                    entity,
                    lazy,
                    pose,
                    &mut twin_parts,
                );
            }
        }
        if drawn == host.active {
            continue;
        }
        host.active = drawn;
        // `AnimParked` holds the pose evaluation, compose and finalize; these commands apply
        // before `AnimationSystems`, so a resume's re-arm evaluates the frame the marker drops.
        if drawn {
            commands.entity(entity).remove::<AnimParked>();
        } else {
            host.parked_at = now; // the reaper's age key
            commands.entity(entity).insert(AnimParked);
        }
        if let Some(mut p) = player {
            if drawn {
                if let Some((node, duration)) = host.clip {
                    let anim = p.start(node);
                    anim.repeat();
                    if duration > 0.0 {
                        // Phase from the current arm, not from spawn: the variation re-arms.
                        anim.seek_to((now - host.armed_at).rem_euclid(duration));
                    }
                }
            } else {
                // Stop, not pause: `animate_targets` still applies a paused animation each frame.
                p.stop_all();
            }
        }
        if let Some(mut d) = drive {
            // No re-seek: the drive's cursor, sceneNow − attach, is a pure function of the clock.
            d.set_paused(!drawn);
        }
    }
}

pub struct DoodadAnimPlugin;

impl Plugin for DoodadAnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UvAnimMaterials>();
        app.init_resource::<TintAnimMaterials>();
        // The one `rand()` stream: the reference seeds it once per process before `WinMain`
        // (`srand(GetTickCount())`, `0x5d1c70`); a capture leaves it unseeded.
        app.init_resource::<wrathplus_assets::AnimRng>();
        let deterministic = crate::dev_state::deterministic_run();
        app.world_mut()
            .resource_mut::<wrathplus_assets::AnimRng>()
            .seed_for_session(deterministic);
        // The re-roll runs before the draw gate, or a host re-appearing as its window expires
        // resumes the old variation for a frame.
        app.add_systems(
            PostUpdate,
            (reroll_doodad_variation, gate_doodad_anim)
                .chain()
                .before(AnimationSystems),
        );
        app.add_systems(Update, lazy::reap_parked_rigs);
        // `sample_mat_anim` runs before the visibility authority, which composes its value the
        // same frame; the material tick runs after, since its draw gate reads that verdict.
        app.add_systems(
            Update,
            (
                sample_mat_anim.before(crate::model_render::ModelVisSet),
                tick_anim_materials.after(crate::model_render::ModelVisSet),
                // After the tick, so the probe's `row` column is this frame's write.
                mat_anim::matanim_probe.after(tick_anim_materials),
            ),
        );
    }
}

#[cfg(test)]
mod tests;
