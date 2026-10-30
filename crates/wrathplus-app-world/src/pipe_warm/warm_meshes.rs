//! The menagerie's stand-in meshes: a quad per model-lane layout, built through the production
//! submesh builders, and the sky and water lanes' own layouts.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use wrathplus_formats::{FogPolicy, ModelBlend, RenderSubmesh};

/// A tiny triangle with POSITION + UV_0 only, the `Stars.m2` layout. Main-world-resident, so
/// `calculate_bounds` covers it.
pub(super) fn warm_pos_uv_mesh() -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
    );
    m.insert_indices(Indices::U32(vec![0, 1, 2]));
    m
}

/// A tiny quad in the liquid grid's layout: POSITION + NORMAL + UV_0 + UV_1, plus
/// `ATTRIBUTE_COLOR` when `body_color` is set, as `liquid_bevy_mesh` does for a non-fullbright
/// interior WMO pool.
pub(super) fn warm_liquid_mesh(body_color: bool) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    );
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs);
    if body_color {
        // As `liquid_bevy_mesh` inserts it: one RGBA per vertex, alpha 1.
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; 4]);
    }
    m.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    m
}

/// A unit quad in each attribute combination the model lane ships. Every `RenderSubmesh` field is
/// spelled out so a new field breaks this build.
pub(super) fn warm_quad(colors: bool, skinned: bool) -> RenderSubmesh {
    let n = 4usize;
    RenderSubmesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; n],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        uvs2: Vec::new(),
        stage2: None,
        indices: vec![0, 1, 2, 0, 2, 3],
        texture: None,
        skin_slot: None,
        geoset_id: 0,
        char_slot: None,
        blend: ModelBlend::Opaque,
        wrap_x: true,
        wrap_y: true,
        two_sided: false,
        joints: if skinned { vec![[0; 4]; n] } else { Vec::new() },
        weights: if skinned {
            vec![[1.0, 0.0, 0.0, 0.0]; n]
        } else {
            Vec::new()
        },
        vertex_colors: if colors {
            vec![[1.0, 1.0, 1.0, 1.0]; n]
        } else {
            Vec::new()
        },
        interior: false,
        emissive: false,
        icon_slot: false,
        uv_rot_seq: None,
        uv_scale_seq: None,
        sidn: None,
        window: false,
        additive: false,
        no_depth_write: false,
        no_depth_test: false,
        fog_policy: FogPolicy::Scene,
        env_map: false,
        billboard: None,
        welded_billboard: false,
        alpha_anim: None,
        uv_anim: None,
        uv_seq: None,
        rgb_anim: None,
        rgb_seq: None,
        wmo_batch: None,
        wmo_unified: None,
        section: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mesh's vertex layout: the attributes it carries, in order, with their formats.
    fn layout(m: &Mesh) -> Vec<String> {
        m.attributes()
            .map(|(a, _)| format!("{:?}:{:?}", a.id, a.format))
            .collect()
    }

    /// The live-compile hole of 2026-10-26: every production model mesh's layout, with or
    /// without a second texture stage, must be one the menagerie renders a rig through, or its
    /// whole material cross compiles live outside the cover.
    #[test]
    fn every_production_model_layout_is_one_the_warm_pass_renders() {
        let mut warmed = Vec::new();
        for colors in [false, true] {
            warmed.push(layout(&wrathplus_assets::submesh_to_static_mesh(
                &warm_quad(colors, false),
            )));
            warmed.push(layout(&wrathplus_assets::submesh_to_skinned_mesh(
                &warm_quad(colors, true),
            )));
        }
        for colors in [false, true] {
            for skinned in [false, true] {
                for second_stage in [false, true] {
                    let mut sub = warm_quad(colors, skinned);
                    if second_stage {
                        sub.uvs2 = vec![[0.5, 0.5]; sub.positions.len()];
                    }
                    let mesh = if skinned {
                        wrathplus_assets::submesh_to_skinned_mesh(&sub)
                    } else {
                        wrathplus_assets::submesh_to_static_mesh(&sub)
                    };
                    assert!(
                        warmed.contains(&layout(&mesh)),
                        "colors={colors} skinned={skinned} second_stage={second_stage}: \
                         layout {:?} is not warmed",
                        layout(&mesh)
                    );
                }
            }
        }
    }

    /// The merged static blobs keep one layout too, second stage or not.
    #[test]
    fn a_merged_blob_has_one_layout_with_or_without_a_second_stage() {
        let blob = |second: bool| {
            let mut sub = warm_quad(false, false);
            if second {
                sub.uvs2 = vec![[0.5, 0.5]; sub.positions.len()];
            }
            let part = std::sync::Arc::new(sub);
            let (mesh, ..) = wrathplus_assets::merged_static_mesh_faded(
                &[(part, Transform::IDENTITY)],
                &[Vec4::new(0.0, 0.0, 0.0, 1.0)],
                None,
            );
            layout(&mesh)
        };
        assert_eq!(blob(false), blob(true));
    }
}
