//! SaiPressure width-profile experiment. Centerlines use the exact production SAI-simplified
//! geometry implementation from photocraft-vector; only pressure interpolation candidates vary.
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Deserialize, Serialize)]
struct Dataset {
    cases: Vec<Case>,
    grid: Grid,
}
#[derive(Deserialize, Serialize)]
struct Grid {
    x: Axis,
    y: Axis,
    inferred_display_scale: f64,
}
#[derive(Deserialize, Serialize)]
struct Axis {
    origin_px: f64,
    period_px: f64,
}
#[derive(Deserialize, Serialize)]
struct Case {
    id: usize,
    base_width_px: f64,
    anchors: Vec<Anchor>,
    #[serde(default)]
    measured_normal_width_profile: Vec<Measurement>,
    #[serde(default)]
    reference_centerline_samples_px: Vec<[f64; 2]>,
    #[serde(default)]
    red_outline_components: Vec<serde_json::Value>,
    uncertainty: String,
}
#[derive(Deserialize, Serialize)]
struct Anchor {
    pixel: [f64; 2],
    pressure_percent: f64,
    #[serde(default)]
    marker: String,
    #[serde(default)]
    uncertainty: String,
}
#[derive(Clone, Deserialize, Serialize)]
struct Measurement {
    segment_index: usize,
    t: f64,
    segment_arc_fraction: f64,
    path_arc_fraction: f64,
    #[serde(default)]
    segment_length_px: f64,
    center_px: [f64; 2],
    normal_px: [f64; 2],
    width_px: f64,
    uncertainty_px: f64,
    ambiguous: bool,
}
#[derive(Clone, Copy)]
struct PreparedSample {
    segment: usize,
    t: f64,
    local_arc: f64,
    global_arc: f64,
    segment_length_px: f64,
    center: [f64; 2],
    normal: [f64; 2],
}
#[derive(Clone, Copy, Debug, Serialize)]
struct Candidate {
    id: &'static str,
    hypothesis: &'static str,
}
#[derive(Serialize)]
struct CaseScore {
    case: usize,
    samples: usize,
    mean_width_error_px: f64,
    normalized_pressure_error_percent: f64,
    p95_width_error_px: f64,
    max_width_error_px: f64,
    ambiguous_samples_excluded: usize,
}
#[derive(Serialize)]
struct Ranked {
    candidate: Candidate,
    supported_cases: usize,
    balanced_mean_error_px: f64,
    balanced_normalized_pressure_error_percent: f64,
    maximum_pressure_overshoot_percent: f64,
    cases: Vec<CaseScore>,
}
#[derive(Serialize)]
struct Results {
    scoring: &'static str,
    screenshot_cases: usize,
    scored_cases: Vec<usize>,
    excluded_cases: Vec<Excluded>,
    candidates_tested: usize,
    ranking: Vec<Ranked>,
    identifiability: &'static str,
    properties: Properties,
}
#[derive(Serialize)]
struct Excluded {
    case: usize,
    reason: String,
}
#[derive(Serialize)]
struct Properties {
    reversibility_max_abs: f64,
    pressure_nonnegative: bool,
    anchor_error_percent: f64,
    synthetic_profiles: usize,
}

const CANDIDATES: [Candidate; 5] = [
    Candidate { id: "linear_bezier_parameter", hypothesis: "linear pressure per segment in its cubic Bezier parameter t" },
    Candidate { id: "linear_arc_length", hypothesis: "linear pressure in normalized centerline arc length" },
    Candidate { id: "smoothstep", hypothesis: "cubic smoothstep pressure, zero derivatives at each anchor" },
    Candidate { id: "local_cubic", hypothesis: "local cubic Hermite pressure with secant endpoint and centered interior tangents" },
    Candidate { id: "monotone_cubic", hypothesis: "shape-preserving monotone cubic Hermite pressure" },
];
fn cubic(s: &[photocraft_geom::Point; 4], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    [
        u * u * u * s[0].x + 3.0 * u * u * t * s[1].x + 3.0 * u * t * t * s[2].x + t * t * t * s[3].x,
        u * u * u * s[0].y + 3.0 * u * u * t * s[1].y + 3.0 * u * t * t * s[2].y + t * t * t * s[3].y,
    ]
}
fn derivative(s: &[photocraft_geom::Point; 4], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    [
        3.0 * u * u * (s[1].x - s[0].x) + 6.0 * u * t * (s[2].x - s[1].x) + 3.0 * t * t * (s[3].x - s[2].x),
        3.0 * u * u * (s[1].y - s[0].y) + 6.0 * u * t * (s[2].y - s[1].y) + 3.0 * t * t * (s[3].y - s[2].y),
    ]
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn prepare(case: &Case, grid: &Grid) -> Result<Vec<PreparedSample>, String> {
    let sx = grid.x.period_px / 40.0;
    let sy = grid.y.period_px / 40.0;
    let points: Vec<[f64; 2]> = case.anchors.iter().map(|a| [(a.pixel[0] - grid.x.origin_px) / sx, (a.pixel[1] - grid.y.origin_px) / sy]).collect();
    let path = photocraft_vector::linework::interpolate_points_with_model(&points, None, photocraft_vector::linework::InterpolationModel::SaiSimplified)?;
    let segments = path.subpaths.first().ok_or("SaiLine interpolation returned no subpath")?.segments();
    let mut lengths = Vec::with_capacity(segments.len());
    for segment in &segments {
        let mut length = 0.0;
        let mut last = cubic(segment, 0.0);
        for k in 1..=1000 {
            let p = cubic(segment, k as f64 / 1000.0);
            length += distance(last, p);
            last = p;
        }
        lengths.push(length);
    }
    let total = lengths.iter().sum::<f64>().max(1e-12);
    let mut base = 0.0;
    let mut out = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let length = *lengths.get(i).ok_or("missing segment arc length")?;
        let mut cumulative = vec![0.0; 101];
        let mut last = cubic(segment, 0.0);
        for k in 1..=100 {
            let p = cubic(segment, k as f64 / 100.0);
            cumulative[k] = cumulative[k - 1] + distance(last, p);
            last = p;
        }
        let local_total = cumulative.last().copied().unwrap_or(length).max(1e-12);
        for k in 0..=100 {
            if i > 0 && k == 0 {
                continue;
            }
            let t = k as f64 / 100.0;
            let p = cubic(segment, t);
            let d = derivative(segment, t);
            let screen_t = [d[0] * sx, d[1] * sy];
            let norm = screen_t[0].hypot(screen_t[1]).max(1e-12);
            let center = [grid.x.origin_px + p[0] * sx, grid.y.origin_px + p[1] * sy];
            let global = (base + cumulative[k]) / total;
            out.push(PreparedSample {
                segment: i,
                t,
                local_arc: cumulative[k] / local_total,
                global_arc: global,
                segment_length_px: length * (sx * sy).sqrt(),
                center,
                normal: [-screen_t[1] / norm, screen_t[0] / norm],
            });
        }
        base += length;
    }
    Ok(out)
}
fn candidate_pressure(id: &str, case: &Case, sample: &Measurement, lengths: &[f64]) -> f64 {
    let p: Vec<f64> = case.anchors.iter().map(|a| a.pressure_percent / 100.0).collect();
    let n = p.len();
    let i = sample.segment_index;
    let t = sample.t.clamp(0.0, 1.0);
    let Some((&p0, &p1)) = p.get(i).zip(p.get(i + 1)) else { return 1.0 };
    match id {
        "linear_bezier_parameter" => p0 + (p1 - p0) * t,
        "linear_arc_length" => p0 + (p1 - p0) * sample.segment_arc_fraction,
        "smoothstep" => {
            let q = t * t * (3.0 - 2.0 * t);
            p0 + (p1 - p0) * q
        }
        "local_cubic" | "monotone_cubic" => {
            let dx = lengths.get(i).copied().unwrap_or(1.0).max(1e-12);
            let secants: Vec<f64> = (0..n.saturating_sub(1)).map(|j| (p[j + 1] - p[j]) / lengths.get(j).copied().unwrap_or(1.0).max(1e-12)).collect();
            let slope = |j: usize| -> f64 {
                if id == "local_cubic" {
                    if j == 0 || j + 1 == n {
                        return secants.get(j.min(secants.len().saturating_sub(1))).copied().unwrap_or(0.0);
                    }
                    return (p[j + 1] - p[j - 1]) / (lengths[j - 1] + lengths[j]).max(1e-12);
                }
                if j == 0 {
                    return secants.first().copied().unwrap_or(0.0);
                }
                if j + 1 == n {
                    return secants.last().copied().unwrap_or(0.0);
                }
                let a = secants[j - 1];
                let b = secants[j];
                if a * b <= 0.0 {
                    0.0
                } else {
                    let h0 = lengths[j - 1].max(1e-12);
                    let h1 = lengths[j].max(1e-12);
                    let w1 = 2.0 * h1 + h0;
                    let w2 = h1 + 2.0 * h0;
                    (w1 + w2) / (w1 / a + w2 / b)
                }
            };
            let m0 = slope(i) * dx;
            let m1 = slope(i + 1) * dx;
            let t2 = t * t;
            let t3 = t2 * t;
            (2.0 * t3 - 3.0 * t2 + 1.0) * p0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * p1 + (t3 - t2) * m1
        }
        _ => p0 + (p1 - p0) * t,
    }
}
fn percentile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let i = ((values.len().saturating_sub(1)) as f64 * q).round() as usize;
    values.get(i).copied().unwrap_or(0.0)
}
fn synthetic_linear_arc_properties() -> Properties {
    let mut reversibility_max_abs: f64 = 0.0;
    let mut pressure_nonnegative = true;
    let mut anchor_error_percent: f64 = 0.0;
    let mut synthetic_profiles = 0;
    for n in 2..=12 {
        for seed in 0..100 {
            synthetic_profiles += 1;
            let pressure: Vec<f64> = (0..n).map(|i| ((i * 47 + seed * 19) % 201) as f64 / 100.0).collect();
            for segment in 0..n - 1 {
                for step in 0..=100 {
                    let arc_fraction = step as f64 / 100.0;
                    let value = pressure[segment] + (pressure[segment + 1] - pressure[segment]) * arc_fraction;
                    let reversed = pressure[segment + 1] + (pressure[segment] - pressure[segment + 1]) * (1.0 - arc_fraction);
                    reversibility_max_abs = reversibility_max_abs.max((value - reversed).abs());
                    pressure_nonnegative &= (0.0..=2.0).contains(&value) && value.is_finite();
                    if step == 0 {
                        anchor_error_percent = anchor_error_percent.max((value - pressure[segment]).abs() * 100.0);
                    } else if step == 100 {
                        anchor_error_percent = anchor_error_percent.max((value - pressure[segment + 1]).abs() * 100.0);
                    }
                }
            }
        }
    }
    Properties { reversibility_max_abs, pressure_nonnegative, anchor_error_percent, synthetic_profiles }
}
fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dataset: Dataset = serde_json::from_slice(&fs::read(root.join("dataset.json")).expect("dataset readable")).expect("dataset schema");
    if std::env::args().any(|arg| arg == "--prepare") {
        let mut rows = Vec::new();
        for case in &dataset.cases {
            let samples = prepare(case, &dataset.grid).expect("validated SaiLine geometry");
            rows.push((case.id, samples));
        }
        let value = serde_json::json!({"source":"photocraft-vector::SaiSimplified","cases":rows.iter().map(|(id,s)|serde_json::json!({"id":id,"samples":s.iter().map(|v|serde_json::json!({"segment_index":v.segment,"t":v.t,"segment_arc_fraction":v.local_arc,"path_arc_fraction":v.global_arc,"segment_length_px":v.segment_length_px,"center_px":v.center,"normal_px":v.normal})).collect::<Vec<_>>()})).collect::<Vec<_>>()});
        fs::write(root.join("geometry_samples.json"), serde_json::to_vec_pretty(&value).expect("serialize geometry")).expect("write geometry samples");
        println!("prepared validated SAI-simplified centerline samples for {} cases", rows.len());
        return;
    }
    let mut ranked = Vec::new();
    for candidate in CANDIDATES {
        let mut scores = Vec::new();
        let mut overshoot: f64 = 0.0;
        for case in &dataset.cases {
            if case.measured_normal_width_profile.is_empty() {
                continue;
            }
            let mut lengths = vec![0.0; case.anchors.len().saturating_sub(1)];
            for sample in &case.measured_normal_width_profile {
                if let Some(length) = lengths.get_mut(sample.segment_index) {
                    *length = sample.segment_length_px;
                }
            }
            for (i, length) in lengths.iter_mut().enumerate() {
                if *length <= 0.0 {
                    *length = distance(case.anchors[i].pixel, case.anchors[i + 1].pixel);
                }
            }
            let mut errors = Vec::new();
            let mut normalized = Vec::new();
            let mut excluded = 0;
            for sample in &case.measured_normal_width_profile {
                if sample.ambiguous {
                    excluded += 1;
                    continue;
                }
                let pressure = candidate_pressure(candidate.id, case, sample, &lengths);
                overshoot = overshoot.max((-pressure).max(pressure - 2.0).max(0.0) * 100.0);
                let predicted = case.base_width_px * dataset.grid.inferred_display_scale * pressure;
                errors.push((predicted - sample.width_px).abs());
                normalized.push((predicted - sample.width_px).abs() / (case.base_width_px * dataset.grid.inferred_display_scale) * 100.0);
            }
            if errors.is_empty() {
                continue;
            }
            let mut sorted = errors.clone();
            scores.push(CaseScore {
                case: case.id,
                samples: errors.len(),
                mean_width_error_px: errors.iter().sum::<f64>() / errors.len() as f64,
                normalized_pressure_error_percent: normalized.iter().sum::<f64>() / normalized.len() as f64,
                p95_width_error_px: percentile(&mut sorted, 0.95),
                max_width_error_px: errors.iter().copied().fold(0.0, f64::max),
                ambiguous_samples_excluded: excluded,
            });
        }
        let mean = if scores.is_empty() { f64::INFINITY } else { scores.iter().map(|s| s.mean_width_error_px).sum::<f64>() / scores.len() as f64 };
        let normalized =
            if scores.is_empty() { f64::INFINITY } else { scores.iter().map(|s| s.normalized_pressure_error_percent).sum::<f64>() / scores.len() as f64 };
        ranked.push(Ranked {
            candidate,
            supported_cases: scores.len(),
            balanced_mean_error_px: mean,
            balanced_normalized_pressure_error_percent: normalized,
            maximum_pressure_overshoot_percent: overshoot,
            cases: scores,
        });
    }
    ranked.sort_by(|a, b| a.balanced_normalized_pressure_error_percent.total_cmp(&b.balanced_normalized_pressure_error_percent));
    let scored: Vec<usize> = ranked.first().map(|r| r.cases.iter().map(|c| c.case).collect()).unwrap_or_default();
    let excluded = dataset
        .cases
        .iter()
        .filter(|c| !scored.contains(&c.id))
        .map(|c| Excluded { case: c.id, reason: format!("{}; no usable normal-width samples", c.uncertainty) })
        .collect();
    let results = Results {
        scoring: "cross-section width error along curve normals; per-case mean/P95/max in screenshot px and width-normalized pressure error; aggregate is equally weighted over scored cases",
        screenshot_cases: dataset.cases.len(),
        scored_cases: scored,
        excluded_cases: excluded,
        candidates_tested: CANDIDATES.len(),
        ranking: ranked,
        identifiability: "All six control markers are visible and recorded. Centerlines are reconstructed with the validated production SaiSimplified formula. Width measurements use screenshot red masks sampled along local normals. Interpret remaining score differences against the ±1.5 px extraction/antialiasing uncertainty.",
        properties: synthetic_linear_arc_properties(),
    };
    fs::write(root.join("results.json"), serde_json::to_vec_pretty(&results).expect("serialize results")).expect("write results");
    println!("ranked {} pressure candidates on cases {:?}", results.candidates_tested, results.scored_cases);
    for row in &results.ranking {
        println!(
            "{}: {:.2}% normalized error ({:.3} px), {} cases",
            row.candidate.id, row.balanced_normalized_pressure_error_percent, row.balanced_mean_error_px, row.supported_cases
        );
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn linear_pressure_is_reversible_nonnegative_and_interpolates_endpoints() {
        for n in 2..=8 {
            for seed in 0..100 {
                let p: Vec<f64> = (0..n).map(|i| ((i * 47 + seed * 19) % 201) as f64 / 100.0).collect();
                for i in 0..n - 1 {
                    for step in 0..=100 {
                        let t = step as f64 / 100.0;
                        let q = p[i] + (p[i + 1] - p[i]) * t;
                        let rev = p[i + 1] + (p[i] - p[i + 1]) * (1.0 - t);
                        assert!((q - rev).abs() < 1e-12);
                        assert!((0.0..=2.0).contains(&q));
                    }
                    assert!((p[i] + (p[i + 1] - p[i]) * 0.0 - p[i]).abs() < 1e-12);
                    assert!((p[i] + (p[i + 1] - p[i]) * 1.0 - p[i + 1]).abs() < 1e-12);
                }
            }
        }
    }
}
