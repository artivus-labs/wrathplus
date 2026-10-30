//! Headless numerical execution of the pure function copied directly out of each production
//! shader. Full shader composition/validation is covered separately by the renderer smoke tests.
use bevy::render::render_resource::*;
use bevy::render::renderer::{initialize_renderer, RenderDevice, RenderQueue};
use bevy::render::settings::{Backends, WgpuSettings};
use std::{borrow::Cow, time::Duration};

const MODEL: &str = include_str!("../../wrathplus-assets/src/shaders/wow_model.wgsl");
const RETAINED: &str = include_str!("shaders/static_gx.wgsl");
const FUNCTION: &str = "wmo_unified_light";

struct Case {
    name: &'static str,
    ambient: [f32; 3],
    diffuse: [f32; 3],
    root_ambient: [f32; 3],
    residual: [f32; 4],
    ndotl: f32,
    interior: bool,
    unlit: bool,
    window: bool,
    point: [f32; 3],
    emission: [f32; 3],
    expected: [f32; 3],
}

/// Extract, never reimplement, the production function. Naga below checks that it is standalone:
/// an accidental new dependency on renderer globals/helpers must fail rather than be substituted.
fn production_function(source: &str) -> &str {
    let marker = format!("fn {FUNCTION}(");
    let start = source
        .find(&marker)
        .expect("production pure lighting function");
    let body = start + source[start..].find('{').expect("function body");
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[body..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=body + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated production lighting function")
}

fn vector(values: &[f32]) -> String {
    let components: Vec<_> = values.iter().map(|v| format!("{v:?}")).collect();
    format!("vec{}<f32>({})", values.len(), components.join(", "))
}

fn compute_source(production: &str, cases: &[Case]) -> String {
    let mut source = format!(
        "{}\n@group(0) @binding(0) var<storage, read_write> results: array<vec4<f32>>;\n\
         @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
         switch id.x {{\n",
        production_function(production)
    );
    for (i, case) in cases.iter().enumerate() {
        source.push_str(&format!(
            "case {i}u: {{ results[{i}] = vec4<f32>({FUNCTION}({}, {}, {}, {}, {:?}, {}, {}, {}, {}, {}), 1.0); }}\n",
            vector(&case.ambient), vector(&case.diffuse), vector(&case.root_ambient),
            vector(&case.residual), case.ndotl, case.interior, case.unlit, case.window,
            vector(&case.point), vector(&case.emission),
        ));
    }
    source.push_str("default: {}\n}\n}\n");
    source
}

fn execute(device: &RenderDevice, queue: &RenderQueue, label: &str, source: &str, cases: &[Case]) {
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some(label),
        source: ShaderSource::Wgsl(Cow::Borrowed(source)),
    });
    let layout = device.create_bind_group_layout(
        Some(label),
        &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    );
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let size = (cases.len() * 16) as u64;
    let buffer = |usage| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let output = buffer(BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let readback = buffer(BufferUsages::MAP_READ | BufferUsages::COPY_DST);
    let group = device.create_bind_group(
        label,
        &layout,
        &BindGroupEntries::single(output.as_entire_binding()),
    );
    let mut encoder =
        device.create_command_encoder(&CommandEncoderDescriptor { label: Some(label) });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &*group, &[]);
        pass.dispatch_workgroups(cases.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
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
        .expect("unified lighting dispatch/readback completed");
    rx.recv_timeout(Duration::from_secs(30))
        .expect("readback callback")
        .expect("map lighting results");
    let bytes = readback.slice(..).get_mapped_range();
    for (i, case) in cases.iter().enumerate() {
        for channel in 0..4 {
            let offset = i * 16 + channel * 4;
            let actual = f32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let expected = if channel == 3 {
                1.0
            } else {
                case.expected[channel]
            };
            assert!(
                actual.is_finite() && (actual - expected).abs() <= 2e-5,
                "{label}: {} channel {channel}: GPU {actual}, scalar oracle {expected}",
                case.name
            );
        }
        println!("{label}: numerical GPU case passed: {}", case.name);
    }
    drop(bytes);
    readback.unmap();
}

/// Run explicitly with `cargo test -p wrathplus-world --lib wmo_unified_lighting_gpu -- --ignored --nocapture`.
#[test]
#[ignore = "on-demand real GPU numerical lighting/readback; no window, game or install"]
fn wmo_unified_lighting_gpu() {
    let cases = cases();
    assert!(!cases.is_empty(), "numerical fixtures must not be vacuous");
    let resources = futures_lite::future::block_on(initialize_renderer(
        Backends::all(),
        None,
        &WgpuSettings::default(),
    ));
    println!("WMO unified lighting GPU adapter: {:?}", resources.2 .0);
    for (label, production) in [("wow_model", MODEL), ("static_gx", RETAINED)] {
        let source = compute_source(production, &cases);
        execute(&resources.0, &resources.1, label, &source, &cases);
    }
}

// Independent native ARB no-shadow oracle: c28=127/255, clamp vertex primary,
// pixel RGB×2. Literal cases separate light saturation from primary saturation.
fn cases() -> Vec<Case> {
    let h = 127.0 / 255.0;
    let seed = || Case {
        name: "black exterior",
        ambient: [0.2, 0.3, 0.4],
        diffuse: [0.6, 0.4, 0.2],
        root_ambient: [33.0 / 255.0; 3],
        residual: [0., 0., 0., 1.],
        ndotl: 0.5,
        interior: false,
        unlit: false,
        window: false,
        point: [0.; 3],
        emission: [0.; 3],
        expected: [h; 3],
    };
    let mut black = seed();
    black.expected = [h, h, h];
    let mut residual = seed();
    residual.name = "colored residual additive";
    residual.residual = [0.1, 0.2, 0.3, 1.];
    residual.expected = [h + 0.2, h + 0.4, h + 0.6];
    let mut alpha = seed();
    alpha.name = "zero alpha does not extinguish light";
    alpha.residual[3] = 0.;
    let mut interior = seed();
    interior.name = "interior root ambient not sunlight";
    interior.interior = true;
    interior.expected = [2.0 * 127.0 * 33.0 / (255.0 * 255.0); 3];
    let mut interior_unlit = seed();
    interior_unlit.name = "interior ignores UNLIT";
    interior_unlit.interior = true;
    interior_unlit.unlit = true;
    interior_unlit.expected = interior.expected;
    let mut unlit = seed();
    unlit.name = "exterior unlit copies residual";
    unlit.unlit = true;
    unlit.residual = [0.1, 0.3, 0.7, 1.];
    unlit.expected = [0.2, 0.6, 1.4];
    let mut lit_clamp = seed();
    lit_clamp.name = "light clamps before primary";
    lit_clamp.ambient = [2.; 3];
    lit_clamp.expected = [2.0 * h; 3];
    let mut primary_clamp = seed();
    primary_clamp.name = "primary clamps before pixel double";
    primary_clamp.residual = [0.9; 4];
    primary_clamp.expected = [2.; 3];
    let mut emission = seed();
    emission.name = "emission beside residual and light";
    emission.emission = [0.1, 0.2, 0.3];
    emission.expected = [h + 0.2, h + 0.4, h + 0.6];
    let mut point = seed();
    point.name = "local diffuse before light clamp";
    point.point = [0.1, 0.2, 0.8];
    point.expected = [1.2 * h, 1.4 * h, 2.0 * h];
    let mut window = seed();
    window.name = "WINDOW quantized midpoint +16";
    window.interior = true;
    window.window = true;
    window.ambient = [0.0, 33.0 / 255.0, 1.0];
    window.diffuse = [1.0, 100.0 / 255.0, 1.0];
    window.expected = [
        2.0 * h * (143.0 + 127.0 * 0.5) / 255.0,
        2.0 * h * (82.0 + 66.0 * 0.5) / 255.0,
        2.0 * h,
    ];
    vec![
        black,
        residual,
        alpha,
        interior,
        interior_unlit,
        unlit,
        lit_clamp,
        primary_clamp,
        emission,
        point,
        window,
    ]
}
