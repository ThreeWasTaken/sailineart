//! Headless SaiLine milestone commands.
//!
//! The command stores canonical point data in the dedicated document model. The interpolated path
//! remains a response aid for clients that want an immediate preview.

use photocraft_color::Color;
use photocraft_doc::{Layer, LayerContent, LineworkLayer, LineworkPath, ShapeLayer};
use photocraft_raster::Surface;
use photocraft_vector as vector;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "linework.create";
const MOVE_CMD: &str = "linework.movePoint";
const MOVE_MANY_CMD: &str = "linework.movePoints";
const DELETE_CMD: &str = "linework.deletePoint";
const INSERT_CMD: &str = "linework.insertPoint";
const WIDTH_CMD: &str = "linework.setPointWidth";
const COLOR_CMD: &str = "linework.setPathColor";
const INTERPOLATION_CMD: &str = "linework.setInterpolation";
const MAX_POINTS: usize = 4096;

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
) -> Result<ShapeLayer> {
    vector::linework::variable_width_shape_with_model(points, factors, f64::from(width), color, handle, model).map_err(EngineError::Other)
}

fn composite_cache(rect: photocraft_geom::Rect, format: photocraft_color::PixelFormat, base: Option<Surface>, overlay: Surface) -> Surface {
    let mut result = base.unwrap_or_else(|| Surface::new(format));
    if result.channels() != 4 || overlay.channels() != 4 {
        return overlay;
    }
    let src = overlay.read_region(rect);
    let mut dst = result.read_region(rect);
    let (src, _) = src.as_chunks::<4>();
    let (dst, _) = dst.as_chunks_mut::<4>();
    for (source, target) in src.iter().zip(dst.iter_mut()) {
        let sa = source[3].clamp(0.0, 1.0);
        let da = target[3].clamp(0.0, 1.0);
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 1e-6 {
            target.fill(0.0);
            continue;
        }
        for channel in 0..3 {
            target[channel] = (source[channel] * sa + target[channel] * da * (1.0 - sa)) / out_a;
        }
        target[3] = out_a;
    }
    let flattened: Vec<f32> = dst.iter().flat_map(|pixel| pixel.iter().copied()).collect();
    result.write_region(rect, &flattened);
    result
}

fn refresh_linework_cache(doc: &mut photocraft_doc::Document, layer_id: photocraft_doc::LayerId, model: vector::linework::InterpolationModel) -> Result<()> {
    let paths = {
        let layer = doc.layer(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
        let LayerContent::Linework(linework) = &layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
        linework.paths.clone()
    };
    let bounds = doc.bounds();
    let format = doc.pixel_format();
    let mut cache = None;
    for path in paths {
        let points: Vec<[f64; 2]> = path.points.iter().map(|point| point.position).collect();
        let factors: Vec<f32> = path.points.iter().map(|point| point.width_factor).collect();
        let mut shape = render_shape(&points, &factors, path.color, path.width, path.handle, model)?;
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        let rendered = shape.cache.ok_or(EngineError::Other("linework render cache missing".into()))?;
        cache = Some(composite_cache(bounds, format, cache, rendered));
    }
    let layer = doc.layer_mut(layer_id).ok_or(EngineError::NoLayer(layer_id))?;
    let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
    linework.cache = cache;
    Ok(())
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let model = interpolation(p, CMD)?;
    let points = points(p)?;
    let live_handle = handle(p)?;
    let path = vector::linework::interpolate_points_with_model(&points, live_handle, model)
        .map_err(|message| EngineError::BadParams { cmd: CMD.into(), msg: message })?;
    let color = solid_color(s, p)?;
    let stroke_width = width(p)?;
    let factors = width_factors(p, points.len())?;
    let mut line = LineworkPath::new(points.clone(), color, stroke_width).map_err(|message| EngineError::BadParams { cmd: CMD.into(), msg: message })?;
    for (point, factor) in line.points.iter_mut().zip(factors.iter().copied()) {
        point.width_factor = factor;
    }
    line.handle = live_handle;
    let mut shape = render_shape(&points, &factors, color, stroke_width, live_handle, model)
        .map_err(|error| EngineError::BadParams { cmd: CMD.into(), msg: error.to_string() })?;
    let requested_name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let id = s.edit("New Linework Path", |doc, active| {
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        let name = requested_name.unwrap_or_else(|| doc.next_layer_name("Linework"));
        let rendered = shape.cache.take().ok_or(EngineError::Other("linework render cache missing".into()))?;
        let bounds = doc.bounds();
        let format = doc.pixel_format();
        if let Some(active_id) = *active
            && let Some(layer) = doc.layer_mut(active_id)
            && let LayerContent::Linework(linework) = &mut layer.content
        {
            linework.cache = Some(composite_cache(bounds, format, linework.cache.take(), rendered));
            linework.paths.push(line);
            return Ok(active_id);
        }
        let id = doc.insert_above(*active, Layer::new(name, LayerContent::Linework(LineworkLayer { paths: vec![line], cache: Some(rendered) })));
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
    s.edit("Move Linework Point", |doc, _| {
        let (points, factors, color, width, handle) = {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.position = [x, y];
            (
                path.points.iter().map(|point| point.position).collect::<Vec<_>>(),
                path.points.iter().map(|point| point.width_factor).collect::<Vec<_>>(),
                path.color,
                path.width,
                path.handle,
            )
        };
        let mut shape = render_shape(&points, &factors, color, width, handle, model)?;
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content
            && let Some(path) = linework.paths.iter_mut().find(|path| path.id == path_id)
        {
            path.handle = handle;
            linework.cache = shape.cache;
        }
        refresh_linework_cache(doc, layer, model)?;
        Ok(())
    })?;
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
    s.edit("Move Linework Points", |doc, _| {
        let mut layers = Vec::new();
        for (layer_id, path_id, point_id, x, y) in &moves {
            let layer = doc.layer_mut(*layer_id).ok_or(EngineError::NoLayer(*layer_id))?;
            let LayerContent::Linework(linework) = &mut layer.content else { return Err(EngineError::Other("layer is not Linework".into())) };
            let path = linework.paths.iter_mut().find(|path| path.id == *path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == *point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.position = [*x, *y];
            if !layers.contains(layer_id) {
                layers.push(*layer_id);
            }
        }
        for layer in layers {
            refresh_linework_cache(doc, layer, model)?;
        }
        Ok(())
    })?;
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
        let (points, factors, color, width, handle) = {
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
            (
                path.points.iter().map(|point| point.position).collect::<Vec<_>>(),
                path.points.iter().map(|point| point.width_factor).collect::<Vec<_>>(),
                path.color,
                path.width,
                path.handle,
            )
        };
        let mut shape = render_shape(&points, &factors, color, width, handle, model)?;
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content {
            linework.cache = shape.cache;
        }
        refresh_linework_cache(doc, layer, model)?;
        Ok(())
    })?;
    Ok(json!({"layer": layer.0, "path": path_id, "point": point_id}))
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
        let (points, factors, color, width, handle, id) = {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            if index > path.points.len() {
                return Err(EngineError::Other("point insertion index out of range".into()));
            }
            let mut point = photocraft_doc::LineworkPoint::new([x, y]).map_err(EngineError::Other)?;
            point.width_factor = width_factor;
            let id = point.id;
            path.points.insert(index, point);
            (
                path.points.iter().map(|point| point.position).collect::<Vec<_>>(),
                path.points.iter().map(|point| point.width_factor).collect::<Vec<_>>(),
                path.color,
                path.width,
                path.handle,
                id,
            )
        };
        let mut shape = render_shape(&points, &factors, color, width, handle, model)?;
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content {
            linework.cache = shape.cache;
        }
        refresh_linework_cache(doc, layer, model)?;
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
        let (points, factors, color, width, handle) = {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            let point = path.points.iter_mut().find(|point| point.id == point_id).ok_or_else(|| EngineError::Other("linework point not found".into()))?;
            point.width_factor = factor as f32;
            (
                path.points.iter().map(|point| point.position).collect::<Vec<_>>(),
                path.points.iter().map(|point| point.width_factor).collect::<Vec<_>>(),
                path.color,
                path.width,
                path.handle,
            )
        };
        let mut shape = render_shape(&points, &factors, color, width, handle, model)?;
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content {
            linework.cache = shape.cache;
        }
        refresh_linework_cache(doc, layer, model)?;
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
        let (points, factors, width, handle) = {
            let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content else {
                return Err(EngineError::Other("layer is not Linework".into()));
            };
            let path = linework.paths.iter_mut().find(|path| path.id == path_id).ok_or_else(|| EngineError::Other("linework path not found".into()))?;
            path.color = color;
            (
                path.points.iter().map(|point| point.position).collect::<Vec<_>>(),
                path.points.iter().map(|point| point.width_factor).collect::<Vec<_>>(),
                path.width,
                path.handle,
            )
        };
        let mut shape = render_shape(&points, &factors, color, width, handle, model)?;
        crate::vector_cmds::refresh_shape(doc, &mut shape);
        if let photocraft_doc::LayerContent::Linework(linework) = &mut doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content {
            linework.cache = shape.cache;
        }
        refresh_linework_cache(doc, layer, model)?;
        Ok(())
    })?;
    let rgb = color.to_rgb();
    Ok(json!({"layer": layer.0, "path": path_id, "color": [rgb[0], rgb[1], rgb[2], color.alpha]}))
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
        refresh_linework_cache(&mut doc, layer, model)?;
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
            id: COLOR_CMD,
            label: "Set Linework Path Color",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id,"path":id,"color":"#rrggbb"|"#rrggbbaa"|[r,g,b,a],"interpolation":"legacy"|"saiSimplified"|"saiOptimized"?} → {layer,path,color}"##,
            enabled: has_doc,
            run: set_path_color,
            journal: true,
        },
    ]
}
