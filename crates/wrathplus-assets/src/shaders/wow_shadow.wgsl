// Plus shadow/depth-only prepass for WowModelExt, Bevy 0.18.1.
// Not a forward, deferred, normal-buffer or temporal-animation implementation. No registration
// outside materials.rs: Classic has no world shadow sun. Bindings 90/100 and the StandardMaterial
// bindings are reused; the complete WowLight ABI below MUST track wow_model.wgsl, including
// every unread field before the runtime palette tail.
//
// WOW_RIG_SKIN: custom joint inputs 10/11; WOW_SHADOW_EXCLUDE: merged viewer-fade meshes.
// WOW_SHADOW_SOURCE_OPAQUE: future hook's explicit source provenance for an opaque fade twin.
// It MUST NOT be inferred from model_flags.y: authored translucent twins share that lane.
// Cutout twins already have marker bit 10. Both supported twin types ignore instance fade
// tags and camera distance; zfill/far-side copies never cast. Environment-map and UI-clip
// materials are deliberately excluded too: their coverage is camera-dependent.

#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::view,
    prepass_io::VertexOutput,
    pbr_bindings,
    pbr_types,
}

struct ModelParams {
    clutter_fade: vec4<f32>,
    model_flags: vec4<f32>,
    // x = the terrain-shade selector (see the doodad sun), y = the WMO batch order, zw = the
    // UV-scroll seed.
    sun_scale: vec4<f32>,
    // xyz = the animated M2Color tint (identity when static); w = the WMO interior batch class:
    // 0 exterior law, 1 INT, 2 TRANS.
    tint: vec4<f32>,
    // WMO glass, 0 on M2: xyz = the MOMT SIDN (0x10) emissive (gamma /255), w = MOMT WINDOW (0x20).
    sidn: vec4<f32>,
    wmo_unified: vec4<f32>,
    // Rows of `wow_light.matanim`, 0 = identity: x = UV scroll, y = tint, z = texture-transform
    // affine, w = the UI tile's cell clip.
    anim_slots: vec4<f32>,
    stage2: vec4<f32>,
    stage2_anim: vec4<f32>,
    stage2_seed: vec4<f32>,
    light_sphere: vec4<f32>,
};
fn second_stage_uv(uv: vec2<f32>) -> vec2<f32> {
    if (m.stage2.z > 2.0) { return uv; }
    let uv_t = uv + m.stage2_seed.xy + wow_light.matanim[u32(m.stage2_anim.x)].xy;
    let affine = wow_light.matanim[u32(m.stage2_anim.z)];
    let d = (uv_t - vec2<f32>(0.5)) * vec2<f32>(1.0 + affine.z,1.0 + affine.w);
    let c = 1.0 + affine.x;
    return vec2<f32>(0.5 + d.x*c-d.y*affine.y,0.5+d.x*affine.y+d.y*c);
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var second_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var second_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> m: ModelParams;

// The shared light buffer (`lighting::global_light`, rows as packed there), updated once per
// frame; terrain.wgsl mirrors the prefix, so a tree and the ground under it fog alike.
struct WowLight {
    light_ambient: vec4<f32>, // rgb ambient; w = Mod2x scale
    light_diffuse: vec4<f32>, // rgb sun diffuse; w = clamp-light flag (>0.5 ⇒ saturate)
    light_sun: vec4<f32>,     // xyz sun travel dir (to-light = −xyz); w = directional enable
    light_spec: vec4<f32>,    // terrain's specular (w = its shininess); unread here
    fog_color: vec4<f32>,     // rgb row-7 fog (gamma); w = enable (>0.5)
    fog_params: vec4<f32>,    // x = start, y = end, w = farclip wall; z unused
    // Rows 6-12: the reference's `Model2.bls` order-2 SH probe of the scene light at intensity 1:
    // ambient DC in the c10 `.w` lanes, the sun's bands in c10.xyz, c13 and c16.xyz. Every sun band
    // is linear in the committed colour, so all scale by the intensity (never I²).
    sh_c10_r: vec4<f32>,
    sh_c10_g: vec4<f32>,
    sh_c10_b: vec4<f32>,
    sh_c13_r: vec4<f32>,
    sh_c13_g: vec4<f32>,
    sh_c13_b: vec4<f32>,
    // xyz = the c16 quadratic band (x²−y²) per channel; w unused.
    sh_c16: vec4<f32>,
    _water: array<vec4<f32>, 4>, // rows 13-16: the liquid swatches, unread here
    // x = the SIDN night fraction (1 overnight, 0 by day); yzw = the sun's SH DC at intensity 1.
    grade: vec4<f32>,
    // Rows 18-19: the interior fog, the 4 s camera-in-WMO MFOG crossfade (the scene fog outdoors).
    wmo_fog_color: vec4<f32>,    // rgb interior fog (gamma); w = enable (mirrors fog_color.w)
    wmo_fog_params: vec4<f32>,   // x = start yd, y = end yd; zw unused
    // Row 20 `.x` = the point-light count, then per light `[pos.xyz, range]`, `[rgb, 0]`. Here, not
    // in Bevy's clusterables, which the view layout gives the fragment stage only.
    point_count: vec4<f32>,
    points: array<vec4<f32>, 512>,
    // The interior-prop SH probes, 7 rows per slot (`MAX_PROP_PROBES` = 8192 slots). Only this
    // shader declares this tail; the other shaders bind the same buffer by its prefix.
    prop_probes: array<vec4<f32>, 57344>,
    // Per rig slot (sizes mirrored in rig_palette.rs): the base bone index into `palettes`, whose
    // 3 rows per bone are `rig_from_joint × inverse_bindpose` from the rig's own origin.
    rig_table: array<u32, 2048>,
    // Per rig slot: the CM2 body tint (`model+0x184/188/18c`) packed `0xFFRRGGBB` like the
    // reference's node value (`0x60d840`: `param | 0xff000000`); 0 is identity.
    rig_tint: array<u32, 2048>,
    // Per rig slot: the world origin its palette rows are measured from.
    rig_origin: array<vec4<f32>, 2048>,
    // The mat-anim rows (size mirrored in mat_anim_table.rs), row 0 zero: a UV-scroll delta (xy),
    // a tint delta (xyz), a texture-transform affine `[cos − 1, sin, sx − 1, sy − 1]` or a UI cell.
    matanim: array<vec4<f32>, 2048>,
    // Per rig slot, the straddle waterline (size mirrored in straddle.rs): x = its world height
    // (Bevy Y), y = the side the near copy keeps (+1 above, −1 below, 0 not straddling).
    water_clip: array<vec2<f32>, 2048>,
    palettes: array<vec4<f32>>,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(90) var<storage, read> wow_light: WowLight;

fn wow_normalize(v: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), normalize(v), dot(v, v) > 1e-12);
}

#ifdef WOW_RIG_SKIN
fn wow_rig_slot(instance_index: u32) -> u32 {
    return (mesh_functions::get_tag(instance_index) >> 19u) & 0x7ffu;
}

// Blends the four weighted bones' palette rows into `rig_from_local`, which replaces the mesh's
// world matrix like Bevy's `skin_model`; its translation is relative to `rig_origin[slot]`.
fn wow_skin_model(instance_index: u32, indices: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
    let base = wow_light.rig_table[wow_rig_slot(instance_index)];
    let b0 = 3u * (base + indices.x);
    let b1 = 3u * (base + indices.y);
    let b2 = 3u * (base + indices.z);
    let b3 = 3u * (base + indices.w);
    let r0 = weights.x * wow_light.palettes[b0]
        + weights.y * wow_light.palettes[b1]
        + weights.z * wow_light.palettes[b2]
        + weights.w * wow_light.palettes[b3];
    let r1 = weights.x * wow_light.palettes[b0 + 1u]
        + weights.y * wow_light.palettes[b1 + 1u]
        + weights.z * wow_light.palettes[b2 + 1u]
        + weights.w * wow_light.palettes[b3 + 1u];
    let r2 = weights.x * wow_light.palettes[b0 + 2u]
        + weights.y * wow_light.palettes[b1 + 2u]
        + weights.z * wow_light.palettes[b2 + 2u]
        + weights.w * wow_light.palettes[b3 + 2u];
    // r0/r1/r2 are the affine's rows; a WGSL matrix is column-major.
    return mat4x4<f32>(
        vec4<f32>(r0.x, r1.x, r2.x, 0.0),
        vec4<f32>(r0.y, r1.y, r2.y, 0.0),
        vec4<f32>(r0.z, r1.z, r2.z, 0.0),
        vec4<f32>(r0.w, r1.w, r2.w, 1.0),
    );
}

// bevy_pbr::skinning's inverse-transpose via the adjugate, verbatim.
fn inverse_transpose_3x3m(in: mat3x3<f32>) -> mat3x3<f32> {
    let x = cross(in[1], in[2]);
    let y = cross(in[2], in[0]);
    let z = cross(in[0], in[1]);
    let det = dot(in[2], z);
    return mat3x3<f32>(x / det, y / det, z / det);
}

fn wow_skin_normals(frame_from_local: mat4x4<f32>, normal: vec3<f32>) -> vec3<f32> {
    return wow_normalize(
        inverse_transpose_3x3m(mat3x3<f32>(
            frame_from_local[0].xyz,
            frame_from_local[1].xyz,
            frame_from_local[2].xyz
        )) * normal
    );
}
#endif

// Bevy prepass input locations (NOT the forward layout). Merged attributes 12/13
// are intentionally absent: extra fields in the mesh buffer are harmless when not requested.
struct WowShadowVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_UVS_A
    @location(1) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(2) uv_b: vec2<f32>,
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    @location(3) normal: vec3<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) tangent: vec4<f32>,
#endif
#endif
#ifdef VERTEX_COLORS
    @location(7) color: vec4<f32>,
#endif
#ifdef WOW_RIG_SKIN
    @location(10) joint_indices: vec4<u32>,
    @location(11) joint_weights: vec4<f32>,
#endif
}

@vertex
fn vertex(v: WowShadowVertex) -> VertexOutput {
    var out: VertexOutput;
#ifdef WOW_RIG_SKIN
    let frame_from_local = wow_skin_model(v.instance_index, v.joint_indices, v.joint_weights);
    let frame_origin = wow_light.rig_origin[wow_rig_slot(v.instance_index)].xyz;
#else
    var frame_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let frame_origin = frame_from_local[3].xyz;
    frame_from_local[3] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#endif
    // Same origin-relative palette and precision strategy as the visible model pass.
    // This view is the light view, never the player camera used by the forward fade law.
    let p_cam = (frame_from_local * vec4<f32>(v.position, 1.0)).xyz
        + (frame_origin - view.world_position);
    out.world_position = vec4<f32>(p_cam + view.world_position, 1.0);
    let view_rot = mat3x3<f32>(view.view_from_world[0].xyz,
        view.view_from_world[1].xyz, view.view_from_world[2].xyz);
    out.position = view.clip_from_view * vec4<f32>(view_rot * p_cam, 1.0);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = v.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = v.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = v.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = v.instance_index;
#endif
    // These fields keep the Bevy interface well formed, not a promise of camera-prepass
    // support. The future hook must restrict this shader to shadow/depth-only pipelines.
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
#ifdef WOW_RIG_SKIN
    out.world_normal = wow_skin_normals(frame_from_local, v.normal);
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
#endif
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        frame_from_local, v.tangent, v.instance_index);
#endif
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.previous_world_position = out.world_position;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = 0;
#endif
    return out;
}

fn shadow_uv(uv: vec2<f32>) -> vec2<f32> {
    // Exact M2 affine, including translation before scale/rotation about (1/2, 1/2).
    let uv_t = uv + m.sun_scale.zw + wow_light.matanim[u32(m.anim_slots.x)].xy;
    let affine = wow_light.matanim[u32(m.anim_slots.z)];
    let d = (uv_t - vec2<f32>(0.5)) * vec2<f32>(1.0 + affine.z, 1.0 + affine.w);
    let c = 1.0 + affine.x;
    return vec2<f32>(0.5 + d.x * c - d.y * affine.y, 0.5 + d.x * affine.y + d.y * c);
}

struct ShadowOutput {
    @builtin(frag_depth) frag_depth: f32,
#ifdef NORMAL_PREPASS
    @location(0) normal: vec4<f32>,
#endif
#ifdef MOTION_VECTOR_PREPASS
    @location(1) motion_vector: vec2<f32>,
#endif
#ifdef DEFERRED_PREPASS
    @location(2) deferred: vec4<u32>,
    @location(3) deferred_lighting_pass_id: u32,
#endif
}

#ifdef WOW_SHADOW_FRAGMENT
@fragment
fn fragment(in: VertexOutput) -> ShadowOutput {
    var out: ShadowOutput;
    let markers = u32(m.clutter_fade.z);
    // 0/1 no depth, 2 glow, 7/8 multiply, 9 zfill, 11 far side, 12 env map, 13 sky.
    // Neither depth state nor distance fade is borrowed from the forward pipeline.
    if ((markers & 0x3b87u) != 0u || m.clutter_fade.w > 0.5 || m.anim_slots.w > 0.5) {
        discard;
    }
#ifdef WOW_SHADOW_EXCLUDE
    discard;
#endif
    let flags = pbr_bindings::material.flags;
    let mode = flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    let is_opaque = mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE;
    let is_mask = mode == pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_MASK;
    let source_cutout = (markers & 1024u) != 0u;
    var source_opaque = false;
#ifdef WOW_SHADOW_SOURCE_OPAQUE
    source_opaque = true;
#endif
    let is_twin = m.model_flags.y > 0.5;
    // Author translucent is never approximated by Bevy's 0.05 blend-shadow cutoff.
    // Only explicit source provenance permits a Blend twin to cast.
    if (is_twin) {
        if (!source_cutout && !source_opaque) { discard; }
    } else if (!is_opaque && !is_mask) {
        discard;
    }

    if (m.stage2.x > 0.5 && m.stage2.z > 2.0) { discard; }
    var alpha = pbr_bindings::material.base_color.a;
#ifdef VERTEX_COLORS
    // Classic MOCV.a is lighting; ordinary unified MOCV.a is coverage, as in forward.
    if (m.model_flags.x <= 0.5 || m.wmo_unified.w > 0.5) { alpha *= in.color.a; }
#endif
#ifdef VERTEX_UVS
#ifdef STANDARD_MATERIAL_BASE_COLOR_UV_B
    var uv = in.uv_b;
#else
    var uv = shadow_uv(in.uv);
#endif
    uv = (pbr_bindings::material.uv_transform * vec3<f32>(uv, 1.0)).xy;
    if ((flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u) {
        // No light-camera mip bias or distance-dependent dither/fade. Normal implicit LOD.
        let t0a = textureSample(pbr_bindings::base_color_texture,
            pbr_bindings::base_color_sampler, uv).a;
        if (m.stage2.x > 0.5 && u32(m.stage2.y) == 8u) {
            alpha += t0a; // Combiners_Add_Mod: `primary.a + t0.a`, under t1.a below
        } else if (m.stage2.x < 0.5 || m.stage2.w < 0.5) {
            alpha *= t0a;
        }
    }
#endif
#ifdef VERTEX_UVS
    if (m.stage2.x > 0.5) {
        let op = u32(m.stage2.y);
        var second_uv = in.uv;
#ifdef VERTEX_UVS_B
        second_uv = in.uv_b;
#endif
        let a1 = textureSample(second_texture, second_sampler, second_stage_uv(second_uv)).a;
        if ((is_mask || source_cutout) && m.stage2.z > 2.0 && op != 0u && op != 6u && op != 7u) { discard; }
        // Combiners_*_Add: `primary.a + t1.a` (`t0.a·primary.a + t1.a` after Mod).
        if (op == 3u) {
            alpha += a1;
        }
        else if (op != 0u && op != 6u && op != 7u) {
            alpha *= a1 * select(1.0, 2.0, op == 4u);
        }
    }
#endif
    if (source_cutout && alpha < 224.0 / 255.0) { discard; }
    if (!is_twin && is_mask && alpha < pbr_bindings::material.alpha_cutoff) { discard; }
    // Opaque sources ignore even garbage texture alpha, just like their canonical opaque draw.
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = in.unclipped_depth;
#else
    out.frag_depth = in.position.z;
#endif
#ifdef NORMAL_PREPASS
    out.normal = vec4<f32>(in.world_normal * 0.5 + vec3<f32>(0.5), 1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = vec2<f32>(0.0);
#endif
#ifdef DEFERRED_PREPASS
    out.deferred = vec4<u32>(0u);
    out.deferred_lighting_pass_id = 0u;
#endif
    return out;
}
#endif
