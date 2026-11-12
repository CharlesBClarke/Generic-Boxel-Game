//! `look_noise.wgsl` runs on the GPU and gives what `api::look` gives in Rust: the same hash
//! bits, the same noise values (to float rounding), the same colors picked and the same grain.
//! Skips when there's no GPU to run it on.

use api::look::{Grain, hash, pick, scale_units, value, world_texel};
use wgpu::util::DeviceExt;

/// One point to check: a world texel, a scale in sixteenths, a seed and a grain direction.
#[derive(Clone, Copy)]
struct Case {
    p: [u32; 3],
    scale: [u32; 3],
    seed: u32,
    direction: u32,
    board: u32,
}

/// How far the GPU's floats may differ: it may fuse multiplies and adds.
const CLOSE: f32 = 1e-5;

const ENTRY: &str = r#"
struct Case { p: vec4<u32>, scale: vec4<u32>, extra: vec4<u32> };
struct Out { value: f32, pick: u32, grain: f32, hash: u32 };
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> outs: array<Out>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&cases) { return; }
    let c = cases[i];
    let v = value(c.scale.xyz, c.p.w, c.p.xyz);
    let g = grain_factor(c.scale.xyz, c.p.w, 0.1, c.extra.x, c.extra.y, c.p.xyz, 0.0);
    outs[i] = Out(v, pick(v, 4u), g, hash(c.p.xyz, c.p.w));
}
"#;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// What the GPU makes of each case: value, pick, grain, hash.
fn run(device: &wgpu::Device, queue: &wgpu::Queue, cases: &[Case]) -> Vec<(f32, u32, f32, u32)> {
    let library: String =
        include_str!("../src/look_noise.wgsl").lines().filter(|l| !l.starts_with("#define_import_path")).collect::<Vec<_>>().join("\n");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look_noise"),
        source: wgpu::ShaderSource::Wgsl(format!("{library}\n{ENTRY}").into()),
    });
    let words: Vec<u32> = cases
        .iter()
        .flat_map(|c| [c.p[0], c.p[1], c.p[2], c.seed, c.scale[0], c.scale[1], c.scale[2], 0, c.direction, c.board, 0, 0])
        .collect();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>(),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (cases.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(cases.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = read.slice(..).get_mapped_range().to_vec();
    let word = |i: usize| u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap());
    (0..cases.len()).map(|i| (f32::from_bits(word(4 * i)), word(4 * i + 1), f32::from_bits(word(4 * i + 2)), word(4 * i + 3))).collect()
}

#[test]
fn the_shader_noise_matches_the_rust_noise() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU: skipped");
        return;
    };
    let mut cases = Vec::new();
    // Near the middle, on both sides of 0, across block edges, and far out.
    let places: [i64; 9] = [0, -1, 15, 16, -17, 1000, -54_321, 100_000_000 * 16, -100_000_000 * 16];
    let scales = [[1.0, 1.0, 1.0], [1.5, 1.5, 1.5], [2.0, 3.0, 4.0], [12.0, 1.0, 1.0], [5.0, 5.0, 5.0]];
    for (i, &x) in places.iter().enumerate() {
        for (j, scale) in scales.iter().enumerate() {
            for k in 0..8i64 {
                let p = world_texel([x + k * 7, x / 3 - k, k * 11 - 40]);
                cases.push(Case {
                    p,
                    scale: scale.map(scale_units),
                    seed: (i * 31 + j * 7) as u32 + k as u32,
                    direction: 1 + (k as u32 % 3),
                    board: (i + j) as u32 % 16,
                });
            }
        }
    }
    let colors = [[10, 0, 0], [20, 0, 0], [30, 0, 0], [40, 0, 0]];
    let got = run(&device, &queue, &cases);
    for (c, (v, p, g, h)) in cases.iter().zip(got) {
        assert_eq!(h, hash(c.p, c.seed), "hash at {:?}", c.p);
        let want = value(c.scale, c.seed, c.p);
        assert!((v - want).abs() < CLOSE, "value at {:?} scale {:?}: {v} vs {want}", c.p, c.scale);
        // The picked color can only differ where the value sits right on a boundary.
        if colors[p as usize] != pick(&colors, want) {
            let edge = ((want - 0.5) * 1.8 + 0.5) * 4.0;
            assert!((edge - edge.round()).abs() < 1e-4, "pick at {:?}", c.p);
        }
        // Grain takes the scale as along, across, through.
        let u = c.scale;
        let s = match c.direction {
            1 => u,
            2 => [u[1], u[0], u[2]],
            _ => [u[1], u[2], u[0]],
        };
        let want_grain = 1.0 + 0.1 * (2.0 * value(s, c.seed.wrapping_add(c.board * 7919), c.p) - 1.0);
        assert!((g - want_grain).abs() < CLOSE, "grain at {:?}: {g} vs {want_grain}", c.p);
    }
}

#[test]
fn grain_in_rust_reads_its_scale_as_the_shader_does() {
    // The GPU test rebuilds grain from scale units; check that matches `Grain::factor`.
    let g = Grain { scale: [12.0, 1.5, 2.0], strength: 0.1, seed: 9 };
    let p = world_texel([5, -3, 70]);
    for direction in 1..=3u8 {
        let u = g.scale.map(scale_units);
        let s = match direction {
            1 => u,
            2 => [u[1], u[0], u[2]],
            _ => [u[1], u[2], u[0]],
        };
        let want = 1.0 + 0.1 * (2.0 * value(s, 9u32.wrapping_add(4 * 7919), p) - 1.0);
        assert_eq!(g.factor(direction, 4, p), want);
    }
}
