//! Geometry helpers for the SaiLine linework prototype.
//!
//! The document keeps canonical anchors and an optional live handle; this module derives the cubic
//! Bézier path consumed by the existing renderer so the engine and UI share one interpolation.

use photocraft_color::Color;
use photocraft_doc::{Fill, Knot, Path, PathOp, ShapeLayer, Subpath};
use photocraft_geom::Point;

const MAX_POINTS: usize = 4096;

/// Interpolation choices retained for research and regression tests. SaiSimplified is the
/// production model; Legacy remains available as a comparison baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InterpolationModel {
    Legacy,
    #[default]
    SaiSimplified,
    SaiOptimized,
}

impl InterpolationModel {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "legacy" => Some(Self::Legacy),
            "saiSimplified" => Some(Self::SaiSimplified),
            "saiOptimized" => Some(Self::SaiOptimized),
            _ => None,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::SaiSimplified => "saiSimplified",
            Self::SaiOptimized => "saiOptimized",
        }
    }
}

fn smoothstep01(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn endpoint_extrapolation_weight(first: [f64; 2], second: [f64; 2]) -> f64 {
    let first_length = first[0].hypot(first[1]);
    let second_length = second[0].hypot(second[1]);
    if first_length <= f64::EPSILON || second_length <= f64::EPSILON {
        return 0.0;
    }
    let turn = (first[0] * second[1] - first[1] * second[0]).atan2(first[0] * second[0] + first[1] * second[1]).abs();
    let ratio = second_length / first_length;
    let length_weight = smoothstep01((ratio - 0.75) / 0.25) * smoothstep01(3.0 - ratio);
    let turn_weight = smoothstep01((140.0_f64.to_radians() - turn) / 30.0_f64.to_radians());
    length_weight * turn_weight
}

/// Build an open cubic Bézier path that passes through every input point.
///
/// Interior handles start from the centred difference `(next - previous) / 2`, with their lengths
/// adjusted independently for the adjoining segments. A short outgoing return also turns toward
/// its own chord, so the previous long sweep can arrive gently before the path turns sharply.
/// Both handles shrink toward zero when two anchors approach each other.
pub fn interpolate_points(points: &[[f64; 2]]) -> Result<Path, String> {
    interpolate_points_with_handle(points, None)
}

/// Interpolate with an optional live handle at the last anchor. SaiLine uses the handle's
/// direction for the incoming tangent and its distance as the rounding amount.
pub fn interpolate_points_with_handle(points: &[[f64; 2]], handle: Option<[f64; 2]>) -> Result<Path, String> {
    interpolate_points_with_model(points, handle, InterpolationModel::SaiSimplified)
}

/// Interpolate with one of the experimental SAI parameter sets, or the unchanged Legacy model.
pub fn interpolate_points_with_model(points: &[[f64; 2]], handle: Option<[f64; 2]>, model: InterpolationModel) -> Result<Path, String> {
    if model != InterpolationModel::Legacy {
        return interpolate_sai(points, model);
    }
    if points.len() < 2 {
        return Err("linework needs at least two points".into());
    }
    if points.len() > MAX_POINTS {
        return Err(format!("linework accepts at most {MAX_POINTS} points"));
    }
    if points.iter().any(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0)) {
        return Err("linework points must be finite and within ±1000000 px".into());
    }

    let n = points.len();
    let mut knots = Vec::with_capacity(n);
    for i in 0..n {
        let point = *points.get(i).ok_or("linework point index out of range")?;
        let previous = if i == 0 { point } else { *points.get(i - 1).ok_or("linework point index out of range")? };
        let next = if i + 1 == n { point } else { *points.get(i + 1).ok_or("linework point index out of range")? };
        let previous_length = (point[0] - previous[0]).hypot(point[1] - previous[1]);
        let next_length = (next[0] - point[0]).hypot(next[1] - point[1]);
        let previous_coincident = i > 0 && previous_length <= f64::EPSILON;
        let next_coincident = i + 1 < n && next_length <= f64::EPSILON;
        let (centered, in_scale, out_scale) = if previous_coincident || next_coincident {
            ([0.0, 0.0], 0.0, 0.0)
        } else {
            let ratio = previous_length.min(next_length) / previous_length.max(next_length);
            // The old symmetric ratio flattened long spans almost into their chord. Give the
            // longer side more reach, while the short side only eases toward the full tangent
            // when the neighboring lengths are comparable. Both scales still vanish as the
            // shorter span approaches zero, preserving sharp corners from close anchors.
            let elastic = (1.15 * ratio / (ratio + 0.15)).min(1.0);
            let similarity = ((ratio - 0.4) / 0.2).clamp(0.0, 1.0);
            let short = ratio + (elastic - ratio) * similarity * 0.25;
            // The extra pull on the longer side must vanish as the two lengths converge.
            // Otherwise a subpixel change across equality swaps 1.4 and 1.0 between the
            // handles and visibly jumps the preceding segment.
            let separation = smoothstep01((1.0 - ratio) / 0.4);
            let long = short + ((elastic * 1.4).min(1.4) - short) * separation;
            let (in_scale, out_scale) = if previous_length >= next_length { (long, short) } else { (short, long) };
            ([(next[0] - previous[0]) * 0.5, (next[1] - previous[1]) * 0.5], in_scale, out_scale)
        };
        let in_tangent = if i == 0 || previous_coincident {
            [0.0, 0.0]
        } else if i + 1 == n {
            if handle.is_none() { [point[0] - previous[0], point[1] - previous[1]] } else { [0.0, 0.0] }
        } else {
            [centered[0] * in_scale, centered[1] * in_scale]
        };
        let out_tangent = if i + 1 == n || next_coincident {
            [0.0, 0.0]
        } else if i == 0 {
            [next[0] - point[0], next[1] - point[1]]
        } else if next_length < previous_length {
            // A long incoming sweep should not dictate the departure angle of a much shorter
            // return. Turn its outgoing handle toward the next chord while preserving the
            // incoming handle; this allows the sharper bend seen at the end of a SAI sweep.
            let centered_length = centered[0].hypot(centered[1]);
            if centered_length <= f64::EPSILON {
                [0.0, 0.0]
            } else {
                let turn = 1.0 - next_length / previous_length;
                let direction = [
                    (1.0 - turn) * centered[0] / centered_length + turn * (next[0] - point[0]) / next_length,
                    (1.0 - turn) * centered[1] / centered_length + turn * (next[1] - point[1]) / next_length,
                ];
                // Reorient the handle toward the short chord without borrowing the long
                // incoming handle's magnitude. A regularized norm lets opposite directions
                // cancel to a zero handle instead of flipping a full-length handle instantly.
                let softening = 0.2_f64;
                let norm = (direction[0] * direction[0] + direction[1] * direction[1] + softening * softening).sqrt();
                let scale = centered_length * out_scale * (1.0 + softening * softening).sqrt() / norm;
                [direction[0] * scale, direction[1] * scale]
            }
        } else {
            [centered[0] * out_scale, centered[1] * out_scale]
        };
        let anchor = Point::new(point[0], point[1]);
        let in_ctrl = Point::new(point[0] - in_tangent[0] / 3.0, point[1] - in_tangent[1] / 3.0);
        let out_ctrl = Point::new(point[0] + out_tangent[0] / 3.0, point[1] + out_tangent[1] / 3.0);
        let cross = in_tangent[0] * out_tangent[1] - in_tangent[1] * out_tangent[0];
        let aligned = cross.abs() <= 1e-4 * in_tangent[0].hypot(in_tangent[1]) * out_tangent[0].hypot(out_tangent[1]);
        let smooth = i > 0 && i + 1 < n && aligned && in_tangent[0] * out_tangent[0] + in_tangent[1] * out_tangent[1] > 0.0;
        knots.push(Knot { anchor, in_ctrl, out_ctrl, smooth });
    }
    if n > 2 {
        // A free start whose following span is at least as long should already lean into the
        // coming turn. Using the first chord alone flattens the beginning of three-point arcs.
        let first = [points[1][0] - points[0][0], points[1][1] - points[0][1]];
        let second = [points[2][0] - points[1][0], points[2][1] - points[1][1]];
        let weight = endpoint_extrapolation_weight(first, second);
        if weight > 0.0
            && let Some(next) = knots.get(1)
        {
            let next_incoming = [3.0 * (next.anchor.x - next.in_ctrl.x), 3.0 * (next.anchor.y - next.in_ctrl.y)];
            if let Some(start) = knots.first_mut() {
                start.out_ctrl = Point::new(
                    start.anchor.x + (first[0] + 0.4 * weight * (first[0] - next_incoming[0])) / 3.0,
                    start.anchor.y + (first[1] + 0.4 * weight * (first[1] - next_incoming[1])) / 3.0,
                );
            }
        }
    }
    // With two anchors SaiLine treats the live cursor as the quadratic control point for the
    // whole arc. This keeps both ends on the same side of the chord instead of twisting the
    // first tangent away from the cursor.
    if n == 2
        && let Some(control) = handle
    {
        let a = points[0];
        let b = points[1];
        knots[0].out_ctrl = Point::new(a[0] + (control[0] - a[0]) * (2.0 / 3.0), a[1] + (control[1] - a[1]) * (2.0 / 3.0));
        knots[1].in_ctrl = Point::new(b[0] + (control[0] - b[0]) * (2.0 / 3.0), b[1] + (control[1] - b[1]) * (2.0 / 3.0));
    }
    if n > 2
        && let Some(control) = handle
    {
        let point = points[n - 1];
        let mut tangent = [control[0] - point[0], control[1] - point[1]];
        let segment = [point[0] - points[n - 2][0], point[1] - points[n - 2][1]];
        let max_len = (segment[0].hypot(segment[1]) * 3.0).max(1.0);
        let len = tangent[0].hypot(tangent[1]);
        if len > max_len {
            let scale = max_len / len;
            tangent[0] *= scale;
            tangent[1] *= scale;
        }
        knots[n - 1].in_ctrl = Point::new(point[0] - tangent[0] / 3.0, point[1] - tangent[1] / 3.0);
    }
    if n > 2
        && handle.is_none()
        && let Some(previous) = knots.get(n - 2)
    {
        // Continue a gentle change of direction only when a long span ends in a shorter return.
        // On an equally long or longer final span, this continuation bows the curve away from
        // the SAI centerline (especially visible on a long diagonal after a horizontal segment).
        let outgoing = [previous.out_ctrl.x - previous.anchor.x, previous.out_ctrl.y - previous.anchor.y];
        let chord = [points[n - 1][0] - points[n - 2][0], points[n - 1][1] - points[n - 2][1]];
        let chord_length = chord[0].hypot(chord[1]);
        let preceding = [points[n - 2][0] - points[n - 3][0], points[n - 2][1] - points[n - 3][1]];
        let preceding_length = preceding[0].hypot(preceding[1]);
        if outgoing[0].hypot(outgoing[1]) > f64::EPSILON && chord_length > f64::EPSILON && preceding_length > f64::EPSILON {
            let cross = outgoing[0] * chord[1] - outgoing[1] * chord[0];
            let dot = outgoing[0] * chord[0] + outgoing[1] * chord[1];
            let turn = cross.atan2(dot);
            // Fade the endpoint correction in as the last span becomes shorter, and out as
            // the turn approaches a right angle. Hard gates at either boundary visibly jump
            // the curve under tiny point movements.
            let shortness = smoothstep01(((preceding_length - chord_length) / preceding_length) / 0.4);
            let gentle_turn = smoothstep01((std::f64::consts::FRAC_PI_2 - turn.abs()) / 0.35);
            let weight = shortness * gentle_turn;
            if weight > 0.0 {
                let direction = chord[1].atan2(chord[0]) + turn * 0.25;
                let length = chord_length * (1.0 - 0.3 * (turn.abs() / 0.5).min(1.0));
                if let Some(last) = knots.last_mut() {
                    let point = points[n - 1];
                    let adjusted = [point[0] - direction.cos() * length / 3.0, point[1] - direction.sin() * length / 3.0];
                    last.in_ctrl =
                        Point::new(last.in_ctrl.x + (adjusted[0] - last.in_ctrl.x) * weight, last.in_ctrl.y + (adjusted[1] - last.in_ctrl.y) * weight);
                }
            }
            // Mirror the free-start continuation on a long final span. Fade it out for sharp
            // reversals and short returns, whose separate endpoint rule applies above.
            let continuation = endpoint_extrapolation_weight(preceding, chord);
            if continuation > 0.0
                && let Some(last) = knots.last_mut()
            {
                let extra = [0.35 * continuation * (chord[0] - outgoing[0] * 3.0), 0.35 * continuation * (chord[1] - outgoing[1] * 3.0)];
                last.in_ctrl = Point::new(last.in_ctrl.x - extra[0] / 3.0, last.in_ctrl.y - extra[1] / 3.0);
            }
        }
    }
    Ok(Path::new(vec![Subpath { closed: false, knots, op: PathOp::Combine }]))
}

/// Port of `experiments/sailine_sai126/src/main.rs::catmull`, including its two-point
/// secant special case and coincident-anchor handling. The balance power is the experiment's
/// β parameter; α controls chord-length parameterization.
fn interpolate_sai(points: &[[f64; 2]], model: InterpolationModel) -> Result<Path, String> {
    if points.len() < 2 {
        return Err("linework needs at least two points".into());
    }
    if points.len() > MAX_POINTS {
        return Err(format!("linework accepts at most {MAX_POINTS} points"));
    }
    if points.iter().any(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0)) {
        return Err("linework points must be finite and within ±1000000 px".into());
    }
    let (alpha, beta, tension) = if model == InterpolationModel::SaiSimplified { (1.0 / 3.0, 0.5, -0.2) } else { (0.3375, 0.4875, -0.2) };
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let n = points.len();
    let h: Vec<f64> = points.windows(2).map(|w| distance(w[0], w[1]).max(1e-9).powf(alpha)).collect();
    let mut d = vec![[0.0; 2]; n];
    for i in 1..n - 1 {
        let a = h[i - 1];
        let b = h[i];
        let sa = [(points[i][0] - points[i - 1][0]) / a, (points[i][1] - points[i - 1][1]) / a];
        let sb = [(points[i + 1][0] - points[i][0]) / b, (points[i + 1][1] - points[i][1]) / b];
        d[i] = [(sa[0] * b + sb[0] * a) / (a + b), (sa[1] * b + sb[1] * a) / (a + b)];
        let previous = distance(points[i], points[i - 1]);
        let next = distance(points[i + 1], points[i]);
        let ratio = previous.min(next) / previous.max(next).max(1e-12);
        let scale = ratio.max(1e-12).powf(beta);
        d[i][0] *= scale;
        d[i][1] *= scale;
    }
    // The reference experiment sets both endpoint tangents to zero for n > 2. With
    // exactly two points it uses secants, so the segment remains straight.
    if n == 2 {
        d[0] = [(points[1][0] - points[0][0]) / h[0], (points[1][1] - points[0][1]) / h[0]];
        d[1] = d[0];
    }
    let scale = 1.0 - tension;
    d.iter_mut().for_each(|v| {
        v[0] *= scale;
        v[1] *= scale;
    });
    for i in 0..n - 1 {
        if distance(points[i], points[i + 1]) <= f64::EPSILON {
            d[i] = [0.0, 0.0];
            d[i + 1] = [0.0, 0.0];
        }
    }
    let knots = (0..n)
        .map(|i| {
            let anchor = Point::new(points[i][0], points[i][1]);
            let (in_ctrl, out_ctrl) = if i == 0 {
                (anchor, Point::new(anchor.x + d[i][0] * h[0] / 3.0, anchor.y + d[i][1] * h[0] / 3.0))
            } else if i + 1 == n {
                (Point::new(anchor.x - d[i][0] * h[i - 1] / 3.0, anchor.y - d[i][1] * h[i - 1] / 3.0), anchor)
            } else {
                (
                    Point::new(anchor.x - d[i][0] * h[i - 1] / 3.0, anchor.y - d[i][1] * h[i - 1] / 3.0),
                    Point::new(anchor.x + d[i][0] * h[i] / 3.0, anchor.y + d[i][1] * h[i] / 3.0),
                )
            };
            Knot { anchor, in_ctrl, out_ctrl, smooth: i > 0 && i + 1 < n }
        })
        .collect();
    Ok(Path::new(vec![Subpath { closed: false, knots, op: PathOp::Combine }]))
}

/// Build a filled outline with linearly interpolated width factors at the logical anchors.
pub fn variable_width_shape(points: &[[f64; 2]], factors: &[f32], width: f64, color: Color, handle: Option<[f64; 2]>) -> Result<ShapeLayer, String> {
    variable_width_shape_with_model(points, factors, width, color, handle, InterpolationModel::SaiSimplified)
}

/// Build the variable-width outline using the requested centerline interpolation model.
pub fn variable_width_shape_with_model(
    points: &[[f64; 2]],
    factors: &[f32],
    width: f64,
    color: Color,
    handle: Option<[f64; 2]>,
    model: InterpolationModel,
) -> Result<ShapeLayer, String> {
    if factors.len() != points.len() {
        return Err("linework width factors must match point count".into());
    }
    if !width.is_finite() || width <= 0.0 {
        return Err("linework width must be positive and finite".into());
    }
    if factors.iter().any(|factor| !factor.is_finite() || *factor < 0.0) {
        return Err("linework width factors must be finite and non-negative".into());
    }
    // A very short endpoint return is wholly covered by the round cap at ordinary pressure.
    // Drawing its offset normals nevertheless folds the outline back on itself and produces
    // a visible spur. Keep the canonical points and interpolation untouched; only merge this
    // covered portion of the *raster outline* into the endpoint, fading the merge out smoothly.
    let mut outline_points = points.to_vec();
    if points.len() >= 3 {
        let endpoint_overlap = |end: usize, adjacent: usize, following: usize| {
            let pressure_similarity = 1.0 - smoothstep01((f64::from((factors[end] - factors[adjacent]).abs()) - 0.02) / 0.08);
            if pressure_similarity <= 0.0 {
                return 0.0;
            }
            let short = (points[adjacent][0] - points[end][0]).hypot(points[adjacent][1] - points[end][1]);
            if short <= 1e-9 {
                return pressure_similarity;
            }
            let next = (points[following][0] - points[adjacent][0]).hypot(points[following][1] - points[adjacent][1]);
            let diameter = width * f64::from((factors[end] + factors[adjacent]) * 0.5);
            if next <= 1e-9 || diameter <= 1e-9 {
                return 0.0;
            }
            let incoming = [points[adjacent][0] - points[end][0], points[adjacent][1] - points[end][1]];
            let outgoing = [points[following][0] - points[adjacent][0], points[following][1] - points[adjacent][1]];
            let cosine = (incoming[0] * outgoing[0] + incoming[1] * outgoing[1]) / (short * next);
            let reversal = smoothstep01(-cosine / 0.2);
            let covered = 1.0 - smoothstep01(short / diameter - 1.0);
            let local = 1.0 - smoothstep01((short / next - 0.03) / 0.07);
            reversal * covered * local * pressure_similarity
        };
        let first = endpoint_overlap(0, 1, 2);
        let last = endpoint_overlap(points.len() - 1, points.len() - 2, points.len() - 3);
        let (end, adjacent, amount) = if last > first { (points.len() - 1, points.len() - 2, last) } else { (0, 1, first) };
        if amount > 0.0 {
            outline_points[adjacent][0] += (points[end][0] - points[adjacent][0]) * amount;
            outline_points[adjacent][1] += (points[end][1] - points[adjacent][1]) * amount;
        }
        if points.len() > 3 {
            let (end, adjacent, amount) = if last > first { (0, 1, first) } else { (points.len() - 1, points.len() - 2, last) };
            if amount > 0.0 {
                outline_points[adjacent][0] += (points[end][0] - points[adjacent][0]) * amount;
                outline_points[adjacent][1] += (points[end][1] - points[adjacent][1]) * amount;
            }
        }
    }
    let center = interpolate_points_with_model(&outline_points, handle, model)?;
    let segments = center.subpaths.first().ok_or("linework path has no subpath")?.segments();
    let mut samples = Vec::new();
    let last_visible = outline_points.windows(2).rposition(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]) > 1e-9);
    for (index, segment) in segments.iter().enumerate() {
        let segment_length = (outline_points[index + 1][0] - outline_points[index][0]).hypot(outline_points[index + 1][1] - outline_points[index][1]);
        if segment_length <= 1e-9 {
            continue;
        }
        let minimum_radius = (width * 0.25).max(0.25);
        let sample_count = ((segment_length / minimum_radius).ceil() as usize + 1).clamp(64, 512);
        let steps = if Some(index) == last_visible { sample_count } else { sample_count - 1 };
        for step in 0..steps {
            let t = step as f64 / (sample_count - 1) as f64;
            let u = 1.0 - t;
            let p = [
                u * u * u * segment[0].x + 3.0 * u * u * t * segment[1].x + 3.0 * u * t * t * segment[2].x + t * t * t * segment[3].x,
                u * u * u * segment[0].y + 3.0 * u * u * t * segment[1].y + 3.0 * u * t * t * segment[2].y + t * t * t * segment[3].y,
            ];
            let factor = factors[index] as f64 * (1.0 - t) + factors[index + 1] as f64 * t;
            let radius = width * factor.max(0.0) * 0.5;
            samples.push((p, radius));
        }
    }
    let mut left = Vec::with_capacity(samples.len());
    let mut right = Vec::with_capacity(samples.len());
    for (index, (center, radius)) in samples.iter().enumerate() {
        let previous = samples.get(index.saturating_sub(1)).map_or(*center, |sample| sample.0);
        let next = samples.get((index + 1).min(samples.len().saturating_sub(1))).map_or(*center, |sample| sample.0);
        let tangent = [next[0] - previous[0], next[1] - previous[1]];
        let length = tangent[0].hypot(tangent[1]).max(1e-9);
        let normal = [-tangent[1] / length, tangent[0] / length];
        left.push((center[0] + normal[0] * radius, center[1] + normal[1] * radius));
        right.push((center[0] - normal[0] * radius, center[1] - normal[1] * radius));
    }
    right.reverse();
    left.extend(right);
    let mut subpaths = if left.len() >= 3 { vec![Subpath::polygon(&left).with_op(PathOp::Combine)] } else { Vec::new() };
    // Round open endpoints with their actual effective radius. The sampled side outline closes
    // with a diameter; these unions turn that butt closure into the round cap SAI shows.
    for index in [0, points.len().saturating_sub(1)] {
        let radius = width * f64::from(factors[index].max(0.0)) * 0.5;
        if radius <= 0.0 {
            continue;
        }
        let circle: Vec<(f64, f64)> = (0..48)
            .map(|step| {
                let angle = std::f64::consts::TAU * step as f64 / 48.0;
                (points[index][0] + angle.cos() * radius, points[index][1] + angle.sin() * radius)
            })
            .collect();
        subpaths.push(Subpath::polygon(&circle).with_op(PathOp::Combine));
    }
    for (index, point) in outline_points.iter().enumerate() {
        if index == 0 || index + 1 == points.len() {
            continue;
        }
        let radius = width * f64::from((factors[index] - 1.0).max(0.0)) * 0.5;
        if radius <= 0.0 {
            continue;
        }
        let circle: Vec<(f64, f64)> = (0..24)
            .map(|step| {
                let angle = std::f64::consts::TAU * step as f64 / 24.0;
                (point[0] + angle.cos() * radius, point[1] + angle.sin() * radius)
            })
            .collect();
        subpaths.push(Subpath::polygon(&circle).with_op(PathOp::Combine));
    }
    Ok(ShapeLayer { path: Path::new(subpaths), fill: Some(Fill::Solid(color)), stroke: None, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(segment: &[Point; 4], t: f64) -> [f64; 2] {
        let u = 1.0 - t;
        [
            u * u * u * segment[0].x + 3.0 * u * u * t * segment[1].x + 3.0 * u * t * t * segment[2].x + t * t * t * segment[3].x,
            u * u * u * segment[0].y + 3.0 * u * u * t * segment[1].y + 3.0 * u * t * t * segment[2].y + t * t * t * segment[3].y,
        ]
    }

    fn sampled_path_distance(a: &[[f64; 2]], b: &[[f64; 2]]) -> f64 {
        let a = interpolate_points_with_model(a, None, InterpolationModel::Legacy).unwrap();
        let b = interpolate_points_with_model(b, None, InterpolationModel::Legacy).unwrap();
        let a_segments = a.subpaths[0].segments();
        let b_segments = b.subpaths[0].segments();
        assert_eq!(a_segments.len(), b_segments.len());
        a_segments
            .iter()
            .zip(b_segments.iter())
            .flat_map(|(a, b)| {
                (0..=20).map(move |step| {
                    let t = f64::from(step) / 20.0;
                    let u = 1.0 - t;
                    let eval = |s: &[Point; 4]| {
                        [
                            u * u * u * s[0].x + 3.0 * u * u * t * s[1].x + 3.0 * u * t * t * s[2].x + t * t * t * s[3].x,
                            u * u * u * s[0].y + 3.0 * u * u * t * s[1].y + 3.0 * u * t * t * s[2].y + t * t * t * s[3].y,
                        ]
                    };
                    let a = eval(a);
                    let b = eval(b);
                    (a[0] - b[0]).hypot(a[1] - b[1])
                })
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn interpolation_preserves_all_anchors_and_is_open() {
        let points = [[0.0, 0.0], [10.0, 20.0], [30.0, 5.0], [50.0, 15.0]];
        let path = interpolate_points(&points).unwrap();
        let subpath = path.subpaths.first().unwrap();
        assert!(!subpath.closed);
        assert_eq!(subpath.knots.len(), points.len());
        for (knot, point) in subpath.knots.iter().zip(points) {
            assert_eq!(knot.anchor, Point::new(point[0], point[1]));
        }
        assert!(subpath.knots[1].smooth);
        assert!(!subpath.knots[0].smooth);
        assert!(!subpath.knots[3].smooth);
    }

    #[test]
    fn two_points_produce_a_straight_cubic() {
        let path = interpolate_points_with_model(&[[0.0, 0.0], [12.0, 6.0]], None, InterpolationModel::Legacy).unwrap();
        let segment = path.subpaths[0].segments()[0];
        assert_eq!(segment[1], Point::new(4.0, 2.0));
        assert_eq!(segment[2], Point::new(8.0, 4.0));
        assert_eq!(segment[3], Point::new(12.0, 6.0));
    }

    #[test]
    fn coincident_anchors_create_a_cusp_without_a_looping_handle() {
        let path = interpolate_points(&[[0.0, 0.0], [0.0, 0.0], [30.0, 20.0]]).unwrap();
        let knots = &path.subpaths[0].knots;
        assert_eq!(knots[0].out_ctrl, knots[0].anchor);
        assert_eq!(knots[1].in_ctrl, knots[1].anchor);
    }

    #[test]
    fn near_coincident_anchors_dampen_their_shared_tangent() {
        let path = interpolate_points(&[[0.0, 0.0], [0.001, 0.0], [30.0, 20.0]]).unwrap();
        let knot = &path.subpaths[0].knots[1];
        assert!((knot.in_ctrl.x - knot.anchor.x).abs() < 0.01);
        assert!((knot.out_ctrl.x - knot.anchor.x).abs() < 0.01);
    }

    #[test]
    fn long_spans_keep_their_arc_while_short_turns_stay_tight() {
        // Geometry from a grid comparison with SAI: the long lower sweep should bow below its
        // anchors, while its following short turn must not produce a large loop to the right.
        let points = [[214.0, 142.0], [251.0, 604.0], [1142.0, 498.0], [1036.0, 139.0]];
        let path = interpolate_points_with_model(&points, None, InterpolationModel::Legacy).unwrap();
        let knots = &path.subpaths[0].knots;
        let lower = &knots[1];
        let right = &knots[2];
        assert!(lower.out_ctrl.y > lower.anchor.y + 65.0, "the long sweep should leave downward");
        assert!(right.in_ctrl.y > right.anchor.y + 85.0, "the long sweep should arrive from below");
        assert!(right.out_ctrl.x < right.anchor.x + 60.0, "the short return should not make a wide loop");
        assert!(right.anchor.y - right.out_ctrl.y > right.out_ctrl.x - right.anchor.x, "the return should leave mostly upward");
        assert!(!right.smooth, "the compact return should retain its sharper bend");
        assert!(knots[3].in_ctrl.y < knots[3].anchor.y + 100.0, "the endpoint should keep curving toward the tip");
        assert!(lower.out_ctrl.y - lower.anchor.y > lower.anchor.y - lower.in_ctrl.y);
    }

    #[test]
    fn long_final_diagonal_does_not_inherit_short_return_endpoint_bend() {
        // A thin SAI comparison exposes the previous endpoint rule: after a horizontal span,
        // it made this longer final diagonal dip well below SAI at its midpoint.
        let points = [[1117.0, 483.0], [573.0, 483.0], [1115.0, 750.0]];
        let path = interpolate_points_with_model(&points, None, InterpolationModel::Legacy).unwrap();
        let knots = &path.subpaths[0].knots;
        let start = &knots[1];
        let end = &knots[2];
        let midpoint_y = (start.anchor.y + 3.0 * start.out_ctrl.y + 3.0 * end.in_ctrl.y + end.anchor.y) / 8.0;
        assert!(midpoint_y < 615.0, "the long final diagonal should not dip below its SAI reference: {midpoint_y}");
    }

    #[test]
    fn nearly_equal_neighbor_lengths_change_the_previous_arc_continuously() {
        fn previous_midpoint_y(last_y: f64) -> f64 {
            let path = interpolate_points_with_model(&[[0.0, 0.0], [320.0, 0.0], [320.0, last_y]], None, InterpolationModel::Legacy).unwrap();
            let knots = &path.subpaths[0].knots;
            (knots[0].anchor.y + 3.0 * knots[0].out_ctrl.y + 3.0 * knots[1].in_ctrl.y + knots[1].anchor.y) / 8.0
        }
        let just_shorter = previous_midpoint_y(319.9);
        let equal = previous_midpoint_y(320.0);
        let just_longer = previous_midpoint_y(320.1);
        assert!((just_shorter - equal).abs() < 1.0, "the incoming handle jumped at equal lengths");
        assert!((equal - just_longer).abs() < 1.0, "the incoming handle jumped at equal lengths");
    }

    #[test]
    fn final_arc_is_continuous_when_segment_lengths_cross() {
        let previous = [[0.0, 0.0], [100.0, 0.0]];
        let at = |length: f64| [previous[0], previous[1], [100.0 + length * 0.5, length * 0.8660254037844386]];
        for threshold in [75.0, 100.0, 200.0, 300.0] {
            let distance = sampled_path_distance(&at(threshold - 0.001), &at(threshold + 0.001));
            assert!(distance < 0.01, "endpoint changed abruptly near length {threshold}: {distance}");
        }
    }

    #[test]
    fn three_point_l_arcs_bend_outward_at_both_free_ends() {
        let equal = interpolate_points_with_model(&[[469.0, 172.0], [776.0, 172.0], [772.0, 479.0]], None, InterpolationModel::Legacy).unwrap();
        let knots = &equal.subpaths[0].knots;
        assert!(knots[0].out_ctrl.y < knots[0].anchor.y - 15.0, "the start should rise into the outer arc");
        assert!(knots[2].in_ctrl.x > knots[2].anchor.x + 15.0, "the end should arrive from the outside");

        let shorter_return = interpolate_points_with_model(&[[9.0, 172.0], [355.0, 172.0], [355.0, 440.0]], None, InterpolationModel::Legacy).unwrap();
        let start = &shorter_return.subpaths[0].knots[0];
        assert!(start.out_ctrl.y > start.anchor.y - 3.0, "the shorter return should retain its established start");
    }

    #[test]
    fn small_point_moves_do_not_jump_across_extreme_lengths_and_angles() {
        let lengths = [0.0, 1e-9, 0.0001, 0.01, 1.0, 40.0, 50.0, 60.0, 80.0, 99.999, 100.0, 100.001, 120.0, 1000.0, 100_000.0];
        for length in lengths {
            for degrees in 0..360 {
                let angle = f64::from(degrees).to_radians();
                let last = [100.0 + length * angle.cos(), length * angle.sin()];
                let before = [[0.0, 0.0], [100.0, 0.0], last];
                for delta in [[0.001, 0.0], [0.0, 0.001]] {
                    let after = [before[0], before[1], [last[0] + delta[0], last[1] + delta[1]]];
                    let distance = sampled_path_distance(&before, &after);
                    assert!(distance < 0.01, "curve jumped {distance:.3}px for length {length}, angle {degrees}°, move {delta:?}");
                }
            }
        }
    }

    #[test]
    fn moving_interior_points_stays_continuous_near_cusps_and_length_crossings() {
        let lengths = [0.0, 1e-9, 0.0001, 0.01, 1.0, 50.0, 60.0, 99.999, 100.0, 100.001, 1000.0, 100_000.0];
        for previous_length in lengths {
            for next_length in lengths {
                for degrees in (0..360).step_by(5) {
                    let angle = f64::from(degrees).to_radians();
                    let next = [next_length * angle.cos(), next_length * angle.sin()];
                    let before = [[-previous_length, 0.0], [0.0, 0.0], next, [next[0] + 100.0, next[1] + 30.0]];
                    for moved in [1, 2] {
                        let mut after = before;
                        after[moved][0] += 0.001;
                        after[moved][1] -= 0.001;
                        let distance = sampled_path_distance(&before, &after);
                        assert!(distance < 0.01, "curve jumped {distance:.3}px for lengths {previous_length}/{next_length}, angle {degrees}°, point {moved}");
                    }
                }
            }
        }
    }

    #[test]
    fn live_handle_changes_the_last_tangent_and_rounding() {
        let straight = interpolate_points_with_model(&[[0.0, 0.0], [20.0, 20.0], [40.0, 0.0]], None, InterpolationModel::Legacy).unwrap();
        let rounded = interpolate_points_with_handle(&[[0.0, 0.0], [20.0, 20.0], [40.0, 0.0]], Some([60.0, 20.0])).unwrap();
        assert_ne!(straight.subpaths[0].knots[2].in_ctrl, rounded.subpaths[0].knots[2].in_ctrl);
    }

    #[test]
    fn two_point_handle_controls_the_whole_arc() {
        let path = interpolate_points_with_model(&[[0.0, 0.0], [40.0, 0.0]], Some([20.0, 30.0]), InterpolationModel::Legacy).unwrap();
        let first = path.subpaths[0].knots[0].out_ctrl;
        let second = path.subpaths[0].knots[1].in_ctrl;
        assert!((first.x - 40.0 / 3.0).abs() < 1e-9 && (first.y - 20.0).abs() < 1e-9);
        assert!((second.x - 80.0 / 3.0).abs() < 1e-9 && (second.y - 20.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_point_lists_are_rejected() {
        assert!(interpolate_points(&[[0.0, 0.0]]).is_err());
        assert!(interpolate_points(&[[0.0, f64::NAN], [1.0, 1.0]]).is_err());
        assert!(interpolate_points(&[[0.0, 0.0], [1_000_001.0, 1.0]]).is_err());
    }

    #[test]
    fn default_model_is_sai_simplified_and_legacy_remains_selectable() {
        let points = [[0.0, 0.0], [120.0, -30.0], [170.0, 90.0], [400.0, 40.0]];
        assert_eq!(
            interpolate_points_with_handle(&points, None).unwrap(),
            interpolate_points_with_model(&points, None, InterpolationModel::SaiSimplified).unwrap()
        );
        assert!(interpolate_points_with_model(&points, None, InterpolationModel::Legacy).is_ok());
    }

    #[test]
    fn experimental_models_interpolate_points_and_are_reversible() {
        let points = [[0.0, 0.0], [37.0, 0.0], [37.0, 83.0], [190.0, 97.0], [230.0, 12.0]];
        for model in [InterpolationModel::SaiSimplified, InterpolationModel::SaiOptimized] {
            let path = interpolate_points_with_model(&points, None, model).unwrap();
            let reversed_points: Vec<_> = points.iter().rev().copied().collect();
            let reversed = interpolate_points_with_model(&reversed_points, None, model).unwrap();
            let segments = path.subpaths[0].segments();
            let reverse_segments = reversed.subpaths[0].segments();
            assert_eq!(segments.len(), points.len() - 1);
            for (i, point) in points.iter().enumerate() {
                let knot = &path.subpaths[0].knots[i];
                assert_eq!([knot.anchor.x, knot.anchor.y], *point);
            }
            for (forward, backward) in segments.iter().zip(reverse_segments.iter().rev()) {
                for step in 0..=20 {
                    let t = step as f64 / 20.0;
                    let a = eval(forward, t);
                    let b = eval(backward, 1.0 - t);
                    assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-8, "{model:?} should reverse without changing geometry");
                }
            }
        }
        let simplified = interpolate_points_with_model(&points, None, InterpolationModel::SaiSimplified).unwrap();
        let optimized = interpolate_points_with_model(&points, None, InterpolationModel::SaiOptimized).unwrap();
        let legacy = interpolate_points_with_model(&points, None, InterpolationModel::Legacy).unwrap();
        assert_ne!(simplified, optimized, "the two experiment parameter sets should remain independently selectable");
        assert_ne!(legacy, optimized, "the experimental model should differ from the production baseline on this geometry");
        for path in [&simplified, &optimized] {
            let knots = &path.subpaths[0].knots;
            assert_eq!(knots[0].anchor, knots[0].out_ctrl, "SAI experimental start tangent is zero");
            assert_eq!(knots[knots.len() - 1].anchor, knots[knots.len() - 1].in_ctrl, "SAI experimental end tangent is zero");
        }
    }

    #[test]
    fn experimental_tangents_are_directionally_continuous_and_local() {
        let points = [[0.0, 0.0], [31.0, 4.0], [60.0, 35.0], [104.0, 40.0], [135.0, 5.0], [170.0, 0.0], [220.0, 30.0]];
        let moved = [[0.0, 0.0], [31.0, 4.0], [63.0, 39.0], [104.0, 40.0], [135.0, 5.0], [170.0, 0.0], [220.0, 30.0]];
        for model in [InterpolationModel::SaiSimplified, InterpolationModel::SaiOptimized] {
            let base = interpolate_points_with_model(&points, None, model).unwrap();
            let altered = interpolate_points_with_model(&moved, None, model).unwrap();
            let a = base.subpaths[0].segments();
            let b = altered.subpaths[0].segments();
            // Moving P2 can affect segments 0..3; segments four and beyond must stay identical.
            assert_eq!(a[4..], b[4..]);
            for i in 1..points.len() - 1 {
                let left = a[i - 1];
                let right = a[i];
                let incoming = [left[3].x - left[2].x, left[3].y - left[2].y];
                let outgoing = [right[1].x - right[0].x, right[1].y - right[0].y];
                let cross = incoming[0] * outgoing[1] - incoming[1] * outgoing[0];
                let dot = incoming[0] * outgoing[0] + incoming[1] * outgoing[1];
                assert!(cross.abs() < 1e-8 && dot >= 0.0, "tangent direction breaks at anchor {i}");
            }
        }
    }

    #[test]
    fn experimental_models_remain_finite_for_short_and_coincident_segments() {
        let cases =
            [vec![[0.0, 0.0], [1e-10, 0.0], [1.0, 0.0]], vec![[0.0, 0.0], [0.0, 0.0], [1.0, 1.0]], vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0]]];
        for points in cases {
            for model in [InterpolationModel::SaiSimplified, InterpolationModel::SaiOptimized] {
                let path = interpolate_points_with_model(&points, None, model).unwrap();
                assert!(
                    path.subpaths[0]
                        .knots
                        .iter()
                        .all(|k| [k.anchor.x, k.anchor.y, k.in_ctrl.x, k.in_ctrl.y, k.out_ctrl.x, k.out_ctrl.y].iter().all(|v| v.is_finite()))
                );
            }
        }
        let straight = interpolate_points_with_model(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]], None, InterpolationModel::SaiOptimized).unwrap();
        for segment in straight.subpaths[0].segments() {
            let mut previous_x = segment[0].x;
            for step in 1..=100 {
                let point = eval(&segment, step as f64 / 100.0);
                assert!(point[0] >= previous_x - 1e-10, "unexpected loop or reversal on monotone collinear input");
                previous_x = point[0];
            }
        }
    }

    #[test]
    fn zero_pressure_keeps_the_anchor_and_nonzero_open_endpoint_gets_a_round_cap() {
        let shape = variable_width_shape_with_model(
            &[[10.0, 20.0], [90.0, 20.0]],
            &[0.0, 2.0],
            8.0,
            Color::rgba(0.0, 0.0, 0.0, 1.0),
            None,
            InterpolationModel::SaiSimplified,
        )
        .unwrap();
        let subpath = shape.path.subpaths.first().unwrap();
        assert!(subpath.knots.len() > 4, "outline should have sampled coverage geometry");
        assert_eq!(shape.path.subpaths.len(), 2, "the nonzero endpoint receives exactly one round cap");
        assert_eq!(subpath.knots[0].anchor, Point::new(10.0, 20.0));
        let cap = shape.path.subpaths.get(1).unwrap();
        let xs: Vec<_> = cap.knots.iter().map(|knot| knot.anchor.x).collect();
        assert!((xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 98.0).abs() < 0.1);
        assert!((xs.iter().copied().fold(f64::INFINITY, f64::min) - 82.0).abs() < 0.1);
    }

    #[test]
    fn thick_short_endpoint_return_merges_into_its_round_cap_without_a_spur() {
        use photocraft_color::PixelFormat;
        use photocraft_geom::Rect;

        let points = [[64.0, 43.0], [58.0, 50.0], [756.0, 120.0]];
        let canonical = interpolate_points_with_model(&points, None, InterpolationModel::SaiSimplified).unwrap();
        assert_eq!(canonical.subpaths[0].knots[1].anchor, Point::new(58.0, 50.0));
        let mut masks = Vec::new();
        for order in [points.to_vec(), points.into_iter().rev().collect()] {
            let shape = variable_width_shape_with_model(&order, &[1.0; 3], 16.0, Color::BLACK, None, InterpolationModel::SaiSimplified).unwrap();
            let raster = crate::render_shape(&shape, PixelFormat::RGBA8, Rect::new(0, 0, 800, 170));
            let spur_area = raster.read_region(Rect::new(48, 30, 56, 42));
            assert!(spur_area.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0.0), "short return must stay inside the round cap");
            let cap = raster.read_region(Rect::new(64, 43, 65, 44));
            assert!(cap[3] > 0.9);
            masks.push(raster.read_region(Rect::new(0, 0, 800, 170)));
        }
        let mean_alpha_difference =
            masks[0].as_chunks::<4>().0.iter().zip(masks[1].as_chunks::<4>().0.iter()).map(|(a, b)| (a[3] - b[3]).abs()).sum::<f32>() / (800 * 170) as f32;
        assert!(mean_alpha_difference < 0.002, "outline should remain reversible: {mean_alpha_difference}");
        assert!(variable_width_shape_with_model(&points, &[1.0, 0.5, 1.0], 16.0, Color::BLACK, None, InterpolationModel::SaiSimplified).is_ok());
    }

    #[test]
    fn coincident_endpoint_points_keep_a_visible_cap() {
        use photocraft_color::PixelFormat;
        use photocraft_geom::Rect;

        for points in [[[32.0, 32.0], [32.0, 32.0], [120.0, 50.0]], [[32.0, 32.0]; 3]] {
            let shape = variable_width_shape_with_model(&points, &[1.0; 3], 12.0, Color::BLACK, None, InterpolationModel::SaiSimplified).unwrap();
            let raster = crate::render_shape(&shape, PixelFormat::RGBA8, Rect::new(0, 0, 160, 80));
            assert!(raster.read_region(Rect::new(32, 32, 33, 33))[3] > 0.9);
        }
    }
}
