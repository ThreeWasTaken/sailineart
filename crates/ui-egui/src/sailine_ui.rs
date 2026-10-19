//! SaiCurve click-to-create tool. Drafts never mutate the document; commit uses the engine.
use egui::{Key, Modifiers, Stroke};
use photocraft_doc::{DocId, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::{PhotocraftApp, canvas::ViewXform, state::Tool, theme::Tokens};

static NEXT_DRAG: AtomicU64 = AtomicU64::new(1);

const fn interpolation_id() -> &'static str {
    "saiSimplified"
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub document: DocId,
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub straight: bool,
    #[serde(default)]
    pub redo: Vec<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub document: DocId,
    pub layer: LayerId,
    pub path: u64,
    pub point: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointDrag {
    pub selections: Vec<Selection>,
    pub origins: Vec<[f64; 2]>,
    pub start: [f64; 2],
    pub key: u64,
    #[serde(skip)]
    last_preview_at: Option<Instant>,
    #[serde(skip)]
    pending_point: Option<[f64; 2]>,
    #[serde(skip)]
    rendered_point: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointPressureDrag {
    pub selection: Selection,
    pub selections: Vec<Selection>,
    pub origins: Vec<f32>,
    pub start_x: f64,
    pub start_factor: f32,
    pub current_factor: f32,
    pub moved: bool,
    pub key: u64,
    #[serde(skip)]
    last_preview_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionBox {
    pub start: [f64; 2],
    pub current: [f64; 2],
    #[serde(default)]
    pub additive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SharedTarget {
    Insert { layer: LayerId, path: u64, index: usize, position: [f64; 2], factor: f32 },
    Delete { selection: Selection },
    DeleteSegment { layer: LayerId, path: u64, segment: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SharedGesture {
    pub start: [f64; 2],
    pub target: SharedTarget,
    pub moved: bool,
}

pub fn is_sailine_tool(tool: Tool) -> bool {
    matches!(tool, Tool::SaiCurve | Tool::SaiPressure | Tool::SaiStraight | Tool::SaiWeight | Tool::SaiCombine | Tool::SaiPointed | Tool::SaiColor)
}

pub fn clear_selection(app: &mut PhotocraftApp) {
    app.ui.sailine_selection = None;
    app.ui.sailine_selections.clear();
    app.ui.sailine_selection_box = None;
    app.ui.sailine_point_drag = None;
    app.ui.sailine_pressure_drag = None;
    app.ui.sailine_shared_gesture = None;
}

pub fn modifier_right_click_deselects(tool: Tool, mods: Modifiers, transforming: bool, has_selection: bool) -> bool {
    is_sailine_tool(tool) && !transforming && has_selection && (mods.ctrl || mods.alt || mods.shift)
}

/// Leaving the tool or its document discards the uncommitted draft.
pub fn sync(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::SaiCombine {
        app.ui.sailine_combine_first = None;
    }
    if app
        .ui
        .sailine
        .as_ref()
        .is_some_and(|draft| !matches!(app.ui.tool, Tool::SaiCurve | Tool::SaiStraight) || app.session.active().is_none_or(|s| s.doc.id != draft.document))
    {
        app.ui.sailine = None;
    }
    if !is_sailine_tool(app.ui.tool)
        || app.session.active().is_none_or(|state| {
            app.ui.sailine_selection.as_ref().is_some_and(|selection| state.doc.id != selection.document)
                || app.ui.sailine_combine_first.as_ref().is_some_and(|selection| state.doc.id != selection.document)
        })
    {
        app.ui.sailine_shared_gesture = None;
    }
    if app
        .ui
        .sailine_selection
        .as_ref()
        .is_some_and(|selection| !is_sailine_tool(app.ui.tool) || app.session.active().is_none_or(|s| s.doc.id != selection.document))
    {
        app.ui.sailine_selection = None;
        app.ui.sailine_selections.clear();
        app.ui.sailine_point_drag = None;
        app.ui.sailine_pressure_drag = None;
        app.ui.sailine_selection_box = None;
        app.ui.sailine_combine_first = None;
    }
}

#[allow(dead_code)]
pub fn down(app: &mut PhotocraftApp, point: [f64; 2]) {
    down_with_mods(app, point, Modifiers::NONE);
}

pub fn down_with_mods(app: &mut PhotocraftApp, point: [f64; 2], mods: Modifiers) {
    sync(app);
    let Some(document) = app.session.active().map(|s| s.doc.id) else { return };
    if point.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
        return;
    }
    if app.ui.sailine.is_none()
        && let Some((layer, path, point_id)) = nearest_point(app, point)
    {
        if mods.alt {
            let clicked = Selection { document, layer, path, point: point_id };
            if app.ui.sailine_selections.contains(&clicked) {
                delete_selected_points(app);
            } else {
                let model = interpolation_id();
                match app.run("linework.deletePoint", json!({"layer": layer.0, "path": path, "point": point_id, "interpolation": model})) {
                    Ok(_) => {
                        app.ui.sailine_selections.retain(|candidate| *candidate != clicked);
                        app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
                    }
                    Err(error) => {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                }
            }
            return;
        }
        let selection = Selection { document, layer, path, point: point_id };
        if mods.shift {
            if let Some(index) = app.ui.sailine_selections.iter().position(|candidate| *candidate == selection) {
                app.ui.sailine_selections.remove(index);
            } else {
                app.ui.sailine_selections.push(selection);
            }
            app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
        } else {
            app.ui.sailine_selection = Some(selection);
            app.ui.sailine_selections = vec![selection];
        }
        return;
    }
    if app.ui.sailine.is_none() {
        app.ui.sailine_selection = None;
        app.ui.sailine_selections.clear();
    }
    let draft = app.ui.sailine.get_or_insert_with(|| Draft { document, points: Vec::new(), straight: app.ui.tool == Tool::SaiStraight, redo: Vec::new() });
    if draft.points.len() < 4096 && draft.points.last() != Some(&point) {
        draft.points.push(point);
        draft.redo.clear();
    }
}

pub fn tool_down(app: &mut PhotocraftApp, point: [f64; 2], mods: Modifiers) {
    if mods.shift {
        if let Some((layer, path, point_id)) = nearest_point(app, point) {
            if let Some(document) = app.session.active().map(|s| s.doc.id) {
                let selection = Selection { document, layer, path, point: point_id };
                if let Some(index) = app.ui.sailine_selections.iter().position(|candidate| *candidate == selection) {
                    app.ui.sailine_selections.remove(index);
                } else {
                    app.ui.sailine_selections.push(selection);
                }
                app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
            }
            return;
        }
        if begin_selection_box_with_mode(app, point, true) {
            return;
        }
    }
    let target_path = || nearest_point(app, point).map(|(l, p, _)| (l, p)).or_else(|| nearest_segment(app, point).map(|(l, p, _, _, _)| (l, p)));
    match app.ui.tool {
        Tool::SaiCurve | Tool::SaiStraight => down_with_mods(app, point, mods),
        Tool::SaiPressure => {
            let _ = begin_pressure_drag(app, point);
        }
        Tool::SaiColor => {
            if let Some((layer, path)) = target_path() {
                let color = app.session.tools.foreground;
                if let Err(error) = app.run("linework.setPathColor", json!({"layer":layer.0,"path":path,"color":color,"interpolation":interpolation_id()})) {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
        }
        Tool::SaiWeight => {
            if let Some((layer, path)) = target_path() {
                let width = app.ui.tool_options.sailine_width.clamp(0.7, 30.0);
                if let Err(error) = app.run("linework.setPathWidth", json!({"layer":layer.0,"path":path,"width":width,"interpolation":interpolation_id()})) {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
        }
        Tool::SaiPointed => {
            if let Some((layer, path, point_id)) = nearest_point(app, point) {
                if let Err(error) =
                    app.run("linework.setPointPointed", json!({"layer":layer.0,"path":path,"point":point_id,"interpolation":interpolation_id()}))
                {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
        }
        Tool::SaiCombine => {
            let Some((layer, path, point)) = nearest_point(app, point) else { return };
            let endpoint = app.session.active().and_then(|s| s.doc.layer(layer)).and_then(|l| match &l.content {
                photocraft_doc::LayerContent::Linework(lw) => lw
                    .paths
                    .iter()
                    .find(|p| p.id == path)
                    .and_then(|p| p.points.first().filter(|x| x.id == point).or_else(|| p.points.last().filter(|x| x.id == point)))
                    .map(|x| x.id),
                _ => None,
            });
            let Some(point) = endpoint else {
                app.ui.status = "SaiCombine requires a path endpoint".into();
                return;
            };
            let document = app.session.active().map(|s| s.doc.id).unwrap_or(DocId(0));
            let current = Selection { document, layer, path, point };
            if let Some(first) = app.ui.sailine_combine_first.take() {
                if first.layer == current.layer && first.path != current.path {
                    match app.run("linework.combinePaths",json!({"layer":current.layer.0,"pathA":first.path,"pointA":first.point,"pathB":current.path,"pointB":current.point,"interpolation":interpolation_id()})) {
                        Ok(_) => { app.ui.sailine_selection=None; app.ui.sailine_selections.clear(); }
                        Err(error) => { app.ui.status=error;app.ui.status_error=true; }
                    }
                } else {
                    app.ui.sailine_combine_first = Some(current);
                }
            } else {
                app.ui.sailine_combine_first = Some(current);
            }
            if app.ui.sailine_combine_first.is_some() {
                app.ui.sailine_selection = Some(current);
                app.ui.sailine_selections = vec![current];
                app.ui.status = "SaiCombine: choose the endpoint of another path".into();
            }
        }
        _ => {}
    }
}

pub fn begin_drag(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    if !is_sailine_tool(app.ui.tool) || app.ui.sailine.is_some() {
        return false;
    }
    let Some(document) = app.session.active().map(|s| s.doc.id) else { return false };
    let (layer, path, point_id) = if let Some(found) = nearest_point(app, point) {
        found
    } else {
        let Some((layer, path, index, position, t)) = nearest_segment(app, point) else { return false };
        let model = interpolation_id();
        let factor = segment_width_factor(app, layer, path, index, t);
        let Ok(result) = app.run(
            "linework.insertPoint",
            json!({"layer": layer.0, "path": path, "index": index, "x": position[0], "y": position[1], "factor": factor, "interpolation": model}),
        ) else {
            return false;
        };
        let Some(point_id) = result["point"].as_u64() else { return false };
        (layer, path, point_id)
    };
    start_point_drag(app, Selection { document, layer, path, point: point_id }, point)
}

fn start_point_drag(app: &mut PhotocraftApp, selection: Selection, start: [f64; 2]) -> bool {
    if !app.ui.sailine_selections.contains(&selection) {
        app.ui.sailine_selections = vec![selection];
    }
    app.ui.sailine_selection = Some(selection);
    let origins = app.ui.sailine_selections.iter().filter_map(|candidate| point_position(app, *candidate)).collect();
    app.ui.sailine_point_drag = Some(PointDrag {
        selections: app.ui.sailine_selections.clone(),
        origins,
        start,
        key: NEXT_DRAG.fetch_add(1, Ordering::Relaxed),
        last_preview_at: None,
        pending_point: None,
        rendered_point: Some(start),
    });
    true
}

/// Shared point-edit shortcuts run before the active SaiLine tool's own click behavior.
pub fn shared_pointer(app: &mut PhotocraftApp, ev: crate::canvas::ToolEvent, mods: Modifiers) -> bool {
    if !is_sailine_tool(app.ui.tool) {
        return false;
    }
    // Leave the existing quick-pick chord (Ctrl+Alt+Cmd-click) to its global handler.
    if mods.ctrl && mods.alt && mods.command {
        return false;
    }
    match ev {
        crate::canvas::ToolEvent::Down { x, y, .. } => {
            let point = [x, y];
            if mods.ctrl {
                if nearest_point(app, point).is_some() {
                    return begin_drag(app, point);
                }
                if let Some((layer, path, index, position, t)) = nearest_segment(app, point) {
                    app.ui.sailine_shared_gesture = Some(SharedGesture {
                        start: point,
                        target: SharedTarget::Insert { layer, path, index, position, factor: segment_width_factor(app, layer, path, index, t) },
                        moved: false,
                    });
                    return true;
                }
                return false;
            }
            if mods.alt
                && let Some((layer, path, point)) = nearest_point(app, point)
            {
                let document = app.session.active().map(|s| s.doc.id).unwrap_or(DocId(0));
                app.ui.sailine_shared_gesture =
                    Some(SharedGesture { start: [x, y], target: SharedTarget::Delete { selection: Selection { document, layer, path, point } }, moved: false });
                return true;
            }
            if mods.alt
                && let Some((layer, path, end_index, _, _)) = nearest_segment(app, point)
            {
                app.ui.sailine_shared_gesture =
                    Some(SharedGesture { start: point, target: SharedTarget::DeleteSegment { layer, path, segment: end_index - 1 }, moved: false });
                return true;
            }
            false
        }
        crate::canvas::ToolEvent::Move { x, y, .. } => {
            if app.ui.sailine_point_drag.is_some() {
                return drag_to(app, [x, y]);
            }
            let threshold = 3.0 / f64::from(app.point_zoom().max(0.01));
            let Some(mut gesture) = app.ui.sailine_shared_gesture else { return false };
            gesture.moved |= (x - gesture.start[0]).hypot(y - gesture.start[1]) > threshold;
            if gesture.moved
                && mods.ctrl
                && let SharedTarget::Insert { layer, path, index, position, factor } = gesture.target
            {
                app.ui.sailine_shared_gesture = None;
                match app.run(
                    "linework.insertPoint",
                    json!({"layer":layer.0,"path":path,"index":index,"x":position[0],"y":position[1],"factor":factor,"interpolation":interpolation_id()}),
                ) {
                    Ok(result) => {
                        if let (Some(point), Some(document)) = (result["point"].as_u64(), app.session.active().map(|s| s.doc.id)) {
                            let selection = Selection { document, layer, path, point };
                            start_point_drag(app, selection, position);
                            drag_to(app, [x, y]);
                        }
                    }
                    Err(error) => {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                }
                return true;
            }
            app.ui.sailine_shared_gesture = Some(gesture);
            true
        }
        crate::canvas::ToolEvent::Up { x, y } => {
            if app.ui.sailine_point_drag.is_some() {
                drag_to(app, [x, y]);
                end_drag(app);
                return true;
            }
            let Some(gesture) = app.ui.sailine_shared_gesture.take() else { return false };
            if gesture.moved {
                return true;
            }
            match gesture.target {
                SharedTarget::Insert { layer, path, index, position, factor } => {
                    match app.run(
                        "linework.insertPoint",
                        json!({"layer":layer.0,"path":path,"index":index,"x":position[0],"y":position[1],"factor":factor,"interpolation":interpolation_id()}),
                    ) {
                        Ok(result) => {
                            if let Some(point) = result["point"].as_u64() {
                                if let Some(document) = app.session.active().map(|s| s.doc.id) {
                                    let selection = Selection { document, layer, path, point };
                                    app.ui.sailine_selection = Some(selection);
                                    app.ui.sailine_selections = vec![selection];
                                }
                            }
                        }
                        Err(error) => {
                            app.ui.status = error;
                            app.ui.status_error = true;
                        }
                    }
                }
                SharedTarget::Delete { selection } => {
                    if app.ui.sailine_selections.contains(&selection) {
                        delete_selected_points(app);
                    } else {
                        let _ = app.run(
                            "linework.deletePoint",
                            json!({"layer":selection.layer.0,"path":selection.path,"point":selection.point,"interpolation":interpolation_id()}),
                        );
                        app.ui.sailine_selections.retain(|s| *s != selection);
                        app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
                    }
                }
                SharedTarget::DeleteSegment { layer, path, segment } => {
                    match app.run("linework.deleteSegment", json!({"layer":layer.0,"path":path,"segment":segment,"interpolation":interpolation_id()})) {
                        Ok(result) => {
                            if result["removedPath"].as_bool() == Some(true) {
                                app.ui.sailine_selections.retain(|selection| !(selection.layer == layer && selection.path == path));
                            }
                            if let Some(removed) = result["removedPoint"].as_u64() {
                                app.ui
                                    .sailine_selections
                                    .retain(|selection| !(selection.layer == layer && selection.path == path && selection.point == removed));
                            }
                            if let Some(new_path) = result["newPath"].as_u64() {
                                let moved: std::collections::HashSet<_> =
                                    result["movedPoints"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_u64).collect();
                                for selection in &mut app.ui.sailine_selections {
                                    if selection.layer == layer && selection.path == path && moved.contains(&selection.point) {
                                        selection.path = new_path;
                                    }
                                }
                            }
                            app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
                        }
                        Err(error) => {
                            app.ui.status = error;
                            app.ui.status_error = true;
                        }
                    }
                }
            }
            true
        }
    }
}

pub fn drag_to(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(drag) = app.ui.sailine_point_drag.as_mut() else { return false };
    drag.pending_point = Some(point);
    if drag.rendered_point == Some(point) {
        return true;
    }
    let now = Instant::now();
    if drag.last_preview_at.is_some_and(|last| now.duration_since(last) < Duration::from_millis(33)) {
        return true;
    }
    drag.last_preview_at = Some(now);
    drag.rendered_point = Some(point);
    render_drag(app, point)
}

fn render_drag(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(drag) = app.ui.sailine_point_drag.clone() else { return false };
    let dx = point[0] - drag.start[0];
    let dy = point[1] - drag.start[1];
    let points: Vec<_> = drag
        .selections
        .iter()
        .zip(drag.origins.iter())
        .map(|(selection, origin)| json!({"layer": selection.layer.0, "path": selection.path, "point": selection.point, "x": origin[0] + dx, "y": origin[1] + dy}))
        .collect();
    let model = interpolation_id();
    let _ = app.run("linework.movePoints", json!({"points": points, "coalesce": format!("sailine-move:{}", drag.key), "interpolation": model}));
    true
}

pub fn end_drag(app: &mut PhotocraftApp) {
    if let Some(drag) = &app.ui.sailine_point_drag
        && let Some(point) = drag.pending_point
        && drag.rendered_point != Some(point)
    {
        render_drag(app, point);
    }
    app.ui.sailine_point_drag = None;
}

#[allow(dead_code)]
pub fn begin_selection_box(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    begin_selection_box_with_mode(app, point, false)
}

fn begin_selection_box_with_mode(app: &mut PhotocraftApp, point: [f64; 2], additive: bool) -> bool {
    if !is_sailine_tool(app.ui.tool) || app.ui.sailine.is_some() || nearest_point(app, point).is_some() {
        return false;
    }
    let Some(state) = app.session.active() else { return false };
    let Some(layer) = state.active_layer.and_then(|id| state.doc.layer(id)) else { return false };
    if !matches!(layer.content, photocraft_doc::LayerContent::Linework(_)) {
        return false;
    }
    app.ui.sailine_selection_box = Some(SelectionBox { start: point, current: point, additive });
    true
}

pub fn selection_box_to(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(selection) = app.ui.sailine_selection_box.as_mut() else { return false };
    selection.current = point;
    true
}

pub fn end_selection_box(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(selection_box) = app.ui.sailine_selection_box.take() else { return false };
    let min_x = selection_box.start[0].min(point[0]);
    let max_x = selection_box.start[0].max(point[0]);
    let min_y = selection_box.start[1].min(point[1]);
    let max_y = selection_box.start[1].max(point[1]);
    let Some(state) = app.session.active() else { return true };
    let Some(layer_id) = state.active_layer else { return true };
    let Some(layer) = state.doc.layer(layer_id) else { return true };
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return true };
    let document = state.doc.id;
    let points = linework.paths.iter().flat_map(|path| {
        path.points.iter().filter_map(|point| {
            (point.position[0] >= min_x && point.position[0] <= max_x && point.position[1] >= min_y && point.position[1] <= max_y).then_some(Selection {
                document,
                layer: layer_id,
                path: path.id,
                point: point.id,
            })
        })
    });
    let points: Vec<_> = points.collect();
    if selection_box.additive {
        for selection in points {
            if !app.ui.sailine_selections.contains(&selection) {
                app.ui.sailine_selections.push(selection);
            }
        }
    } else {
        app.ui.sailine_selections = points;
    }
    app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
    true
}

pub(crate) fn nearest_point(app: &PhotocraftApp, point: [f64; 2]) -> Option<(LayerId, u64, u64)> {
    let state = app.session.active()?;
    let layer_id = state.active_layer?;
    let layer = state.doc.layer(layer_id)?;
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return None };
    let tolerance = 8.0 / f64::from(app.point_zoom().max(0.01));
    linework
        .paths
        .iter()
        .flat_map(|path| path.points.iter().map(move |p| (path.id, p.id, p.position)))
        .filter_map(|(path, id, position)| {
            let distance = (position[0] - point[0]).hypot(position[1] - point[1]);
            (distance <= tolerance).then_some((distance, path, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, path, id)| (layer_id, path, id))
}

fn point_position(app: &PhotocraftApp, selection: Selection) -> Option<[f64; 2]> {
    let state = app.session.active()?;
    let layer = state.doc.layer(selection.layer)?;
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return None };
    linework.paths.iter().find(|path| path.id == selection.path)?.points.iter().find(|point| point.id == selection.point).map(|point| point.position)
}

fn point_width(app: &PhotocraftApp, selection: Selection) -> Option<f32> {
    let state = app.session.active()?;
    let layer = state.doc.layer(selection.layer)?;
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return None };
    linework.paths.iter().find(|path| path.id == selection.path)?.points.iter().find(|point| point.id == selection.point).map(|point| point.width_factor)
}

fn segment_width_factor(app: &PhotocraftApp, layer_id: LayerId, path_id: u64, end_index: usize, t: f64) -> f32 {
    let Some(path) = app.session.active().and_then(|state| state.doc.layer(layer_id)).and_then(|layer| match &layer.content {
        photocraft_doc::LayerContent::Linework(linework) => linework.paths.iter().find(|path| path.id == path_id),
        _ => None,
    }) else {
        return 1.0;
    };
    let (Some(a), Some(b)) = (end_index.checked_sub(1).and_then(|i| path.points.get(i)), path.points.get(end_index)) else {
        return 1.0;
    };
    let t = t.clamp(0.0, 1.0);
    (f64::from(a.width_factor) * (1.0 - t) + f64::from(b.width_factor) * t).clamp(0.0, 2.0) as f32
}

pub fn begin_pressure_drag(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    if app.ui.tool != Tool::SaiPressure {
        return false;
    }
    let Some(document) = app.session.active().map(|s| s.doc.id) else { return false };
    let Some((layer, path, point_id)) = nearest_point(app, point) else { return false };
    let selection = Selection { document, layer, path, point: point_id };
    let Some(factor) = point_width(app, selection) else { return false };
    let factor = factor.clamp(0.0, 2.0);
    if !app.ui.sailine_selections.contains(&selection) {
        app.ui.sailine_selections = vec![selection];
    }
    app.ui.sailine_selection = Some(selection);
    let selections: Vec<_> = app.ui.sailine_selections.iter().copied().filter(|s| point_width(app, *s).is_some()).collect();
    let origins = selections.iter().filter_map(|s| point_width(app, *s)).collect();
    app.ui.sailine_pressure_drag = Some(PointPressureDrag {
        selection,
        selections,
        origins,
        start_x: point[0],
        start_factor: factor,
        current_factor: factor,
        moved: false,
        key: NEXT_DRAG.fetch_add(1, Ordering::Relaxed),
        last_preview_at: None,
    });
    true
}

pub fn pressure_drag_to(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let (moved, should_render) = {
        let Some(drag) = app.ui.sailine_pressure_drag.as_mut() else { return false };
        let dx = point[0] - drag.start_x;
        drag.moved |= dx.abs() > 0.01;
        drag.current_factor = (drag.start_factor + (dx / 120.0) as f32).clamp(0.0, 2.0);
        let now = Instant::now();
        let should_render = drag.last_preview_at.is_none_or(|last| now.duration_since(last) >= Duration::from_millis(33));
        if should_render {
            drag.last_preview_at = Some(now);
        }
        (drag.moved, should_render)
    };
    if !moved || !should_render {
        return true;
    }
    render_pressure_drag(app);
    true
}

fn render_pressure_drag(app: &mut PhotocraftApp) {
    let Some(drag) = app.ui.sailine_pressure_drag.as_ref() else { return };
    let delta = drag.current_factor - drag.start_factor;
    let points: Vec<_> = drag.selections.iter().zip(&drag.origins).map(|(selection, origin)| {
        json!({"layer": selection.layer.0, "path": selection.path, "point": selection.point, "factor": (origin + delta).clamp(0.0, 2.0)})
    }).collect();
    let key = drag.key;
    let model = interpolation_id();
    if let Err(error) = app.run("linework.setPointWidths", json!({"points": points, "coalesce": format!("sailine-pressure:{key}"), "interpolation": model})) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

pub fn end_pressure_drag(app: &mut PhotocraftApp, point: [f64; 2]) {
    if app.ui.sailine_pressure_drag.is_none() {
        return;
    }
    pressure_drag_to(app, point);
    let Some(drag) = app.ui.sailine_pressure_drag.take() else { return };
    if drag.moved {
        app.ui.sailine_pressure_drag = Some(drag);
        render_pressure_drag(app);
        app.ui.sailine_pressure_drag = None;
    }
}

pub fn delete_selected_points(app: &mut PhotocraftApp) -> bool {
    if !is_sailine_tool(app.ui.tool) || app.ui.sailine.is_some() || app.ui.sailine_selections.is_empty() {
        return false;
    }
    let points: Vec<_> =
        app.ui.sailine_selections.iter().map(|selection| json!({"layer": selection.layer.0, "path": selection.path, "point": selection.point})).collect();
    match app.run("linework.deletePoints", json!({"points": points, "interpolation": interpolation_id()})) {
        Ok(_) => {
            app.ui.sailine_selection = None;
            app.ui.sailine_selections.clear();
            app.ui.status = "SaiCombine: paths combined".into();
        }
        Err(error) => {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    true
}

pub(crate) fn nearest_segment(app: &PhotocraftApp, point: [f64; 2]) -> Option<(LayerId, u64, usize, [f64; 2], f64)> {
    let state = app.session.active()?;
    let layer_id = state.active_layer?;
    let layer = state.doc.layer(layer_id)?;
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return None };
    let screen_tolerance = 10.0 / f64::from(app.point_zoom().max(0.01));
    let mut best: Option<(f64, u64, usize, [f64; 2], f64)> = None;
    for path in &linework.paths {
        let path_tolerance = screen_tolerance.max(
            f64::from(path.width) * path.points.iter().map(|point| f64::from(point.width_factor)).fold(1.0, f64::max) * 0.5
                + 4.0 / f64::from(app.point_zoom().max(0.01)),
        );
        let points: Vec<_> = path.points.iter().map(|point| point.position).collect();
        let pointed: Vec<_> = path.points.iter().map(|point| point.pointed).collect();
        let Ok(curve) = photocraft_vector::linework::interpolate_points_with_model_and_pointed(
            &points,
            path.handle,
            photocraft_vector::linework::InterpolationModel::SaiSimplified,
            &pointed,
        ) else {
            continue;
        };
        let Some(subpath) = curve.subpaths.first() else { continue };
        for (index, segment) in subpath.segments().iter().enumerate() {
            for step in 0..=64 {
                let t = f64::from(step) / 64.0;
                let u = 1.0 - t;
                let position = [
                    u.powi(3) * segment[0].x + 3.0 * u * u * t * segment[1].x + 3.0 * u * t * t * segment[2].x + t.powi(3) * segment[3].x,
                    u.powi(3) * segment[0].y + 3.0 * u * u * t * segment[1].y + 3.0 * u * t * t * segment[2].y + t.powi(3) * segment[3].y,
                ];
                let distance = (position[0] - point[0]).hypot(position[1] - point[1]);
                if distance <= path_tolerance && best.as_ref().is_none_or(|candidate| distance < candidate.0) {
                    best = Some((distance, path.id, index + 1, position, t));
                }
            }
        }
    }
    best.map(|(_, path, index, position, t)| (layer_id, path, index, position, t))
}

pub fn commit(app: &mut PhotocraftApp) {
    sync(app);
    let Some(draft) = app.ui.sailine.as_ref().filter(|d| d.points.len() >= 2) else { return };
    let model = interpolation_id();
    let params = json!({"points": draft.points, "width": app.ui.tool_options.sailine_width, "color": app.session.tools.foreground, "straight": draft.straight, "interpolation": model});
    match app.run("linework.create", params) {
        Ok(_) => app.ui.sailine = None,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    sync(app);
    if is_sailine_tool(app.ui.tool)
        && app.ui.sailine.is_none()
        && !app.ui.sailine_selections.is_empty()
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace) || i.consume_key(Modifiers::NONE, Key::Delete))
    {
        return delete_selected_points(app);
    }
    if !matches!(app.ui.tool, Tool::SaiCurve | Tool::SaiStraight) || app.ui.sailine.is_none() {
        return false;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        app.ui.sailine = None;
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        commit(app);
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) {
        if let Some(draft) = &mut app.ui.sailine
            && let Some(point) = draft.points.pop()
        {
            draft.redo.push(point);
        }
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Z)) {
        if let Some(draft) = &mut app.ui.sailine
            && let Some(point) = draft.points.pop()
        {
            draft.redo.push(point);
        }
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Y) || i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Z)) {
        if let Some(draft) = &mut app.ui.sailine
            && let Some(point) = draft.redo.pop()
        {
            draft.points.push(point);
        }
        return true;
    }
    false
}

// Tool foreground components are sRGB encoded, while egui::Rgba expects linear RGB.
fn screen_color(rgba: [f32; 4]) -> egui::Color32 {
    let [r, g, b, a] = rgba.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn draw_round_anchor(painter: &egui::Painter, center: egui::Pos2, selected: bool, accent: egui::Color32) {
    painter.circle_filled(center, 3.5, if selected { accent } else { egui::Color32::WHITE });
    painter.circle_stroke(center, 3.5, egui::Stroke::new(1.0, accent));
}

pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let pressure_tool = app.ui.tool == Tool::SaiPressure;
    ui.label(app.ui.tool.label());
    ui.label(tl!("Width:"));
    ui.add(egui::DragValue::new(&mut app.ui.tool_options.sailine_width).range(0.7..=30.0).suffix(" px"));
    if let Some(selection) = app.ui.sailine_selection
        && let Some(factor) = app.session.active().and_then(|state| state.doc.layer(selection.layer)).and_then(|layer| match &layer.content {
            photocraft_doc::LayerContent::Linework(linework) => linework
                .paths
                .iter()
                .find(|path| path.id == selection.path)
                .and_then(|path| path.points.iter().find(|point| point.id == selection.point))
                .map(|point| point.width_factor),
            _ => None,
        })
    {
        let factor = app.ui.sailine_pressure_drag.as_ref().map_or(factor, |drag| drag.current_factor);
        if pressure_tool {
            ui.label(format!("Point pressure: {:.0}%", (factor * 100.0).clamp(0.0, 200.0)));
            ui.label("Shift+click/drag: select · Drag: pressure · Ctrl+drag: move · Ctrl+click: add · Delete: remove selected");
        }
    }
    if let Some(selection) = app.ui.sailine_selection {
        let foreground = screen_color(app.session.tools.foreground);
        if ui.add(egui::Button::new("Path color ← foreground").fill(foreground)).clicked() {
            let color = app.session.tools.foreground;
            let _ = app
                .run("linework.setPathColor", json!({"layer": selection.layer.0, "path": selection.path, "color": color, "interpolation": interpolation_id()}));
        }
    }
    ui.label(tl!("Color:"));
    let color = screen_color(app.session.tools.foreground);
    if ui.add(egui::Button::new("").fill(color).min_size(egui::vec2(24.0, 20.0))).clicked() {
        crate::color_picker_ui::open(app, "foreground");
    }
    let ready = app.ui.sailine.as_ref().is_some_and(|d| d.points.len() >= 2);
    if ui.add_enabled(ready, egui::Button::new(tl!("Done"))).clicked() {
        commit(app);
    }
    if ui.add_enabled(app.ui.sailine.is_some(), egui::Button::new(tl!("Cancel"))).clicked() {
        app.ui.sailine = None;
    }
    ui.label("Enter/right click: finish · Esc: cancel")
        .on_hover_text("Click to add a point. Move around the last point to set the curve angle and rounding. Backspace or Ctrl+Z removes a draft point; Ctrl+Y restores it. Leaving the tool or document cancels the draft.");
}

pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let to_screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    if let Some(draft) = app
        .ui
        .sailine
        .as_ref()
        .filter(|d| matches!(app.ui.tool, Tool::SaiCurve | Tool::SaiStraight) && app.session.active().is_some_and(|s| s.doc.id == d.document))
    {
        let mut points = draft.points.clone();
        if let Some(hover) = app.hover_doc
            && points.len() < 4096
            && points.last() != Some(&hover)
        {
            points.push(hover);
        }
        let handle = app.hover_doc.filter(|_| draft.points.len() >= 2);
        let model = photocraft_vector::linework::InterpolationModel::SaiSimplified;
        let pointed = vec![draft.straight; points.len()];
        if let Ok(path) = photocraft_vector::linework::interpolate_points_with_model_and_pointed(&points, handle, model, &pointed) {
            let color = screen_color(app.session.tools.foreground);
            let width = (app.ui.tool_options.sailine_width * xf.zoom).max(0.5);
            for (curve, _) in crate::vector_ui::path_lines(&path, &to_screen) {
                painter.add(egui::Shape::line(curve, Stroke::new(width, color)));
            }
            let radius = width * 0.5;
            if let Some(start) = points.first() {
                painter.circle_filled(to_screen(*start), radius, color);
            }
            if let Some(end) = points.last() {
                painter.circle_filled(to_screen(*end), radius, color);
            }
        }
        let accent = Tokens::get(painter.ctx()).accent;
        for point in &draft.points {
            draw_round_anchor(painter, to_screen(*point), true, accent);
        }
    }
    if app.ui.sailine_point_drag.is_none()
        && !app.transform_preview.as_ref().is_some_and(crate::transform_tool::is_sailine_points_preview)
        && let Some(layer) = app.session.active().and_then(|s| s.active_layer).and_then(|id| app.session.active().and_then(|s| s.doc.layer(id)))
        && let photocraft_doc::LayerContent::Linework(linework) = &layer.content
    {
        let document = app.session.active().map(|state| state.doc.id);
        let selected: std::collections::HashSet<_> = app
            .ui
            .sailine_selections
            .iter()
            .chain(app.ui.sailine_selection.iter())
            .filter(|selection| Some(selection.document) == document)
            .map(|selection| (selection.layer, selection.path, selection.point))
            .collect();
        let modifiers = painter.ctx().input(|input| input.modifiers);
        let modifier_edit = modifiers.alt || modifiers.shift || modifiers.ctrl;
        let show_handles = modifier_edit || app.ui.sailine_selection_box.is_some();
        let hovered = app.hover_doc.and_then(|point| nearest_point(app, point));
        let accent = Tokens::get(painter.ctx()).accent;
        let multiple_selection = selected.iter().filter(|(selected_layer, _, _)| *selected_layer == layer.id).count() > 1;
        let selected_color = if multiple_selection { egui::Color32::from_rgb(245, 42, 42) } else { accent };
        let hovered_segment = app.hover_doc.and_then(|point| nearest_segment(app, point));
        if modifier_edit
            && let Some((_, path_id, _, _, _)) = hovered_segment
            && let Some(path) = linework.paths.iter().find(|path| path.id == path_id)
        {
            let points: Vec<_> = path.points.iter().map(|point| point.position).collect();
            let pointed: Vec<_> = path.points.iter().map(|point| point.pointed).collect();
            if let Ok(centerline) = photocraft_vector::linework::interpolate_points_with_model_and_pointed(
                &points,
                path.handle,
                photocraft_vector::linework::InterpolationModel::SaiSimplified,
                &pointed,
            ) {
                let skeleton = egui::Color32::from_rgb(45, 155, 245);
                for (curve, _) in crate::vector_ui::path_lines(&centerline, &to_screen) {
                    painter.add(egui::Shape::line(curve, Stroke::new(1.25, skeleton)));
                }
                if modifier_edit {
                    let controls = egui::Color32::from_rgb(120, 220, 95);
                    for point in &path.points {
                        painter.circle_filled(to_screen(point.position), 1.8, controls);
                    }
                }
            }
        }

        // Selected anchors keep their red state after Shift is released. A centerline segment
        // is selected when both of its endpoint anchors are selected, matching SAI's point-based
        // multi-selection model.
        if multiple_selection && modifier_edit {
            for path in &linework.paths {
                if !path.points.windows(2).enumerate().any(|(index, pair)| {
                    selected.contains(&(layer.id, path.id, pair[0].id)) && selected.contains(&(layer.id, path.id, pair[1].id)) && index + 1 < path.points.len()
                }) {
                    continue;
                }
                let points: Vec<_> = path.points.iter().map(|point| point.position).collect();
                let pointed: Vec<_> = path.points.iter().map(|point| point.pointed).collect();
                let Ok(centerline) = photocraft_vector::linework::interpolate_points_with_model_and_pointed(
                    &points,
                    path.handle,
                    photocraft_vector::linework::InterpolationModel::SaiSimplified,
                    &pointed,
                ) else {
                    continue;
                };
                let Some(subpath) = centerline.subpaths.first() else { continue };
                for (index, segment) in subpath.segments().iter().enumerate() {
                    if !selected.contains(&(layer.id, path.id, path.points[index].id)) || !selected.contains(&(layer.id, path.id, path.points[index + 1].id)) {
                        continue;
                    }
                    let curve: Vec<_> = (0..=32)
                        .map(|step| {
                            let t = f64::from(step) / 32.0;
                            let u = 1.0 - t;
                            let position = [
                                u.powi(3) * segment[0].x + 3.0 * u * u * t * segment[1].x + 3.0 * u * t * t * segment[2].x + t.powi(3) * segment[3].x,
                                u.powi(3) * segment[0].y + 3.0 * u * u * t * segment[1].y + 3.0 * u * t * t * segment[2].y + t.powi(3) * segment[3].y,
                            ];
                            to_screen(position)
                        })
                        .collect();
                    painter.add(egui::Shape::line(curve, Stroke::new(2.0, selected_color)));
                }
            }
        }

        if show_handles {
            for path in &linework.paths {
                for candidate in &path.points {
                    let is_selected = selected.contains(&(layer.id, path.id, candidate.id));
                    let is_hovered = hovered
                        .is_some_and(|(hover_layer, hover_path, hover_point)| hover_layer == layer.id && hover_path == path.id && hover_point == candidate.id);
                    if is_selected || is_hovered || modifiers.alt || modifiers.shift || modifiers.ctrl {
                        draw_round_anchor(painter, to_screen(candidate.position), is_selected, if is_selected { selected_color } else { accent });
                    }
                }
            }
        }
        if let Some(selection) = app.ui.sailine_selection_box {
            let a = to_screen(selection.start);
            let b = to_screen(selection.current);
            let rect = egui::Rect::from_two_pos(a, b);
            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, accent), egui::StrokeKind::Outside);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use std::time::Instant;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.ui.tool = Tool::SaiCurve;
        app
    }

    /// Explicit egui overlay timing; opt-in so normal test runs stay fast and deterministic.
    #[test]
    #[ignore = "headless SaiLineArt overlay benchmark; run with --ignored --nocapture"]
    fn sailine_overlay_bench() {
        let mut rows = Vec::new();
        for count in [10usize, 100, 1000] {
            for zoom in [0.25f32, 1.0, 4.0] {
                let mut app = app();
                let points: Vec<[f64; 2]> = (0..count)
                    .map(|i| [20.0 + i as f64 * 0.25, 100.0 + ((i % 7) as f64 - 3.0) * 0.5])
                    .collect();
                app.run("linework.create", json!({"points":points,"width":6.0})).unwrap();
                app.ui.tool = Tool::SaiPressure;
                app.hover_doc = Some(points[count / 2]);
                let ctx = egui::Context::default();
                let xf = crate::canvas::ViewXform {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 400.0)),
                    zoom,
                    center: [100.0, 100.0],
                    flip: false,
                    rotation: 0.0,
                    aspect: 1.0,
                };
                let mut samples = Vec::new();
                for iteration in 0..22 {
                    let start = Instant::now();
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        let painter = egui::Painter::new(ui.ctx().clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
                        super::draw(&app, &painter, &xf);
                    });
                    output.textures_delta.clear();
                    if iteration >= 2 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
                }
                samples.sort_by(f64::total_cmp);
                let percentile = |p: f64| {
                    let rank = (samples.len().saturating_sub(1) as f64) * p;
                    let lo = rank.floor() as usize;
                    let hi = (lo + 1).min(samples.len() - 1);
                    samples[lo] + (samples[hi] - samples[lo]) * (rank - lo as f64)
                };
                rows.push(json!({"points":count,"zoom":zoom,"hovered_pressure_overlay_ms":{"iterations":samples.len(),"median_ms":percentile(0.5),"p95_ms":percentile(0.95),"max_ms":samples[samples.len()-1]}}));
            }
        }
        let report = json!({"schema":"sailine-overlay-bench-v1","optimization":"test-profile; override CARGO_PROFILE_TEST_PACKAGE_PHOTOCRAFT_UI_EGUI_OPT_LEVEL=3 for optimized UI","rows":rows});
        let encoded = serde_json::to_string_pretty(&report).unwrap();
        println!("{encoded}");
        if let Ok(path) = std::env::var("SAILINE_BENCH_OUTPUT") {
            std::fs::write(path, encoded).unwrap();
        }
    }

    fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
        tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
    }

    fn path_point_count(app: &PhotocraftApp, layer: LayerId, path: u64) -> usize {
        match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Linework(lw) => lw.paths.iter().find(|p| p.id == path).unwrap().points.len(),
            _ => 0,
        }
    }

    #[test]
    fn shared_ctrl_insert_move_and_alt_delete_work_with_all_seven_tools() {
        for tool in [Tool::SaiCurve, Tool::SaiPressure, Tool::SaiStraight, Tool::SaiWeight, Tool::SaiCombine, Tool::SaiPointed, Tool::SaiColor] {
            let mut app = app();
            let created = app.run("linework.create", json!({"points":[[20,40],[180,40]],"width":4,"color":"#112233"})).unwrap();
            let layer = LayerId(created["layer"].as_u64().unwrap());
            let (path, first) = {
                let photocraft_doc::LayerContent::Linework(lw) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
                (lw.paths[0].id, lw.paths[0].points[0].id)
            };
            app.ui.tool = tool;
            let ctrl = Modifiers::CTRL;
            tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 40.0, pressure: 1.0 }, ctrl);
            tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, ctrl);
            assert_eq!(path_point_count(&app, layer, path), 3, "Ctrl+click insertion for {tool:?}");
            tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 40.0, pressure: 1.0 }, ctrl);
            tool_event(&mut app, ToolEvent::Move { x: 35.0, y: 55.0, pressure: 1.0 }, ctrl);
            tool_event(&mut app, ToolEvent::Up { x: 35.0, y: 55.0 }, ctrl);
            let moved = match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
                photocraft_doc::LayerContent::Linework(lw) => {
                    lw.paths.iter().find(|p| p.id == path).unwrap().points.iter().find(|p| p.id == first).unwrap().position
                }
                _ => unreachable!(),
            };
            assert_eq!(moved, [35.0, 55.0], "Ctrl+drag movement for {tool:?}");
            tool_event(&mut app, ToolEvent::Down { x: 35.0, y: 55.0, pressure: 1.0 }, Modifiers::ALT);
            tool_event(&mut app, ToolEvent::Up { x: 35.0, y: 55.0 }, Modifiers::ALT);
            assert_eq!(path_point_count(&app, layer, path), 2, "Alt+click deletion for {tool:?}");
        }
    }

    #[test]
    fn alt_clicking_an_internal_segment_splits_the_line_from_any_sailine_tool() {
        for tool in [Tool::SaiCurve, Tool::SaiPressure, Tool::SaiStraight, Tool::SaiWeight, Tool::SaiCombine, Tool::SaiPointed, Tool::SaiColor] {
            let mut app = app();
            let created = app.run("linework.create", json!({"points":[[20,40],[60,40],[100,40],[140,40]],"width":4})).unwrap();
            let layer = LayerId(created["layer"].as_u64().unwrap());
            let (path_id, original_point_ids) = match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
                photocraft_doc::LayerContent::Linework(lw) => (lw.paths[0].id, lw.paths[0].points.iter().map(|point| point.id).collect::<Vec<_>>()),
                _ => panic!(),
            };
            app.ui.tool = tool;
            tool_event(&mut app, ToolEvent::Down { x: 80.0, y: 40.0, pressure: 1.0 }, Modifiers::ALT);
            tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 40.0 }, Modifiers::ALT);
            let paths = match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
                photocraft_doc::LayerContent::Linework(lw) => lw.paths.clone(),
                _ => panic!(),
            };
            assert_eq!(paths.len(), 2, "Alt+click segment deletion for {tool:?}");
            let first = paths.iter().find(|path| path.id == path_id).unwrap();
            let second = paths.iter().find(|path| path.id != path_id).unwrap();
            assert_eq!(first.points.iter().map(|point| point.id).collect::<Vec<_>>(), original_point_ids[..2]);
            assert_eq!(second.points.iter().map(|point| point.id).collect::<Vec<_>>(), original_point_ids[2..]);
        }
    }

    #[test]
    fn color_weight_pointed_and_straight_tools_apply_their_own_actions() {
        let mut app = app();
        let created = app.run("linework.create", json!({"points":[[20,40],[100,40],[180,40]],"width":4,"color":"#112233"})).unwrap();
        let layer = LayerId(created["layer"].as_u64().unwrap());
        let (path, mid) = {
            let photocraft_doc::LayerContent::Linework(lw) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
            (lw.paths[0].id, lw.paths[0].points[1].id)
        };
        app.session.tools.foreground = [0.0, 1.0, 0.0, 1.0];
        app.ui.tool = Tool::SaiColor;
        click(&mut app, 100.0, 40.0);
        let get_path = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Linework(lw) => lw.paths.iter().find(|p| p.id == path).unwrap().clone(),
            _ => panic!(),
        };
        assert_eq!(get_path(&app).color.to_rgb(), [0.0, 1.0, 0.0]);
        app.ui.tool = Tool::SaiWeight;
        app.ui.tool_options.sailine_width = 17.5;
        click(&mut app, 20.0, 40.0);
        assert!((get_path(&app).width - 17.5).abs() < 1e-6);
        assert!(get_path(&app).points.iter().all(|p| (p.width_factor - 1.0).abs() < 1e-6));
        app.ui.tool = Tool::SaiPointed;
        click(&mut app, 100.0, 40.0);
        assert!(get_path(&app).points.iter().find(|p| p.id == mid).unwrap().pointed);
        app.ui.tool = Tool::SaiStraight;
        click(&mut app, 30.0, 100.0);
        click(&mut app, 160.0, 120.0);
        commit(&mut app);
        let photocraft_doc::LayerContent::Linework(lw) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
        let straight = lw.paths.last().unwrap();
        assert!(straight.points.iter().all(|p| p.pointed));
    }

    #[test]
    fn combine_tool_joins_selected_endpoints_into_one_path() {
        let mut app = app();
        app.run("linework.create", json!({"points":[[20,40],[70,40]],"width":4})).unwrap();
        app.run("linework.create", json!({"points":[[85,40],[150,40]],"width":4})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.ui.tool = Tool::SaiCombine;
        click(&mut app, 70.0, 40.0);
        assert!(app.ui.sailine_combine_first.is_some());
        click(&mut app, 85.0, 40.0);
        let photocraft_doc::LayerContent::Linework(lw) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
        assert_eq!(lw.paths.len(), 1);
        assert_eq!(lw.paths[0].points.len(), 4);
    }

    #[test]
    fn settings_keep_width_but_never_restore_a_previous_sessions_draft() {
        let mut app = app();
        click(&mut app, 20.0, 30.0);
        app.ui.tool_options.sailine_width = 7.0;
        let saved = serde_json::to_value(&app.ui).unwrap();
        assert!(saved["sailine"].is_object(), "automation can inspect the draft");
        let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
        assert!(restored.sailine.is_none());
        assert_eq!(restored.tool_options.sailine_width, 7.0);
    }

    #[test]
    fn preview_preserves_srgb_tool_colors() {
        assert_eq!(screen_color([0.2, 0.4, 0.6, 1.0]), egui::Color32::from_rgb(51, 102, 153));
    }

    #[test]
    fn clicks_commit_one_engine_transaction_with_history() {
        let mut app = app();
        let before = app.session.active().unwrap().history.entries().len();
        click(&mut app, 20.0, 30.0);
        click(&mut app, 100.0, 140.0);
        click(&mut app, 170.0, 40.0);
        assert_eq!(app.ui.sailine.as_ref().unwrap().points.len(), 3);
        assert_eq!(app.session.active().unwrap().history.entries().len(), before);
        commit(&mut app);
        assert!(app.ui.sailine.is_none());
        assert_eq!(app.session.active().unwrap().history.entries().len(), before + 1);
        assert_eq!(app.session.journal.last().unwrap().0, "linework.create");
        let count = app.session.active().unwrap().doc.layers.len();
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), count - 1);
        assert!(app.session.redo());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), count);
    }

    #[test]
    fn shift_selects_multiple_points_and_ctrl_drag_moves_them_together() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0], [170.0, 40.0]], "width": 3.0})).unwrap();
        down_with_mods(&mut app, [20.0, 30.0], Modifiers::NONE);
        down_with_mods(&mut app, [100.0, 140.0], Modifiers::SHIFT);
        assert_eq!(app.ui.sailine_selections.len(), 2);
        assert!(begin_drag(&mut app, [20.0, 30.0]));
        assert!(drag_to(&mut app, [30.0, 50.0]));
        end_drag(&mut app);
        let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
        let positions: Vec<_> = linework.paths[0].points.iter().map(|point| point.position).collect();
        assert_eq!(positions[0], [30.0, 50.0]);
        assert_eq!(positions[1], [110.0, 160.0]);
    }

    #[test]
    fn throttled_point_drag_commits_exact_release_position() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0]], "width": 3.0})).unwrap();
        assert!(begin_drag(&mut app, [20.0, 30.0]));
        assert!(drag_to(&mut app, [30.0, 40.0]));
        assert!(drag_to(&mut app, [45.0, 55.0]));
        end_drag(&mut app);
        let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
        assert_eq!(linework.paths[0].points[0].position, [45.0, 55.0]);
        assert!(app.session.undo());
        let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
        assert_eq!(linework.paths[0].points[0].position, [20.0, 30.0]);
    }

    #[test]
    fn both_sailine_tools_share_ctrl_click_insert_and_ctrl_drag_point_movement() {
        for tool in [Tool::SaiCurve, Tool::SaiPressure] {
            let mut app = app();
            app.run("linework.create", json!({"points": [[20.0, 50.0], [180.0, 50.0]], "width": 4.0})).unwrap();
            app.ui.tool = tool;
            let mods = Modifiers::CTRL;
            tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 52.0, pressure: 1.0 }, mods);
            tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 52.0 }, mods);
            let inserted = {
                let linework = match &app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().content {
                    photocraft_doc::LayerContent::Linework(linework) => linework,
                    _ => panic!("linework expected"),
                };
                assert_eq!(linework.paths[0].points.len(), 3, "Ctrl+click inserts on both tools");
                linework.paths[0].points[1].position
            };
            tool_event(&mut app, ToolEvent::Down { x: inserted[0], y: inserted[1], pressure: 1.0 }, mods);
            tool_event(&mut app, ToolEvent::Move { x: inserted[0] + 12.0, y: inserted[1] + 9.0, pressure: 1.0 }, mods);
            tool_event(&mut app, ToolEvent::Up { x: inserted[0] + 12.0, y: inserted[1] + 9.0 }, mods);
            let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
            let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
            assert_eq!(linework.paths[0].points[1].position, [inserted[0] + 12.0, inserted[1] + 9.0], "Ctrl+drag moves the inserted point on both tools");
            tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 50.0 }, Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Down { x: 180.0, y: 50.0, pressure: 1.0 }, Modifiers::SHIFT);
            tool_event(&mut app, ToolEvent::Up { x: 180.0, y: 50.0 }, Modifiers::SHIFT);
            assert_eq!(app.ui.sailine_selections.len(), 2, "Shift-click selection should also match on both tools");
        }
    }

    #[test]
    fn ctrl_click_then_drag_inserts_and_immediately_moves_the_new_point() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 50.0], [180.0, 50.0]], "width": 4.0})).unwrap();
        app.ui.tool = Tool::SaiCurve;
        let mods = Modifiers::CTRL;

        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 50.0, pressure: 1.0 }, mods);
        tool_event(&mut app, ToolEvent::Move { x: 114.0, y: 61.0, pressure: 1.0 }, mods);

        let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
        assert_eq!(linework.paths[0].points.len(), 3, "the point is inserted as soon as the held drag starts");
        assert_eq!(linework.paths[0].points[1].position, [114.0, 61.0], "the new point follows the pointer before release");
        assert!(app.ui.sailine_point_drag.is_some(), "the shared gesture changes into a point drag");

        tool_event(&mut app, ToolEvent::Up { x: 114.0, y: 61.0 }, Modifiers::NONE);
        assert!(app.ui.sailine_point_drag.is_none());
    }

    #[test]
    fn clearing_sailine_selection_removes_selected_points_and_transient_gestures() {
        let mut app = app();
        let selection = Selection { document: app.session.active().unwrap().doc.id, layer: LayerId(1), path: 2, point: 3 };
        app.ui.sailine_selection = Some(selection);
        app.ui.sailine_selections = vec![selection];
        app.ui.sailine_selection_box = Some(SelectionBox { start: [0.0, 0.0], current: [5.0, 5.0], additive: false });
        app.ui.sailine_shared_gesture = Some(SharedGesture { start: [0.0, 0.0], target: SharedTarget::Delete { selection }, moved: false });

        clear_selection(&mut app);

        assert!(app.ui.sailine_selection.is_none());
        assert!(app.ui.sailine_selections.is_empty());
        assert!(app.ui.sailine_selection_box.is_none());
        assert!(app.ui.sailine_shared_gesture.is_none());
    }

    #[test]
    fn modified_secondary_click_deselects_only_sailine_selection_outside_transform_mode() {
        for mods in [Modifiers::CTRL, Modifiers::ALT, Modifiers::SHIFT] {
            assert!(modifier_right_click_deselects(Tool::SaiColor, mods, false, true));
        }
        assert!(!modifier_right_click_deselects(Tool::SaiColor, Modifiers::NONE, false, true));
        assert!(!modifier_right_click_deselects(Tool::SaiColor, Modifiers::CTRL, true, true));
        assert!(!modifier_right_click_deselects(Tool::SaiColor, Modifiers::CTRL, false, false));
        assert!(!modifier_right_click_deselects(Tool::Brush, Modifiers::CTRL, false, true));
    }

    #[test]
    fn sai_pressure_click_only_selects_but_horizontal_drag_changes_zero_to_two_hundred_percent() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [160.0, 130.0]], "width": 4.0})).unwrap();
        let layer_id = app.session.active().unwrap().active_layer.unwrap();
        let (path, point) = match &app.session.active().unwrap().doc.layer(layer_id).unwrap().content {
            photocraft_doc::LayerContent::Linework(linework) => (linework.paths[0].id, linework.paths[0].points[0].id),
            _ => panic!("linework expected"),
        };
        app.ui.tool = Tool::SaiPressure;
        let history = app.session.active().unwrap().history.entries().len();
        let journal = app.session.journal.len();
        assert!(begin_pressure_drag(&mut app, [20.0, 30.0]));
        end_pressure_drag(&mut app, [20.0, 30.0]);
        assert_eq!(app.session.active().unwrap().history.entries().len(), history);
        assert_eq!(app.session.journal.len(), journal, "a plain click must not set point pressure");

        assert!(begin_pressure_drag(&mut app, [20.0, 30.0]));
        assert!(pressure_drag_to(&mut app, [140.0, 30.0]));
        assert_eq!(app.ui.sailine_pressure_drag.as_ref().map(|drag| drag.current_factor), Some(2.0));
        end_pressure_drag(&mut app, [140.0, 30.0]);
        let linework = match &app.session.active().unwrap().doc.layer(layer_id).unwrap().content {
            photocraft_doc::LayerContent::Linework(linework) => linework,
            _ => panic!("linework expected"),
        };
        assert_eq!(
            linework
                .paths
                .iter()
                .find(|path_data| path_data.id == path)
                .and_then(|path_data| path_data.points.iter().find(|p| p.id == point))
                .map(|p| p.width_factor),
            Some(2.0)
        );

        assert!(begin_pressure_drag(&mut app, [20.0, 30.0]));
        assert!(pressure_drag_to(&mut app, [-220.0, 30.0]));
        assert_eq!(app.ui.sailine_pressure_drag.as_ref().map(|drag| drag.current_factor), Some(0.0));
        end_pressure_drag(&mut app, [-220.0, 30.0]);
        let linework = match &app.session.active().unwrap().doc.layer(layer_id).unwrap().content {
            photocraft_doc::LayerContent::Linework(linework) => linework,
            _ => panic!("linework expected"),
        };
        assert_eq!(
            linework
                .paths
                .iter()
                .find(|path_data| path_data.id == path)
                .and_then(|path_data| path_data.points.iter().find(|p| p.id == point))
                .map(|p| p.width_factor),
            Some(0.0)
        );
    }

    #[test]
    fn pressure_preview_throttles_cache_rebuilds_but_release_commits_the_latest_value() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [160.0, 130.0]], "width": 4.0})).unwrap();
        let layer_id = app.session.active().unwrap().active_layer.unwrap();
        let (path, point) = match &app.session.active().unwrap().doc.layer(layer_id).unwrap().content {
            photocraft_doc::LayerContent::Linework(linework) => (linework.paths[0].id, linework.paths[0].points[0].id),
            _ => panic!("linework expected"),
        };
        app.ui.tool = Tool::SaiPressure;
        assert!(begin_pressure_drag(&mut app, [20.0, 30.0]));
        let journal_before = app.session.journal.len();
        assert!(pressure_drag_to(&mut app, [80.0, 30.0]));
        let after_first_preview = app.session.journal.len();
        assert_eq!(after_first_preview, journal_before + 1, "first movement should show immediately");

        app.ui.sailine_pressure_drag.as_mut().unwrap().last_preview_at = Some(Instant::now());
        assert!(pressure_drag_to(&mut app, [90.0, 30.0]));
        assert_eq!(app.session.journal.len(), after_first_preview, "rapid pointer updates should skip expensive rerenders");
        let latest = app.ui.sailine_pressure_drag.as_ref().unwrap().current_factor;
        end_pressure_drag(&mut app, [90.0, 30.0]);
        assert_eq!(app.session.journal.len(), after_first_preview + 1, "release must commit the final value");
        let factor = match &app.session.active().unwrap().doc.layer(layer_id).unwrap().content {
            photocraft_doc::LayerContent::Linework(linework) => {
                linework.paths.iter().find(|p| p.id == path).unwrap().points.iter().find(|p| p.id == point).unwrap().width_factor
            }
            _ => panic!("linework expected"),
        };
        assert_eq!(factor, latest);
    }

    #[test]
    fn shift_selection_box_selects_points_inside_bounds() {
        for tool in [Tool::SaiCurve, Tool::SaiPressure] {
            let mut app = app();
            app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0], [170.0, 40.0]], "width": 3.0})).unwrap();
            app.ui.tool = tool;
            assert!(begin_selection_box(&mut app, [0.0, 0.0]));
            assert!(selection_box_to(&mut app, [120.0, 150.0]));
            assert!(end_selection_box(&mut app, [120.0, 150.0]));
            assert_eq!(app.ui.sailine_selections.len(), 2);
        }
    }

    #[test]
    fn shift_drag_selection_box_adds_points_to_existing_selection() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0], [170.0, 40.0]], "width": 3.0})).unwrap();
        app.ui.tool = Tool::SaiCurve;
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let expected_points: std::collections::HashSet<_> = match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Linework(linework) => linework.paths[0].points.iter().map(|point| point.id).collect(),
            _ => panic!("linework expected"),
        };
        down_with_mods(&mut app, [170.0, 40.0], Modifiers::NONE);
        assert_eq!(app.ui.sailine_selections.len(), 1);

        tool_down(&mut app, [0.0, 0.0], Modifiers::SHIFT);
        assert!(selection_box_to(&mut app, [120.0, 150.0]));
        assert!(end_selection_box(&mut app, [120.0, 150.0]));

        assert_eq!(app.ui.sailine_selections.len(), 3, "Shift+drag unions boxed points with the prior selection");
        let selected_points: std::collections::HashSet<_> = app.ui.sailine_selections.iter().map(|selection| selection.point).collect();
        assert_eq!(selected_points, expected_points);
    }

    #[test]
    fn pressure_box_selects_and_drag_changes_all_selected_points() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0], [170.0, 40.0]], "width": 3.0})).unwrap();
        app.ui.tool = Tool::SaiPressure;
        tool_event(&mut app, ToolEvent::Down { x: 0.0, y: 0.0, pressure: 1.0 }, Modifiers::SHIFT);
        tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 150.0, pressure: 1.0 }, Modifiers::SHIFT);
        tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 150.0 }, Modifiers::SHIFT);
        assert_eq!(app.ui.sailine_selections.len(), 2);
        let history = app.session.active().unwrap().history.entries().len();
        assert!(begin_pressure_drag(&mut app, [20.0, 30.0]));
        assert_eq!(app.ui.sailine_selections.len(), 2);
        assert!(pressure_drag_to(&mut app, [80.0, 30.0]));
        end_pressure_drag(&mut app, [80.0, 30.0]);
        let factors = |app: &PhotocraftApp| {
            let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
            let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
            linework.paths[0].points.iter().map(|p| p.width_factor).collect::<Vec<_>>()
        };
        assert_eq!(factors(&app), vec![1.5, 1.5, 1.0]);
        assert_eq!(app.session.active().unwrap().history.entries().len(), history + 1);
        assert!(app.session.undo());
        assert_eq!(factors(&app), vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn new_draft_clears_previous_point_selection() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0]], "width": 3.0})).unwrap();
        down_with_mods(&mut app, [20.0, 30.0], Modifiers::NONE);
        assert_eq!(app.ui.sailine_selections.len(), 1);
        down_with_mods(&mut app, [180.0, 180.0], Modifiers::NONE);
        assert!(app.ui.sailine.is_some());
        assert!(app.ui.sailine_selections.is_empty());
        assert!(app.ui.sailine_selection.is_none());
    }

    #[test]
    fn delete_key_removes_multi_selection_in_one_undo_step_on_both_tools() {
        for tool in [Tool::SaiCurve, Tool::SaiPressure] {
            let mut app = app();
            app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0], [170.0, 40.0]], "width": 3.0})).unwrap();
            app.ui.tool = tool;
            down_with_mods(&mut app, [20.0, 30.0], Modifiers::NONE);
            down_with_mods(&mut app, [100.0, 140.0], Modifiers::SHIFT);
            assert_eq!(app.ui.sailine_selections.len(), 2);
            let history = app.session.active().unwrap().history.entries().len();
            let ctx = egui::Context::default();
            ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key { key: Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                    ..Default::default()
                },
                |ui| assert!(keys(&mut app, ui.ctx())),
            )
            .textures_delta
            .clear();
            let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap();
            let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
            assert!(linework.paths.is_empty());
            assert!(app.ui.sailine_selections.is_empty());
            assert_eq!(app.session.active().unwrap().history.entries().len(), history + 1);
            assert!(app.session.undo());
        }
    }

    #[test]
    fn draft_is_discarded_on_tool_or_document_switch() {
        let mut app = app();
        click(&mut app, 20.0, 30.0);
        app.ui.tool = Tool::Brush;
        sync(&mut app);
        assert!(app.ui.sailine.is_none());
        app.ui.tool = Tool::SaiCurve;
        click(&mut app, 20.0, 30.0);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        commit(&mut app);
        assert!(app.ui.sailine.is_none());
        assert_ne!(app.session.journal.last().unwrap().0, "linework.create");
    }

    #[test]
    fn enter_uses_current_color_and_width_and_needs_two_points() {
        let mut app = app();
        click(&mut app, 20.0, 30.0);
        commit(&mut app);
        assert!(app.ui.sailine.is_some());
        click(&mut app, 100.0, 140.0);
        app.ui.tool_options.sailine_width = 7.0;
        app.session.tools.foreground = [0.25, 0.5, 0.75, 0.5];
        let ctx = egui::Context::default();
        ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                ..Default::default()
            },
            |ui| crate::shortcuts::handle(&mut app, ui.ctx()),
        )
        .textures_delta
        .clear();
        assert!(app.ui.sailine.is_none());
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { panic!("linework expected") };
        assert_eq!(linework.paths[0].width, 7.0);
        assert_eq!(linework.paths[0].points.iter().map(|p| p.position).collect::<Vec<_>>(), vec![[20.0, 30.0], [100.0, 140.0]]);
        assert_eq!(linework.paths[0].color.alpha, 0.5);
    }

    #[test]
    fn invalid_or_excess_points_do_not_grow_the_draft() {
        let mut app = app();
        down(&mut app, [f64::NAN, 1.0]);
        assert!(app.ui.sailine.is_none());
        for i in 0..4100 {
            down(&mut app, [f64::from(i), 10.0]);
        }
        assert_eq!(app.ui.sailine.as_ref().unwrap().points.len(), 4096);
    }

    #[test]
    fn keyboard_cancel_and_remove_never_mutate_document() {
        let mut app = app();
        let before = app.session.active().unwrap().doc.clone();
        click(&mut app, 20.0, 30.0);
        click(&mut app, 100.0, 140.0);
        let ctx = egui::Context::default();
        for key in [Key::Backspace, Key::Escape] {
            ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
                    ..Default::default()
                },
                |ui| {
                    assert!(keys(&mut app, ui.ctx()));
                },
            )
            .textures_delta
            .clear();
        }
        assert!(app.ui.sailine.is_none());
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
}
