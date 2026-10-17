//! Offline SAI 1.2.6 curve-model comparison. No PhotoCraft production code is changed.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

type Pt = [f64; 2];
type Cubic = [Pt; 4];

#[derive(Deserialize)]
struct Dataset {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: usize,
    anchors: Vec<Anchor>,
    reference_segments_px: Vec<Vec<Pt>>,
    uncertainty: Vec<String>,
}

#[derive(Deserialize)]
struct Anchor {
    pixel: Pt,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Endpoint {
    Secant,
    Quadratic,
    Natural,
    Zero,
    ScaledSecant(f64),
}

impl Endpoint {
    const ALL: [Self; 4] = [Self::Secant, Self::Quadratic, Self::Natural, Self::Zero];
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Candidate {
    alpha: f64,
    tension: f64,
    balance_power: f64,
    endpoint: Endpoint,
}

#[derive(Clone, Serialize)]
struct CaseScore {
    id: usize,
    mean_px: f64,
    p95_px: f64,
    max_px: f64,
    weighted_score_px: f64,
}

#[derive(Serialize)]
struct Ranked {
    candidate: Candidate,
    score_px: f64,
    cases: Vec<CaseScore>,
}

#[derive(Serialize)]
struct Fold {
    held_out_ids: Vec<usize>,
    selected: Candidate,
    train_score_px: f64,
    validation_score_px: f64,
}

#[derive(Serialize)]
struct Properties {
    configurations: usize,
    max_reverse_distance_px: f64,
    max_distant_segment_change_px: f64,
    max_anchor_error_px: f64,
    max_tangent_angle_deg: f64,
    max_change_for_001px_move_px: f64,
    reverse_failures_over_001px: usize,
    locality_failures_over_000001px: usize,
}

#[derive(Serialize)]
struct Results {
    scoring: String,
    candidates_tested: usize,
    top: Vec<Ranked>,
    parsimonious: Ranked,
    simple_local: Ranked,
    canonical_family_best: Vec<Ranked>,
    endpoint_family_best: Vec<Ranked>,
    current: Vec<CaseScore>,
    current_score_px: f64,
    current_best_orientation: Vec<CaseScore>,
    current_best_orientation_score_px: f64,
    folds: Vec<Fold>,
    leave_one_out: Vec<Fold>,
    properties_best: Properties,
    properties_current: Properties,
    marker_jitter: JitterSummary,
    extreme_best: ExtremeSummary,
    extreme_current: ExtremeSummary,
    geometry_diagnostics: Vec<GeometryDiagnostics>,
}

#[derive(Serialize)]
struct GeometryDiagnostics {
    id: usize,
    sai_max_longitudinal_overshoot_px: f64,
    best_max_longitudinal_overshoot_px: f64,
    current_max_longitudinal_overshoot_px: f64,
    sai_backward_travel_px: f64,
    best_backward_travel_px: f64,
    current_backward_travel_px: f64,
    best_start_direction_error_deg: f64,
    best_end_direction_error_deg: f64,
    current_start_direction_error_deg: f64,
    current_end_direction_error_deg: f64,
}

#[derive(Serialize)]
struct ExtremeSummary {
    configurations: usize,
    self_intersecting_segments: usize,
    max_change_for_001px_move_non_degenerate_px: f64,
    max_change_for_001px_move_near_degenerate_px: f64,
}

#[derive(Serialize)]
struct JitterSummary {
    trials: usize,
    perturbation_px: f64,
    best_mean_score_px: f64,
    current_mean_score_px: f64,
    best_wins: usize,
}

fn add(a: Pt, b: Pt) -> Pt {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Pt, b: Pt) -> Pt {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: Pt, s: f64) -> Pt {
    [a[0] * s, a[1] * s]
}
fn norm(a: Pt) -> f64 {
    a[0].hypot(a[1])
}
fn dist(a: Pt, b: Pt) -> f64 {
    norm(sub(a, b))
}
fn bezier(b: Cubic, t: f64) -> Pt {
    let u = 1.0 - t;
    add(add(mul(b[0], u * u * u), mul(b[1], 3.0 * u * u * t)), add(mul(b[2], 3.0 * u * t * t), mul(b[3], t * t * t)))
}

fn catmull(points: &[Pt], candidate: Candidate) -> Vec<Cubic> {
    if points.len() < 2 {
        return Vec::new();
    }
    let n = points.len();
    let h: Vec<f64> = points.windows(2).map(|w| if candidate.alpha == 0.0 { 1.0 } else { dist(w[0], w[1]).max(1e-9).powf(candidate.alpha) }).collect();
    let mut d = vec![[0.0, 0.0]; n];
    for i in 1..n - 1 {
        let a = h[i - 1];
        let b = h[i];
        let sa = mul(sub(points[i], points[i - 1]), 1.0 / a);
        let sb = mul(sub(points[i + 1], points[i]), 1.0 / b);
        d[i] = mul(add(mul(sa, b), mul(sb, a)), 1.0 / (a + b));
        if candidate.balance_power != 0.0 {
            let previous = dist(points[i], points[i - 1]);
            let next = dist(points[i + 1], points[i]);
            let ratio = previous.min(next) / previous.max(next).max(1e-12);
            d[i] = mul(d[i], ratio.max(1e-12).powf(candidate.balance_power));
        }
    }
    let first = mul(sub(points[1], points[0]), 1.0 / h[0]);
    let last = mul(sub(points[n - 1], points[n - 2]), 1.0 / h[n - 2]);
    if n == 2 {
        d[0] = first;
        d[1] = last;
    } else {
        let second = mul(sub(points[2], points[1]), 1.0 / h[1]);
        let penultimate = mul(sub(points[n - 2], points[n - 3]), 1.0 / h[n - 3]);
        match candidate.endpoint {
            Endpoint::Secant => {
                d[0] = first;
                d[n - 1] = last;
            }
            Endpoint::Quadratic => {
                d[0] = sub(mul(first, (2.0 * h[0] + h[1]) / (h[0] + h[1])), mul(second, h[0] / (h[0] + h[1])));
                d[n - 1] = sub(mul(last, (2.0 * h[n - 2] + h[n - 3]) / (h[n - 2] + h[n - 3])), mul(penultimate, h[n - 2] / (h[n - 2] + h[n - 3])));
            }
            Endpoint::Natural => {
                d[0] = mul(sub(mul(first, 3.0), d[1]), 0.5);
                d[n - 1] = mul(sub(mul(last, 3.0), d[n - 2]), 0.5);
            }
            Endpoint::Zero => {}
            Endpoint::ScaledSecant(gain) => {
                d[0] = mul(first, gain);
                d[n - 1] = mul(last, gain);
            }
        }
    }
    let scale = 1.0 - candidate.tension;
    d.iter_mut().for_each(|v| *v = mul(*v, scale));
    // Duplicate anchors are an intentional hard corner in SaiLine; make the zero-length
    // segment exactly a point and retract both adjoining handles. Nonzero neighbours use
    // the ordinary Catmull-Rom limit.
    for i in 0..n - 1 {
        if dist(points[i], points[i + 1]) <= f64::EPSILON {
            d[i] = [0.0, 0.0];
            d[i + 1] = [0.0, 0.0];
        }
    }
    (0..n - 1).map(|i| [points[i], add(points[i], mul(d[i], h[i] / 3.0)), sub(points[i + 1], mul(d[i + 1], h[i] / 3.0)), points[i + 1]]).collect()
}

fn current(points: &[Pt]) -> Result<Vec<Cubic>, String> {
    let path = photocraft_vector::linework::interpolate_points(points)?;
    let subpath = path.subpaths.first().ok_or("production path has no subpath")?;
    Ok(subpath
        .knots
        .windows(2)
        .map(|w| {
            let p = &w[0];
            let q = &w[1];
            [[p.anchor.x, p.anchor.y], [p.out_ctrl.x, p.out_ctrl.y], [q.in_ctrl.x, q.in_ctrl.y], [q.anchor.x, q.anchor.y]]
        })
        .collect())
}

fn current_in_reverse(points: &[Pt]) -> Result<Vec<Cubic>, String> {
    let reversed: Vec<Pt> = points.iter().rev().copied().collect();
    Ok(current(&reversed)?.into_iter().rev().map(|b| [b[3], b[2], b[1], b[0]]).collect())
}

fn better_current(case: &Case, points: &[Pt], detail: bool) -> Result<(Vec<Cubic>, CaseScore), String> {
    let forward = current(points)?;
    let backward = current_in_reverse(points)?;
    let a = case_score(case, &forward, detail);
    let b = case_score(case, &backward, detail);
    Ok(if a.weighted_score_px <= b.weighted_score_px { (forward, a) } else { (backward, b) })
}

fn point_segment_distance(p: Pt, a: Pt, b: Pt) -> f64 {
    let v = sub(b, a);
    let length_sq = v[0] * v[0] + v[1] * v[1];
    let t = if length_sq > 0.0 { ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / length_sq } else { 0.0 };
    dist(p, add(a, mul(v, t.clamp(0.0, 1.0))))
}

fn distance_to_polyline(p: Pt, curve: &[Pt]) -> f64 {
    curve.windows(2).map(|w| point_segment_distance(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

fn sample_cubic(cubic: Cubic, count: usize) -> Vec<Pt> {
    (0..=count).map(|i| bezier(cubic, i as f64 / count as f64)).collect()
}

fn case_score(case: &Case, path: &[Cubic], detail: bool) -> CaseScore {
    let mut errors = Vec::new();
    let count = if detail { 60 } else { 16 };
    for (reference, cubic) in case.reference_segments_px.iter().zip(path) {
        if reference.len() < 2 {
            continue;
        }
        let generated = sample_cubic(*cubic, count);
        for p in &generated {
            errors.push(distance_to_polyline(*p, reference));
        }
        for i in 0..=count {
            let idx = (i * (reference.len() - 1)) / count;
            errors.push(distance_to_polyline(reference[idx], &generated));
        }
    }
    errors.sort_unstable_by(f64::total_cmp);
    let mean = if errors.is_empty() { f64::INFINITY } else { errors.iter().sum::<f64>() / errors.len() as f64 };
    let p95 = errors.get((errors.len().saturating_sub(1) * 95) / 100).copied().unwrap_or(f64::INFINITY);
    let max = errors.last().copied().unwrap_or(f64::INFINITY);
    CaseScore { id: case.id, mean_px: mean, p95_px: p95, max_px: max, weighted_score_px: mean + 0.25 * p95 }
}

fn longitudinal_metrics(case: &Case, segments: &[Vec<Pt>]) -> (f64, f64) {
    let mut max_overshoot: f64 = 0.0;
    let mut backward_travel = 0.0;
    for (i, curve) in segments.iter().enumerate() {
        if i + 1 >= case.anchors.len() {
            break;
        }
        let start = case.anchors[i].pixel;
        let chord = sub(case.anchors[i + 1].pixel, start);
        let length = norm(chord);
        if length < 1e-9 {
            continue;
        }
        let project = |p: Pt| (p[0] - start[0]) * chord[0] / length + (p[1] - start[1]) * chord[1] / length;
        let mut previous = None;
        for point in curve {
            let along = project(*point);
            max_overshoot = max_overshoot.max((-along).max(along - length).max(0.0));
            if let Some(before) = previous {
                let reverse: f64 = before - along;
                if reverse > 0.5 {
                    backward_travel += reverse;
                }
            }
            previous = Some(along);
        }
    }
    (max_overshoot, backward_travel)
}

fn endpoint_direction(curve: &[Pt], from_end: bool) -> Pt {
    let points: Vec<Pt> = if from_end { curve.iter().rev().copied().collect() } else { curve.to_vec() };
    let Some(start) = points.first().copied() else { return [0.0, 0.0] };
    let mut travelled = 0.0;
    for pair in points.windows(2) {
        let step = dist(pair[0], pair[1]);
        if travelled + step >= 8.0 && step > 0.0 {
            let t = (8.0 - travelled) / step;
            return sub(add(pair[0], mul(sub(pair[1], pair[0]), t)), start);
        }
        travelled += step;
    }
    points.last().map(|p| sub(*p, start)).unwrap_or([0.0, 0.0])
}

fn direction_error(a: Pt, b: Pt) -> f64 {
    if norm(a) < 1e-9 || norm(b) < 1e-9 {
        return 0.0;
    }
    cross(a, b).atan2(a[0] * b[0] + a[1] * b[1]).abs().to_degrees()
}

fn geometry_diagnostics(data: &Dataset, candidate: Candidate) -> Result<Vec<GeometryDiagnostics>, String> {
    data.cases
        .iter()
        .map(|case| {
            let points = path_points(case);
            let best: Vec<Vec<Pt>> = catmull(&points, candidate).into_iter().map(|b| sample_cubic(b, 100)).collect();
            let current: Vec<Vec<Pt>> = better_current(case, &points, true)?.0.into_iter().map(|b| sample_cubic(b, 100)).collect();
            let sai = &case.reference_segments_px;
            let (sai_overshoot, sai_back) = longitudinal_metrics(case, sai);
            let (best_overshoot, best_back) = longitudinal_metrics(case, &best);
            let (current_overshoot, current_back) = longitudinal_metrics(case, &current);
            let start_ref = sai.first().map(|s| endpoint_direction(s, false)).unwrap_or([0.0, 0.0]);
            let end_ref = sai.last().map(|s| endpoint_direction(s, true)).unwrap_or([0.0, 0.0]);
            let start_best = best.first().map(|s| endpoint_direction(s, false)).unwrap_or([0.0, 0.0]);
            let end_best = best.last().map(|s| endpoint_direction(s, true)).unwrap_or([0.0, 0.0]);
            let start_current = current.first().map(|s| endpoint_direction(s, false)).unwrap_or([0.0, 0.0]);
            let end_current = current.last().map(|s| endpoint_direction(s, true)).unwrap_or([0.0, 0.0]);
            Ok(GeometryDiagnostics {
                id: case.id,
                sai_max_longitudinal_overshoot_px: sai_overshoot,
                best_max_longitudinal_overshoot_px: best_overshoot,
                current_max_longitudinal_overshoot_px: current_overshoot,
                sai_backward_travel_px: sai_back,
                best_backward_travel_px: best_back,
                current_backward_travel_px: current_back,
                best_start_direction_error_deg: direction_error(start_ref, start_best),
                best_end_direction_error_deg: direction_error(end_ref, end_best),
                current_start_direction_error_deg: direction_error(start_ref, start_current),
                current_end_direction_error_deg: direction_error(end_ref, end_current),
            })
        })
        .collect()
}

fn score_average(scores: &[CaseScore], include: impl Fn(usize) -> bool) -> f64 {
    let kept: Vec<_> = scores.iter().filter(|s| include(s.id)).collect();
    kept.iter().map(|s| s.weighted_score_px).sum::<f64>() / kept.len() as f64
}

fn evaluate(data: &Dataset, candidate: Candidate, detail: bool) -> Vec<CaseScore> {
    data.cases
        .iter()
        .map(|case| {
            let points: Vec<Pt> = case.anchors.iter().map(|a| a.pixel).collect();
            case_score(case, &catmull(&points, candidate), detail)
        })
        .collect()
}

fn check_properties(candidate: Option<Candidate>) -> Result<Properties, String> {
    let mut seed = 0x6a09e667f3bcc909_u64;
    let mut summary = Properties {
        configurations: 0,
        max_reverse_distance_px: 0.0,
        max_distant_segment_change_px: 0.0,
        max_anchor_error_px: 0.0,
        max_tangent_angle_deg: 0.0,
        max_change_for_001px_move_px: 0.0,
        reverse_failures_over_001px: 0,
        locality_failures_over_000001px: 0,
    };
    let mut rand = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed as f64 / u64::MAX as f64) * 2.0 - 1.0
    };
    for _ in 0..1000 {
        let mut points = Vec::new();
        let mut at = [0.0, 0.0];
        for _ in 0..7 {
            at = add(at, [rand() * 80.0, rand() * 80.0]);
            points.push(at);
        }
        let model = |p: &[Pt]| -> Result<Vec<Cubic>, String> { if let Some(c) = candidate { Ok(catmull(p, c)) } else { current(p) } };
        let original = model(&points)?;
        let reversed: Vec<Pt> = points.iter().rev().copied().collect();
        let backward = model(&reversed)?;
        let mut max_rev: f64 = 0.0;
        for i in 0..original.len() {
            for j in 0..=12 {
                let t = j as f64 / 12.0;
                max_rev = max_rev.max(dist(bezier(original[i], t), bezier(backward[original.len() - 1 - i], 1.0 - t)));
            }
            summary.max_anchor_error_px = summary.max_anchor_error_px.max(dist(original[i][0], points[i]));
            summary.max_anchor_error_px = summary.max_anchor_error_px.max(dist(original[i][3], points[i + 1]));
        }
        for i in 1..original.len() {
            let incoming = sub(original[i - 1][3], original[i - 1][2]);
            let outgoing = sub(original[i][1], original[i][0]);
            if norm(incoming) > 1e-9 && norm(outgoing) > 1e-9 {
                let cross = incoming[0] * outgoing[1] - incoming[1] * outgoing[0];
                let dot = incoming[0] * outgoing[0] + incoming[1] * outgoing[1];
                summary.max_tangent_angle_deg = summary.max_tangent_angle_deg.max(cross.atan2(dot).abs().to_degrees());
            }
        }
        summary.max_reverse_distance_px = summary.max_reverse_distance_px.max(max_rev);
        if max_rev > 0.01 {
            summary.reverse_failures_over_001px += 1;
        }
        let changed = 3;
        points[changed][0] += 0.001;
        points[changed][1] -= 0.001;
        let moved = model(&points)?;
        let mut max_move: f64 = 0.0;
        let mut max_distant: f64 = 0.0;
        for i in 0..original.len() {
            for j in 0..=12 {
                let t = j as f64 / 12.0;
                let delta = dist(bezier(original[i], t), bezier(moved[i], t));
                max_move = max_move.max(delta);
                if i < changed.saturating_sub(2) || i > changed + 1 {
                    max_distant = max_distant.max(delta);
                }
            }
        }
        summary.max_change_for_001px_move_px = summary.max_change_for_001px_move_px.max(max_move);
        summary.max_distant_segment_change_px = summary.max_distant_segment_change_px.max(max_distant);
        if max_distant > 1e-6 {
            summary.locality_failures_over_000001px += 1;
        }
        summary.configurations += 1;
    }
    Ok(summary)
}

fn marker_jitter(data: &Dataset, candidate: Candidate) -> Result<JitterSummary, String> {
    let mut seed = 0x510e527fade682d1_u64;
    let mut best_total = 0.0;
    let mut current_total = 0.0;
    let mut best_wins = 0;
    let trials = 100;
    for _ in 0..trials {
        let mut best_scores = Vec::new();
        let mut current_scores = Vec::new();
        for case in &data.cases {
            let mut points = path_points(case);
            for point in &mut points {
                for coordinate in point {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    *coordinate += seed as f64 / u64::MAX as f64 - 0.5;
                }
            }
            best_scores.push(case_score(case, &catmull(&points, candidate), true));
            current_scores.push(better_current(case, &points, true)?.1);
        }
        let best = score_average(&best_scores, |_| true);
        let baseline = score_average(&current_scores, |_| true);
        best_total += best;
        current_total += baseline;
        best_wins += usize::from(best < baseline);
    }
    Ok(JitterSummary {
        trials,
        perturbation_px: 0.5,
        best_mean_score_px: best_total / trials as f64,
        current_mean_score_px: current_total / trials as f64,
        best_wins,
    })
}

fn cross(a: Pt, b: Pt) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn segment_self_intersects(cubic: Cubic) -> bool {
    let curve = sample_cubic(cubic, 32);
    for i in 0..curve.len() - 1 {
        for j in i + 2..curve.len() - 1 {
            let a = curve[i];
            let b = curve[i + 1];
            let c = curve[j];
            let d = curve[j + 1];
            let ab = sub(b, a);
            let cd = sub(d, c);
            let c1 = cross(ab, sub(c, a));
            let c2 = cross(ab, sub(d, a));
            let c3 = cross(cd, sub(a, c));
            let c4 = cross(cd, sub(b, c));
            if c1 * c2 < -1e-12 && c3 * c4 < -1e-12 {
                return true;
            }
        }
    }
    false
}

fn extreme_sweep(candidate: Option<Candidate>) -> Result<ExtremeSummary, String> {
    let lengths = [1e-7, 1e-4, 0.01, 0.1, 1.0, 10.0, 100.0, 1000.0];
    let mut result = ExtremeSummary {
        configurations: 0,
        self_intersecting_segments: 0,
        max_change_for_001px_move_non_degenerate_px: 0.0,
        max_change_for_001px_move_near_degenerate_px: 0.0,
    };
    for previous in lengths {
        for next in lengths {
            for degrees in (0..360).step_by(5) {
                let angle = (degrees as f64).to_radians();
                let c = [next * angle.cos(), next * angle.sin()];
                let points = [[-previous, 0.0], [0.0, 0.0], c, [c[0] + 80.0, c[1] + 30.0]];
                let model = |p: &[Pt]| -> Result<Vec<Cubic>, String> { if let Some(candidate) = candidate { Ok(catmull(p, candidate)) } else { current(p) } };
                let before = model(&points)?;
                result.self_intersecting_segments += before.iter().filter(|segment| segment_self_intersects(**segment)).count();
                let mut moved = points;
                moved[2][0] += 0.001;
                moved[2][1] -= 0.001;
                let after = model(&moved)?;
                let mut change: f64 = 0.0;
                for (a, b) in before.iter().zip(&after) {
                    for step in 0..=20 {
                        let t = step as f64 / 20.0;
                        change = change.max(dist(bezier(*a, t), bezier(*b, t)));
                    }
                }
                if previous >= 1.0 && next >= 1.0 {
                    result.max_change_for_001px_move_non_degenerate_px = result.max_change_for_001px_move_non_degenerate_px.max(change);
                } else {
                    result.max_change_for_001px_move_near_degenerate_px = result.max_change_for_001px_move_near_degenerate_px.max(change);
                }
                result.configurations += 1;
            }
        }
    }
    Ok(result)
}

fn path_points(case: &Case) -> Vec<Pt> {
    case.anchors.iter().map(|a| a.pixel).collect()
}

fn draw_board(data: &Dataset, best: Candidate, directory: &PathBuf) -> Result<(), String> {
    let mut s = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1440\" height=\"1080\" viewBox=\"0 0 1440 1080\">\n<rect width=\"100%\" height=\"100%\" fill=\"white\"/>\n",
    );
    for (index, case) in data.cases.iter().enumerate() {
        let x0 = 10.0 + (index % 4) as f64 * 360.0;
        let y0 = 10.0 + (index / 4) as f64 * 360.0;
        let points = path_points(case);
        let all_ref: Vec<Pt> = case.reference_segments_px.iter().flatten().copied().collect();
        if all_ref.is_empty() {
            continue;
        }
        let minx = all_ref.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) - 10.0;
        let maxx = all_ref.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max) + 10.0;
        let miny = all_ref.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min) - 10.0;
        let maxy = all_ref.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) + 10.0;
        let scale = (330.0 / (maxx - minx)).min(300.0 / (maxy - miny));
        let tx = |p: Pt| -> Pt { [x0 + 15.0 + (p[0] - minx) * scale, y0 + 37.0 + (p[1] - miny) * scale] };
        let _ = write!(
            s,
            "<rect x=\"{x0}\" y=\"{y0}\" width=\"350\" height=\"350\" fill=\"#fff\" stroke=\"#ddd\"/><text x=\"{}\" y=\"{}\" font-size=\"17\">Cas {}</text>",
            x0 + 12.0,
            y0 + 23.0,
            case.id
        );
        for (colour, width, curves) in [
            ("#e51b1b", 3.0, case.reference_segments_px.clone()),
            ("#2563eb", 1.8, catmull(&points, best).into_iter().map(|b| sample_cubic(b, 80)).collect()),
            ("#d28b13", 1.3, better_current(case, &points, true)?.0.into_iter().map(|b| sample_cubic(b, 80)).collect()),
        ] {
            for curve in curves {
                let mut path = String::new();
                for (j, p) in curve.iter().enumerate() {
                    let q = tx(*p);
                    let _ = write!(path, "{} {:.2},{:.2}", if j == 0 { "M" } else { "L" }, q[0], q[1]);
                }
                let _ = write!(s, "<path d=\"{path}\" fill=\"none\" stroke=\"{colour}\" stroke-width=\"{width}\"/>");
            }
        }
        for p in points {
            let q = tx(p);
            let _ = write!(s, "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"3\" fill=\"white\" stroke=\"black\"/>", q[0], q[1]);
        }
    }
    s.push_str("<rect x=\"12\" y=\"1051\" width=\"15\" height=\"4\" fill=\"#e51b1b\"/><text x=\"32\" y=\"1058\" font-size=\"12\">SAI</text><rect x=\"95\" y=\"1051\" width=\"15\" height=\"4\" fill=\"#2563eb\"/><text x=\"115\" y=\"1058\" font-size=\"12\">meilleur modèle</text><rect x=\"245\" y=\"1051\" width=\"15\" height=\"4\" fill=\"#d28b13\"/><text x=\"265\" y=\"1058\" font-size=\"12\">PhotoCraft actuel</text></svg>");
    fs::write(directory.join("comparison_board.svg"), s).map_err(|e| e.to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data: Dataset = serde_json::from_str(&fs::read_to_string(directory.join("dataset.json"))?)?;
    if data.cases.len() != 12 || data.cases.iter().any(|c| !c.uncertainty.is_empty()) {
        return Err("expected twelve extracted cases without broken routes".into());
    }
    let current_scores: Vec<CaseScore> = data
        .cases
        .iter()
        .map(|case| {
            let path = current(&path_points(case));
            path.map(|p| case_score(case, &p, true))
        })
        .collect::<Result<_, _>>()?;
    let current_score = score_average(&current_scores, |_| true);
    let current_best_orientation: Vec<CaseScore> =
        data.cases.iter().map(|case| better_current(case, &path_points(case), true).map(|pair| pair.1)).collect::<Result<_, _>>()?;
    let current_best_orientation_score = score_average(&current_best_orientation, |_| true);
    let mut ranked = Vec::new();
    let endpoints: Vec<_> = Endpoint::ALL.into_iter().chain((1..=9).map(|i| Endpoint::ScaledSecant(i as f64 * 0.1))).collect();
    for alpha_step in 0..=40 {
        for tension_step in 0..=40 {
            for endpoint in &endpoints {
                let candidate =
                    Candidate { alpha: alpha_step as f64 * 0.025, tension: -0.25 + tension_step as f64 * 0.025, balance_power: 0.0, endpoint: *endpoint };
                let scores = evaluate(&data, candidate, false);
                ranked.push(Ranked { candidate, score_px: score_average(&scores, |_| true), cases: scores });
            }
        }
    }
    // A single ratio-dependent factor multiplies both sides of an interior knot. This
    // extends the cardinal family without sacrificing locality, reversal or G1 tangents.
    for alpha_step in 0..=16 {
        for tension_step in 0..=10 {
            for balance_step in -6..=10 {
                if balance_step == 0 {
                    continue;
                }
                for endpoint in [Endpoint::Zero, Endpoint::ScaledSecant(0.1)] {
                    let candidate = Candidate {
                        alpha: 0.2 + alpha_step as f64 * 0.025,
                        tension: -0.1 + tension_step as f64 * 0.025,
                        balance_power: balance_step as f64 * 0.05,
                        endpoint,
                    };
                    let scores = evaluate(&data, candidate, false);
                    ranked.push(Ranked { candidate, score_px: score_average(&scores, |_| true), cases: scores });
                }
            }
        }
    }
    // Refine the promising region without adding per-case or threshold rules.
    for alpha_step in 0..=8 {
        for tension_step in 0..=24 {
            for balance_step in 0..=24 {
                for endpoint in [Endpoint::Zero, Endpoint::ScaledSecant(0.1)] {
                    let candidate = Candidate {
                        alpha: 0.30 + alpha_step as f64 * 0.0125,
                        tension: -0.25 + tension_step as f64 * 0.0125,
                        balance_power: 0.25 + balance_step as f64 * 0.0125,
                        endpoint,
                    };
                    let scores = evaluate(&data, candidate, false);
                    ranked.push(Ranked { candidate, score_px: score_average(&scores, |_| true), cases: scores });
                }
            }
        }
    }
    ranked.sort_by(|a, b| f64::total_cmp(&a.score_px, &b.score_px));
    let count = ranked.len();
    let canonical_family_best: Vec<_> = [0.0, 0.5, 1.0]
        .into_iter()
        .filter_map(|alpha| {
            ranked.iter().find(|entry| entry.candidate.balance_power == 0.0 && (entry.candidate.alpha - alpha).abs() < 1e-10).map(|entry| {
                let cases = evaluate(&data, entry.candidate, true);
                Ranked { candidate: entry.candidate, score_px: score_average(&cases, |_| true), cases }
            })
        })
        .collect();
    let endpoint_family_best: Vec<_> = endpoints
        .iter()
        .filter_map(|endpoint| {
            ranked
                .iter()
                .find(|entry| {
                    entry.candidate.balance_power == 0.0
                        && std::mem::discriminant(&entry.candidate.endpoint) == std::mem::discriminant(endpoint)
                        && match (entry.candidate.endpoint, endpoint) {
                            (Endpoint::ScaledSecant(a), Endpoint::ScaledSecant(b)) => (a - b).abs() < 1e-10,
                            _ => true,
                        }
                })
                .map(|entry| {
                    let cases = evaluate(&data, entry.candidate, true);
                    Ranked { candidate: entry.candidate, score_px: score_average(&cases, |_| true), cases }
                })
        })
        .collect();
    let mut folds = Vec::new();
    for fold in 0..3 {
        let selected = ranked
            .iter()
            .min_by(|a, b| f64::total_cmp(&score_average(&a.cases, |id| id % 3 != fold), &score_average(&b.cases, |id| id % 3 != fold)))
            .ok_or("candidate search returned no results")?;
        let detailed = evaluate(&data, selected.candidate, true);
        folds.push(Fold {
            held_out_ids: data.cases.iter().filter(|c| c.id % 3 == fold).map(|c| c.id).collect(),
            selected: selected.candidate,
            train_score_px: score_average(&detailed, |id| id % 3 != fold),
            validation_score_px: score_average(&detailed, |id| id % 3 == fold),
        });
    }
    let mut leave_one_out = Vec::new();
    for held_out in 1..=12 {
        let selected = ranked
            .iter()
            .min_by(|a, b| f64::total_cmp(&score_average(&a.cases, |id| id != held_out), &score_average(&b.cases, |id| id != held_out)))
            .ok_or("candidate search returned no results")?;
        let detailed = evaluate(&data, selected.candidate, true);
        leave_one_out.push(Fold {
            held_out_ids: vec![held_out],
            selected: selected.candidate,
            train_score_px: score_average(&detailed, |id| id != held_out),
            validation_score_px: score_average(&detailed, |id| id == held_out),
        });
    }
    let mut top: Vec<_> = ranked
        .into_iter()
        .take(100)
        .map(|mut entry| {
            entry.cases = evaluate(&data, entry.candidate, true);
            entry.score_px = score_average(&entry.cases, |_| true);
            entry
        })
        .collect();
    top.sort_by(|a, b| f64::total_cmp(&a.score_px, &b.score_px));
    top.truncate(15);
    let best = top.first().ok_or("candidate search returned no results")?.candidate;
    let parsimonious_candidate = Candidate { alpha: 1.0 / 3.0, tension: 0.0, balance_power: 0.0, endpoint: Endpoint::Zero };
    let parsimonious_cases = evaluate(&data, parsimonious_candidate, true);
    let parsimonious = Ranked { candidate: parsimonious_candidate, score_px: score_average(&parsimonious_cases, |_| true), cases: parsimonious_cases };
    let simple_local_candidate = Candidate { alpha: 1.0 / 3.0, tension: -0.2, balance_power: 0.5, endpoint: Endpoint::Zero };
    let simple_local_cases = evaluate(&data, simple_local_candidate, true);
    let simple_local = Ranked { candidate: simple_local_candidate, score_px: score_average(&simple_local_cases, |_| true), cases: simple_local_cases };
    let result = Results { scoring: "Each corresponding segment: symmetric point-to-polyline distances, 60 samples in detailed results, per-case mean + 0.25*p95, unweighted average across twelve cases; search used 16 samples per segment".into(),
        candidates_tested: count, top, parsimonious, simple_local, canonical_family_best, endpoint_family_best,
        current: current_scores, current_score_px: current_score,
        current_best_orientation, current_best_orientation_score_px: current_best_orientation_score,
        folds, leave_one_out, properties_best: check_properties(Some(best))?, properties_current: check_properties(None)?,
        marker_jitter: marker_jitter(&data, best)?, extreme_best: extreme_sweep(Some(best))?,
        extreme_current: extreme_sweep(None)?, geometry_diagnostics: geometry_diagnostics(&data, best)? };
    fs::write(directory.join("results.json"), serde_json::to_string_pretty(&result)? + "\n")?;
    draw_board(&data, best, &directory)?;
    println!(
        "searched {} candidates; best {:?}; detailed score {:.3}px; current {:.3}px; current best orientation {:.3}px",
        result.candidates_tested, best, result.top[0].score_px, result.current_score_px, result.current_best_orientation_score_px
    );
    for f in &result.folds {
        println!("holdout {:?}: {:?}, train {:.3}px, validation {:.3}px", f.held_out_ids, f.selected, f.train_score_px, f.validation_score_px);
    }
    println!(
        "best properties: reverse {:.6}px, locality {:.6}px, anchor {:.6}px, 0.001px move {:.6}px",
        result.properties_best.max_reverse_distance_px,
        result.properties_best.max_distant_segment_change_px,
        result.properties_best.max_anchor_error_px,
        result.properties_best.max_change_for_001px_move_px
    );
    println!(
        "simple local formula {:.3}px; marker jitter best {:.3}px, current {:.3}px, wins {}/{}",
        result.simple_local.score_px,
        result.marker_jitter.best_mean_score_px,
        result.marker_jitter.current_mean_score_px,
        result.marker_jitter.best_wins,
        result.marker_jitter.trials
    );
    println!(
        "extreme sweep {} cases: best loops {}, current loops {}, best max nondegenerate move {:.4}px",
        result.extreme_best.configurations,
        result.extreme_best.self_intersecting_segments,
        result.extreme_current.self_intersecting_segments,
        result.extreme_best.max_change_for_001px_move_non_degenerate_px
    );
    println!(
        "current properties: reverse {:.3}px, locality {:.6}px, anchor {:.6}px, 0.001px move {:.6}px",
        result.properties_current.max_reverse_distance_px,
        result.properties_current.max_distant_segment_change_px,
        result.properties_current.max_anchor_error_px,
        result.properties_current.max_change_for_001px_move_px
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_anchors_collapse_without_a_loop() {
        let c = Candidate { alpha: 1.0 / 3.0, tension: 0.0, balance_power: 0.0, endpoint: Endpoint::Zero };
        let points = [[0.0, 0.0], [100.0, 20.0], [100.0, 20.0], [160.0, 80.0]];
        let path = catmull(&points, c);
        assert!(path[1].iter().all(|p| dist(*p, points[1]) < 1e-12));
        assert_eq!(path[0][3], points[1]);
        assert_eq!(path[2][0], points[2]);
    }

    #[test]
    fn uniform_standard_uses_centered_tangent() {
        let points = [[0.0, 0.0], [50.0, 0.0], [60.0, 30.0]];
        let c = Candidate { alpha: 0.0, tension: 0.0, balance_power: 0.0, endpoint: Endpoint::Secant };
        let path = catmull(&points, c);
        let derivative_at_middle = mul(sub(points[2], points[0]), 0.5);
        assert!(dist(mul(sub(points[1], path[0][2]), 3.0), derivative_at_middle) < 1e-12);
        assert!(dist(mul(sub(path[1][1], points[1]), 3.0), derivative_at_middle) < 1e-12);
    }
}
