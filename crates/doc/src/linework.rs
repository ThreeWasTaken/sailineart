//! Persistent, UI-independent data for SaiLine.
//!
//! The point list is the source of truth. Bézier handles, raster caches and screen-space
//! previews are derived by higher layers. Keeping this module in the document crate makes the
//! model usable by the engine, native format and a future tool plug-in without depending on egui.

use std::sync::atomic::{AtomicU64, Ordering};

use photocraft_color::Color;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

const MAX_PATHS: usize = 4096;
const MAX_POINTS: usize = 4096;
const COORDINATE_LIMIT: f64 = 1_000_000.0;

static NEXT_LINEWORK_ID: AtomicU64 = AtomicU64::new(1);

fn fresh_id() -> u64 {
    NEXT_LINEWORK_ID.fetch_add(1, Ordering::Relaxed)
}

/// A logical SaiLine point. Width is a multiplicative factor on the path's nominal width.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineworkPoint {
    pub id: u64,
    pub position: [f64; 2],
    #[serde(default = "default_width_factor")]
    pub width_factor: f32,
    /// A hard corner whose adjacent curve handles are suppressed.
    #[serde(default)]
    pub pointed: bool,
}

fn default_width_factor() -> f32 {
    1.0
}

impl LineworkPoint {
    pub fn new(position: [f64; 2]) -> Result<Self, String> {
        validate_position(position)?;
        Ok(Self { id: fresh_id(), position, width_factor: 1.0, pointed: false })
    }

    pub fn with_id(id: u64, position: [f64; 2], width_factor: f32) -> Result<Self, String> {
        validate_position(position)?;
        validate_width_factor(width_factor)?;
        if id == 0 {
            return Err("linework point id must be non-zero".into());
        }
        Ok(Self { id, position, width_factor, pointed: false })
    }
}

/// One open or closed logical SaiLine stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineworkPath {
    pub id: u64,
    pub points: Vec<LineworkPoint>,
    pub color: Color,
    pub width: f32,
    /// Optional SaiLine live handle at the last point, stored in document coordinates.
    #[serde(default)]
    pub handle: Option<[f64; 2]>,
    #[serde(default)]
    pub closed: bool,
}

impl LineworkPath {
    pub fn new(points: Vec<[f64; 2]>, color: Color, width: f32) -> Result<Self, String> {
        if points.len() < 2 {
            return Err("linework paths need at least two points".into());
        }
        if points.len() > MAX_POINTS {
            return Err(format!("linework paths accept at most {MAX_POINTS} points"));
        }
        let points = points.into_iter().map(LineworkPoint::new).collect::<Result<Vec<_>, _>>()?;
        validate_width(width)?;
        Ok(Self { id: fresh_id(), points, color, width, handle: None, closed: false })
    }

    /// Duplicate a path while assigning fresh ids to the path and all of its points.
    pub fn duplicate(&self) -> Self {
        let mut copy = self.clone();
        copy.id = fresh_id();
        for point in &mut copy.points {
            point.id = fresh_id();
        }
        copy
    }
}

/// A persistent SaiLine layer containing multiple independent strokes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LineworkLayer {
    pub paths: Vec<LineworkPath>,
    /// Optional rendered cache; the point list remains canonical.
    #[serde(skip)]
    pub cache: Option<Surface>,
    /// Transient raster per path. Kept out of the file and logical equality.
    #[serde(skip)]
    pub path_caches: std::collections::BTreeMap<u64, Surface>,
}

impl PartialEq for LineworkLayer {
    fn eq(&self, other: &Self) -> bool {
        self.paths == other.paths && self.cache == other.cache
    }
}

impl LineworkLayer {
    pub fn validate(&self) -> Result<(), String> {
        if self.paths.len() > MAX_PATHS {
            return Err(format!("linework layers accept at most {MAX_PATHS} paths"));
        }
        let mut path_ids = std::collections::HashSet::with_capacity(self.paths.len());
        let mut point_ids = std::collections::HashSet::new();
        for path in &self.paths {
            if path.id == 0 || !path_ids.insert(path.id) {
                return Err("linework path ids must be unique and non-zero".into());
            }
            if path.points.len() < 2 || path.points.len() > MAX_POINTS {
                return Err("linework paths must contain between 2 and 4096 points".into());
            }
            validate_width(path.width)?;
            for point in &path.points {
                validate_position(point.position)?;
                validate_width_factor(point.width_factor)?;
                if point.id == 0 || !point_ids.insert(point.id) {
                    return Err("linework point ids must be unique and non-zero".into());
                }
            }
        }
        Ok(())
    }

    pub fn add_path(&mut self, path: LineworkPath) -> Result<(), String> {
        if self.paths.len() >= MAX_PATHS {
            return Err(format!("linework layers accept at most {MAX_PATHS} paths"));
        }
        if self.paths.iter().any(|existing| existing.id == path.id) {
            return Err("linework path id already exists".into());
        }
        if path.points.iter().any(|point| self.paths.iter().flat_map(|p| &p.points).any(|other| other.id == point.id)) {
            return Err("linework point id already exists".into());
        }
        let mut candidate = self.clone();
        candidate.paths.push(path);
        candidate.validate()?;
        self.paths = candidate.paths;
        Ok(())
    }

    pub fn duplicate(&self) -> Self {
        Self { paths: self.paths.iter().map(LineworkPath::duplicate).collect(), cache: self.cache.clone(), path_caches: Default::default() }
    }
}

fn validate_position(position: [f64; 2]) -> Result<(), String> {
    if position.iter().any(|v| !v.is_finite() || v.abs() > COORDINATE_LIMIT) {
        return Err("linework point coordinates must be finite and within ±1000000 px".into());
    }
    Ok(())
}

fn validate_width(width: f32) -> Result<(), String> {
    if !width.is_finite() || width <= 0.0 || width > 1_000_000.0 {
        return Err("linework width must be finite, greater than 0 and at most 1000000".into());
    }
    Ok(())
}

fn validate_width_factor(width_factor: f32) -> Result<(), String> {
    if !width_factor.is_finite() || width_factor < 0.0 || width_factor > 1000.0 {
        return Err("linework width factors must be finite and between 0 and 1000".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color() -> Color {
        Color::rgba(0.2, 0.4, 0.8, 1.0)
    }

    #[test]
    fn paths_have_stable_unique_ids_and_duplicate_remaps_them() {
        let path = LineworkPath::new(vec![[0.0, 0.0], [10.0, 5.0]], color(), 3.0).unwrap();
        let copy = path.duplicate();
        assert_ne!(path.id, copy.id);
        assert_eq!(path.points.len(), copy.points.len());
        for (original, duplicate) in path.points.iter().zip(copy.points.iter()) {
            assert_ne!(original.id, duplicate.id);
            assert_eq!(original.position, duplicate.position);
        }
    }

    #[test]
    fn a_layer_accepts_multiple_paths_and_rejects_duplicate_ids() {
        let first = LineworkPath::new(vec![[0.0, 0.0], [10.0, 5.0]], color(), 3.0).unwrap();
        let second = first.duplicate();
        let mut layer = LineworkLayer::default();
        layer.add_path(first.clone()).unwrap();
        layer.add_path(second).unwrap();
        assert_eq!(layer.paths.len(), 2);
        assert!(layer.add_path(first).is_err());
        layer.validate().unwrap();
    }

    #[test]
    fn hostile_values_are_rejected() {
        assert!(LineworkPath::new(vec![[0.0, 0.0]], color(), 1.0).is_err());
        assert!(LineworkPoint::new([f64::NAN, 0.0]).is_err());
        assert!(LineworkPath::new(vec![[0.0, 0.0], [1.0, 1.0]], color(), 0.0).is_err());
        let point = LineworkPoint::with_id(1, [0.0, 0.0], f32::INFINITY);
        assert!(point.is_err());
    }

    #[test]
    fn serde_round_trip_preserves_logical_data() {
        let path = LineworkPath::new(vec![[0.0, 0.0], [10.0, 5.0], [20.0, 0.0]], color(), 4.0).unwrap();
        let layer = LineworkLayer { paths: vec![path], cache: None, path_caches: Default::default() };
        let json = serde_json::to_string(&layer).unwrap();
        let decoded: LineworkLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, layer);
        decoded.validate().unwrap();
    }

    #[test]
    fn old_point_records_without_a_width_factor_default_to_one_hundred_percent() {
        let point: LineworkPoint = serde_json::from_value(serde_json::json!({"id": 41, "position": [12.0, 18.0]})).unwrap();
        assert_eq!(point.width_factor, 1.0);
        assert_eq!(point.position, [12.0, 18.0]);
    }
}
