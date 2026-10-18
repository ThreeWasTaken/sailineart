//! SaiLine click-to-create tool. Drafts never mutate the document; commit uses the engine.
use egui::{Key, Modifiers, Stroke};
use photocraft_doc::{DocId, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{PhotocraftApp, canvas::ViewXform, state::Tool, theme::Tokens};

static NEXT_DRAG: AtomicU64 = AtomicU64::new(1);

fn interpolation_id(app: &PhotocraftApp) -> &'static str {
    match app.ui.tool_options.sailine_interpolation.as_str() {
        "saiSimplified" => "saiSimplified",
        "saiOptimized" => "saiOptimized",
        _ => "legacy",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub document: DocId,
    pub points: Vec<[f64; 2]>,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointWidthDrag {
    pub selection: Selection,
    pub start_x: f64,
    pub start_factor: f32,
    pub current_factor: f32,
    pub moved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionBox {
    pub start: [f64; 2],
    pub current: [f64; 2],
}

/// Leaving the tool or its document discards the uncommitted draft.
pub fn sync(app: &mut PhotocraftApp) {
    if app.ui.sailine.as_ref().is_some_and(|draft| app.ui.tool != Tool::SaiLine || app.session.active().is_none_or(|s| s.doc.id != draft.document)) {
        app.ui.sailine = None;
    }
    if app.ui.sailine_selection.as_ref().is_some_and(|selection| {
        !matches!(app.ui.tool, Tool::SaiLine | Tool::SaiLineWidth) || app.session.active().is_none_or(|s| s.doc.id != selection.document)
    }) {
        app.ui.sailine_selection = None;
        app.ui.sailine_selections.clear();
        app.ui.sailine_point_drag = None;
        app.ui.sailine_width_drag = None;
        app.ui.sailine_selection_box = None;
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
            let model = interpolation_id(app);
            let _ = app.run("linework.deletePoint", json!({"layer": layer.0, "path": path, "point": point_id, "interpolation": model}));
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
    let draft = app.ui.sailine.get_or_insert_with(|| Draft { document, points: Vec::new(), redo: Vec::new() });
    if draft.points.len() < 4096 && draft.points.last() != Some(&point) {
        draft.points.push(point);
        draft.redo.clear();
    }
}

pub fn begin_drag(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    if app.ui.tool != Tool::SaiLine || app.ui.sailine.is_some() {
        return false;
    }
    let Some(document) = app.session.active().map(|s| s.doc.id) else { return false };
    let (layer, path, point_id) = if let Some(found) = nearest_point(app, point) {
        found
    } else {
        let Some((layer, path, index, position)) = nearest_segment(app, point) else { return false };
        let model = interpolation_id(app);
        let Ok(result) = app
            .run("linework.insertPoint", json!({"layer": layer.0, "path": path, "index": index, "x": position[0], "y": position[1], "interpolation": model}))
        else {
            return false;
        };
        let Some(point_id) = result["point"].as_u64() else { return false };
        (layer, path, point_id)
    };
    let selection = Selection { document, layer, path, point: point_id };
    if !app.ui.sailine_selections.contains(&selection) {
        app.ui.sailine_selections = vec![selection];
    }
    app.ui.sailine_selection = Some(selection);
    let origins = app.ui.sailine_selections.iter().filter_map(|candidate| point_position(app, *candidate)).collect();
    app.ui.sailine_point_drag =
        Some(PointDrag { selections: app.ui.sailine_selections.clone(), origins, start: point, key: NEXT_DRAG.fetch_add(1, Ordering::Relaxed) });
    true
}

pub fn drag_to(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let Some(drag) = app.ui.sailine_point_drag.clone() else { return false };
    let dx = point[0] - drag.start[0];
    let dy = point[1] - drag.start[1];
    let points: Vec<_> = drag
        .selections
        .iter()
        .zip(drag.origins.iter())
        .map(|(selection, origin)| json!({"layer": selection.layer.0, "path": selection.path, "point": selection.point, "x": origin[0] + dx, "y": origin[1] + dy}))
        .collect();
    let model = interpolation_id(app);
    let _ = app.run("linework.movePoints", json!({"points": points, "coalesce": format!("sailine-move:{}", drag.key), "interpolation": model}));
    true
}

pub fn end_drag(app: &mut PhotocraftApp) {
    app.ui.sailine_point_drag = None;
}

pub fn begin_selection_box(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    if app.ui.tool != Tool::SaiLine || app.ui.sailine.is_some() || nearest_point(app, point).is_some() {
        return false;
    }
    let Some(state) = app.session.active() else { return false };
    let Some(layer) = state.active_layer.and_then(|id| state.doc.layer(id)) else { return false };
    if !matches!(layer.content, photocraft_doc::LayerContent::Linework(_)) {
        return false;
    }
    app.ui.sailine_selection_box = Some(SelectionBox { start: point, current: point });
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
    app.ui.sailine_selections = points.collect();
    app.ui.sailine_selection = app.ui.sailine_selections.first().copied();
    true
}

fn nearest_point(app: &PhotocraftApp, point: [f64; 2]) -> Option<(LayerId, u64, u64)> {
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

fn path_width(app: &PhotocraftApp, selection: Selection) -> Option<f32> {
    let state = app.session.active()?;
    let layer = state.doc.layer(selection.layer)?;
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return None };
    linework.paths.iter().find(|path| path.id == selection.path).map(|path| path.width)
}

pub fn begin_width_drag(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    if app.ui.tool != Tool::SaiLineWidth {
        return false;
    }
    let Some(document) = app.session.active().map(|s| s.doc.id) else { return false };
    let Some((layer, path, point_id)) = nearest_point(app, point) else { return false };
    let selection = Selection { document, layer, path, point: point_id };
    let factor = point_width(app, selection).unwrap_or(1.0);
    app.ui.sailine_selection = Some(selection);
    app.ui.sailine_selections = vec![selection];
    app.ui.sailine_width_drag = Some(PointWidthDrag { selection, start_x: point[0], start_factor: factor, current_factor: factor, moved: false });
    true
}

pub fn width_drag_to(app: &mut PhotocraftApp, point: [f64; 2]) -> bool {
    let (selection, factor) = {
        let Some(drag) = app.ui.sailine_width_drag.as_mut() else { return false };
        let dx = point[0] - drag.start_x;
        drag.moved |= dx.abs() > 0.01;
        drag.current_factor = (drag.start_factor + (dx / 120.0) as f32).clamp(0.0, 1000.0);
        (drag.selection, drag.current_factor)
    };
    let model = interpolation_id(app);
    let _ = app.run(
        "linework.setPointWidth",
        json!({"layer": selection.layer.0, "path": selection.path, "point": selection.point, "factor": factor, "coalesce": "sailine-width-drag", "interpolation": model}),
    );
    true
}

pub fn end_width_drag(app: &mut PhotocraftApp, point: [f64; 2]) {
    if app.ui.sailine_width_drag.is_none() {
        return;
    }
    width_drag_to(app, point);
    let Some(drag) = app.ui.sailine_width_drag.take() else { return };
    let s = drag.selection;
    let factor = if drag.moved {
        drag.current_factor
    } else {
        let width = app.ui.tool_options.sailine_width;
        let base = path_width(app, s).unwrap_or(width).max(0.001);
        (width / base).clamp(0.0, 1000.0)
    };
    let model = interpolation_id(app);
    let _ = app.run(
        "linework.setPointWidth",
        json!({"layer": s.layer.0, "path": s.path, "point": s.point, "factor": factor, "coalesce": "sailine-width-drag", "interpolation": model}),
    );
    if drag.moved
        && let Some(base) = path_width(app, s)
    {
        app.ui.tool_options.sailine_width = (factor * base).clamp(0.1, 1000.0);
    }
}

fn nearest_segment(app: &PhotocraftApp, point: [f64; 2]) -> Option<(LayerId, u64, usize, [f64; 2])> {
    let state = app.session.active()?;
    let layer_id = state.active_layer?;
    let layer = state.doc.layer(layer_id)?;
    let photocraft_doc::LayerContent::Linework(linework) = &layer.content else { return None };
    let tolerance = 10.0 / f64::from(app.point_zoom().max(0.01));
    linework
        .paths
        .iter()
        .flat_map(|path| path.points.windows(2).enumerate().map(move |(index, points)| (path.id, index, points[0].position, points[1].position)))
        .filter_map(|(path, index, a, b)| {
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / len2 } else { 0.0 };
            let t = t.clamp(0.0, 1.0);
            let closest = [a[0] + t * dx, a[1] + t * dy];
            let distance = (closest[0] - point[0]).hypot(closest[1] - point[1]);
            (distance <= tolerance).then_some((distance, path, index + 1, closest))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, path, index, position)| (layer_id, path, index, position))
}

pub fn commit(app: &mut PhotocraftApp) {
    sync(app);
    let Some(draft) = app.ui.sailine.as_ref().filter(|d| d.points.len() >= 2) else { return };
    let model = interpolation_id(app);
    let params = json!({"points": draft.points, "width": app.ui.tool_options.sailine_width, "color": app.session.tools.foreground, "interpolation": model});
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
    if app.ui.tool != Tool::SaiLine || app.ui.sailine.is_none() {
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
    ui.label(if app.ui.tool == Tool::SaiLineWidth { "SaiLine Width" } else { "SaiLine" });
    let previous_model = interpolation_id(app).to_string();
    egui::ComboBox::from_id_salt("sailine-interpolation")
        .selected_text(match previous_model.as_str() {
            "saiSimplified" => "SAI simplified",
            "saiOptimized" => "SAI optimized",
            _ => "Legacy",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.ui.tool_options.sailine_interpolation, "legacy".into(), "Legacy");
            ui.selectable_value(&mut app.ui.tool_options.sailine_interpolation, "saiSimplified".into(), "SAI simplified");
            ui.selectable_value(&mut app.ui.tool_options.sailine_interpolation, "saiOptimized".into(), "SAI optimized");
        });
    let selected_model = interpolation_id(app);
    if selected_model != previous_model {
        if let Err(error) = app.run("linework.setInterpolation", json!({"interpolation": selected_model})) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    ui.label(format!(
        "Active model: {}",
        match selected_model {
            "saiSimplified" => "SAI simplified",
            "saiOptimized" => "SAI optimized",
            _ => "Legacy",
        }
    ));
    ui.checkbox(&mut app.ui.tool_options.sailine_compare_legacy, "Compare Legacy centerline");
    ui.label(tl!("Width:"));
    ui.add(egui::DragValue::new(&mut app.ui.tool_options.sailine_width).range(0.1..=1000.0).suffix(" px"));
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
        let mut factor = app.ui.sailine_width_drag.as_ref().map_or(factor, |drag| drag.current_factor);
        ui.label("Point width:");
        if ui.add(egui::DragValue::new(&mut factor).range(0.0..=1000.0).speed(0.05)).changed() {
            let model = selected_model;
            let _ = app.run(
                "linework.setPointWidth",
                json!({"layer": selection.layer.0, "path": selection.path, "point": selection.point, "factor": factor, "interpolation": model}),
            );
        }
    }
    if let Some(selection) = app.ui.sailine_selection {
        let foreground = screen_color(app.session.tools.foreground);
        if ui.add(egui::Button::new("Path color ← foreground").fill(foreground)).clicked() {
            let color = app.session.tools.foreground;
            let _ =
                app.run("linework.setPathColor", json!({"layer": selection.layer.0, "path": selection.path, "color": color, "interpolation": selected_model}));
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
    let selected_model = interpolation_id(app);
    if let Some(draft) = app.ui.sailine.as_ref().filter(|d| app.ui.tool == Tool::SaiLine && app.session.active().is_some_and(|s| s.doc.id == d.document)) {
        let mut points = draft.points.clone();
        if let Some(hover) = app.hover_doc
            && points.len() < 4096
            && points.last() != Some(&hover)
        {
            points.push(hover);
        }
        let handle = app.hover_doc.filter(|_| draft.points.len() >= 2);
        let model = photocraft_vector::linework::InterpolationModel::from_id(interpolation_id(app)).unwrap_or_default();
        if let Ok(path) = photocraft_vector::linework::interpolate_points_with_model(&points, handle, model) {
            let color = screen_color(app.session.tools.foreground);
            let width = (app.ui.tool_options.sailine_width * xf.zoom).max(0.5);
            for (curve, _) in crate::vector_ui::path_lines(&path, &to_screen) {
                painter.add(egui::Shape::line(curve, Stroke::new(width, color)));
            }
        }
        let accent = Tokens::get(painter.ctx()).accent;
        for point in &draft.points {
            draw_round_anchor(painter, to_screen(*point), true, accent);
        }
    }
    if let Some(layer) = app.session.active().and_then(|s| s.active_layer).and_then(|id| app.session.active().and_then(|s| s.doc.layer(id)))
        && let photocraft_doc::LayerContent::Linework(linework) = &layer.content
    {
        let selected: std::collections::HashSet<u64> =
            app.ui.sailine_selections.iter().map(|selection| selection.point).chain(app.ui.sailine_selection.iter().map(|selection| selection.point)).collect();
        let hovered = app.hover_doc.and_then(|point| nearest_point(app, point)).map(|(_, _, point)| point);
        let accent = Tokens::get(painter.ctx()).accent;
        for candidate in linework.paths.iter().flat_map(|path| path.points.iter()) {
            draw_round_anchor(painter, to_screen(candidate.position), selected.contains(&candidate.id) || Some(candidate.id) == hovered, accent);
        }
        if app.ui.tool_options.sailine_compare_legacy && selected_model != "legacy" {
            for path in &linework.paths {
                let points: Vec<_> = path.points.iter().map(|point| point.position).collect();
                if let Ok(legacy) = photocraft_vector::linework::interpolate_points_with_handle(&points, path.handle) {
                    let compare = egui::Color32::from_rgb(0, 210, 255);
                    for (curve, _) in crate::vector_ui::path_lines(&legacy, &to_screen) {
                        painter.add(egui::Shape::line(curve, Stroke::new((path.width * xf.zoom as f32).max(1.0), compare)));
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

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.ui.tool = Tool::SaiLine;
        app
    }

    fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
        tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
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
    fn shift_selection_box_selects_points_inside_bounds() {
        let mut app = app();
        app.run("linework.create", json!({"points": [[20.0, 30.0], [100.0, 140.0], [170.0, 40.0]], "width": 3.0})).unwrap();
        assert!(begin_selection_box(&mut app, [0.0, 0.0]));
        assert!(selection_box_to(&mut app, [120.0, 150.0]));
        assert!(end_selection_box(&mut app, [120.0, 150.0]));
        assert_eq!(app.ui.sailine_selections.len(), 2);
    }

    #[test]
    fn draft_is_discarded_on_tool_or_document_switch() {
        let mut app = app();
        click(&mut app, 20.0, 30.0);
        app.ui.tool = Tool::Brush;
        sync(&mut app);
        assert!(app.ui.sailine.is_none());
        app.ui.tool = Tool::SaiLine;
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
