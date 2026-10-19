//! Reproducible, headless SaiLineArt microbenchmarks.
//!
//! Run `cargo run --release -p photocraft-engine --example sailine_bench -- --profile quick --output target/sailine-bench.json`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use photocraft_color::Color;
use photocraft_doc::{LayerContent, LineworkPath, LayerId};
use photocraft_engine::Session;
use photocraft_vector::linework::{InterpolationModel, interpolate_points_with_model_and_pointed, variable_width_shape_with_model_and_pointed};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct Case { paths: usize, points: usize, overlap: bool, width: u32, height: u32 }

fn cases(profile: &str) -> Vec<Case> {
    if profile == "quick" {
        return vec![
            Case { paths: 10, points: 10, overlap: false, width: 512, height: 512 },
            Case { paths: 10, points: 100, overlap: true, width: 1024, height: 1024 },
        ];
    }
    vec![
        Case { paths: 10, points: 10, overlap: false, width: 512, height: 512 },
        Case { paths: 100, points: 10, overlap: false, width: 1024, height: 1024 },
        Case { paths: 1000, points: 10, overlap: false, width: 2048, height: 8192 },
        Case { paths: 10, points: 100, overlap: true, width: 1024, height: 1024 },
        Case { paths: 10, points: 1000, overlap: true, width: 2048, height: 2048 },
        Case { paths: 100, points: 100, overlap: false, width: 2048, height: 2048 },
    ]
}

fn path_points(case: Case, path: usize) -> Vec<[f64; 2]> {
    (0..case.points).map(|i| {
        let x = if case.points <= 1 { 0.0 } else { i as f64 * (case.width as f64 - 40.0) / (case.points - 1) as f64 };
        let lane = if case.overlap { 0.0 } else { path as f64 * 8.0 };
        [x + 20.0, (if case.overlap { case.height as f64 * 0.5 } else { 32.0 }) + lane + ((i % 2) as f64 * 0.5)]
    }).collect()
}

fn prepare(case: Case) -> (Session, u64, Vec<(u64, u64, [f64; 2])>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":case.width,"height":case.height})).unwrap();
    let pts = path_points(case, 0);
    let created = s.execute("linework.create", json!({"points":pts,"width":6.0})).unwrap();
    let layer = created["layer"].as_u64().unwrap();
    s.edit("bench setup", |doc, _| {
        let LayerContent::Linework(linework) = &mut doc.layer_mut(LayerId(layer)).unwrap().content else { unreachable!() };
        for index in 1..case.paths {
            let pts = path_points(case, index);
            let path = LineworkPath::new(pts, Color::BLACK, 6.0).unwrap();
            linework.paths.push(path);
        }
        Ok(())
    }).unwrap();
    s.execute("linework.setInterpolation", json!({"interpolation":"saiSimplified"})).unwrap();
    let targets = {
        let doc = &s.active().unwrap().doc;
        let LayerContent::Linework(linework) = &doc.layer(LayerId(layer)).unwrap().content else { unreachable!() };
        linework.paths.iter().flat_map(|p| p.points.iter().map(move |point| (p.id, point.id, point.position))).collect::<Vec<_>>()
    };
    (s, layer, targets)
}

fn stats(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec(); sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        let rank = (sorted.len().saturating_sub(1) as f64) * p;
        let lo = rank.floor() as usize;
        let hi = (lo + 1).min(sorted.len() - 1);
        sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
    };
    json!({"iterations": samples.len(), "median_ms": percentile(0.5), "p95_ms": percentile(0.95), "max_ms": sorted[sorted.len()-1]})
}

fn peak_rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status").ok()?.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")?.split_whitespace().next()?.parse().ok()
    })
}

fn measure(mut operation: impl FnMut(usize), warmups: usize, reps: usize) -> Value {
    for i in 0..warmups { operation(i); }
    let mut samples = Vec::with_capacity(reps);
    for i in 0..reps {
        let start = Instant::now(); operation(i + warmups); samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    stats(&samples)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let profile = args.windows(2).find(|a| a[0] == "--profile").map(|a| a[1].as_str()).unwrap_or("quick");
    assert!(profile == "quick" || profile == "full", "--profile must be quick or full");
    let output = args.windows(2).find(|a| a[0] == "--output").map(|a| a[1].as_str());
    let selected_case = args.windows(2).find(|a| a[0] == "--case").map(|a| a[1].parse::<usize>().expect("--case must be a zero-based index"));
    let (warmups, default_reps) = if profile == "quick" { (3, 20) } else { (2, 10) };
    let reps = args.windows(2).find(|a| a[0] == "--reps").and_then(|a| a[1].parse().ok()).unwrap_or(default_reps);
    assert!(reps >= 10, "use at least 10 measured repetitions for a useful p95");
    let mut peak_rss = peak_rss_kib();
    let mut rows = Vec::new();
    let all_cases = cases(profile);
    let selected_cases = if let Some(index) = selected_case { vec![*all_cases.get(index).expect("--case index is outside the selected profile")] } else { all_cases };
    for case in selected_cases {
        eprintln!("benchmark {} paths × {} points, overlap={}, canvas={}x{}", case.paths, case.points, case.overlap, case.width, case.height);
        // Pure geometry and contour/raster stages on one representative path.
        let points = path_points(case, 0); let factors = vec![1.0; points.len()]; let pointed = vec![false; points.len()];
        let geometry = measure(|_| { std::hint::black_box(interpolate_points_with_model_and_pointed(&points, None, InterpolationModel::SaiSimplified, &pointed).unwrap()); }, warmups, reps);
        let contour = measure(|_| { std::hint::black_box(variable_width_shape_with_model_and_pointed(&points, &factors, 6.0, Color::BLACK, None, InterpolationModel::SaiSimplified, &pointed).unwrap()); }, warmups, reps);
        let shape = variable_width_shape_with_model_and_pointed(&points, &factors, 6.0, Color::BLACK, None, InterpolationModel::SaiSimplified, &pointed).unwrap();
        let min_x = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
        let max_x = points.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let min_y = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        let max_y = points.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
        let clip = photocraft_geom::Rect::new((min_x - 8.0).floor() as i32, (min_y - 8.0).floor() as i32, (max_x + 8.0).ceil() as i32, (max_y + 8.0).ceil() as i32);
        let raster = measure(|_| { std::hint::black_box(photocraft_vector::render_shape(&shape, photocraft_color::PixelFormat::RGBA32F, clip)); }, warmups, reps);

        let (mut session, layer, targets) = prepare(case);
        let doc = session.active().unwrap().doc.clone();
        let edit_clone_only = measure(|_| { std::hint::black_box((*doc).clone()); }, warmups, reps);
        peak_rss = peak_rss.max(peak_rss_kib());
        let target = targets[targets.len()/2];
        let (damage, cached_paths) = {
            let doc = &session.active().unwrap().doc;
            let LayerContent::Linework(linework) = &doc.layer(LayerId(layer)).unwrap().content else { unreachable!() };
            let damage = linework.path_caches.get(&target.0).map(|cache| cache.bounds).unwrap_or(photocraft_geom::Rect::EMPTY);
            let caches = linework.paths.iter().filter_map(|path| linework.path_caches.get(&path.id).cloned()).collect::<Vec<_>>();
            (doc.bounds().intersect(&damage), caches)
        };
        let recomposition = measure(|_| {
            let mut composite = None;
            for path_cache in &cached_paths {
                composite = Some(photocraft_engine::linework_cmds::bench_composite_cache(damage, composite, path_cache));
            }
            std::hint::black_box(composite);
        }, warmups, reps);
        let move_one = measure(|i| {
            let offset = if i % 2 == 0 { 0.25 } else { -0.25 };
            session.execute("linework.movePoint", json!({"layer":layer,"path":target.0,"point":target.1,"x":target.2[0]+offset,"y":target.2[1],"interpolation":"saiSimplified","coalesce":"sailine-bench-move"})).unwrap();
        }, warmups, reps);
        let pressure = measure(|i| {
            let factor = if i % 2 == 0 { 1.05 } else { 1.0 };
            session.execute("linework.setPointWidth", json!({"layer":layer,"path":target.0,"point":target.1,"factor":factor,"interpolation":"saiSimplified","coalesce":"sailine-bench-pressure"})).unwrap();
        }, warmups, reps);
        let every_n = (targets.len()/20).max(1);
        let selected: Vec<_> = targets.iter().step_by(every_n).take(200).map(|(path, point, pos)| json!({"layer":layer,"path":path,"point":point,"x":pos[0]+0.125,"y":pos[1]+0.125})).collect();
        let move_many = measure(|_| { session.execute("linework.movePoints", json!({"points":selected,"interpolation":"saiSimplified","coalesce":"sailine-bench-multi"})).unwrap(); }, warmups, reps);
        let incremental = move_one.clone();
        let full_rebuild = measure(|_| { session.execute("linework.setInterpolation", json!({"interpolation":"saiSimplified"})).unwrap(); }, warmups, reps);
        peak_rss = peak_rss.max(peak_rss_kib());
        rows.push(json!({"paths":case.paths,"points_per_path":case.points,"overlap":case.overlap,"canvas":[case.width,case.height],"total_points":case.paths*case.points,
            "geometry_ms":geometry,"contour_build_ms":contour,"single_path_raster_ms":raster,"edit_snapshot_clone_only_ms":edit_clone_only,"incremental_region_recomposition_ms":recomposition,"move_one_command_incremental_ms":incremental,"pressure_command_incremental_ms":pressure,"move_selected_command_ms":move_many,"full_layer_rebuild_command_ms":full_rebuild}));
    }
    let report = json!({"schema":"sailine-bench-v1","profile":profile,"case_index":selected_case,"warmups":warmups,"repetitions":reps,"optimization":"release","cache_bounds":std::env::var("SAILINE_CACHE_BOUNDS").unwrap_or_else(|_| "content".into()),"git_commit":std::env::var("SAILINE_GIT_COMMIT").unwrap_or_else(|_| "unknown".into()),"worktree_dirty":std::env::var("SAILINE_WORKTREE_DIRTY").unwrap_or_else(|_| "unknown".into()),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"cpu_model":std::env::var("SAILINE_CPU_MODEL").unwrap_or_else(|_| "unknown".into()),"peak_rss_kib":peak_rss,"rows":rows});
    let serialized = serde_json::to_string_pretty(&report).unwrap();
    if let Some(path) = output { std::fs::write(path, &serialized).unwrap(); } else { println!("{serialized}"); }
}
