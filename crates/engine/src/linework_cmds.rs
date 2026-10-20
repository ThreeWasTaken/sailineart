//! Headless SaiLineArt linework commands.
//!
//! The command stores canonical point data in the dedicated document model. The interpolated path
//! remains a response aid for clients that want an immediate preview.

use photocraft_color::Color;
use photocraft_doc::{Layer, LayerContent, LineworkLayer, LineworkPath, LineworkPathCache, ShapeLayer};
use photocraft_raster::Surface;
use photocraft_vector as vector;
use serde_json::{Value, json};
use std::time::Instant;

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "linework.create";
const MOVE_CMD: &str = "linework.movePoint";
const MOVE_MANY_CMD: &str = "linework.movePoints";
const DELETE_CMD: &str = "linework.deletePoint";
const DELETE_MANY_CMD: &str = "linework.deletePoints";
const DELETE_SEGMENT_CMD: &str = "linework.deleteSegment";
const INSERT_CMD: &str = "linework.insertPoint";
const WIDTH_CMD: &str = "linework.setPointWidth";
const PATH_WIDTH_CMD: &str = "linework.setPathWidth";
const WIDTH_MANY_CMD: &str = "linework.setPointWidths";
const COLOR_CMD: &str = "linework.setPathColor";
const POINTED_CMD: &str = "linework.setPointPointed";
const COMBINE_CMD: &str = "linework.combinePaths";
const INTERPOLATION_CMD: &str = "linework.setInterpolation";
const MAX_POINTS: usize = 4096;

/// Stages of one Linework cache refresh, available only to the interactive latency trace.
/// `shape_build_ms` includes centerline interpolation and variable-width contour construction;
/// those are currently a single vector API call.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineworkRefreshTimings {
    pub snapshot_clone_ms: f64,
    pub shape_build_ms: f64,
    pub raster_ms: f64,
    pub path_cache_ms: f64,
    pub damage_bounds_ms: f64,
    pub composition_ms: f64,
    pub writeback_ms: f64,
    pub refresh_total_ms: f64,
}

thread_local! {
    static LAST_REFRESH_TIMINGS: std::cell::RefCell<Option<LineworkRefreshTimings>> = const { std::cell::RefCell::new(None) };
}

fn linework_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PHOTOCRAFT_STROKE_TRACE").is_some_and(|value| !value.is_empty()))
}

/// Take the cache timing produced by the most recent linework command on this thread.
pub fn take_last_refresh_timings() -> Option<LineworkRefreshTimings> {
    LAST_REFRESH_TIMINGS.with(|last| last.borrow_mut().take())
}

type PointTarget = (photocraft_doc::LayerId, u64, u64, Option<f32>);

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn points(p: &Value) -> Result<Vec<[f64; 2]>> {
    let values = p
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "`points` must be an array of [x, y] points".into() })?;
    if values.len() < 2 {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "at least two points are required".into() });
    }
    if values.len() > MAX_POINTS {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("at most {MAX_POINTS} points are supported") });
    }
    let mut out = Vec::with_capacity(values.len());
    for (i, value) in values.iter().enumerate() {
        let a = value.as_array().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: format!("point {i} must be [x, y]") })?;
        let x = a.first().and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: format!("point {i} needs a finite x") })?;
        let y = a.get(1).and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: format!("point {i} needs a finite y") })?;
        out.push([x, y]);
    }
    Ok(out)
}

fn solid_color(s: &Session, p: &Value) -> Result<Color> {
    if let Some(value) = p.get("color") {
        return crate::vector_cmds::color(value)
            .ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "`color` must be #rrggbb, #rrggbbaa or [r,g,b,a]".into() });
    }
    let fg = s.tools.foreground;
    Ok(Color::rgba(fg[0], fg[1], fg[2], fg[3]))
}

fn width(p: &Value) -> Result<f32> {
    let value = p.get("width").and_then(Value::as_f64).unwrap_or(3.0);
    if !value.is_finite() || !(0.0..=1_000_000.0).contains(&value) || value <= 0.0 {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "`width` must be finite, greater than 0 and at most 1000000".into() });
    }
    Ok(value as f32)
}

fn width_factors(p: &Value, count: usize) -> Result<Vec<f32>> {
    let Some(values) = p.get("widthFactors") else { return Ok(vec![1.0; count]) };
    let values = values.as_array().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "`widthFactors` must be an array".into() })?;
    if values.len() != count {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "`widthFactors` must match the point count".into() });
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let factor = value.as_f64().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: format!("width factor {index} must be a number") })?;
            if !factor.is_finite() || !(0.0..=1000.0).contains(&factor) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("width factor {index} must be between 0 and 1000") });
            }
            Ok(factor as f32)
        })
        .collect()
}

fn inserted_width_factor(p: &Value) -> Result<f32> {
    if let Some(value) = p.get("pressurePercent") {
        let percent = value.as_f64().ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`pressurePercent` must be a number".into() })?;
        if !percent.is_finite() || !(0.0..=200.0).contains(&percent) {
            return Err(EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`pressurePercent` must be between 0 and 200".into() });
        }
        return Ok((percent / 100.0) as f32);
    }
    let Some(value) = p.get("factor") else { return Ok(1.0) };
    let factor = value.as_f64().ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`factor` must be a number".into() })?;
    if !factor.is_finite() || !(0.0..=2.0).contains(&factor) {
        return Err(EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`factor` must be between 0 and 2 (0–200%)".into() });
    }
    Ok(factor as f32)
}

fn handle(p: &Value) -> Result<Option<[f64; 2]>> {
    let Some(value) = p.get("handle") else { return Ok(None) };
    let a = value.as_array().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "`handle` must be [x, y]".into() })?;
    let x = a.first().and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "handle needs a finite x".into() })?;
    let y = a.get(1).and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "handle needs a finite y".into() })?;
    if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "handle must be finite and within ±1000000 px".into() });
    }
    Ok(Some([x, y]))
}

fn interpolation(p: &Value, cmd: &str) -> Result<vector::linework::InterpolationModel> {
    match p.get("interpolation").and_then(Value::as_str).unwrap_or("saiSimplified") {
        "legacy" => Ok(vector::linework::InterpolationModel::Legacy),
        "saiSimplified" => Ok(vector::linework::InterpolationModel::SaiSimplified),
        "saiOptimized" => Ok(vector::linework::InterpolationModel::SaiOptimized),
        value => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unknown interpolation model `{value}`") }),
    }
}

fn render_shape(
    points: &[[f64; 2]],
    factors: &[f32],
    color: Color,
    width: f32,
    handle: Option<[f64; 2]>,
    model: vector::linework::InterpolationModel,
    pointed: &[bool],
    closed: bool,
) -> Result<ShapeLayer> {
    vector::linework::variable_width_shape_with_model_and_pointed_closed(points, factors, f64::from(width), color, handle, model, pointed, closed).map_err(EngineError::Other)
}

fn composite_cache(rect: photocraft_geom::Rect, base: Option<Surface>, overlay: &LineworkPathCache) -> Surface {
    let Some(mut result) = base else { return overlay.to_surface() };
    if result.channels() != 4 || overlay.format.channels() != 4 {
        return overlay.to_surface();
    }
    let rect = rect.intersect(&overlay.bounds);
    if rect.is_empty() {
        return result;
    }
    let mut dst = result.read_region(rect);
    let source_width = overlay.bounds.width() as usize;
    let rect_width = rect.width() as usize;
    for y in 0..rect.height() as usize {
        for x in 0..rect_width {
            let source_index = (((rect.y0 - overlay.bounds.y0) as usize + y) * source_width + (rect.x0 - overlay.bounds.x0) as usize + x) * 4;
            let target_index = (y * rect_width + x) * 4;
            let sa = overlay.pixels[source_index + 3].clamp(0.0, 1.0);
            let da = dst[target_index + 3].clamp(0.0, 1.0);
            let out_a = sa + da * (1.0 - sa);
            if out_a <= 1e-6 {
                dst[target_index..target_index + 4].fill(0.0);
                continue;
            }
            for channel in 0..3 {
                dst[target_index + channel] = (overlay.pixels[source_index + channel] * sa + dst[target_index + channel] * da * (1.0 - sa)) / out_a;
            }
            dst[target_index + 3] = out_a;
        }
    }
    result.write_region(rect, &dst);
    result
}

/// Bounds of pixels whose samples differ in two equally formatted surfaces, clipped to `limit`.
/// Reusing row buffers keeps this exact comparison bounded in memory even for long diagonal
/// strokes whose conservative old/new bounds cover a large rectangle.
fn surface_diff_bounds(a: &Surface, b: &Surface, limit: photocraft_geom::Rect) -> photocraft_geom::Rect {
    if limit.is_empty() {
        return photocraft_geom::Rect::EMPTY;
    }
    if a.format() != b.format() {
        return limit;
    }
    // Linework raster caches are transparent-zero Surfaces. Compare their encoded tile samples
    // directly so discovering a small dirty spot inside a large diagonal AABB does not itself
    // allocate and convert millions of floats. Non-zero-default surfaces use the general path.
    if a.default_pixel().iter().all(|sample| *sample == 0.0) && b.default_pixel().iter().all(|sample| *sample == 0.0) {
        let bytes_per_pixel = a.format().bytes_per_pixel();
        let zero = vec![0u8; bytes_per_pixel];
        let (mut x0, mut y0, mut x1, mut y1) = (limit.x1, limit.y1, limit.x0, limit.y0);
        let mut different = false;
        for coord in limit.tiles() {
            let rect = coord.rect().intersect(&limit);
            let left = a.tile(coord).map(|tile| tile.bytes());
            let right = b.tile(coord).map(|tile| tile.bytes());
            if a.tile(coord).zip(b.tile(coord)).is_some_and(|(l, r)| std::sync::Arc::ptr_eq(l, r)) {
                continue;
            }
            for y in rect.y0..rect.y1 {
                let row_offset = ((y - coord.ty * photocraft_geom::TILE_SIZE) * photocraft_geom::TILE_SIZE
                    + (rect.x0 - coord.tx * photocraft_geom::TILE_SIZE)) as usize
                    * bytes_per_pixel;
                for x in rect.x0..rect.x1 {
                    let offset = row_offset + (x - rect.x0) as usize * bytes_per_pixel;
                    let l = left.map_or(&zero[..bytes_per_pixel], |bytes| &bytes[offset..offset + bytes_per_pixel]);
                    let r = right.map_or(&zero[..bytes_per_pixel], |bytes| &bytes[offset..offset + bytes_per_pixel]);
                    if l != r {
                        x0 = x0.min(x);
                        x1 = x1.max(x + 1);
                        y0 = y0.min(y);
                        y1 = y1.max(y + 1);
                        different = true;
                    }
                }
            }
        }
        return if different { photocraft_geom::Rect::new(x0, y0, x1, y1) } else { photocraft_geom::Rect::EMPTY };
    }
    let channels = a.channels();
    let mut a_row = Vec::new();
    let mut b_row = Vec::new();
    let (mut x0, mut y0, mut x1, mut y1) = (limit.x1, limit.y1, limit.x0, limit.y0);
    let mut different = false;
    for y in limit.y0..limit.y1 {
        let row = photocraft_geom::Rect::new(limit.x0, y, limit.x1, y + 1);
        a.read_region_into(row, &mut a_row);
        b.read_region_into(row, &mut b_row);
        for (x, (left, right)) in a_row.chunks_exact(channels).zip(b_row.chunks_exact(channels)).enumerate() {
            if left != right {
                let x = limit.x0 + x as i32;
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
                different = true;
            }
        }
    }
    if different { photocraft_geom::Rect::new(x0, y0, x1, y1) } else { photocraft_geom::Rect::EMPTY }
}

/// Exact cache-compositor entry point used only by the headless SaiLineArt performance harness.
#[cfg(feature = "sailine-bench")]
pub fn bench_composite_cache(rect: photocraft_geom::Rect, base: Option<Surface>, overlay: &LineworkPathCache) -> Surface {
    composite_cache(rect, base, overlay)
}

fn refresh_linework_cache(
    doc: &mut photocraft_doc::Document,
    layer_id: photocraft_doc::LayerId,
    changed_paths: &[u64],
    model: vector::linework::InterpolationModel,
) -> Result<photocraft_geom::Rect> {
    let trace = linework_trace_enabled();
    let total_started = trace.then(Instant::now);
    let snapshot_started = trace.then(Instant::now);
    let (paths, mut path_caches, previous_cache) = {
        let layer = doc.layer(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
        let LayerContent::Linework(linework) = &layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
        (linework.paths.clone(), linework.path_caches.clone(), linework.cache.clone())
    };
    let mut timings = LineworkRefreshTimings::default();
    if let Some(started) = snapshot_started {
        timings.snapshot_clone_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
    let bounds = doc.bounds();
    let format = doc.pixel_format();
    let changed: std::collections::BTreeSet<_> = changed_paths.iter().copied().collect();
    let valid: std::collections::BTreeSet<_> = paths.iter().map(|path| path.id).collect();
    let can_patch = previous_cache.is_some() && paths.iter().all(|path| path_caches.contains_key(&path.id));

    // A one-path Linework layer is already the final layer image. Keep the compact per-path cache
    // for the existing snapshot/undo and cache-reuse behavior, but do not blend it back into a
    // second Surface: the former is an exact round trip followed by a needless composition.
    // Multi-path layers keep the compositor below to preserve overlap order and alpha blending.
    if paths.len() == 1 {
        let path = &paths[0];
        if !changed.contains(&path.id) && previous_cache.is_some() {
            return Ok(photocraft_geom::Rect::EMPTY);
        }
        let points: Vec<[f64; 2]> = path.points.iter().map(|point| point.position).collect();
        let factors: Vec<f32> = path.points.iter().map(|point| point.width_factor).collect();
        let pointed: Vec<bool> = path.points.iter().map(|point| point.pointed).collect();
        let shape_started = trace.then(Instant::now);
        let mut shape = render_shape(&points, &factors, path.color, path.width, path.handle, model, &pointed, path.closed)?;
        if let Some(started) = shape_started {
            timings.shape_build_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let raster_started = trace.then(Instant::now);
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let Some(started) = raster_started {
            timings.raster_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let cache = shape.cache.ok_or(EngineError::Other("linework render cache missing".into()))?;
        let path_cache_started = trace.then(Instant::now);
        let path_cache = LineworkPathCache::from_surface(&cache);
        if let Some(started) = path_cache_started {
            timings.path_cache_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let damage_started = trace.then(Instant::now);
        let damage = if let Some(previous) = &previous_cache {
            let old_bounds = path_caches.get(&path.id).map_or_else(|| previous.content_bounds(), |old| old.bounds);
            let new_bounds = path_cache.bounds;
            let limit = bounds.intersect(&old_bounds.union(&new_bounds));
            surface_diff_bounds(previous, &cache, limit)
        } else {
            bounds.intersect(&cache.content_bounds())
        };
        if let Some(started) = damage_started {
            timings.damage_bounds_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        let writeback_started = trace.then(Instant::now);
        let layer = doc.layer_mut(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
        let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
        linework.cache = Some(cache);
        path_caches.clear();
        path_caches.insert(path.id, path_cache);
        linework.path_caches = path_caches;
        if let Some(started) = writeback_started {
            timings.writeback_ms = started.elapsed().as_secs_f64() * 1000.0;
            timings.refresh_total_ms = total_started.map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0);
            LAST_REFRESH_TIMINGS.with(|last| *last.borrow_mut() = Some(timings));
        }
        return Ok(damage);
    }

    let mut damage = photocraft_geom::Rect::EMPTY;
    if can_patch {
        for id in changed_paths {
            if let Some(old) = path_caches.get(id) {
                damage = damage.union(&old.bounds);
            }
        }
    }
    path_caches.retain(|id, _| valid.contains(id));
    for path in &paths {
        if path_caches.contains_key(&path.id) && !changed.contains(&path.id) {
            continue;
        }
        let points: Vec<[f64; 2]> = path.points.iter().map(|point| point.position).collect();
        let factors: Vec<f32> = path.points.iter().map(|point| point.width_factor).collect();
        let pointed: Vec<bool> = path.points.iter().map(|point| point.pointed).collect();
        let shape_started = trace.then(Instant::now);
        let mut shape = render_shape(&points, &factors, path.color, path.width, path.handle, model, &pointed, path.closed)?;
        if let Some(started) = shape_started {
            timings.shape_build_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        let raster_started = trace.then(Instant::now);
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let Some(started) = raster_started {
            timings.raster_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        let rendered_surface = shape.cache.ok_or(EngineError::Other("linework render cache missing".into()))?;
        let path_cache_started = trace.then(Instant::now);
        let rendered = LineworkPathCache::from_surface(&rendered_surface);
        if let Some(started) = path_cache_started {
            timings.path_cache_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        if can_patch && changed.contains(&path.id) {
            damage = damage.union(&rendered.bounds);
        }
        path_caches.insert(path.id, rendered);
    }
    let region = if can_patch { bounds.intersect(&damage) } else { bounds };
    let composition_started = trace.then(Instant::now);
    let mut cache = None;
    if !region.is_empty() {
        for path in paths {
            let rendered = path_caches.get(&path.id).ok_or(EngineError::Other("linework path render cache missing".into()))?;
            // Clipping every blend to the affected pixel bounds preserves the rest of the layer cache.
            cache = Some(composite_cache(region, cache, rendered));
        }
    }
    let cache = if let (true, Some(mut previous)) = (can_patch, previous_cache) {
        if !region.is_empty() {
            let pixels = cache.unwrap_or_else(|| Surface::new(format)).read_region(region);
            previous.write_region(region, &pixels);
        }
        Some(previous)
    } else {
        cache
    };
    if let Some(started) = composition_started {
        timings.composition_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
    let writeback_started = trace.then(Instant::now);
    let layer = doc.layer_mut(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
    let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
    linework.cache = cache;
    linework.path_caches = path_caches;
    if let Some(started) = writeback_started {
        timings.writeback_ms = started.elapsed().as_secs_f64() * 1000.0;
        timings.refresh_total_ms = total_started.map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0);
        LAST_REFRESH_TIMINGS.with(|last| *last.borrow_mut() = Some(timings));
    }
    Ok(region)
}

/// Rebuild the transient Linework render cache before operations that need its pixel output.
/// The canonical point data stays untouched; a missing cache (for example after loading a
/// document) is rendered with SaiLineArt's production interpolation model.
pub(crate) fn ensure_rendered(doc: &mut photocraft_doc::Document, layer_id: photocraft_doc::LayerId) -> Result<()> {
    let path_ids = {
        let layer = doc.layer(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
        let LayerContent::Linework(linework) = &layer.content else { return Ok(()) };
        if linework.cache.is_some() {
            return Ok(());
        }
        linework.paths.iter().map(|path| path.id).collect::<Vec<_>>()
    };
    refresh_linework_cache(doc, layer_id, &path_ids, vector::linework::InterpolationModel::SaiSimplified)?;
    Ok(())
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, CMD)?;
    let points = points(p)?;
    let live_handle = handle(p)?;
    let pointed = vec![p.get("straight").and_then(Value::as_bool).unwrap_or(false); points.len()];
    let path = vector::linework::interpolate_points_with_model_and_pointed(&points, live_handle, model, &pointed)
        .map_err(|message| EngineError::BadParams { cmd: CMD.into(), msg: message })?;
    let color = solid_color(s, p)?;
    let stroke_width = width(p)?;
    let factors = width_factors(p, points.len())?;
    let mut line = LineworkPath::new(points.clone(), color, stroke_width).map_err(|message| EngineError::BadParams { cmd: CMD.into(), msg: message })?;
    for (point, factor) in line.points.iter_mut().zip(factors.iter().copied()) {
        point.width_factor = factor;
    }
    for (point, value) in line.points.iter_mut().zip(pointed.iter().copied()) {
        point.pointed = value;
    }
    line.handle = live_handle;
    let mut shape = render_shape(&points, &factors, color, stroke_width, live_handle, model, &pointed, false)
        .map_err(|error| EngineError::BadParams { cmd: CMD.into(), msg: error.to_string() })?;
    let requested_name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit("New Linework Path", |doc, active| {
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        let name = requested_name.unwrap_or_else(|| doc.next_layer_name("Linework"));
        let rendered = shape.cache.take().ok_or(EngineError::Other("linework render cache missing".into()))?;
        let path_cache = LineworkPathCache::from_surface(&rendered);
        let bounds = doc.bounds();
        if let Some(active_id) = *active
            && let Some(layer) = doc.layer_mut(active_id)
            && let LayerContent::Linework(linework) = &mut layer.content
        {
            linework.path_caches.insert(line.id, path_cache.clone());
            linework.cache = Some(composite_cache(bounds, linework.cache.take(), &path_cache));
            linework.paths.push(line);
            return Ok(active_id);
        }
        let path_caches = std::iter::once((line.id, path_cache)).collect();
        let id = doc.insert_above(
            *active,
            Layer::new(name, LayerContent::Linework(LineworkLayer { paths: vec![line], cache: Some(rendered), path_caches })),
        );
        *active = Some(id);
        Ok(id)
    })?;
    let rgb = color.to_rgb();
    Ok(json!({
        "layer": id.0,
        "kind": "linework",
        "points": points,
        "path": crate::vector_cmds::path_json(&path),
        "width": stroke_width,
        "color": [rgb[0], rgb[1], rgb[2], color.alpha],
    }))
}

fn move_point(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, MOVE_CMD)?;
    let layer = p
        .get("layer")
        .and_then(Value::as_u64)
        .map(photocraft_doc::LayerId)
        .ok_or_else(|| EngineError::BadParams { cmd: MOVE_CMD.into(), msg: "`layer` is required".into() })?;
    let path_id = p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: MOVE_CMD.into(), msg: "`path` is required".into() })?;
    let point_id = p.get("point").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: MOVE_CMD.into(), msg: "`point` is required".into() })?;
    let x = p.get("x").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: MOVE_CMD.into(), msg: "`x` is required".into() })?;
    let y = p.get("y").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: MOVE_CMD.into(), msg: "`y` is required".into() })?;
    if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
        return Err(EngineError::BadParams { cmd: MOVE_CMD.into(), msg: "point coordinates must be finite and within ±1000000 px".into() });
    }
    let damage = s.edit("Move Linework Point", |doc, _| {
        {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.position = [x, y];
        };
        refresh_linework_cache(doc, layer, &[path_id], model)
    })?;
    if let Some(state) = s.active_mut() {
        state.last_damage = Some(damage);
    }
    Ok(json!({"layer": layer.0, "path": path_id, "point": point_id, "x": x, "y": y}))
}

fn move_points(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, MOVE_MANY_CMD)?;
    let values = p
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "`points` must be an array".into() })?;
    if values.is_empty() || values.len() > 4096 {
        return Err(EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "`points` must contain 1..4096 moves".into() });
    }
    let mut moves = Vec::with_capacity(values.len());
    for value in values {
        let layer = photocraft_doc::LayerId(
            value
                .get("layer")
                .and_then(Value::as_u64)
                .ok_or_else(|| EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "move `layer` is required".into() })?,
        );
        let path = value
            .get("path")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "move `path` is required".into() })?;
        let point = value
            .get("point")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "move `point` is required".into() })?;
        let x =
            value.get("x").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "move `x` is required".into() })?;
        let y =
            value.get("y").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "move `y` is required".into() })?;
        if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
            return Err(EngineError::BadParams { cmd: MOVE_MANY_CMD.into(), msg: "point coordinates must be finite and within ±1000000 px".into() });
        }
        moves.push((layer, path, point, x, y));
    }
    let damage = s.edit("Move Linework Points", |doc, _| {
        let mut layers: std::collections::BTreeMap<photocraft_doc::LayerId, Vec<u64>> = Default::default();
        for (layer_id, path_id, point_id, x, y) in &moves {
            let layer = doc.layer_mut(*layer_id).ok_or(EngineError::NoLayer(*layer_id))?;
            let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
            let path = linework.paths.iter_mut().find(|path| path.id == *path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == *point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.position = [*x, *y];
            let changed = layers.entry(*layer_id).or_default();
            if !changed.contains(path_id) {
                changed.push(*path_id);
            }
        }
        let mut damage = photocraft_geom::Rect::EMPTY;
        for (layer, paths) in layers {
            damage = damage.union(&refresh_linework_cache(doc, layer, &paths, model)?);
        }
        Ok(damage)
    })?;
    if let Some(state) = s.active_mut() {
        state.last_damage = Some(damage);
    }
    Ok(json!({"count": moves.len()}))
}

fn delete_point(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, DELETE_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: DELETE_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id = p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: DELETE_CMD.into(), msg: "`path` is required".into() })?;
    let point_id =
        p.get("point").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: DELETE_CMD.into(), msg: "`point` is required".into() })?;
    s.edit("Delete Linework Point", |doc, _| {
        {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            if path.points.len() <= 2 {
                return Err(EngineError::Other("a Linework path needs at least two points".into()));
            }
            let before = path.points.len();
            path.points.retain(|point| point.id != point_id);
            if path.points.len() == before {
                return Err(EngineError::Other("linework point not found".into()));
            }
        };
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok(())
    })?;
    Ok(json!({"layer": layer.0, "path": path_id, "point": point_id}))
}

fn point_targets(p: &Value, cmd: &str, with_factor: bool) -> Result<Vec<PointTarget>> {
    let values =
        p.get("points").and_then(Value::as_array).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "`points` must be an array".into() })?;
    if values.is_empty() || values.len() > MAX_POINTS {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "`points` must contain 1..4096 points".into() });
    }
    let mut targets = Vec::with_capacity(values.len());
    for value in values {
        let required = |key: &str| {
            value.get(key).and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("point `{key}` is required") })
        };
        let layer = photocraft_doc::LayerId(required("layer")?);
        let path = required("path")?;
        let point = required("point")?;
        let factor = if with_factor {
            let factor = value
                .get("factor")
                .and_then(Value::as_f64)
                .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "point `factor` is required".into() })?;
            if !factor.is_finite() || !(0.0..=2.0).contains(&factor) {
                return Err(EngineError::BadParams { cmd: cmd.into(), msg: "point `factor` must be between 0 and 2".into() });
            }
            Some(factor as f32)
        } else {
            None
        };
        if targets.iter().any(|(l, p, id, _)| (*l, *p, *id) == (layer, path, point)) {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: "duplicate point".into() });
        }
        targets.push((layer, path, point, factor));
    }
    Ok(targets)
}

fn delete_points(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, DELETE_MANY_CMD)?;
    let targets = point_targets(p, DELETE_MANY_CMD, false)?;
    s.edit("Delete Linework Points", |doc, _| {
        let mut changed: std::collections::BTreeMap<photocraft_doc::LayerId, Vec<u64>> = Default::default();
        // Validate every target before mutating anything, including across paths and layers.
        for (layer_id, path_id, point_id, _) in &targets {
            let layer = doc.layer(*layer_id).ok_or(EngineError::NoLayer(*layer_id))?;
            let LayerContent::Linework(linework) = &layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
            let path = linework.paths.iter().find(|path| path.id == *path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            if !path.points.iter().any(|point| point.id == *point_id) {
                return Err(EngineError::Other("linework point not found".into()));
            }
        }
        for (layer_id, path_id, point_id, _) in &targets {
            let layer = doc.layer_mut(*layer_id).ok_or(EngineError::NoLayer(*layer_id))?;
            let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
            let path = linework.paths.iter_mut().find(|path| path.id == *path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            path.points.retain(|point| point.id != *point_id);
            let ids = changed.entry(*layer_id).or_default();
            if !ids.contains(path_id) {
                ids.push(*path_id);
            }
        }
        for (layer_id, paths) in changed {
            if let LayerContent::Linework(linework) = &mut doc.layer_mut(layer_id).ok_or(EngineError::NoLayer(layer_id))?.content {
                linework.paths.retain(|path| path.points.len() >= 2);
            }
            refresh_linework_cache(doc, layer_id, &paths, model)?;
        }
        Ok(())
    })?;
    Ok(json!({"count": targets.len()}))
}

fn delete_segment(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, DELETE_SEGMENT_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: DELETE_SEGMENT_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id =
        p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: DELETE_SEGMENT_CMD.into(), msg: "`path` is required".into() })?;
    let segment = p
        .get("segment")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| EngineError::BadParams { cmd: DELETE_SEGMENT_CMD.into(), msg: "`segment` is required".into() })?;
    let result = s.edit("Delete Linework Segment", |doc, _| {
        let (new_path, removed_point, removed_path, moved_points) = {
            let LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path_index = linework.paths.iter().position(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let original = linework.paths[path_index].clone();
            if original.closed {
                return Err(EngineError::Other("segment deletion is currently supported on open Linework paths".into()));
            }
            if segment + 1 >= original.points.len() {
                return Err(EngineError::BadParams { cmd: DELETE_SEGMENT_CMD.into(), msg: "segment index is out of range".into() });
            }
            let count = original.points.len();
            if count == 2 {
                linework.paths.remove(path_index);
                (None, None, true, Vec::new())
            } else if segment == 0 || segment + 1 == count - 1 {
                let remove_index = if segment == 0 { 0 } else { count - 1 };
                let removed = original.points[remove_index].id;
                let path = &mut linework.paths[path_index];
                path.points.remove(remove_index);
                (None, Some(removed), false, Vec::new())
            } else {
                let mut tail = linework.paths[path_index].points.split_off(segment + 1);
                let moved: Vec<u64> = tail.iter().map(|point| point.id).collect();
                let mut second =
                    LineworkPath::new(tail.iter().map(|point| point.position).collect(), original.color, original.width).map_err(EngineError::Other)?;
                second.points = std::mem::take(&mut tail);
                second.handle = original.handle;
                second.closed = false;
                linework.paths[path_index].handle = None;
                let new_path = second.id;
                linework.paths.push(second);
                (Some(new_path), None, false, moved)
            }
        };
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok((new_path, removed_point, removed_path, moved_points))
    })?;
    let moved: Vec<Value> = result.3.iter().copied().map(Value::from).collect();
    Ok(json!({"layer":layer.0,"path":path_id,"newPath":result.0,"removedPoint":result.1,"removedPath":result.2,"movedPoints":moved}))
}

fn set_point_widths(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, WIDTH_MANY_CMD)?;
    let targets = point_targets(p, WIDTH_MANY_CMD, true)?;
    s.edit("Set Linework Point Widths", |doc, _| {
        let mut changed: std::collections::BTreeMap<photocraft_doc::LayerId, Vec<u64>> = Default::default();
        for (layer_id, path_id, point_id, _) in &targets {
            let layer = doc.layer(*layer_id).ok_or(EngineError::NoLayer(*layer_id))?;
            let LayerContent::Linework(linework) = &layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
            let path = linework.paths.iter().find(|path| path.id == *path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            if !path.points.iter().any(|point| point.id == *point_id) {
                return Err(EngineError::Other("linework point not found".into()));
            }
        }
        for (layer_id, path_id, point_id, factor) in &targets {
            let factor = factor.ok_or_else(|| EngineError::BadParams { cmd: WIDTH_MANY_CMD.into(), msg: "point `factor` is required".into() })?;
            let layer = doc.layer_mut(*layer_id).ok_or(EngineError::NoLayer(*layer_id))?;
            let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
            let path = linework.paths.iter_mut().find(|path| path.id == *path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == *point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.width_factor = factor;
            let ids = changed.entry(*layer_id).or_default();
            if !ids.contains(path_id) {
                ids.push(*path_id);
            }
        }
        for (layer, paths) in changed {
            refresh_linework_cache(doc, layer, &paths, model)?;
        }
        Ok(())
    })?;
    Ok(json!({"count": targets.len()}))
}

fn insert_point(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, INSERT_CMD)?;
    let width_factor = inserted_width_factor(p)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id = p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`path` is required".into() })?;
    let index = p
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`index` is required".into() })?;
    let x = p.get("x").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`x` is required".into() })?;
    let y = p.get("y").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: INSERT_CMD.into(), msg: "`y` is required".into() })?;
    s.edit("Insert Linework Point", |doc, _| {
        let id = {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            if index > path.points.len() {
                return Err(EngineError::Other("point insertion index out of range".into()));
            }
            let mut point = photocraft_doc::LineworkPoint::new([x, y]).map_err(EngineError::Other)?;
            point.width_factor = width_factor;
            point.pointed = index > 0 && index < path.points.len() && path.points[index - 1].pointed && path.points[index].pointed;
            let id = point.id;
            path.points.insert(index, point);
            id
        };
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok(id)
    })
    .map(|id| json!({"layer": layer.0, "path": path_id, "point": id, "x": x, "y": y}))
}

fn set_point_width(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, WIDTH_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: WIDTH_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id = p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: WIDTH_CMD.into(), msg: "`path` is required".into() })?;
    let point_id = p.get("point").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: WIDTH_CMD.into(), msg: "`point` is required".into() })?;
    let factor = p.get("factor").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: WIDTH_CMD.into(), msg: "`factor` is required".into() })?;
    if !factor.is_finite() || !(0.0..=1000.0).contains(&factor) {
        return Err(EngineError::BadParams { cmd: WIDTH_CMD.into(), msg: "`factor` must be between 0 and 1000".into() });
    }
    s.edit("Set Linework Point Width", |doc, _| {
        {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.width_factor = factor as f32;
        };
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok(())
    })?;
    Ok(json!({"layer": layer.0, "path": path_id, "point": point_id, "factor": factor}))
}

fn set_path_color(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, COLOR_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COLOR_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id = p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COLOR_CMD.into(), msg: "`path` is required".into() })?;
    let color = p
        .get("color")
        .and_then(crate::vector_cmds::color)
        .ok_or_else(|| EngineError::BadParams { cmd: COLOR_CMD.into(), msg: "`color` must be #rrggbb, #rrggbbaa or [r,g,b,a]".into() })?;
    s.edit("Set Linework Path Color", |doc, _| {
        {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            path.color = color;
        };
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok(())
    })?;
    let rgb = color.to_rgb();
    Ok(json!({"layer": layer.0, "path": path_id, "color": [rgb[0], rgb[1], rgb[2], color.alpha]}))
}

fn set_path_width(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, PATH_WIDTH_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: PATH_WIDTH_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id =
        p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: PATH_WIDTH_CMD.into(), msg: "`path` is required".into() })?;
    let width =
        p.get("width").and_then(Value::as_f64).ok_or_else(|| EngineError::BadParams { cmd: PATH_WIDTH_CMD.into(), msg: "`width` is required".into() })?;
    if !width.is_finite() || !(0.7..=30.0).contains(&width) {
        return Err(EngineError::BadParams { cmd: PATH_WIDTH_CMD.into(), msg: "`width` must be between 0.7 and 30 px".into() });
    }
    s.edit("Set Linework Path Width", |doc, _| {
        {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            path.width = width as f32;
        }
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok(())
    })?;
    Ok(json!({"layer": layer.0, "path": path_id, "width": width}))
}

fn set_point_pointed(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, POINTED_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: POINTED_CMD.into(), msg: "`layer` is required".into() })?,
    );
    let path_id = p.get("path").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: POINTED_CMD.into(), msg: "`path` is required".into() })?;
    let point_id =
        p.get("point").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: POINTED_CMD.into(), msg: "`point` is required".into() })?;
    s.edit("Set Linework Point Type", |doc, _| {
        let pointed = {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.pointed = p.get("pointed").and_then(Value::as_bool).unwrap_or(!point.pointed);
            point.pointed
        };
        refresh_linework_cache(doc, layer, &[path_id], model)?;
        Ok(pointed)
    })?;
    let pointed = s
        .active()
        .and_then(|st| st.doc.layer(layer))
        .and_then(|layer| match &layer.content {
            LayerContent::Linework(linework) => {
                linework.paths.iter().find(|path| path.id == path_id)?.points.iter().find(|point| point.id == point_id).map(|point| point.pointed)
            }
            _ => None,
        })
        .unwrap_or(false);
    Ok(json!({"layer": layer.0, "path": path_id, "point": point_id, "pointed": pointed}))
}

fn combine_paths(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, COMBINE_CMD)?;
    let layer = photocraft_doc::LayerId(
        p.get("layer").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "`layer` required".into() })?,
    );
    let path_a = p.get("pathA").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "`pathA` required".into() })?;
    let point_a = p.get("pointA").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "`pointA` required".into() })?;
    let path_b = p.get("pathB").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "`pathB` required".into() })?;
    let point_b = p.get("pointB").and_then(Value::as_u64).ok_or_else(|| EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "`pointB` required".into() })?;
    let self_close = path_a == path_b;
    s.edit("Combine Linework Paths", |doc, _| {
        {
            let LayerContent::Linework(lw) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let ia = lw.paths.iter().position(|p| p.id == path_a).ok_or_else(|| EngineError::Other("first path not found".into()))?;
            if self_close {
                let path = &mut lw.paths[ia];
                if path.closed {
                    return Err(EngineError::Other("path is already closed".into()));
                }
                let first = path.points.first().ok_or_else(|| EngineError::Other("linework path has no points".into()))?.id;
                let last = path.points.last().ok_or_else(|| EngineError::Other("linework path has no points".into()))?.id;
                if path.points.len() < 3 || !((point_a == first && point_b == last) || (point_a == last && point_b == first)) {
                    return Err(EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "closing a path requires its two distinct endpoints and at least three points".into() });
                }
                path.closed = true;
                path.handle = None;
            } else {
                let ib = lw.paths.iter().position(|p| p.id == path_b).ok_or_else(|| EngineError::Other("second path not found".into()))?;
                let mut a = lw.paths[ia].clone();
                let mut b = lw.paths[ib].clone();
                let ea = a.points.iter().position(|p| p.id == point_a).ok_or_else(|| EngineError::Other("first endpoint not found".into()))?;
                let eb = b.points.iter().position(|p| p.id == point_b).ok_or_else(|| EngineError::Other("second endpoint not found".into()))?;
                if ea != 0 && ea + 1 != a.points.len() || eb != 0 && eb + 1 != b.points.len() {
                    return Err(EngineError::BadParams { cmd: COMBINE_CMD.into(), msg: "combine requires path endpoints".into() });
                }
                if ea == 0 {
                    a.points.reverse();
                }
                if eb + 1 == b.points.len() {
                    b.points.reverse();
                }
                let same = (a.points.last().unwrap().position[0] - b.points.first().unwrap().position[0])
                    .hypot(a.points.last().unwrap().position[1] - b.points.first().unwrap().position[1])
                    < 1e-6;
                if same {
                    b.points.remove(0);
                }
                a.points.extend(b.points);
                a.handle = None;
                lw.paths.retain(|p| p.id != path_b);
                let ai = lw.paths.iter().position(|p| p.id == path_a).unwrap();
                lw.paths[ai] = a;
            }
        }
        let changed = if self_close { vec![path_a] } else { vec![path_a, path_b] };
        refresh_linework_cache(doc, layer, &changed, model)?;
        Ok(())
    })?;
    Ok(json!({"layer":layer.0,"path":path_a,"removedPath":if self_close { None } else { Some(path_b) },"closed":self_close}))
}

/// Rebuild ephemeral Linework raster caches for a UI-selected model. This is deliberately not an
/// edit: control points, file data, dirty state and undo history remain untouched.
fn set_interpolation(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, INTERPOLATION_CMD)?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let layers: Vec<_> = st.doc.layers.iter().filter_map(|layer| matches!(&layer.content, LayerContent::Linework(_)).then_some(layer.id)).collect();
    if layers.is_empty() {
        return Ok(json!({"interpolation": model.id()}));
    }
    let clean = st.saved_revision == st.revision;
    let mut doc = (*st.doc).clone();
    for layer in layers {
        let all_paths: Vec<u64> = match &doc.layer(layer).ok_or(EngineError::NoLayer(layer))?.content {
            LayerContent::Linework(linework) => linework.paths.iter().map(|path| path.id).collect(),
            _ => return Err(EngineError::Other("layer is not Linework".into())),
        };
        if let LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content {
            linework.path_caches.clear();
        }
        refresh_linework_cache(&mut doc, layer, &all_paths, model)?;
    }
    st.doc = std::sync::Arc::new(doc);
    st.revision = st.revision.wrapping_add(1);
    if clean {
        st.saved_revision = st.revision;
    }
    st.last_damage = Some(st.doc.bounds());
    Ok(json!({"interpolation": model.id()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: INTERPOLATION_CMD,
            label: "Preview Linework Interpolation",
            menu: &[],
            shortcut: None,
            params: r##"{"interpolation":"legacy"|"saiSimplified"|"saiOptimized"} → {interpolation}"##,
            enabled: has_doc,
            run: set_interpolation,
            journal: false,
        },
        CommandSpec {
            id: CMD,
            label: "Create Linework Path",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[[x,y],…],"handle":[x,y]? ,"width":px=3,"color":"#rrggbb"|"#rrggbbaa"|[r,g,b,a],"name":str?,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"} → {layer,kind,points,path,width,color}"##,
            enabled: has_doc,
            run: create,
            journal: true,
        },
        CommandSpec {
            id: MOVE_CMD,
            label: "Move Linework Point",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"point":id,"x":px,"y":px,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,point,x,y}"##,
            enabled: has_doc,
            run: move_point,
            journal: true,
        },
        CommandSpec {
            id: MOVE_MANY_CMD,
            label: "Move Linework Points",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[{"layer":id,"path":id,"point":id,"x":px,"y":px}…],"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {count}"##,
            enabled: has_doc,
            run: move_points,
            journal: true,
        },
        CommandSpec {
            id: DELETE_CMD,
            label: "Delete Linework Point",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"point":id,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,point}"##,
            enabled: has_doc,
            run: delete_point,
            journal: true,
        },
        CommandSpec {
            id: DELETE_MANY_CMD,
            label: "Delete Linework Points",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[{"layer":id,"path":id,"point":id}…],"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {count}"##,
            enabled: has_doc,
            run: delete_points,
            journal: true,
        },
        CommandSpec {
            id: DELETE_SEGMENT_CMD,
            label: "Delete Linework Segment",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"segment":zero_based_index,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,newPath,removedPoint,removedPath,movedPoints}"##,
            enabled: has_doc,
            run: delete_segment,
            journal: true,
        },
        CommandSpec {
            id: INSERT_CMD,
            label: "Insert Linework Point",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"index":n,"x":px,"y":px,"pressurePercent":0..200? | "factor":0..2?,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,point,x,y}"##,
            enabled: has_doc,
            run: insert_point,
            journal: true,
        },
        CommandSpec {
            id: WIDTH_CMD,
            label: "Set Linework Point Width",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"point":id,"factor":0..1000,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,point,factor}"##,
            enabled: has_doc,
            run: set_point_width,
            journal: true,
        },
        CommandSpec {
            id: WIDTH_MANY_CMD,
            label: "Set Linework Point Widths",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[{"layer":id,"path":id,"point":id,"factor":0..2}…],"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {count}"##,
            enabled: has_doc,
            run: set_point_widths,
            journal: true,
        },
        CommandSpec {
            id: COLOR_CMD,
            label: "Set Linework Path Color",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"color":"#rrggbb"|"#rrggbbaa"|[r,g,b,a],"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,color}"##,
            enabled: has_doc,
            run: set_path_color,
            journal: true,
        },
        CommandSpec {
            id: PATH_WIDTH_CMD,
            label: "Set Linework Path Width",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"width":0.7..30,"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,width}"##,
            enabled: has_doc,
            run: set_path_width,
            journal: true,
        },
        CommandSpec {
            id: POINTED_CMD,
            label: "Set Linework Point Type",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"point":id,"pointed":bool?} → {layer,path,point,pointed}"##,
            enabled: has_doc,
            run: set_point_pointed,
            journal: true,
        },
        CommandSpec {
            id: COMBINE_CMD,
            label: "Combine Linework Paths",
            menu: &[],
            shortcut: None,
            params: "{layer,pathA,pointA,pathB,pointB} → {layer,path,removedPath}",
            enabled: has_doc,
            run: combine_paths,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_path_cache_matches_compact_composition_and_reports_exact_pixel_damage() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":320,"height":240})).unwrap();
        let created = session.execute("linework.create", json!({
            "points":[[24.0,38.0],[142.0,104.0],[278.0,54.0]],
            "width":30.0,
            "widthFactors":[0.0,2.0,1.0],
            "color":"#193f87"
        })).unwrap();
        let layer = photocraft_doc::LayerId(created["layer"].as_u64().unwrap());
        let (path, point) = {
            let doc = &session.active().unwrap().doc;
            let LayerContent::Linework(linework) = &doc.layer(layer).unwrap().content else { panic!("expected Linework") };
            (linework.paths[0].id, linework.paths[0].points[1].id)
        };
        let (before, original_path_pixels) = {
            let doc = &session.active().unwrap().doc;
            let LayerContent::Linework(linework) = &doc.layer(layer).unwrap().content else { panic!("expected Linework") };
            (linework.cache.clone().unwrap(), linework.path_caches[&path].pixels.clone())
        };
        let old_compact_roundtrip = LineworkPathCache::from_surface(&before).to_surface();
        assert_eq!(before.content_bounds(), old_compact_roundtrip.content_bounds());
        assert_eq!(before.read_region(before.content_bounds()), old_compact_roundtrip.read_region(before.content_bounds()));

        session.execute("linework.movePoint", json!({
            "layer":layer.0,"path":path,"point":point,"x":151.0,"y":112.0,
            "interpolation":"saiSimplified"
        })).unwrap();
        let (after, after_path_pixels, actual_damage, doc_bounds) = {
            let state = session.active().unwrap();
            let LayerContent::Linework(linework) = &state.doc.layer(layer).unwrap().content else { panic!("expected Linework") };
            (linework.cache.clone().unwrap(), linework.path_caches[&path].pixels.clone(), state.last_damage.unwrap(), state.doc.bounds())
        };
        assert_eq!(actual_damage, surface_diff_bounds(&before, &after, doc_bounds));
        let full = LineworkPathCache::from_surface(&after).to_surface();
        assert_eq!(after.content_bounds(), full.content_bounds());
        assert_eq!(after.read_region(doc_bounds), full.read_region(doc_bounds));
        let before_pixels = before.read_region(doc_bounds);
        let after_pixels = after.read_region(doc_bounds);
        let channels = before.channels();
        for (index, (left, right)) in before_pixels.chunks_exact(channels).zip(after_pixels.chunks_exact(channels)).enumerate() {
            if left != right {
                let x = doc_bounds.x0 + (index % doc_bounds.width() as usize) as i32;
                let y = doc_bounds.y0 + (index / doc_bounds.width() as usize) as i32;
                assert!(actual_damage.contains(x, y), "changed pixel ({x},{y}) is outside reported damage {actual_damage:?}");
            }
        }

        assert!(session.undo());
        {
            let state = session.active().unwrap();
            let LayerContent::Linework(linework) = &state.doc.layer(layer).unwrap().content else { panic!("expected Linework") };
            assert_eq!(linework.cache.as_ref().unwrap(), &before);
            assert!(std::sync::Arc::ptr_eq(&linework.path_caches[&path].pixels, &original_path_pixels));
        }
        assert!(session.redo());
        {
            let state = session.active().unwrap();
            let LayerContent::Linework(linework) = &state.doc.layer(layer).unwrap().content else { panic!("expected Linework") };
            assert_eq!(linework.cache.as_ref().unwrap(), &after);
            assert!(std::sync::Arc::ptr_eq(&linework.path_caches[&path].pixels, &after_path_pixels));
        }
    }
}
