//! Ignored, headless real-device validation of the production WoW shadow prepass.
#![cfg(test)]
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemState;
use bevy::mesh::{MeshVertexBufferLayoutRef, MeshVertexBufferLayouts};
use bevy::pbr::*;
use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::{initialize_renderer, RenderDevice};
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::texture::FallbackImage;
use bevy::render::{RenderApp, RenderPlugin, RenderStartup};
use bevy::shader::{PipelineCacheError, ShaderCache, ShaderCacheSource, ShaderRef, ValidateShader};
use std::{any::TypeId, borrow::Cow, sync::Arc, time::Duration};
use wrathplus_assets::materials::{WowModelExt, WowModelMaterial};

mod fixtures;
use fixtures::{fixture_group, load_module, specialize_material};

#[test]
#[ignore = "on-demand real GPU WoW shadow depth draws/readback; no window or game"]
fn wow_shadowcaster_gpu_smoke() {
    eprintln!("shadow smoke: initialize renderer");
    let resources = futures_lite::future::block_on(initialize_renderer(
        Backends::all(),
        None,
        &WgpuSettings::default(),
    ));
    let device = resources.0.clone();
    let queue = resources.1.clone();
    println!("WoW shadow GPU adapter: {:?}", resources.2 .0);
    // The fixture uses the desktop storage MeshUniform ABI, not the WebGL uniform batch ABI.
    assert!(
        device.limits().max_storage_buffers_per_shader_stage > 0,
        "this on-demand fixture requires desktop storage buffers"
    );
    eprintln!("shadow smoke: embedded shader library");
    let (mut shaders, mut assets) = crate::shaders::tests::library_cache(ShaderCache::new(
        device.features(),
        DownlevelFlags::all(),
        load_module,
    ));
    eprintln!("shadow smoke: build manual render app");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::winit::WinitPlugin>()
            // Keep RenderApp in-process for direct startup/resource inspection.
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .set(WindowPlugin {
                primary_window: None,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Manual(resources),
                ..default()
            }),
    );
    app.add_plugins(crate::shaders::plugin);
    wrathplus_assets::materials::register_shaders(&mut app);
    eprintln!("shadow smoke: plugin finish");
    app.finish();
    eprintln!("shadow smoke: plugin cleanup");
    app.cleanup();
    // No app.update(): no extraction, game systems, windows, or asynchronous asset IO required.
    let render = app.sub_app_mut(RenderApp);
    eprintln!("shadow smoke: render startup");
    render.world_mut().run_schedule(RenderStartup);
    eprintln!("shadow smoke: startup complete");
    // DefaultPlugins initialized MeshPipeline via its real FromWorld implementation; startup
    // ran init_material_pipeline and init_prepass_pipeline with that actual device/adapter.
    assert!(render.world().contains_resource::<MeshPipeline>());
    let prepass = render
        .world_mut()
        .remove_resource::<PrepassPipeline>()
        .expect("production init_prepass_pipeline ran");
    // Reuse the library's shader ID: inserting an alias for the same import path can
    // create cyclic dependents in ShaderCache::clear when the alias is inserted twice.
    let shader_id = assets
        .iter()
        .find(|(_, shader)| shader.path.ends_with("/wow_shadow.wgsl"))
        .expect("embedded shadow shader")
        .0;
    for reference in [
        WowModelMaterial::prepass_vertex_shader(),
        WowModelMaterial::prepass_fragment_shader(),
    ] {
        let ShaderRef::Path(path) = reference else {
            panic!("expected production path")
        };
        assert_eq!(
            path.to_string(),
            "embedded://wrathplus_assets/shaders/wow_shadow.wgsl"
        );
    }
    let vertex = assets
        .get_strong_handle(shader_id)
        .expect("library shader handle");
    let fragment = vertex.clone();
    eprintln!("shadow smoke: reused embedded shadow shader");
    let material_layout = WowModelMaterial::bind_group_layout_descriptor(&device);
    // Own the cache while SystemState temporarily borrows the render world for AsBindGroup.
    let cache = render
        .world_mut()
        .remove_resource::<PipelineCache>()
        .expect("render startup pipeline cache");
    // The light fixture follows the production WowLight payload ABI, NOT a substitute layout.
    // Zero row 0 matanim means identity; palette bone zero is identity for the custom-rig case.
    let palette_offset = crate::rig_palette::palette_region_offset() as usize;
    let mut light_bytes = vec![0u8; palette_offset + 3 * 16];
    for row in 0..3 {
        let offset = palette_offset + row * 16 + row * 4;
        light_bytes[offset..offset + 4].copy_from_slice(&1f32.to_ne_bytes());
    }
    let light = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("shadow fixture WowLight and identity palette"),
        contents: &light_bytes,
        usage: BufferUsages::STORAGE,
    });
    let mut mesh_layouts = MeshVertexBufferLayouts::default();
    for (case, alpha_mode, alpha, markers, rig, casts, wmo, unified, vertex_alpha) in [
        ("opaque", AlphaMode::Opaque, 1., 0., false, true),
        ("mask pass", AlphaMode::Mask(0.5), 1., 0., false, true),
        ("mask discard", AlphaMode::Mask(0.5), 0., 0., false, false),
        (
            "keyed no-depth-write exclusion",
            AlphaMode::Opaque,
            1.,
            1.,
            false,
            false,
        ),
        ("custom rig identity", AlphaMode::Opaque, 1., 0., true, true),
    ]
    .into_iter()
    .map(|(case, mode, alpha, markers, rig, casts)| {
        (case, mode, alpha, markers, rig, casts, false, false, 1.)
    })
    .chain([
        // Texture/base alpha=1: only the production vertex coverage law can discard these.
        (
            "unified WMO vertex cutout discard",
            AlphaMode::Mask(224. / 255.),
            1.,
            0.,
            false,
            false,
            true,
            true,
            0.5,
        ),
        (
            "unified WMO vertex cutout pass",
            AlphaMode::Mask(224. / 255.),
            1.,
            0.,
            false,
            true,
            true,
            true,
            0.9,
        ),
        (
            "unified WMO alpha applied once",
            AlphaMode::Mask(224. / 255.),
            1.,
            0.,
            false,
            true,
            true,
            true,
            0.94,
        ),
        (
            "classic WMO ignores vertex alpha",
            AlphaMode::Mask(224. / 255.),
            1.,
            0.,
            false,
            true,
            true,
            false,
            0.,
        ),
        (
            "M2 keeps vertex coverage",
            AlphaMode::Mask(224. / 255.),
            1.,
            0.,
            false,
            false,
            false,
            false,
            0.5,
        ),
    ]) {
        eprintln!("shadow smoke: begin {case}");
        let material = WowModelMaterial {
            base: StandardMaterial {
                base_color: Color::srgba(1., 1., 1., alpha),
                alpha_mode,
                cull_mode: None,
                ..default()
            },
            extension: WowModelExt {
                clutter_fade: Vec4::new(0., 0., markers, 0.),
                model_flags: Vec4::new(if wmo { 1. } else { 0. }, 0., 0., 0.),
                sun_scale: Vec4::ZERO,
                tint: Vec4::ONE,
                sidn: Vec4::ZERO,
                wmo_unified: Vec4::new(0., 0., 0., if unified { 1. } else { 0. }),
                anim_slots: Vec4::ZERO,
                stage2: Vec4::ZERO,
                stage2_anim: Vec4::ZERO,
                stage2_seed: Vec4::ZERO,
                light_sphere: Vec4::ZERO,
                texture2: None,
                light_buf: light.clone(),
            },
        };
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        // Oversized CCW triangle covers the entire tiny target at reverse-Z depth 0.5.
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-1., -1., 0.5], [3., -1., 0.5], [-1., 3., 0.5]],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5f32, 0.5]; 3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1., 1., 1., vertex_alpha]; 3]);
        if rig {
            mesh.insert_attribute(
                wrathplus_assets::ATTRIBUTE_WOW_JOINT_INDEX,
                bevy::mesh::VertexAttributeValues::Uint16x4(vec![[0u16; 4]; 3]),
            );
            mesh.insert_attribute(
                wrathplus_assets::ATTRIBUTE_WOW_JOINT_WEIGHT,
                vec![[1f32, 0., 0., 0.]; 3],
            );
        }
        let mesh_layout = mesh.get_mesh_vertex_buffer_layout(&mut mesh_layouts);
        let mut mesh_key = MeshPipelineKey::from_msaa_samples(1)
            | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::TriangleList)
            | MeshPipelineKey::UNCLIPPED_DEPTH_ORTHO;
        if matches!(alpha_mode, AlphaMode::Mask(_)) {
            mesh_key |= MeshPipelineKey::MAY_DISCARD;
        }
        let material_key = ErasedMaterialKey::new(material.bind_group_data());
        let mut properties = MaterialProperties {
            alpha_mode,
            material_layout: Some(material_layout.clone()),
            material_key: material_key.clone(),
            specialize: Some(specialize_material),
            bindless: false,
            shadows_enabled: true,
            prepass_enabled: true,
            ..default()
        };
        properties.add_shader(PrepassVertexShader, vertex.clone());
        properties.add_shader(PrepassFragmentShader, fragment.clone());
        let specializer = PrepassPipelineSpecializer {
            pipeline: prepass.clone(),
            properties: Arc::new(properties),
        };
        let mut desc = specializer
            .specialize(
                ErasedMaterialPipelineKey {
                    mesh_key,
                    material_key,
                    type_id: TypeId::of::<WowModelMaterial>(),
                },
                &mesh_layout,
            )
            .expect("production WoW prepass specialization");
        // PipelineCache appends this device-global definition when compiling a descriptor.
        // Direct ShaderCache composition must reproduce that production step.
        let storage = bevy::shader::ShaderDefVal::UInt(
            "AVAILABLE_STORAGE_BUFFER_BINDINGS".into(),
            device.limits().max_storage_buffers_per_shader_stage,
        );
        desc.vertex.shader_defs.push(storage.clone());
        desc.fragment.as_mut().unwrap().shader_defs.push(storage);
        assert_eq!(desc.layout.len(), 4);
        assert_eq!(desc.layout[3], material_layout);
        let fragment = desc.fragment.as_ref().expect("WoW shadow fragment hook");
        assert!(
            fragment.targets.is_empty(),
            "shadow pass must be depth-only"
        );
        let unprepared = {
            let world = render.world_mut();
            let mut state = SystemState::<<WowModelMaterial as AsBindGroup>::Param>::new(world);
            let mut param = state.get_mut(world);
            material
                .unprepared_bind_group(
                    &cache.get_bind_group_layout(&desc.layout[3]),
                    &device,
                    &mut param,
                    false,
                )
                .expect("production material unprepared AsBindGroup")
        };
        // Follow Bevy material preparation: #[data] resources need the allocator.
        let mut allocator = MaterialBindGroupAllocator::new(
            &device,
            "shadow material",
            None,
            desc.layout[3].clone(),
            None,
        );
        let binding = allocator.allocate_unprepared(unprepared, &desc.layout[3]);
        allocator.prepare_bind_groups(
            &device,
            &cache,
            render.world().resource::<FallbackBindlessResources>(),
            render.world().resource::<FallbackImage>(),
        );
        let material_group = allocator.get(binding.group).unwrap().bind_group().unwrap();
        let layouts: Vec<_> = desc
            .layout
            .iter()
            .map(|d| cache.get_bind_group_layout(d))
            .collect();
        let refs: Vec<_> = layouts.iter().map(|layout| &**layout).collect();
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some(case),
            bind_group_layouts: &refs,
            push_constant_ranges: &[],
        });
        let vertex_module = shaders
            .get(
                &device,
                0,
                desc.vertex.shader.id(),
                &desc.vertex.shader_defs,
            )
            .expect("production shadow vertex composition");
        let fragment_module = shaders
            .get(&device, 1, fragment.shader.id(), &fragment.shader_defs)
            .expect("production shadow fragment composition");
        let vertex_layouts: Vec<_> = desc
            .vertex
            .buffers
            .iter()
            .map(|buffer| RawVertexBufferLayout {
                array_stride: buffer.array_stride,
                step_mode: buffer.step_mode,
                attributes: &buffer.attributes,
            })
            .collect();
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some(case),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &vertex_module,
                entry_point: desc.vertex.entry_point.as_deref(),
                compilation_options: default(),
                buffers: &vertex_layouts,
            },
            fragment: Some(RawFragmentState {
                module: &fragment_module,
                entry_point: fragment.entry_point.as_deref(),
                compilation_options: default(),
                targets: &fragment.targets,
            }),
            primitive: desc.primitive,
            depth_stencil: desc.depth_stencil.clone(),
            multisample: desc.multisample,
            multiview: None,
            cache: None,
        });
        let groups: Vec<_> = desc.layout[..3]
            .iter()
            .enumerate()
            .map(|(i, descriptor)| fixture_group(&device, &cache, descriptor, i == 0, i == 2))
            .collect();
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(case),
            contents: &mesh.create_packed_vertex_buffer_data(),
            usage: BufferUsages::VERTEX,
        });
        let depth = device.create_texture(&TextureDescriptor {
            label: Some(case),
            size: Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: desc.depth_stencil.as_ref().unwrap().format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        assert_eq!(
            desc.depth_stencil.as_ref().unwrap().format,
            TextureFormat::Depth32Float
        );
        let depth_view = depth.create_view(&default());
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("shadow depth readback"),
            size: 256 * 4,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&CommandEncoderDescriptor { label: Some(case) });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(case),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            for (index, (group, offsets)) in groups.iter().enumerate() {
                pass.set_bind_group(index as u32, &**group, offsets);
            }
            pass.set_bind_group(3, &**material_group, &[]);
            pass.set_vertex_buffer(0, *vertices.slice(..));
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &depth,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::DepthOnly,
            },
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(4),
                },
            },
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        readback.slice(..).map_async(MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(30)),
            })
            .expect("shadow draw/readback completed");
        rx.recv_timeout(Duration::from_secs(30))
            .expect("map callback")
            .expect("map depth readback");
        let bytes = readback.slice(..).get_mapped_range();
        for y in 0..4 {
            for x in 0..4 {
                let offset = y * 256 + x * 4;
                let actual = f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let expected = if casts { 0.5 } else { 0. };
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "{case}: depth at ({x}, {y}) = {actual}, expected {expected}"
                );
            }
        }
        drop(bytes);
        readback.unmap();
        println!("WoW shadow GPU validated: {case}");
    }
}
