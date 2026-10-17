//! Geometry helpers for the SaiLine linework prototype.
//!
//! The document keeps canonical anchors and an optional live handle; this module derives the cubic
//! Bézier path consumed by the existing renderer so the engine and UI share one interpolation.

use photocraft_color::Color;
use photocraft_doc::{Fill, Knot, Path, PathOp, ShapeLayer, Subpath};
use photocraft_geom::Point;

const MAX_POINTS: usize = 4096;

/// Build an open cubic Bézier path that passes through every input point.
///
/// Interior tangents use the centred difference `(next - previous) / 2`; endpoints use a
/// one-sided difference. Bézier handles are one third of the tangent, which makes adjacent
/// segments C1-continuous at smooth interior anchors while preserving every anchor exactly.
pub fn interpolate_points(points: &[[f64; 2]]) -> Result<Path, String> {
    interpolate_points_with_handle(points, None)
}

/// Interpolate with an optional live handle at the last anchor. SaiLine uses the handle's
/// direction for the incoming tangent and its distance as the rounding amount.
pub fn interpolate_points_with_handle(points: &[[f64; 2]], handle: Option<[f64; 2]>) -> Result<Path, String> {
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
        let centered = if previous_coincident || next_coincident {
            [0.0, 0.0]
        } else {
            // Damp the centered tangent when adjacent segments have very different
            // lengths, so near-coincident anchors converge smoothly toward a cusp.
            let scale = previous_length.min(next_length) / previous_length.max(next_length);
            [(next[0] - previous[0]) * 0.5 * scale, (next[1] - previous[1]) * 0.5 * scale]
        };
        let in_tangent = if i == 0 || previous_coincident {
            [0.0, 0.0]
        } else if i + 1 == n {
            if handle.is_none() { [point[0] - previous[0], point[1] - previous[1]] } else { [0.0, 0.0] }
        } else {
            centered
        };
        let out_tangent = if i + 1 == n || next_coincident {
            [0.0, 0.0]
        } else if i == 0 {
            [next[0] - point[0], next[1] - point[1]]
        } else {
            centered
        };
        let anchor = Point::new(point[0], point[1]);
        let in_ctrl = Point::new(point[0] - in_tangent[0] / 3.0, point[1] - in_tangent[1] / 3.0);
        let out_ctrl = Point::new(point[0] + out_tangent[0] / 3.0, point[1] + out_tangent[1] / 3.0);
        knots.push(Knot { anchor, in_ctrl, out_ctrl, smooth: i > 0 && i + 1 < n });
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
    Ok(Path::new(vec![Subpath { closed: false, knots, op: PathOp::Combine }]))
}

/// Build a filled outline with linearly interpolated width factors at the logical anchors.
pub fn variable_width_shape(points: &[[f64; 2]], factors: &[f32], width: f64, color: Color, handle: Option<[f64; 2]>) -> Result<ShapeLayer, String> {
    if factors.len() != points.len() {
        return Err("linework width factors must match point count".into());
    }
    if !width.is_finite() || width <= 0.0 {
        return Err("linework width must be positive and finite".into());
    }
    if factors.iter().any(|factor| !factor.is_finite() || *factor < 0.0) {
        return Err("linework width factors must be finite and non-negative".into());
    }
    let center = interpolate_points_with_handle(points, handle)?;
    let segments = center.subpaths.first().ok_or("linework path has no subpath")?.segments();
    let mut samples = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let segment_length = (points[index + 1][0] - points[index][0]).hypot(points[index + 1][1] - points[index][1]);
        let minimum_radius = (width * 0.25).max(0.25);
        let sample_count = ((segment_length / minimum_radius).ceil() as usize + 1).clamp(64, 512);
        let steps = if index + 1 == segments.len() { sample_count } else { sample_count - 1 };
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
    let mut subpaths = vec![Subpath::polygon(&left).with_op(PathOp::Combine)];
    for (index, point) in points.iter().enumerate() {
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
        let path = interpolate_points(&[[0.0, 0.0], [12.0, 6.0]]).unwrap();
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
    fn live_handle_changes_the_last_tangent_and_rounding() {
        let straight = interpolate_points(&[[0.0, 0.0], [20.0, 20.0], [40.0, 0.0]]).unwrap();
        let rounded = interpolate_points_with_handle(&[[0.0, 0.0], [20.0, 20.0], [40.0, 0.0]], Some([60.0, 20.0])).unwrap();
        assert_ne!(straight.subpaths[0].knots[2].in_ctrl, rounded.subpaths[0].knots[2].in_ctrl);
    }

    #[test]
    fn two_point_handle_controls_the_whole_arc() {
        let path = interpolate_points_with_handle(&[[0.0, 0.0], [40.0, 0.0]], Some([20.0, 30.0])).unwrap();
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
}
